use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::artifact_ref::{self, ArtifactContract};
use crate::capability::{reject_duplicate_json_keys, sha256_hex};

const CONDITIONS: [&str; 3] = ["C0", "C1", "Cfull"];
const MAX_INPUT_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match run(raw) {
        Ok(report) => {
            println!("{}", serde_json::to_string(&report).unwrap());
            0
        }
        Err((code, message)) => {
            eprintln!("{message}");
            code
        }
    }
}

struct Cli {
    corpus: PathBuf,
    runs: PathBuf,
    artifact_root: PathBuf,
    out: PathBuf,
}

fn run(raw: &[String]) -> Result<Value, (i32, String)> {
    let cli = parse_cli(raw)?;
    let corpus = read_json(&cli.corpus, "corpus")?;
    let runs = read_json(&cli.runs, "runs")?;
    let attestations =
        verify_attestations(&corpus, &runs, &cli.artifact_root).map_err(|message| (65, message))?;
    let report = evaluate(&corpus, &runs, &attestations).map_err(|message| (65, message))?;
    publish_reports(&cli.out, &report)?;
    Ok(report)
}

fn parse_cli(raw: &[String]) -> Result<Cli, (i32, String)> {
    if raw.first().map(String::as_str) != Some("tools") {
        return Err((64, usage()));
    }
    let mut corpus = None;
    let mut runs = None;
    let mut artifact_root = None;
    let mut out = None;
    let mut index = 1;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(flag, "--corpus" | "--runs" | "--artifact-root" | "--out") {
            return Err((64, format!("unknown tool benchmark argument: {flag}")));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| (64, format!("{flag} requires one value")))?;
        match flag {
            "--corpus" if corpus.replace(PathBuf::from(value)).is_some() => {
                return Err((64, "duplicate --corpus".into()))
            }
            "--runs" if runs.replace(PathBuf::from(value)).is_some() => {
                return Err((64, "duplicate --runs".into()))
            }
            "--artifact-root" if artifact_root.replace(PathBuf::from(value)).is_some() => {
                return Err((64, "duplicate --artifact-root".into()))
            }
            "--out" if out.replace(PathBuf::from(value)).is_some() => {
                return Err((64, "duplicate --out".into()))
            }
            _ => {}
        }
        index += 2;
    }
    Ok(Cli {
        corpus: corpus.ok_or_else(|| (64, usage()))?,
        runs: runs.ok_or_else(|| (64, usage()))?,
        artifact_root: artifact_root.ok_or_else(|| (64, usage()))?,
        out: out.ok_or_else(|| (64, usage()))?,
    })
}

fn usage() -> String {
    "usage: benchmark tools --corpus <corpus.json> --runs <runs.json> --artifact-root <directory> --out <directory>".into()
}

fn read_json(path: &Path, label: &str) -> Result<Value, (i32, String)> {
    let metadata =
        fs::metadata(path).map_err(|error| (74, format!("read {label} metadata: {error}")))?;
    if metadata.len() > MAX_INPUT_BYTES {
        return Err((65, format!("{label} exceeds {MAX_INPUT_BYTES} bytes")));
    }
    let bytes = fs::read(path).map_err(|error| (74, format!("read {label}: {error}")))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| (65, format!("{label} must be UTF-8 JSON: {error}")))?;
    reject_duplicate_json_keys(text).map_err(|error| (65, format!("parse {label}: {error}")))?;
    serde_json::from_slice(&bytes).map_err(|error| (65, format!("parse {label}: {error}")))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    std::io::Write::write_all(&mut file, bytes)
        .map_err(|error| format!("write {}: {error}", path.display()))
}

