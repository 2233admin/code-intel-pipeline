use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::adapter_contract::{AdapterArtifact, AdapterDomainVerdict, AdapterError, AdapterOutput};
use crate::artifact_ref::VerifiedArtifact;
use crate::capability::sha256_hex;

const METHOD_CARDS: [&str; 2] = ["critical-path-pert", "value-stream-queue-delay"];
const METHOD_CATALOG_BYTES: &[u8] =
    include_bytes!("../../../orchestration/methods/catalog.v1.json");
const CRITICAL_PATH_CARD_BYTES: &[u8] =
    include_bytes!("../../../orchestration/methods/cards/critical-path-pert.v1.json");
const VALUE_STREAM_CARD_BYTES: &[u8] =
    include_bytes!("../../../orchestration/methods/cards/value-stream-queue-delay.v1.json");

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if request["options"]
        .as_object()
        .map_or(true, |options| !options.is_empty())
    {
        return Err(AdapterError::InvalidOptions(
            "delivery.light-speed-measure accepts no options".into(),
        ));
    }
    let inputs = Inputs::parse(request, verified_inputs)?;
    let input = inputs.timing;
    let timing: Value = serde_json::from_slice(input.bytes())
        .map_err(|error| AdapterError::Contract(format!("parse run timing events: {error}")))?;
    if timing["measurementSnapshotIdentity"] != request["snapshot"]["identity"]
        || input.consumed_snapshot_identity()
            != request["snapshot"]["identity"].as_str().unwrap_or_default()
    {
        return Err(AdapterError::Contract(
            "run timing measurement does not match the request snapshot".into(),
        ));
    }
    let report = evaluate(&timing, input.sha256(), &inputs, request)?;
    let bytes = serde_json::to_vec(&report).map_err(|error| {
        AdapterError::Internal(format!("serialize light-speed report: {error}"))
    })?;
    let markdown = render(&report).into_bytes();
    publish(out, "light-speed-report.json", &bytes)?;
    publish(out, "light-speed-report.md", &markdown)?;
    Ok(AdapterOutput {
        artifacts: vec![
            AdapterArtifact {
                artifact_schema: "code-intel-delivery-light-speed.v1".into(),
                artifact_type: "delivery.light-speed-report".into(),
                relative_path: "light-speed-report.json".into(),
                bytes,
            },
            AdapterArtifact {
                artifact_schema: "code-intel-delivery-light-speed-markdown.v1".into(),
                artifact_type: "delivery.light-speed-report-view".into(),
                relative_path: "light-speed-report.md".into(),
                bytes: markdown,
            },
        ],
        observed_effects: vec!["local_write".into()],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

struct Inputs<'a> {
    timing: &'a VerifiedArtifact,
    commits: Vec<(&'a Value, Value)>,
    manifests: Vec<(&'a Value, Value)>,
    method_provenance: Value,
}

impl<'a> Inputs<'a> {
    fn parse(request: &'a Value, verified: &'a [VerifiedArtifact]) -> Result<Self, AdapterError> {
        let refs = request["inputs"]
            .as_array()
            .ok_or_else(|| contract("request inputs must be an array"))?;
        if refs.len() != verified.len() || verified.len() != 8 {
            return Err(contract("delivery.light-speed-measure requires timing, two commits, two manifests, method catalog, and two method cards as A03 inputs"));
        }
        let mut timing = None;
        let mut commits = Vec::new();
        let mut manifests = Vec::new();
        let mut catalog = None;
        let mut cards = BTreeMap::new();
        for (reference, artifact) in refs.iter().zip(verified) {
            match (artifact.artifact_schema(), artifact.artifact_type()) {
                ("code-intel-run-timing-events.v1", "delivery.run-timing-events") => {
                    if timing.replace(artifact).is_some() {
                        return Err(contract("duplicate timing input"));
                    }
                }
                ("code-intel-run-commit.v1", "run.commit") => {
                    commits.push((reference, parse_json(artifact, "run commit")?))
                }
                ("code-intel-run-manifest.v1", "run.manifest") => {
                    manifests.push((reference, parse_json(artifact, "run manifest")?))
                }
                ("code-intel-method-catalog.v1", "method.catalog") => {
                    if catalog.replace((reference, artifact)).is_some() {
                        return Err(contract("duplicate method catalog input"));
                    }
                }
                ("code-intel-method-card.v1", "method.card") => {
                    let card = parse_json(artifact, "method card")?;
                    let id = card["id"]
                        .as_str()
                        .ok_or_else(|| contract("method card id is invalid"))?;
                    if cards
                        .insert(id.to_string(), (reference, artifact))
                        .is_some()
                    {
                        return Err(contract("duplicate method card input"));
                    }
                }
                _ => return Err(contract("unexpected delivery light-speed input contract")),
            }
        }
        let timing = timing.ok_or_else(|| contract("missing timing input"))?;
        if commits.len() != 2 || manifests.len() != 2 || cards.len() != 2 {
            return Err(contract(
                "delivery light-speed input cardinality is invalid",
            ));
        }
        let (catalog_ref, catalog_artifact) =
            catalog.ok_or_else(|| contract("missing method catalog input"))?;
        require_managed_bytes(catalog_artifact, METHOD_CATALOG_BYTES, "method catalog")?;
        let mut card_shas = Vec::new();
        let mut card_refs = Vec::new();
        for (id, expected) in [
            (METHOD_CARDS[0], CRITICAL_PATH_CARD_BYTES),
            (METHOD_CARDS[1], VALUE_STREAM_CARD_BYTES),
        ] {
            let (reference, artifact) = cards
                .get(id)
                .ok_or_else(|| contract(format!("missing method card {id}")))?;
            require_managed_bytes(artifact, expected, id)?;
            card_shas.push(artifact.sha256());
            card_refs.push((*reference).clone());
        }
        let method_provenance = json!({
            "catalogRef":catalog_ref,
            "catalogSha256":catalog_artifact.sha256(),
            "methodCardRefs":card_refs,
            "methodCardSha256":card_shas
        });
        Ok(Self {
            timing,
            commits,
            manifests,
            method_provenance,
        })
    }

    fn bind_commit(&self, commit_ref: &Value) -> Result<&Value, AdapterError> {
        let (_, commit) = self
            .commits
            .iter()
            .find(|(reference, _)| *reference == commit_ref)
            .ok_or_else(|| {
                contract("timing commitRef was not supplied as an A03-verified input")
            })?;
        let manifest_ref = self
            .manifests
            .iter()
            .find(|(reference, _)| {
                reference["path"] == commit["manifest"]["path"]
                    && reference["sha256"] == commit["manifest"]["sha256"]
            })
            .ok_or_else(|| {
                contract("A07 commit manifest object was not supplied as an A03-verified input")
            })?;
        let manifest = &manifest_ref.1;
        if manifest["runIdentity"] != commit["runIdentity"]
            || manifest["snapshotIdentity"] != commit["snapshotIdentity"]
        {
            return Err(contract(
                "A07 commit does not match its A03-verified run manifest",
            ));
        }
        Ok(commit)
    }
}

fn parse_json(artifact: &VerifiedArtifact, label: &str) -> Result<Value, AdapterError> {
    serde_json::from_slice(artifact.bytes())
        .map_err(|error| contract(format!("parse {label}: {error}")))
}

fn require_managed_bytes(
    artifact: &VerifiedArtifact,
    expected: &[u8],
    label: &str,
) -> Result<(), AdapterError> {
    if artifact.sha256() != sha256_hex(expected) || artifact.bytes() != expected {
        return Err(contract(format!(
            "{label} differs from the managed C01 method evidence"
        )));
    }
    Ok(())
}

fn evaluate(
    input: &Value,
    source_sha256: &str,
    inputs: &Inputs<'_>,
    request: &Value,
) -> Result<Value, AdapterError> {
    exact(
        input,
        &[
            "schema",
            "measurementSnapshotIdentity",
            "telemetry",
            "baseline",
            "current",
        ],
        "run timing events",
    )?;
    exact(
        &input["telemetry"],
        &["mode", "clock", "externalPlatform"],
        "telemetry policy",
    )?;
    if input["schema"] != "code-intel-run-timing-events.v1"
        || !digest(&input["measurementSnapshotIdentity"])
        || input["telemetry"]["mode"] != "local_opt_in"
        || input["telemetry"]["clock"] != "monotonic_elapsed_ms"
        || input["telemetry"]["externalPlatform"] != false
    {
        return Err(contract(
            "run timing events must be local opt-in monotonic telemetry",
        ));
    }
    let baseline_commit = inputs.bind_commit(&input["baseline"]["commitRef"])?;
    let current_commit = inputs.bind_commit(&input["current"]["commitRef"])?;
    if current_commit["snapshotIdentity"] != request["snapshot"]["identity"] {
        return Err(contract(
            "current A07 commit does not match the request snapshot",
        ));
    }
    let baseline = measure_trace(
        &input["baseline"],
        baseline_commit,
        "baseline",
        source_sha256,
    )?;
    let current = measure_trace(&input["current"], current_commit, "current", source_sha256)?;
    if baseline.run_identity == current.run_identity {
        return Err(contract("baseline and current committed runs must differ"));
    }
    let delta = delta(&baseline.document, &current.document);
    Ok(json!({
        "schema":"code-intel-delivery-light-speed.v1",
        "measurementSnapshotIdentity":input["measurementSnapshotIdentity"],
        "authority":"derived_measurement_no_schedule_commitment",
        "method":{
            "methodCardIds":METHOD_CARDS,
            "provenance":inputs.method_provenance,
            "valueStreamFormula":"leadTimeMs = max(completedAtMs) - min(startedAtMs); touch/category time is the sum of explicit event intervals",
            "queueFormula":"queueMs is the duration of explicit queue events; it is not inferred from mandatory verification",
            "criticalPathFormula":"maximum-duration predecessor closure; every join predecessor and ancestor is included once; lexical event-id order breaks equal-duration ties",
            "deltaFormula":"current minus baseline in milliseconds"
        },
        "rules":rules(),
        "baseline":baseline.document,
        "current":current.document,
        "delta":delta,
        "limitations":[
            "local opt-in events are measured only after an A07 commit marker is bound into each trace",
            "attribution reports observed intervals and does not promise a delivery date, staffing level, or schedule",
            "resource contention, arrival rates, and capacity remain unknown unless represented by explicit committed events"
        ]
    }))
}

struct TraceMeasurement {
    run_identity: String,
    document: Value,
}

#[derive(Clone)]
struct Event<'a> {
    value: &'a Value,
    id: &'a str,
    kind: &'a str,
    subject: &'a str,
    start: u64,
    end: u64,
    predecessors: Vec<&'a str>,
    pointer: String,
}

fn measure_trace(
    trace: &Value,
    commit: &Value,
    label: &str,
    source_sha256: &str,
) -> Result<TraceMeasurement, AdapterError> {
    exact(trace, &["commitRef", "events"], &format!("{label} trace"))?;
    let values = trace["events"]
        .as_array()
        .filter(|events| !events.is_empty())
        .ok_or_else(|| contract(format!("{label} events must be a non-empty array")))?;
    let mut events = Vec::with_capacity(values.len());
    let mut ids = BTreeSet::new();
    for (index, value) in values.iter().enumerate() {
        exact(
            value,
            &[
                "id",
                "kind",
                "subject",
                "startedAtMs",
                "completedAtMs",
                "mandatory",
                "coordinationNeed",
                "predecessors",
            ],
            &format!("{label} event"),
        )?;
        let id = nonempty(value, "id", "event id")?;
        let kind = nonempty(value, "kind", "event kind")?;
        let subject = nonempty(value, "subject", "event subject")?;
        let start = value["startedAtMs"]
            .as_u64()
            .ok_or_else(|| contract("event startedAtMs is invalid"))?;
        let end = value["completedAtMs"]
            .as_u64()
            .filter(|end| *end > start)
            .ok_or_else(|| contract("event completedAtMs must be after startedAtMs"))?;
        if !ids.insert(id) || !valid_id(id) {
            return Err(contract("event ids must be unique stable ids"));
        }
        let mandatory = value["mandatory"]
            .as_bool()
            .ok_or_else(|| contract("event mandatory is invalid"))?;
        validate_classification(value, kind, mandatory)?;
        let predecessors = value["predecessors"]
            .as_array()
            .ok_or_else(|| contract("event predecessors must be an array"))?
            .iter()
            .map(|item| {
                item.as_str()
                    .filter(|item| valid_id(item))
                    .ok_or_else(|| contract("event predecessor is invalid"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if predecessors.iter().collect::<BTreeSet<_>>().len() != predecessors.len() {
            return Err(contract("event predecessors must be unique"));
        }
        events.push(Event {
            value,
            id,
            kind,
            subject,
            start,
            end,
            predecessors,
            pointer: format!("/{label}/events/{index}"),
        });
    }
    events.sort_by_key(|event| (event.start, event.id));
    let by_id = events
        .iter()
        .map(|event| (event.id, event.clone()))
        .collect::<BTreeMap<_, _>>();
    for event in &events {
        for predecessor in &event.predecessors {
            let prior = by_id
                .get(predecessor)
                .ok_or_else(|| contract("event predecessor is missing"))?;
            if prior.end > event.start {
                return Err(contract(
                    "event predecessors must complete before the dependent event starts",
                ));
            }
        }
    }
    let categories = classify(&events);
    let critical_ids = critical_path(&events, &by_id)?;
    let critical_events = critical_ids
        .iter()
        .map(|id| by_id.get(id.as_str()).unwrap().clone())
        .collect::<Vec<_>>();
    let critical = classify(&critical_events);
    let first = events.iter().map(|event| event.start).min().unwrap();
    let last = events.iter().map(|event| event.end).max().unwrap();
    let touch = events.iter().map(duration).sum::<u64>();
    let provenance = provenance(&categories, source_sha256);
    let run_identity = commit["runIdentity"].as_str().unwrap().to_string();
    let document = json!({
        "commitRef":trace["commitRef"],
        "commit":commit,
        "leadTimeMs":last-first,
        "touchTimeMs":touch,
        "categories":categories.values(),
        "avoidableDelayMs":categories.avoidable(),
        "criticalPath":{
            "eventIds":critical_ids,
            "durationMs":critical_events.iter().map(duration).sum::<u64>(),
            "irreducibleTechnicalWorkMs":critical.irreducible,
            "mandatoryVerificationMs":critical.verification,
            "requiredCoordinationMs":critical.required_coordination,
            "queueMs":critical.queue,
            "handoffMs":critical.handoff,
            "repeatedUnderstandingMs":critical.repeated_understanding,
            "reworkMs":critical.rework,
            "unnecessaryCoordinationMs":critical.unnecessary_coordination,
            "avoidableDelayMs":critical.avoidable()
        },
        "provenance":provenance
    });
    Ok(TraceMeasurement {
        run_identity,
        document,
    })
}

#[derive(Default)]
struct Categories {
    irreducible: u64,
    verification: u64,
    required_coordination: u64,
    queue: u64,
    handoff: u64,
    repeated_understanding: u64,
    rework: u64,
    unnecessary_coordination: u64,
    ids: BTreeMap<&'static str, Vec<String>>,
    pointers: BTreeMap<&'static str, Vec<String>>,
}

impl Categories {
    fn add(&mut self, category: &'static str, event: &Event<'_>) {
        let value = duration(event);
        match category {
            "irreducibleTechnicalWorkMs" => self.irreducible += value,
            "mandatoryVerificationMs" => self.verification += value,
            "requiredCoordinationMs" => self.required_coordination += value,
            "queueMs" => self.queue += value,
            "handoffMs" => self.handoff += value,
            "repeatedUnderstandingMs" => self.repeated_understanding += value,
            "reworkMs" => self.rework += value,
            "unnecessaryCoordinationMs" => self.unnecessary_coordination += value,
            _ => unreachable!(),
        }
        self.ids
            .entry(category)
            .or_default()
            .push(event.id.to_string());
        self.pointers
            .entry(category)
            .or_default()
            .push(event.pointer.clone());
    }

    fn avoidable(&self) -> u64 {
        self.queue
            + self.handoff
            + self.repeated_understanding
            + self.rework
            + self.unnecessary_coordination
    }

    fn values(&self) -> Value {
        json!({
            "irreducibleTechnicalWorkMs":self.irreducible,
            "mandatoryVerificationMs":self.verification,
            "requiredCoordinationMs":self.required_coordination,
            "queueMs":self.queue,
            "handoffMs":self.handoff,
            "repeatedUnderstandingMs":self.repeated_understanding,
            "reworkMs":self.rework,
            "unnecessaryCoordinationMs":self.unnecessary_coordination
        })
    }
}

fn classify(events: &[Event<'_>]) -> Categories {
    let mut categories = Categories::default();
    let mut understood = BTreeSet::new();
    for event in events {
        let category = match event.kind {
            "technical_work" => "irreducibleTechnicalWorkMs",
            "test" | "verification" => "mandatoryVerificationMs",
            "queue" => "queueMs",
            "handoff" => "handoffMs",
            "rework" => "reworkMs",
            "understanding" if understood.insert(event.subject) => "irreducibleTechnicalWorkMs",
            "understanding" => "repeatedUnderstandingMs",
            "coordination" if event.value["coordinationNeed"] == "required" => {
                "requiredCoordinationMs"
            }
            "coordination" => "unnecessaryCoordinationMs",
            _ => unreachable!(),
        };
        categories.add(category, event);
    }
    categories
}

fn critical_path<'a>(
    events: &[Event<'a>],
    by_id: &BTreeMap<&'a str, Event<'a>>,
) -> Result<Vec<String>, AdapterError> {
    let mut closures: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for event in events {
        let mut closure = BTreeSet::new();
        for predecessor in &event.predecessors {
            let predecessor_closure = closures
                .get(predecessor)
                .ok_or_else(|| contract("event graph is cyclic or not predecessor-closed"))?;
            closure.extend(predecessor_closure.iter().cloned());
        }
        closure.insert(event.id.to_string());
        closures.insert(event.id, closure);
    }
    let selected = closures
        .values()
        .map(|closure| {
            let duration_ms = closure
                .iter()
                .map(|id| duration(by_id.get(id.as_str()).expect("closure ids are verified")))
                .sum::<u64>();
            (duration_ms, closure)
        })
        .max_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| right.1.iter().cmp(left.1.iter()))
        })
        .map(|(_, closure)| closure)
        .ok_or_else(|| contract("critical path requires events"))?;
    Ok(events
        .iter()
        .filter(|event| selected.contains(event.id))
        .map(|event| event.id.to_string())
        .collect())
}

fn provenance(categories: &Categories, source_sha256: &str) -> Value {
    let mut result = serde_json::Map::new();
    for (category, rule) in [
        (
            "irreducibleTechnicalWorkMs",
            "irreducible-and-first-understanding",
        ),
        (
            "mandatoryVerificationMs",
            "mandatory-verification-protection",
        ),
        ("requiredCoordinationMs", "coordination-necessity"),
        ("queueMs", "explicit-queue-attribution"),
        ("handoffMs", "explicit-handoff-attribution"),
        ("repeatedUnderstandingMs", "repeat-understanding-by-subject"),
        ("reworkMs", "explicit-rework-attribution"),
        ("unnecessaryCoordinationMs", "coordination-necessity"),
    ] {
        result.insert(
            category.into(),
            json!({
                "sourceArtifactSha256":source_sha256,
                "jsonPointers":categories.pointers.get(category).cloned().unwrap_or_default(),
                "eventIds":categories.ids.get(category).cloned().unwrap_or_default(),
                "ruleId":rule
            }),
        );
    }
    Value::Object(result)
}

fn delta(baseline: &Value, current: &Value) -> Value {
    let difference = |current: u64, baseline: u64| i128::from(current) - i128::from(baseline);
    let field =
        |document: &Value, pointer: &str| document.pointer(pointer).unwrap().as_u64().unwrap();
    json!({
        "formula":"current_minus_baseline_ms",
        "leadTimeMs":difference(field(current,"/leadTimeMs"),field(baseline,"/leadTimeMs")),
        "touchTimeMs":difference(field(current,"/touchTimeMs"),field(baseline,"/touchTimeMs")),
        "irreducibleTechnicalWorkMs":difference(field(current,"/categories/irreducibleTechnicalWorkMs"),field(baseline,"/categories/irreducibleTechnicalWorkMs")),
        "mandatoryVerificationMs":difference(field(current,"/categories/mandatoryVerificationMs"),field(baseline,"/categories/mandatoryVerificationMs")),
        "requiredCoordinationMs":difference(field(current,"/categories/requiredCoordinationMs"),field(baseline,"/categories/requiredCoordinationMs")),
        "queueMs":difference(field(current,"/categories/queueMs"),field(baseline,"/categories/queueMs")),
        "handoffMs":difference(field(current,"/categories/handoffMs"),field(baseline,"/categories/handoffMs")),
        "repeatedUnderstandingMs":difference(field(current,"/categories/repeatedUnderstandingMs"),field(baseline,"/categories/repeatedUnderstandingMs")),
        "reworkMs":difference(field(current,"/categories/reworkMs"),field(baseline,"/categories/reworkMs")),
        "unnecessaryCoordinationMs":difference(field(current,"/categories/unnecessaryCoordinationMs"),field(baseline,"/categories/unnecessaryCoordinationMs")),
        "avoidableDelayMs":difference(field(current,"/avoidableDelayMs"),field(baseline,"/avoidableDelayMs"))
    })
}

fn rules() -> Value {
    json!([
        {"id":"explicit-queue-attribution","methodCardIds":METHOD_CARDS,"rule":"only explicit queue intervals contribute to queueMs"},
        {"id":"explicit-handoff-attribution","methodCardIds":METHOD_CARDS,"rule":"only explicit handoff intervals contribute to handoffMs"},
        {"id":"repeat-understanding-by-subject","methodCardIds":METHOD_CARDS,"rule":"the first understanding interval per subject is required; later intervals for that subject are repeated understanding"},
        {"id":"explicit-rework-attribution","methodCardIds":METHOD_CARDS,"rule":"explicit rework intervals are reported separately"},
        {"id":"coordination-necessity","methodCardIds":METHOD_CARDS,"rule":"coordinationNeed required is protected; unnecessary is avoidable"},
        {"id":"mandatory-verification-protection","methodCardIds":METHOD_CARDS,"rule":"test and verification intervals must be mandatory and never contribute to avoidable delay"},
        {"id":"predecessor-critical-path","methodCardIds":METHOD_CARDS,"rule":"critical path is the maximum-duration full predecessor closure with deterministic lexical tie-breaking"}
    ])
}

fn validate_classification(value: &Value, kind: &str, mandatory: bool) -> Result<(), AdapterError> {
    if !matches!(
        kind,
        "technical_work"
            | "test"
            | "verification"
            | "queue"
            | "handoff"
            | "understanding"
            | "rework"
            | "coordination"
    ) {
        return Err(contract("event kind is unknown"));
    }
    let coordination = value["coordinationNeed"].as_str();
    match kind {
        "test" | "verification" if !mandatory || coordination.is_some() => Err(contract(
            "test/verification events must be mandatory and have no coordinationNeed",
        )),
        "test" | "verification" => Ok(()),
        "coordination"
            if !matches!(coordination, Some("required" | "unnecessary"))
                || mandatory != (coordination == Some("required")) =>
        {
            Err(contract(
                "coordination must declare required/mandatory or unnecessary/non-mandatory",
            ))
        }
        "coordination" => Ok(()),
        _ if mandatory || !value["coordinationNeed"].is_null() => Err(contract(
            "only mandatory verification and required coordination may be mandatory",
        )),
        _ => Ok(()),
    }
}

fn render(report: &Value) -> String {
    format!(
        "# Delivery Light-Speed Measurement\n\n- Authority: derived measurement; no schedule commitment\n- Baseline lead time: {} ms\n- Current lead time: {} ms\n- Lead-time delta (current - baseline): {} ms\n- Baseline avoidable delay: {} ms\n- Current avoidable delay: {} ms\n- Avoidable-delay delta (current - baseline): {} ms\n- Baseline mandatory verification protected: {} ms\n- Current mandatory verification protected: {} ms\n",
        report["baseline"]["leadTimeMs"],
        report["current"]["leadTimeMs"],
        report["delta"]["leadTimeMs"],
        report["baseline"]["avoidableDelayMs"],
        report["current"]["avoidableDelayMs"],
        report["delta"]["avoidableDelayMs"],
        report["baseline"]["categories"]["mandatoryVerificationMs"],
        report["current"]["categories"]["mandatoryVerificationMs"]
    )
}

fn publish(out: &Path, relative: &str, bytes: &[u8]) -> Result<(), AdapterError> {
    fs::create_dir_all(out)
        .map_err(|error| AdapterError::Io(format!("create light-speed output: {error}")))?;
    let path = out.join(relative);
    if path.exists() {
        return Err(AdapterError::Io(format!(
            "refusing to overwrite light-speed artifact: {relative}"
        )));
    }
    fs::write(path, bytes).map_err(|error| AdapterError::Io(format!("write {relative}: {error}")))
}

fn duration(event: &Event<'_>) -> u64 {
    event.end - event.start
}

fn nonempty<'a>(value: &'a Value, field: &str, context: &str) -> Result<&'a str, AdapterError> {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| contract(format!("{context} is invalid")))
}

fn exact(value: &Value, fields: &[&str], context: &str) -> Result<(), AdapterError> {
    let actual = value
        .as_object()
        .ok_or_else(|| contract(format!("{context} must be an object")))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = fields.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(contract(format!("{context} fields are not exact")));
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

fn digest(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn contract(message: impl Into<String>) -> AdapterError {
    AdapterError::Contract(message.into())
}
