use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

const MAX_RECORD_BYTES: u64 = 1024 * 1024;
const RECORD_SCHEMA: &str = "code-intel-decision-record.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecisionRecordError(String);

impl fmt::Display for DecisionRecordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DecisionRecordError {}

#[derive(Debug, Clone)]
pub(crate) struct DecisionRecordStore {
    root: PathBuf,
}

impl DecisionRecordStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn record(&self, resolution: &Value) -> Result<Value, DecisionRecordError> {
        let _lock = self.lock_store()?;
        let loaded = self.load_locked()?;
        let candidate = build_record(resolution)?;
        let candidate_id = candidate["id"].as_str().unwrap();
        if let Some(existing) = loaded
            .records
            .iter()
            .find(|record| record["id"] == candidate_id)
        {
            return Ok(outcome(
                "replay",
                false,
                Some(existing.clone()),
                None,
                loaded.diagnostics,
            ));
        }

        let authority_id = candidate["authorityEvent"]["id"].as_str().unwrap();
        if loaded
            .records
            .iter()
            .any(|record| record["authorityEvent"]["id"] == authority_id)
        {
            return Err(DecisionRecordError(
                "authority event replay is rejected".to_string(),
            ));
        }
        if loaded.records.iter().any(|record| {
            record["gap"]["id"] == candidate["gap"]["id"]
                && record["snapshotIdentity"] == candidate["snapshotIdentity"]
                && record["evidenceBinding"]["digest"] == candidate["evidenceBinding"]["digest"]
        }) {
            return Err(DecisionRecordError(
                "decision gap is already recorded for unchanged evidence".to_string(),
            ));
        }