fn publish_reports(out: &Path, report: &Value) -> Result<(), (i32, String)> {
    let parent = out
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            (
                74,
                "benchmark output requires a parent directory".to_string(),
            )
        })?;
    let name = out
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            (
                74,
                "benchmark output name must be portable UTF-8".to_string(),
            )
        })?;
    let staging = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    fs::create_dir(&staging)
        .map_err(|error| (74, format!("create benchmark staging directory: {error}")))?;
    let report_bytes = serde_json::to_vec(report)
        .map_err(|error| (74, format!("serialize tool benchmark report: {error}")))?;
    let markdown_bytes = render_markdown(report).into_bytes();
    let completion = serde_json::to_vec(&json!({
        "schema":"code-intel-tool-effectiveness-publication.v1",
        "reportSha256":sha256_hex(&report_bytes),
        "markdownSha256":sha256_hex(&markdown_bytes)
    }))
    .map_err(|error| {
        (
            74,
            format!("serialize benchmark completion manifest: {error}"),
        )
    })?;
    let staged = (|| -> Result<(), String> {
        write_new(&staging.join("report.json"), &report_bytes)?;
        write_new(&staging.join("report.md"), &markdown_bytes)?;
        write_new(&staging.join("completion.json"), &completion)
    })();
    if let Err(message) = staged {
        let _ = fs::remove_dir_all(&staging);
        return Err((74, message));
    }
    if let Err(error) = crate::run_commit::publish_directory_no_replace(&staging, out) {
        let _ = fs::remove_dir_all(&staging);
        return Err((74, format!("commit benchmark output directory: {error}")));
    }
    Ok(())
}

#[derive(Clone)]
struct Case {
    snapshot_identity: String,
    relevant: BTreeSet<String>,
    root_causes: BTreeSet<String>,
}

#[derive(Clone)]
struct ScoredRun {
    run_id: String,
    case_id: String,
    condition: String,
    repetition: u64,
    profile_digest: String,
    treatment_digest: String,
    context_digest: String,
    attestation_digest: String,
    status: String,
    root_cause_recall: f64,
    evidence_precision: f64,
    reciprocal_rank: f64,
    diagnosis_root_cause_recall: f64,
    diagnosis_exact: f64,
    attestation_success: f64,
    unsupported_claim_rate: f64,
    wall_time_ms: f64,
    input_tokens: f64,
    output_tokens: f64,
    tool_calls: f64,
    artifact_bytes: f64,
}

fn verify_attestations(
    corpus: &Value,
    runs: &Value,
    artifact_root: &Path,
) -> Result<BTreeMap<String, bool>, String> {
    require_schema(
        corpus,
        "code-intel-tool-effectiveness-corpus.v1",
        "tool effectiveness corpus",
    )?;
    require_schema(
        runs,
        "code-intel-tool-effectiveness-runs.v1",
        "tool effectiveness runs",
    )?;
    let cases = parse_cases(corpus)?;
    let treatments = parse_treatments(corpus)?;
    let verifier_identity =
        required_string(corpus, "verifierIdentity", "tool effectiveness corpus")?;
    let values = runs
        .get("runs")
        .and_then(Value::as_array)
        .ok_or_else(|| "tool effectiveness runs must be an array".to_string())?;
    let contract = ArtifactContract {
        artifact_schema: "code-intel-external-attestation.v1",
        artifact_type: "benchmark.external-attestation",
        max_bytes: MAX_INPUT_BYTES,
        validate_payload: validate_attestation_payload,
    };
    let mut verified = BTreeMap::new();
    for value in values {
        let run_id = required_string(value, "runId", "run")?;
        let case_id = required_string(value, "caseId", "run")?;
        let case = cases
            .get(case_id)
            .ok_or_else(|| format!("run {run_id} references unknown case {case_id}"))?;
        let condition = required_string(value, "condition", "run")?;
        let treatment_digest = treatments
            .get(condition)
            .ok_or_else(|| format!("run {run_id} has unsupported condition {condition}"))?;
        let artifact = value
            .get("attestationRef")
            .ok_or_else(|| format!("run {run_id} requires attestationRef"))?;
        let attestation = artifact_ref::verify_artifact_ref(
            artifact_root,
            &case.snapshot_identity,
            contract,
            artifact,
        )
        .map_err(|error| {
            format!(
                "run {run_id} ExternalAttestation verification failed: {}",
                error.message()
            )
        })?;
        let payload: Value = serde_json::from_slice(attestation.bytes())
            .map_err(|error| format!("parse run {run_id} ExternalAttestation: {error}"))?;
        if payload["runId"] != run_id
            || payload["caseId"] != case_id
            || payload["snapshotIdentity"] != case.snapshot_identity
            || payload["condition"] != condition
            || payload["treatmentDigest"] != treatment_digest.as_str()
            || payload["contextDigest"] != value["contextDigest"]
            || payload["experimentProfileDigest"] != value["experimentProfileDigest"]
            || payload["verifierIdentity"] != verifier_identity
        {
            return Err(format!(
                "run {run_id} ExternalAttestation is not bound to its run, case, snapshot, treatment, context, experiment, and verifier"
            ));
        }
        if verified
            .insert(
                run_id.to_string(),
                required_bool(&payload, "passed", "ExternalAttestation")?,
            )
            .is_some()
        {
            return Err(format!("duplicate tool effectiveness run: {run_id}"));
        }
    }
    Ok(verified)
}