        self.publish(&candidate)?;
        Ok(outcome(
            "recorded",
            false,
            Some(candidate),
            None,
            loaded.diagnostics,
        ))
    }

    pub(crate) fn replay(&self, query: &Value) -> Result<Value, DecisionRecordError> {
        validate_query(query)?;
        let _lock = self.lock_store()?;
        let loaded = self.load_locked()?;
        let gap_id = query["gapId"].as_str().unwrap();
        let Some(record) = loaded
            .records
            .iter()
            .filter(|record| record["gap"]["id"] == gap_id)
            .max_by_key(|record| record["recordedAt"].as_u64().unwrap_or_default())
        else {
            return Ok(outcome(
                "reopen",
                true,
                None,
                Some("no_record"),
                loaded.diagnostics,
            ));
        };

        if string_set(&query["affectedBranches"], "query affectedBranches")?
            != string_set(&record["affectedBranches"], "record affectedBranches")?
        {
            return Err(DecisionRecordError(
                "decision replay branch scope mismatch".to_string(),
            ));
        }
        let query_evidence = validate_evidence_refs(&query["evidenceRefs"])?;
        let now = unsigned(&query["now"], "query now")?;
        if now < record["recordedAt"].as_u64().unwrap()
            || query_evidence
                .iter()
                .any(|evidence| now < evidence["observedAt"].as_u64().unwrap())
        {
            return Err(DecisionRecordError(
                "decision replay time precedes the record or observed evidence".to_string(),
            ));
        }
        if query["snapshotIdentity"] != record["snapshotIdentity"] {
            return Ok(outcome(
                "reopen",
                true,
                Some(record.clone()),
                Some("snapshot_changed"),
                loaded.diagnostics,
            ));
        }
        let query_digest = digest_value(&Value::Array(query_evidence));
        if record["evidenceBinding"]["digest"] != query_digest {
            return Ok(outcome(
                "reopen",
                true,
                Some(record.clone()),
                Some("evidence_changed"),
                loaded.diagnostics,
            ));
        }
        if now > record["freshness"]["evidenceExpiresAt"].as_u64().unwrap() {
            return Ok(outcome(
                "reopen",
                true,
                Some(record.clone()),
                Some("evidence_stale"),
                loaded.diagnostics,
            ));
        }
        Ok(outcome(
            "replay",
            false,
            Some(record.clone()),
            None,
            loaded.diagnostics,
        ))
    }

    fn ensure_root(&self) -> Result<(), DecisionRecordError> {
        let existed = self.root.is_dir();
        fs::create_dir_all(&self.root).map_err(|error| {
            DecisionRecordError(format!("create decision record store: {error}"))
        })?;
        if !existed {
            if let Some(parent) = self.root.parent() {
                crate::staged_artifact::sync_directory_path(parent).map_err(|error| {
                    DecisionRecordError(format!("sync decision store parent: {error}"))
                })?;
            }
        }
        crate::staged_artifact::sync_directory_path(&self.root)
            .map_err(|error| DecisionRecordError(format!("sync decision store: {error}")))
    }

    fn lock_store(&self) -> Result<File, DecisionRecordError> {
        self.ensure_root()?;
        let lock_path = self.root.join(".decision-record.lock");
        let existed = lock_path.exists();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)
            .map_err(|error| DecisionRecordError(format!("open decision store lock: {error}")))?;
        if !existed {
            file.sync_all().map_err(|error| {
                DecisionRecordError(format!("sync decision store lock: {error}"))
            })?;
            crate::staged_artifact::sync_directory_path(&self.root).map_err(|error| {
                DecisionRecordError(format!("sync decision store lock directory: {error}"))
            })?;
        }
        file.lock()
            .map_err(|error| DecisionRecordError(format!("lock decision store: {error}")))?;
        Ok(file)
    }

    fn load_locked(&self) -> Result<Loaded, DecisionRecordError> {
        let mut records = Vec::new();
        let mut diagnostics = Vec::new();
        let entries = fs::read_dir(&self.root)
            .map_err(|error| DecisionRecordError(format!("read decision record store: {error}")))?;
        for entry_result in entries {
            let entry = match entry_result {
                Ok(entry) => entry,
                Err(error) => {
                    diagnostics.push(format!("decision store read_dir entry failed: {error}"));
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".decision-record.lock" {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    diagnostics.push(format!(
                        "decision store entry {name} file_type failed: {error}"
                    ));
                    continue;
                }
            };
            if file_type.is_symlink() {
                diagnostics.push(format!("ignored symlink decision store entry {name}"));
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                if name == ".staging" {
                    match fs::read_dir(&path) {
                        Ok(mut staged) => match staged.next() {
                            None => {}
                            Some(Ok(_)) => diagnostics
                                .push("ignored uncommitted decision staging directory".to_string()),
                            Some(Err(error)) => diagnostics
                                .push(format!("decision staging read_dir entry failed: {error}")),
                        },
                        Err(error) => {
                            diagnostics.push(format!("decision staging read_dir failed: {error}"))
                        }
                    }
                    continue;
                }
                match load_committed_record(&path) {
                    Ok(record) => records.push(record),
                    Err(error) => diagnostics.push(format!(
                        "ignored invalid or unreadable committed decision run {name}: {error}"
                    )),
                }
            } else if file_type.is_file() {
                let diagnosis = read_record(&path).and_then(|record| validate_record(&record));
                diagnostics.push(match diagnosis {
                    Ok(()) => format!("ignored uncommitted legacy decision record file {name}"),
                    Err(error) => {
                        format!("ignored invalid or unreadable decision store file {name}: {error}")
                    }
                });
            } else {
                diagnostics.push(format!("ignored unsupported decision store entry {name}"));
            }
        }
        records.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
        Ok(Loaded {
            records,
            diagnostics,
        })
    }

    fn publish(&self, record: &Value) -> Result<(), DecisionRecordError> {
        let digest = record["bindingDigest"].as_str().unwrap();
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|error| DecisionRecordError(format!("serialize decision record: {error}")))?;
        validate_decision_record_artifact(&bytes).map_err(DecisionRecordError)?;
        let snapshot = record["snapshotIdentity"].as_str().unwrap();
        let mut writer = crate::staged_artifact::StagedWriter::begin(&self.root, snapshot)
            .map_err(|error| DecisionRecordError(format!("begin decision staging: {error}")))?;
        let record_ref = writer
            .stage(
                &bytes,
                crate::staged_artifact::ArtifactWriteContract {
                    artifact_schema: RECORD_SCHEMA,
                    artifact_type: "decision.record",
                    max_bytes: MAX_RECORD_BYTES,
                    validate_payload: validate_decision_record_artifact,
                },
            )
            .map_err(|error| DecisionRecordError(format!("stage decision record: {error}")))?
            .to_artifact_ref_value();
        let manifest = json!({
            "schema":"code-intel-run-manifest.v1",
            "runIdentity":format!("dag-v1:{digest}"),
            "snapshotIdentity":snapshot,
            "outcome":"completed",
            "nodes":{"decision_record":{"status":"succeeded","verdict":"pass","artifacts":[record_ref]}}
        });
        let manifest_bytes = serde_json::to_vec(&manifest).map_err(|error| {
            DecisionRecordError(format!("serialize decision manifest: {error}"))
        })?;
        let manifest_ref = writer
            .stage(
                &manifest_bytes,
                crate::staged_artifact::ArtifactWriteContract {
                    artifact_schema: "code-intel-run-manifest.v1",
                    artifact_type: "run.manifest",
                    max_bytes: 8 * 1024 * 1024,
                    validate_payload: crate::run_commit::validate_run_manifest_bytes,
                },
            )
            .map_err(|error| DecisionRecordError(format!("stage decision manifest: {error}")))?
            .to_artifact_ref_value();
        let staged = writer
            .seal()
            .map_err(|error| DecisionRecordError(format!("seal decision staging: {error}")))?;
        let committed = crate::run_commit::commit(
            staged,
            &manifest_ref,
            &format!("decision-{digest}"),
            crate::run_commit::CommitOptions::default(),
        )
        .map_err(|error| DecisionRecordError(format!("commit decision run: {error}")))?;
        let loaded = load_committed_record(&committed.final_path)?;
        if loaded != *record {
            return Err(DecisionRecordError(
                "committed decision record verification mismatch".to_string(),
            ));
        }
        Ok(())
    }
}

struct Loaded {
    records: Vec<Value>,
    diagnostics: Vec<String>,
}

pub(crate) fn validate_decision_record_artifact(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("decision record exceeds size limit".to_string());
    }
    crate::artifact_ref::validate_decision_record_schema(bytes)?;
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("decision record artifact is invalid JSON: {error}"))?;
    validate_record(&value).map_err(|error| error.to_string())
}

fn load_committed_record(path: &Path) -> Result<Value, DecisionRecordError> {
    let (_, manifest) = crate::run_commit::validate_committed_run(path)
        .map_err(|error| DecisionRecordError(format!("validate committed run: {error}")))?;
    let mut refs = Vec::new();
    for node in manifest["nodes"].as_object().unwrap().values() {
        if let Some(artifacts) = node["artifacts"].as_array() {
            refs.extend(artifacts.iter().filter(|artifact| {
                artifact["artifactSchema"] == RECORD_SCHEMA && artifact["type"] == "decision.record"
            }));
        }
    }
    if refs.len() != 1 {
        return Err(DecisionRecordError(
            "committed decision run must contain exactly one decision record artifact".to_string(),
        ));
    }
    let artifact = refs[0];
    let relative = artifact["path"].as_str().ok_or_else(|| {
        DecisionRecordError("decision record artifact path is invalid".to_string())
    })?;
    let components = relative.split('/').collect::<Vec<_>>();
    let stable = crate::stable_artifact::read_beneath(path, &components, MAX_RECORD_BYTES)
        .map_err(|error| {
            DecisionRecordError(format!("read committed decision record: {error:?}"))
        })?;
    if artifact["sha256"] != crate::capability::sha256_hex(&stable.bytes) {
        return Err(DecisionRecordError(
            "committed decision record digest mismatch".to_string(),
        ));
    }
    validate_decision_record_artifact(&stable.bytes).map_err(DecisionRecordError)?;
    serde_json::from_slice(&stable.bytes)
        .map_err(|error| DecisionRecordError(format!("parse committed decision record: {error}")))
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match run_cli(raw) {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            0
        }
        Err(error) => {
            eprintln!("decision record: {error}");
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "schema":"code-intel-decision-record-operation-result.v1",
                    "status":"rejected","questionRequired":true,"record":null,
                    "reason":"contract_rejected","diagnostics":[error.to_string()]
                }))
                .unwrap()
            );
            65
        }
    }
}