fn validate_attestation_payload(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("ExternalAttestation must be UTF-8 JSON: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("parse ExternalAttestation: {error}"))?;
    require_exact_fields(
        &value,
        "ExternalAttestation",
        &[
            "schema",
            "runId",
            "caseId",
            "snapshotIdentity",
            "condition",
            "treatmentDigest",
            "contextDigest",
            "experimentProfileDigest",
            "passed",
            "verifierIdentity",
            "method",
            "evidenceRefs",
        ],
    )?;
    require_schema(
        &value,
        "code-intel-external-attestation.v1",
        "ExternalAttestation",
    )?;
    required_string(&value, "runId", "ExternalAttestation")?;
    required_string(&value, "caseId", "ExternalAttestation")?;
    required_digest(&value, "snapshotIdentity", "ExternalAttestation")?;
    let condition = required_string(&value, "condition", "ExternalAttestation")?;
    if !CONDITIONS.contains(&condition) {
        return Err(format!(
            "ExternalAttestation.condition has unsupported value {condition}"
        ));
    }
    required_digest(&value, "treatmentDigest", "ExternalAttestation")?;
    required_digest(&value, "contextDigest", "ExternalAttestation")?;
    required_digest(&value, "experimentProfileDigest", "ExternalAttestation")?;
    required_bool(&value, "passed", "ExternalAttestation")?;
    required_string(&value, "verifierIdentity", "ExternalAttestation")?;
    let method = required_string(&value, "method", "ExternalAttestation")?;
    if method != "frozen_evidence_replay" {
        return Err(format!(
            "ExternalAttestation.method has unsupported value {method}"
        ));
    }
    let evidence_refs = string_array(&value, "evidenceRefs", "ExternalAttestation")?;
    if evidence_refs.is_empty() {
        return Err("ExternalAttestation.evidenceRefs must not be empty".into());
    }
    for reference in evidence_refs {
        validate_artifact_entity_ref(reference)?;
    }
    Ok(())
}

fn validate_experiment_profile(run: &Value, case: &Case, run_id: &str) -> Result<String, String> {
    let experiment = run
        .get("experiment")
        .ok_or_else(|| format!("run {run_id} requires experiment"))?;
    require_exact_fields(
        experiment,
        "run.experiment",
        &[
            "model",
            "reasoning",
            "promptDigest",
            "budgetDigest",
            "permissionProfileDigest",
            "toolSnapshotDigest",
            "seed",
        ],
    )?;
    let canonical = json!({
        "snapshotIdentity":case.snapshot_identity,
        "model":required_string(experiment, "model", "run.experiment")?,
        "reasoning":required_string(experiment, "reasoning", "run.experiment")?,
        "promptDigest":required_digest(experiment, "promptDigest", "run.experiment")?,
        "budgetDigest":required_digest(experiment, "budgetDigest", "run.experiment")?,
        "permissionProfileDigest":required_digest(experiment, "permissionProfileDigest", "run.experiment")?,
        "toolSnapshotDigest":required_digest(experiment, "toolSnapshotDigest", "run.experiment")?,
        "seed":required_u64(experiment, "seed", "run.experiment")?
    });
    let actual = sha256_hex(
        &serde_json::to_vec(&canonical)
            .map_err(|error| format!("serialize run {run_id} experiment profile: {error}"))?,
    );
    let declared = required_digest(run, "experimentProfileDigest", "run")?;
    if actual != declared {
        return Err(format!(
            "run {run_id} experimentProfileDigest does not match its frozen experiment fields"
        ));
    }
    Ok(actual)
}

pub(crate) fn evaluate(
    corpus: &Value,
    runs: &Value,
    attestations: &BTreeMap<String, bool>,
) -> Result<Value, String> {
    require_exact_fields(
        corpus,
        "tool effectiveness corpus",
        &[
            "schema",
            "corpusId",
            "topK",
            "repetitions",
            "verifierIdentity",
            "treatments",
            "cases",
        ],
    )?;
    require_exact_fields(
        runs,
        "tool effectiveness runs",
        &["schema", "corpusId", "runs"],
    )?;
    require_schema(
        corpus,
        "code-intel-tool-effectiveness-corpus.v1",
        "tool effectiveness corpus",
    )?;
    require_schema(
        runs,
        "code-intel-tool-effectiveness-runs.v1",
        "tool effectiveness runs",
    )?;
    let corpus_id = required_string(corpus, "corpusId", "corpus")?;
    if required_string(runs, "corpusId", "runs")? != corpus_id {
        return Err("tool effectiveness runs target a different corpus".into());
    }
    let top_k = required_u64(corpus, "topK", "corpus")?;
    if !(1..=100).contains(&top_k) {
        return Err("tool effectiveness corpus topK must be 1..100".into());
    }
    let repetitions = required_u64(corpus, "repetitions", "corpus")?;
    if !(1..=10).contains(&repetitions) {
        return Err("tool effectiveness corpus repetitions must be 1..10".into());
    }
    let cases = parse_cases(corpus)?;
    let treatments = parse_treatments(corpus)?;
    let scored = parse_runs(runs, &cases, &treatments, attestations, top_k)?;
    validate_pairing(&cases, &scored, repetitions)?;
    let valid_pair_count = count_valid_pairs(&cases, &scored, repetitions);
    let expected_pair_count = cases.len() * repetitions as usize;
    let verdict = if valid_pair_count == expected_pair_count {
        "baseline_recorded"
    } else {
        "insufficient_evidence"
    };

    let variants = CONDITIONS
        .iter()
        .map(|condition| aggregate(condition, &scored))
        .collect::<Vec<_>>();
    let comparisons = ["C1", "Cfull"]
        .iter()
        .map(|condition| paired_comparison(condition, &scored))
        .collect::<Vec<_>>();
    let case_results = scored.iter().map(scored_run_json).collect::<Vec<_>>();

    Ok(json!({
        "schema":"code-intel-tool-effectiveness-benchmark.v1",
        "verdict":verdict,
        "inputs":{
            "corpusSha256":document_digest(corpus)?,
            "runsSha256":document_digest(runs)?,
            "verifierIdentity":required_string(corpus, "verifierIdentity", "corpus")?,
            "treatments":treatments
        },
        "corpus":{
            "id":corpus_id,
            "caseCount":cases.len(),
            "repetitions":repetitions,
            "topK":top_k
        },
        "method":{
            "conditions":CONDITIONS,
            "pairing":"same_case_repetition_and_experiment_profile",
            "unavailablePolicy":"reported_separately_not_scored_as_incorrect",
            "oracleVisibility":"external_to_agent_and_capsule_assembler",
            "compositeScore":false
        },
        "evidenceGate":{
            "validPairCount":valid_pair_count,
            "expectedPairCount":expected_pair_count,
            "complete":valid_pair_count == expected_pair_count
        },
        "caseResults":case_results,
        "variants":variants,
        "pairedComparisons":comparisons,
        "limitations":[
            "v0 is a recorded paired baseline, not a population-level statistical claim",
            "quality aggregates include completed and honest domain-unknown runs; unavailable and process-failed runs remain separate reliability outcomes",
            "attestation truth depends on the corpus-pinned verifier and authority root; the scorer verifies artifact integrity and complete run/treatment/context binding"
        ]
    }))
}

fn parse_cases(corpus: &Value) -> Result<BTreeMap<String, Case>, String> {
    let values = corpus
        .get("cases")
        .and_then(Value::as_array)
        .ok_or_else(|| "tool effectiveness corpus cases must be an array".to_string())?;
    if values.is_empty() {
        return Err("tool effectiveness corpus must contain at least one case".into());
    }
    let mut cases = BTreeMap::new();
    for value in values {
        require_exact_fields(
            value,
            "case",
            &[
                "id",
                "snapshotIdentity",
                "relevantEntities",
                "rootCauseEntities",
            ],
        )?;
        let id = required_string(value, "id", "case")?.to_string();
        let relevant = string_set(value, "relevantEntities", "case")?;
        let root_causes = string_set(value, "rootCauseEntities", "case")?;
        if root_causes.is_empty() || !root_causes.is_subset(&relevant) {
            return Err(format!(
                "case {id} rootCauseEntities must be a non-empty subset of relevantEntities"
            ));
        }
        let snapshot_identity = required_digest(value, "snapshotIdentity", "case")?.to_string();
        if cases
            .insert(
                id.clone(),
                Case {
                    snapshot_identity,
                    relevant,
                    root_causes,
                },
            )
            .is_some()
        {
            return Err(format!("duplicate tool effectiveness case: {id}"));
        }
    }
    Ok(cases)
}

fn parse_treatments(corpus: &Value) -> Result<BTreeMap<String, String>, String> {
    let treatments = corpus
        .get("treatments")
        .ok_or_else(|| "tool effectiveness corpus requires treatments".to_string())?;
    require_exact_fields(treatments, "corpus.treatments", &CONDITIONS)?;
    CONDITIONS
        .iter()
        .map(|condition| {
            Ok((
                (*condition).to_string(),
                required_digest(treatments, condition, "corpus.treatments")?.to_string(),
            ))
        })
        .collect()
}

fn parse_runs(
    document: &Value,
    cases: &BTreeMap<String, Case>,
    treatments: &BTreeMap<String, String>,
    attestations: &BTreeMap<String, bool>,
    top_k: u64,
) -> Result<Vec<ScoredRun>, String> {
    let values = document
        .get("runs")
        .and_then(Value::as_array)
        .ok_or_else(|| "tool effectiveness runs must be an array".to_string())?;
    let mut ids = BTreeSet::new();
    let mut scored = Vec::with_capacity(values.len());
    for value in values {
        require_exact_fields(
            value,
            "run",
            &[
                "runId",
                "caseId",
                "condition",
                "repetition",
                "status",
                "snapshotIdentity",
                "experiment",
                "experimentProfileDigest",
                "treatmentDigest",
                "contextDigest",
                "rankedEntities",
                "diagnosisEntities",
                "attestationRef",
                "unsupportedClaims",
                "claimCount",
                "wallTimeMs",
                "inputTokens",
                "outputTokens",
                "toolCalls",
                "artifactBytes",
            ],
        )?;
        let run_id = required_string(value, "runId", "run")?.to_string();
        if !ids.insert(run_id.clone()) {
            return Err(format!("duplicate tool effectiveness run: {run_id}"));
        }
        let case_id = required_string(value, "caseId", "run")?.to_string();
        let case = cases
            .get(&case_id)
            .ok_or_else(|| format!("run {run_id} references unknown case {case_id}"))?;
        if required_digest(value, "snapshotIdentity", "run")? != case.snapshot_identity {
            return Err(format!(
                "run {run_id} snapshot identity differs from its case"
            ));
        }
        let condition = required_string(value, "condition", "run")?.to_string();
        if !CONDITIONS.contains(&condition.as_str()) {
            return Err(format!(
                "run {run_id} has unsupported condition {condition}"
            ));
        }
        let treatment_digest = required_digest(value, "treatmentDigest", "run")?.to_string();
        if treatments.get(&condition) != Some(&treatment_digest) {
            return Err(format!(
                "run {run_id} treatmentDigest differs from corpus treatment {condition}"
            ));
        }
        let repetition = required_u64(value, "repetition", "run")?;
        let status = required_string(value, "status", "run")?.to_string();
        if ![
            "completed",
            "domain_unknown",
            "unavailable",
            "process_failed",
        ]
        .contains(&status.as_str())
        {
            return Err(format!("run {run_id} has unsupported status {status}"));
        }
        let profile_digest = validate_experiment_profile(value, case, &run_id)?;
        let context_digest = required_digest(value, "contextDigest", "run")?.to_string();
        let attestation_digest = required_digest(
            value
                .get("attestationRef")
                .ok_or_else(|| format!("run {run_id} requires attestationRef"))?,
            "sha256",
            "run.attestationRef",
        )?
        .to_string();
        let ranked = string_array(value, "rankedEntities", "run")?;
        if ranked.iter().copied().collect::<BTreeSet<_>>().len() != ranked.len() {
            return Err(format!("run {run_id} rankedEntities contains duplicates"));
        }
        let diagnosis = string_set(value, "diagnosisEntities", "run")?;
        let unsupported_claims = required_u64(value, "unsupportedClaims", "run")?;
        let claim_count = required_u64(value, "claimCount", "run")?;
        let attestation_passed = *attestations
            .get(&run_id)
            .ok_or_else(|| format!("run {run_id} lacks an A03-verified ExternalAttestation"))?;
        if unsupported_claims > claim_count {
            return Err(format!("run {run_id} unsupportedClaims exceeds claimCount"));
        }
        let ranked_top = ranked
            .iter()
            .copied()
            .take(top_k as usize)
            .collect::<Vec<_>>();
        let relevant_hits = ranked_top
            .iter()
            .filter(|entity| case.relevant.contains(**entity))
            .count();
        let root_hits = ranked_top
            .iter()
            .filter(|entity| case.root_causes.contains(**entity))
            .count();
        let reciprocal_rank = ranked_top
            .iter()
            .position(|entity| case.root_causes.contains(*entity))
            .map(|index| 1.0 / (index as f64 + 1.0))
            .unwrap_or(0.0);
        let quality_observed = status == "completed" || status == "domain_unknown";
        let denominator = ranked_top.len();
        scored.push(ScoredRun {
            run_id,
            case_id,
            condition,
            repetition,
            profile_digest,
            treatment_digest,
            context_digest,
            attestation_digest,
            status,
            root_cause_recall: if quality_observed {
                root_hits as f64 / case.root_causes.len() as f64
            } else {
                0.0
            },
            evidence_precision: if quality_observed && denominator > 0 {
                relevant_hits as f64 / denominator as f64
            } else {
                0.0
            },
            reciprocal_rank: if quality_observed {
                reciprocal_rank
            } else {
                0.0
            },
            diagnosis_root_cause_recall: if quality_observed {
                diagnosis.intersection(&case.root_causes).count() as f64
                    / case.root_causes.len() as f64
            } else {
                0.0
            },
            diagnosis_exact: if quality_observed && diagnosis == case.root_causes {
                1.0
            } else {
                0.0
            },
            attestation_success: if quality_observed && attestation_passed {
                1.0
            } else {
                0.0
            },
            unsupported_claim_rate: if quality_observed && claim_count > 0 {
                unsupported_claims as f64 / claim_count as f64
            } else {
                0.0
            },
            wall_time_ms: required_u64(value, "wallTimeMs", "run")? as f64,
            input_tokens: required_u64(value, "inputTokens", "run")? as f64,
            output_tokens: required_u64(value, "outputTokens", "run")? as f64,
            tool_calls: required_u64(value, "toolCalls", "run")? as f64,
            artifact_bytes: required_u64(value, "artifactBytes", "run")? as f64,
        });
    }
    Ok(scored)
}

fn scored_run_json(run: &ScoredRun) -> Value {
    json!({
        "runId":run.run_id,
        "caseId":run.case_id,
        "condition":run.condition,
        "repetition":run.repetition,
        "status":run.status,
        "provenance":{
            "experimentProfileDigest":run.profile_digest,
            "treatmentDigest":run.treatment_digest,
            "contextDigest":run.context_digest,
            "attestationSha256":run.attestation_digest
        },
        "quality":if quality_observed(run) { json!({
            "rootCauseRecallAtK":run.root_cause_recall,
            "relevantEntityPrecisionAtK":run.evidence_precision,
            "reciprocalRank":run.reciprocal_rank,
            "diagnosisRootCauseRecall":run.diagnosis_root_cause_recall,
            "diagnosisExact":run.diagnosis_exact == 1.0,
            "attestationPassed":run.attestation_success == 1.0,
            "unsupportedClaimRate":run.unsupported_claim_rate
        }) } else { Value::Null },
        "cost":{
            "wallTimeMs":run.wall_time_ms,
            "inputTokens":run.input_tokens,
            "outputTokens":run.output_tokens,
            "toolCalls":run.tool_calls,
            "artifactBytes":run.artifact_bytes
        }
    })
}

fn validate_pairing(
    cases: &BTreeMap<String, Case>,
    runs: &[ScoredRun],
    repetitions: u64,
) -> Result<(), String> {
    for case_id in cases.keys() {
        for repetition in 1..=repetitions {
            let paired = runs
                .iter()
                .filter(|run| run.case_id == *case_id && run.repetition == repetition)
                .collect::<Vec<_>>();
            if paired.len() != CONDITIONS.len() {
                return Err(format!(
                    "case {case_id} repetition {repetition} must have exactly C0, C1, and Cfull"
                ));
            }
            let condition_set = paired
                .iter()
                .map(|run| run.condition.as_str())
                .collect::<BTreeSet<_>>();
            if condition_set != CONDITIONS.into_iter().collect() {
                return Err(format!(
                    "case {case_id} repetition {repetition} has incomplete conditions"
                ));
            }
            let profile_set = paired
                .iter()
                .map(|run| run.profile_digest.as_str())
                .collect::<BTreeSet<_>>();
            if profile_set.len() != 1 {
                return Err(format!(
                    "case {case_id} repetition {repetition} changed the experiment profile"
                ));
            }
        }
    }
    if runs.len() != cases.len() * repetitions as usize * CONDITIONS.len() {
        return Err("tool effectiveness runs contain unexpected case repetitions".into());
    }
    Ok(())
}

fn count_valid_pairs(
    cases: &BTreeMap<String, Case>,
    runs: &[ScoredRun],
    repetitions: u64,
) -> usize {
    cases
        .keys()
        .flat_map(|case_id| (1..=repetitions).map(move |repetition| (case_id, repetition)))
        .filter(|(case_id, repetition)| {
            CONDITIONS.iter().all(|condition| {
                runs.iter().any(|run| {
                    run.case_id == case_id.as_str()
                        && run.repetition == *repetition
                        && run.condition == **condition
                        && quality_observed(run)
                })
            })
        })
        .count()
}

mod scoring;
use scoring::*;

#[cfg(test)]
mod tests;