fn run_cli(raw: &[String]) -> Result<Value, DecisionRecordError> {
    let operation = raw
        .first()
        .map(String::as_str)
        .ok_or_else(|| DecisionRecordError("expected record or replay subcommand".to_string()))?;
    let mut input = None;
    let mut store = None;
    let mut index = 1;
    while index < raw.len() {
        let flag = raw[index].as_str();
        let value = raw
            .get(index + 1)
            .ok_or_else(|| DecisionRecordError(format!("{flag} requires a value")))?;
        match flag {
            "--resolution" if operation == "record" && input.is_none() => input = Some(value),
            "--query" if operation == "replay" && input.is_none() => input = Some(value),
            "--store" if store.is_none() => store = Some(value),
            _ => {
                return Err(DecisionRecordError(format!(
                    "unknown or duplicate option {flag}"
                )))
            }
        }
        index += 2;
    }
    let input = read_json(Path::new(input.ok_or_else(|| {
        DecisionRecordError(format!(
            "--{} is required",
            if operation == "record" {
                "resolution"
            } else {
                "query"
            }
        ))
    })?))?;
    let store = DecisionRecordStore::new(PathBuf::from(
        store.ok_or_else(|| DecisionRecordError("--store is required".to_string()))?,
    ));
    match operation {
        "record" => store.record(&input),
        "replay" => store.replay(&input),
        _ => Err(DecisionRecordError(
            "expected record or replay subcommand".to_string(),
        )),
    }
}

fn build_record(resolution: &Value) -> Result<Value, DecisionRecordError> {
    exact(
        resolution,
        &[
            "schema",
            "gap",
            "request",
            "response",
            "authorityEvent",
            "snapshotIdentity",
            "recordedAt",
        ],
        "decision record request",
    )?;
    if resolution["schema"] != "code-intel-decision-record-request.v1" {
        return Err(DecisionRecordError(
            "decision record request schema is invalid".to_string(),
        ));
    }
    let snapshot = digest(&resolution["snapshotIdentity"], "snapshotIdentity")?;
    let recorded_at = unsigned(&resolution["recordedAt"], "recordedAt")?;
    validate_gap(&resolution["gap"])?;
    validate_request(&resolution["request"])?;
    validate_response(&resolution["response"])?;
    bind_exchange(resolution, recorded_at)?;

    let evidence = validate_evidence_refs(&resolution["request"]["evidenceRefs"])?;
    let evidence_ids = evidence
        .iter()
        .map(|value| value["refId"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    let empty = BTreeSet::new();
    crate::authority::validate_authority_event(
        &resolution["authorityEvent"],
        recorded_at,
        &evidence_ids,
        &evidence_ids,
        &empty,
    )
    .map_err(DecisionRecordError)?;
    let actor = &resolution["response"]["actorProvenance"];
    let approver = &resolution["authorityEvent"]["approver"];
    if actor["actorId"] != approver["id"]
        || actor["authorityKind"] != approver["role"]
        || actor["authorityKind"] != resolution["request"]["authorityNeeded"]["kind"]
    {
        return Err(DecisionRecordError(
            "authority event approver does not bind the accepted response".to_string(),
        ));
    }

    let accepted = resolution["response"]["answer"].clone();
    let consequences = match accepted["kind"].as_str() {
        Some("choice") => {
            let option_id = accepted["optionId"].as_str().unwrap();
            vec![resolution["request"]["options"]
                .as_array()
                .unwrap()
                .iter()
                .find(|option| option["id"] == option_id)
                .unwrap()["consequence"]
                .clone()]
        }
        Some("free-form") => Vec::new(),
        _ => unreachable!(),
    };
    let evidence_expires_at = evidence
        .iter()
        .filter_map(|value| value["expiresAt"].as_u64())
        .min()
        .unwrap();
    if recorded_at > evidence_expires_at {
        return Err(DecisionRecordError(
            "decision record cannot bind expired evidence".to_string(),
        ));
    }
    let evidence_digest = digest_value(&Value::Array(evidence.clone()));
    let binding = json!({
        "gap":resolution["gap"],"request":resolution["request"],"response":resolution["response"],
        "authorityEvent":resolution["authorityEvent"],"snapshotIdentity":snapshot,"recordedAt":recorded_at
    });
    let binding_digest = digest_value(&binding);
    Ok(json!({
        "schema":RECORD_SCHEMA,"id":format!("decision-record-v1:{binding_digest}"),"bindingDigest":binding_digest,
        "gap":resolution["gap"],"request":resolution["request"],"response":resolution["response"],
        "evidenceBinding":{"refs":evidence,"digest":evidence_digest},"snapshotIdentity":snapshot,
        "acceptedChoice":accepted,"authorityEvent":resolution["authorityEvent"],"consequences":consequences,
        "affectedBranches":resolution["request"]["affectedBranches"],"recordedAt":recorded_at,
        "freshness":{"evidenceExpiresAt":evidence_expires_at,"state":"current"},
        "reopenRule":{"evidenceDigestChanged":true,"snapshotChanged":true,"evidenceExpired":true}
    }))
}

fn validate_record(record: &Value) -> Result<(), DecisionRecordError> {
    exact(
        record,
        &[
            "schema",
            "id",
            "bindingDigest",
            "gap",
            "request",
            "response",
            "evidenceBinding",
            "snapshotIdentity",
            "acceptedChoice",
            "authorityEvent",
            "consequences",
            "affectedBranches",
            "recordedAt",
            "freshness",
            "reopenRule",
        ],
        "decision record",
    )?;
    if record["schema"] != RECORD_SCHEMA {
        return Err(DecisionRecordError(
            "decision record schema is invalid".to_string(),
        ));
    }
    let reconstructed = build_record(&json!({
        "schema":"code-intel-decision-record-request.v1","gap":record["gap"],"request":record["request"],
        "response":record["response"],"authorityEvent":record["authorityEvent"],
        "snapshotIdentity":record["snapshotIdentity"],"recordedAt":record["recordedAt"]
    }))?;
    if reconstructed != *record {
        return Err(DecisionRecordError(
            "decision record content binding is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_gap(gap: &Value) -> Result<(), DecisionRecordError> {
    exact(
        gap,
        &[
            "schema",
            "id",
            "kind",
            "blockedDecision",
            "discoverableFactsChecked",
            "options",
            "recommendedAnswer",
            "affectedBranches",
            "authorityRequired",
            "authorityState",
            "effects",
        ],
        "decision gap",
    )?;
    if gap["schema"] != "code-intel-decision-gap.v1"
        || !matches!(
            gap["kind"].as_str(),
            Some("intent" | "priority" | "resource_allocation" | "risk_acceptance" | "tradeoff")
        )
        || gap["authorityRequired"] != true
        || gap["authorityState"] != "unresolved"
        || gap["effects"] != json!([])
    {
        return Err(DecisionRecordError(
            "decision gap contract is invalid".to_string(),
        ));
    }
    nonempty(&gap["id"], "gap id")?;
    nonempty(&gap["blockedDecision"], "blocked decision")?;
    let facts = gap["discoverableFactsChecked"].as_array().ok_or_else(|| {
        DecisionRecordError("discoverableFactsChecked must be an array".to_string())
    })?;
    if facts.is_empty() {
        return Err(DecisionRecordError(
            "discoverableFactsChecked must not be empty".to_string(),
        ));
    }
    let mut fact_ids = BTreeSet::new();
    for fact in facts {
        exact(fact, &["factId", "status"], "fact check")?;
        let id = nonempty(&fact["factId"], "fact id")?;
        if fact["status"] != "resolved" || !fact_ids.insert(id.to_string()) {
            return Err(DecisionRecordError(
                "fact checks must be resolved and unique".to_string(),
            ));
        }
    }
    let options = validate_options(&gap["options"])?;
    exact(
        &gap["recommendedAnswer"],
        &["kind", "optionId", "rationale"],
        "recommended answer",
    )?;
    let recommended = nonempty(&gap["recommendedAnswer"]["optionId"], "recommended option")?;
    if gap["recommendedAnswer"]["kind"] != "proposal"
        || !options.contains(recommended)
        || nonempty(
            &gap["recommendedAnswer"]["rationale"],
            "recommendation rationale",
        )
        .is_err()
    {
        return Err(DecisionRecordError(
            "recommended answer contract is invalid".to_string(),
        ));
    }
    string_set(&gap["affectedBranches"], "gap affectedBranches")?;
    Ok(())
}

fn validate_request(request: &Value) -> Result<(), DecisionRecordError> {
    exact(
        request,
        &[
            "schema",
            "correlationId",
            "gapId",
            "question",
            "recommendation",
            "evidenceRefs",
            "options",
            "authorityNeeded",
            "issuedAt",
            "expiresAt",
            "affectedBranches",
        ],
        "decision request",
    )?;
    if request["schema"] != "code-intel-decision-request.v1" {
        return Err(DecisionRecordError(
            "decision request schema is invalid".to_string(),
        ));
    }
    identifier(&request["correlationId"], "request correlationId")?;
    nonempty(&request["gapId"], "request gapId")?;
    nonempty(&request["question"], "request question")?;
    exact(
        &request["recommendation"],
        &["optionId", "rationale"],
        "request recommendation",
    )?;
    nonempty(&request["recommendation"]["optionId"], "recommended option")?;
    nonempty(
        &request["recommendation"]["rationale"],
        "recommendation rationale",
    )?;
    let options = validate_options(&request["options"])?;
    if !options.contains(request["recommendation"]["optionId"].as_str().unwrap()) {
        return Err(DecisionRecordError(
            "recommendation names unknown option".to_string(),
        ));
    }
    validate_evidence_refs(&request["evidenceRefs"])?;
    exact(
        &request["authorityNeeded"],
        &["kind", "actorIds"],
        "authority needed",
    )?;
    nonempty(&request["authorityNeeded"]["kind"], "authority kind")?;
    string_set(
        &request["authorityNeeded"]["actorIds"],
        "authority actorIds",
    )?;
    let issued = unsigned(&request["issuedAt"], "request issuedAt")?;
    let expires = unsigned(&request["expiresAt"], "request expiresAt")?;
    if issued >= expires {
        return Err(DecisionRecordError(
            "request expiry must follow issue".to_string(),
        ));
    }
    string_set(&request["affectedBranches"], "request affectedBranches")?;
    Ok(())
}

fn validate_response(response: &Value) -> Result<(), DecisionRecordError> {
    exact(
        response,
        &[
            "schema",
            "correlationId",
            "gapId",
            "answer",
            "actorProvenance",
            "timestamp",
        ],
        "decision response",
    )?;
    if response["schema"] != "code-intel-decision-response.v1" {
        return Err(DecisionRecordError(
            "decision response schema is invalid".to_string(),
        ));
    }
    identifier(&response["correlationId"], "response correlationId")?;
    nonempty(&response["gapId"], "response gapId")?;
    match response["answer"]["kind"].as_str() {
        Some("choice") => {
            exact(&response["answer"], &["kind", "optionId"], "choice answer")?;
            nonempty(&response["answer"]["optionId"], "answer optionId")?;
        }
        Some("free-form") => {
            exact(&response["answer"], &["kind", "text"], "free-form answer")?;
            nonempty(&response["answer"]["text"], "answer text")?;
        }
        _ => {
            return Err(DecisionRecordError(
                "response answer kind is invalid".to_string(),
            ))
        }
    }
    exact(
        &response["actorProvenance"],
        &["actorId", "authorityKind", "source"],
        "actor provenance",
    )?;
    nonempty(&response["actorProvenance"]["actorId"], "actor id")?;
    nonempty(
        &response["actorProvenance"]["authorityKind"],
        "actor authority kind",
    )?;
    nonempty(&response["actorProvenance"]["source"], "actor source")?;
    unsigned(&response["timestamp"], "response timestamp")?;
    Ok(())
}

fn bind_exchange(resolution: &Value, recorded_at: u64) -> Result<(), DecisionRecordError> {
    let gap = &resolution["gap"];
    let request = &resolution["request"];
    let response = &resolution["response"];
    if gap["id"] != request["gapId"] || request["gapId"] != response["gapId"] {
        return Err(DecisionRecordError(
            "decision response gap binding mismatch".to_string(),
        ));
    }
    if request["correlationId"] != response["correlationId"] {
        return Err(DecisionRecordError(
            "decision response correlation mismatch".to_string(),
        ));
    }
    if gap["options"] != request["options"]
        || gap["recommendedAnswer"]["optionId"] != request["recommendation"]["optionId"]
        || gap["recommendedAnswer"]["rationale"] != request["recommendation"]["rationale"]
        || gap["affectedBranches"] != request["affectedBranches"]
    {
        return Err(DecisionRecordError(
            "gap and request binding mismatch".to_string(),
        ));
    }
    if let Some(option) = response["answer"]["optionId"].as_str() {
        let options = validate_options(&request["options"])?;
        if !options.contains(option) {
            return Err(DecisionRecordError(
                "response selects unknown option".to_string(),
            ));
        }
    }
    let actor = response["actorProvenance"]["actorId"].as_str().unwrap();
    if !string_set(
        &request["authorityNeeded"]["actorIds"],
        "authority actorIds",
    )?
    .contains(actor)
        || response["actorProvenance"]["authorityKind"] != request["authorityNeeded"]["kind"]
    {
        return Err(DecisionRecordError(
            "response actor lacks requested authority".to_string(),
        ));
    }
    let timestamp = response["timestamp"].as_u64().unwrap();
    if timestamp < request["issuedAt"].as_u64().unwrap()
        || timestamp > request["expiresAt"].as_u64().unwrap()
        || timestamp > recorded_at
        || recorded_at > request["expiresAt"].as_u64().unwrap()
        || request["evidenceRefs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|evidence| timestamp > evidence["expiresAt"].as_u64().unwrap())
    {
        return Err(DecisionRecordError(
            "response is outside request or evidence freshness".to_string(),
        ));
    }
    Ok(())
}

fn validate_query(query: &Value) -> Result<(), DecisionRecordError> {
    exact(
        query,
        &[
            "schema",
            "gapId",
            "snapshotIdentity",
            "evidenceRefs",
            "affectedBranches",
            "now",
        ],
        "decision replay query",
    )?;
    if query["schema"] != "code-intel-decision-replay-query.v1" {
        return Err(DecisionRecordError(
            "decision replay query schema is invalid".to_string(),
        ));
    }
    nonempty(&query["gapId"], "query gapId")?;
    digest(&query["snapshotIdentity"], "query snapshotIdentity")?;
    let evidence = validate_evidence_refs(&query["evidenceRefs"])?;
    string_set(&query["affectedBranches"], "query affectedBranches")?;
    let now = unsigned(&query["now"], "query now")?;
    if evidence
        .iter()
        .any(|reference| now < reference["observedAt"].as_u64().unwrap())
    {
        return Err(DecisionRecordError(
            "decision replay time precedes observed evidence".to_string(),
        ));
    }
    Ok(())
}

fn validate_options(value: &Value) -> Result<BTreeSet<String>, DecisionRecordError> {
    let options = value
        .as_array()
        .ok_or_else(|| DecisionRecordError("options must be an array".to_string()))?;
    if options.len() < 2 {
        return Err(DecisionRecordError(
            "at least two options are required".to_string(),
        ));
    }
    let mut ids = BTreeSet::new();
    for option in options {
        exact(option, &["id", "label", "consequence"], "decision option")?;
        let id = nonempty(&option["id"], "option id")?;
        nonempty(&option["label"], "option label")?;
        nonempty(&option["consequence"], "option consequence")?;
        if !ids.insert(id.to_string()) {
            return Err(DecisionRecordError("duplicate option id".to_string()));
        }
    }
    Ok(ids)
}

fn validate_evidence_refs(value: &Value) -> Result<Vec<Value>, DecisionRecordError> {
    let values = value
        .as_array()
        .ok_or_else(|| DecisionRecordError("evidenceRefs must be an array".to_string()))?;
    if values.is_empty() {
        return Err(DecisionRecordError(
            "evidenceRefs must not be empty".to_string(),
        ));
    }
    let mut refs = BTreeMap::new();
    for evidence in values {
        exact(
            evidence,
            &["refId", "sha256", "observedAt", "expiresAt"],
            "evidence ref",
        )?;
        let id = nonempty(&evidence["refId"], "evidence refId")?;
        digest(&evidence["sha256"], "evidence sha256")?;
        let observed = unsigned(&evidence["observedAt"], "evidence observedAt")?;
        let expires = unsigned(&evidence["expiresAt"], "evidence expiresAt")?;
        if observed >= expires {
            return Err(DecisionRecordError(
                "evidence expiry must follow observation".to_string(),
            ));
        }
        if refs.insert(id.to_string(), evidence.clone()).is_some() {
            return Err(DecisionRecordError("duplicate evidence refId".to_string()));
        }
    }
    Ok(refs.into_values().collect())
}

fn outcome(
    status: &str,
    question_required: bool,
    record: Option<Value>,
    reason: Option<&str>,
    diagnostics: Vec<String>,
) -> Value {
    json!({
        "schema":"code-intel-decision-record-operation-result.v1","status":status,
        "questionRequired":question_required,"record":record,"reason":reason,"diagnostics":diagnostics
    })
}

fn read_record(path: &Path) -> Result<Value, DecisionRecordError> {
    let metadata = fs::metadata(path)
        .map_err(|error| DecisionRecordError(format!("inspect record: {error}")))?;
    if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES {
        return Err(DecisionRecordError(
            "record must be a bounded regular file".to_string(),
        ));
    }
    read_json(path)
}

fn read_json(path: &Path) -> Result<Value, DecisionRecordError> {
    let bytes =
        fs::read(path).map_err(|error| DecisionRecordError(format!("read JSON: {error}")))?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(DecisionRecordError("JSON exceeds size limit".to_string()));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| DecisionRecordError("JSON must be UTF-8".to_string()))?;
    crate::capability::reject_duplicate_json_keys(text).map_err(DecisionRecordError)?;
    serde_json::from_str(text).map_err(|_| DecisionRecordError("invalid JSON".to_string()))
}

fn digest_value(value: &Value) -> String {
    crate::capability::sha256_hex(&serde_json::to_vec(&canonical(value)).unwrap())
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut sorted = object.iter().collect::<Vec<_>>();
            sorted.sort_by(|left, right| left.0.cmp(right.0));
            let mut result = Map::new();
            for (key, value) in sorted {
                result.insert(key.clone(), canonical(value));
            }
            Value::Object(result)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}

fn exact(value: &Value, fields: &[&str], label: &str) -> Result<(), DecisionRecordError> {
    let object = value
        .as_object()
        .ok_or_else(|| DecisionRecordError(format!("{label} must be an object")))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = fields.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(DecisionRecordError(format!("{label} fields are invalid")))
    }
}

fn nonempty<'a>(value: &'a Value, label: &str) -> Result<&'a str, DecisionRecordError> {
    value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| DecisionRecordError(format!("{label} is invalid")))
}

fn identifier<'a>(value: &'a Value, label: &str) -> Result<&'a str, DecisionRecordError> {
    let value = nonempty(value, label)?;
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Ok(value)
    } else {
        Err(DecisionRecordError(format!("{label} is not portable")))
    }
}

fn digest<'a>(value: &'a Value, label: &str) -> Result<&'a str, DecisionRecordError> {
    let value = nonempty(value, label)?;
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(value)
    } else {
        Err(DecisionRecordError(format!(
            "{label} must be lowercase SHA-256"
        )))
    }
}

fn unsigned(value: &Value, label: &str) -> Result<u64, DecisionRecordError> {
    value
        .as_u64()
        .ok_or_else(|| DecisionRecordError(format!("{label} must be an unsigned integer")))
}

fn string_set(value: &Value, label: &str) -> Result<BTreeSet<String>, DecisionRecordError> {
    let values = value
        .as_array()
        .ok_or_else(|| DecisionRecordError(format!("{label} must be an array")))?;
    if values.is_empty() {
        return Err(DecisionRecordError(format!("{label} must not be empty")));
    }
    let mut result = BTreeSet::new();
    for value in values {
        let item = nonempty(value, label)?.to_string();
        if !result.insert(item) {
            return Err(DecisionRecordError(format!("{label} contains duplicates")));
        }
    }
    Ok(result)
}
