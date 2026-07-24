use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Index;

use serde_json::{json, Value};

const SUBJECT_KINDS: [&str; 5] = [
    "design_reference",
    "evidence_provider",
    "method_implementation",
    "adapted_capability",
    "selectively_owned_implementation",
];
const ADOPTION_RUNGS: [&str; 7] = [
    "invoke",
    "adapt",
    "depend",
    "vendor",
    "fork",
    "port",
    "reimplement",
];
const LIFECYCLE_STATES: [&str; 6] = [
    "research",
    "production_enabled",
    "rollback",
    "replaced",
    "retired",
    "out_of_scope",
];
const TRANSITIONS: [(&str, &str); 10] = [
    ("research", "production_enabled"),
    ("research", "out_of_scope"),
    ("research", "retired"),
    ("production_enabled", "rollback"),
    ("production_enabled", "replaced"),
    ("production_enabled", "retired"),
    ("rollback", "production_enabled"),
    ("rollback", "replaced"),
    ("rollback", "retired"),
    ("replaced", "retired"),
];

pub(crate) struct Evaluation {
    value: Value,
    evaluated_record: Value,
}

impl Index<&str> for Evaluation {
    type Output = Value;

    fn index(&self, key: &str) -> &Self::Output {
        &self.value[key]
    }
}

impl fmt::Display for Evaluation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(formatter)
    }
}

pub(crate) fn evaluate_record(
    record: &Value,
    evaluated_at: u64,
    known_evidence_ids: &[String],
    consumed_authority_event_ids: &[String],
) -> Result<Evaluation, String> {
    validate_shape(record)?;
    let known = unique_input_set(known_evidence_ids, "known evidence")?;
    let consumed = unique_input_set(consumed_authority_event_ids, "consumed authority events")?;
    let all_evidence = record_evidence_ids(record)?;
    let mut diagnostics = Vec::new();

    for (label, evidence) in evidence_classes(record) {
        assess_evidence(label, evidence, evaluated_at, &known, &mut diagnostics)?;
    }
    for modification in record["ownedModifications"].as_array().unwrap() {
        assess_ids(
            "owned modification",
            &modification["evidenceIds"],
            &known,
            &mut diagnostics,
        )?;
    }
    assess_ids(
        "lifecycle",
        &record["lifecycle"]["evidenceIds"],
        &known,
        &mut diagnostics,
    )?;
    if record["update"]["nextCheckAt"].as_u64().unwrap() < evaluated_at {
        diagnostics.push("update check is overdue".to_string());
    }

    let lifecycle = &record["lifecycle"];
    let current = lifecycle["status"].as_str().unwrap();
    let previous = lifecycle["previousStatus"].as_str();
    let changed = previous
        .map(|value| value != current)
        .unwrap_or(current != "research");
    let mut lifecycle_diagnostics = Vec::new();
    let mut validated_event_id = None;
    if changed {
        let from = previous.unwrap_or("research");
        if !TRANSITIONS.contains(&(from, current)) {
            lifecycle_diagnostics.push(format!(
                "lifecycle transition {from}->{current} is not allowed"
            ));
        } else if lifecycle["effectiveAt"].as_u64().unwrap() > evaluated_at {
            lifecycle_diagnostics.push("lifecycle transition is future-dated".to_string());
        } else if lifecycle["authorityEvent"].is_null() {
            lifecycle_diagnostics.push("lifecycle transition requires A05 authority".to_string());
        } else {
            let repository_sign_off = record
                .pointer("/authorityRequirements/repositoryGovernedAttestation")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let validate = if repository_sign_off {
                crate::authority::validate_signed_authority_event
            } else {
                crate::authority::validate_authority_event
            };
            let required_evidence = if repository_sign_off {
                lifecycle["evidenceIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_string())
                    .collect()
            } else {
                all_evidence.clone()
            };
            match validate(
                &lifecycle["authorityEvent"],
                evaluated_at,
                &known,
                &required_evidence,
                &consumed,
            ) {
                Ok(id) => validated_event_id = Some(id),
                Err(message) => lifecycle_diagnostics.push(message),
            }
        }
    } else if !lifecycle["authorityEvent"].is_null() {
        lifecycle_diagnostics
            .push("unchanged lifecycle must not consume an authority event".to_string());
    }

    match current {
        "rollback" if record["rollback"]["evidence"]["evidenceIds"] == json!([]) => {
            lifecycle_diagnostics.push("rollback state requires rollback evidence".to_string())
        }
        "replaced" if lifecycle["replacementRecordId"].is_null() => lifecycle_diagnostics
            .push("replaced state requires a replacement record id".to_string()),
        "retired" if record["retirement"]["status"] != "completed" => lifecycle_diagnostics
            .push("retired state requires completed retirement evidence".to_string()),
        _ => {}
    }
    diagnostics.extend(lifecycle_diagnostics.iter().cloned());
    diagnostics.sort();
    diagnostics.dedup();
    let lifecycle_accepted = lifecycle_diagnostics.is_empty();
    let consumed_event_id = if lifecycle_accepted && diagnostics.is_empty() {
        validated_event_id
    } else {
        None
    };
    let production_enabled =
        current == "production_enabled" && lifecycle_accepted && diagnostics.is_empty();

    Ok(Evaluation {
        value: json!({
            "schema":"code-intel-internalization-evaluation.v1",
            "recordId":record["id"],
            "status":current,
            "researchAllowed":true,
            "productionEnabled":production_enabled,
            "lifecycleAccepted":lifecycle_accepted,
            "consumedAuthorityEventId":consumed_event_id,
            "diagnostics":diagnostics,
            "engineeringFacts":[]
        }),
        evaluated_record: record.clone(),
    })
}

pub(crate) fn record_evidence_ids(record: &Value) -> Result<BTreeSet<String>, String> {
    validate_shape(record)?;
    let mut result = BTreeSet::new();
    for (label, evidence) in evidence_classes(record) {
        extend_ids(&mut result, &evidence["evidenceIds"], label)?;
    }
    for modification in record["ownedModifications"].as_array().unwrap() {
        extend_ids(
            &mut result,
            &modification["evidenceIds"],
            "owned modification evidence",
        )?;
    }
    extend_ids(
        &mut result,
        &record["lifecycle"]["evidenceIds"],
        "lifecycle evidence",
    )?;
    Ok(result)
}

pub(crate) fn project_reuse_record(
    record: &Value,
    evaluation: &Evaluation,
) -> Result<Value, String> {
    validate_shape(record)?;
    if evaluation.evaluated_record != *record {
        return Err("internalization evaluation does not match record".to_string());
    }
    Ok(json!({
        "schema":"code-intel-reuse-record.v1",
        "id":record["id"],
        "projectId":record["projectId"],
        "subject":record["subject"]["name"],
        "subjectKind":record["subject"]["kind"],
        "source":record["subject"]["source"]["uri"],
        "sourceRevision":record["subject"]["source"]["revision"],
        "license":record["subject"]["license"],
        "adoptionRung":record["adoption"]["rung"],
        "ownedBoundary":record["adoption"]["ownedBoundary"],
        "necessityEvidence":record["adoption"]["necessityEvidence"],
        "compatibilityEvidence":record["adoption"]["compatibilityEvidence"],
        "conformanceEvidence":record["adoption"]["conformanceEvidence"],
        "economics":record["economics"],
        "assurance":record["assurance"],
        "ownedModifications":record["ownedModifications"],
        "update":record["update"],
        "rollback":record["rollback"],
        "exit":record["exit"],
        "retirement":record["retirement"],
        "lifecycle":record["lifecycle"]["status"],
        "researchAllowed":evaluation["researchAllowed"],
        "productionEnabled":evaluation["productionEnabled"],
        "diagnostics":evaluation["diagnostics"],
        "provenance":record["provenance"],
        "engineeringFacts":[]
    }))
}

pub(crate) fn project_notice_provenance(
    record: &Value,
    evaluation: &Evaluation,
) -> Result<Value, String> {
    validate_shape(record)?;
    if evaluation.evaluated_record != *record {
        return Err("internalization evaluation does not match record".to_string());
    }
    let obligations = record["subject"]["license"]["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("; ");
    let notice = format!(
        "{} — source {} at revision {}; license {}; obligations: {}.",
        record["subject"]["name"].as_str().unwrap(),
        record["subject"]["source"]["uri"].as_str().unwrap(),
        record["subject"]["source"]["revision"].as_str().unwrap(),
        record["subject"]["license"]["id"].as_str().unwrap(),
        obligations
    );
    Ok(json!({
        "schema":"code-intel-notice-provenance.v1",
        "recordId":record["id"],
        "noticeText":notice,
        "provenance":{
            "source":record["subject"]["source"]["uri"],
            "revision":record["subject"]["source"]["revision"],
            "license":record["subject"]["license"],
            "ownedModifications":record["ownedModifications"],
            "recordedAt":record["provenance"]["recordedAt"],
            "recordedBy":record["provenance"]["recordedBy"]
        }
    }))
}

#[derive(Default)]
pub(crate) struct RecordStore {
    records: BTreeMap<String, Value>,
}

impl RecordStore {
    pub(crate) fn insert(&mut self, record: Value) -> Result<(), String> {
        validate_shape(&record)?;
        let id = record["id"].as_str().unwrap().to_string();
        if self.records.contains_key(&id) {
            return Err(format!("duplicate internalization record: {id}"));
        }
        self.records.insert(id, record);
        Ok(())
    }

    pub(crate) fn project_reuse_records(
        &self,
        evaluated_at: u64,
        known_evidence_ids: &[String],
        consumed_authority_event_ids: &[String],
    ) -> Result<Vec<Value>, String> {
        self.records
            .values()
            .map(|record| {
                let evaluation = evaluate_record(
                    record,
                    evaluated_at,
                    known_evidence_ids,
                    consumed_authority_event_ids,
                )?;
                project_reuse_record(record, &evaluation)
            })
            .collect()
    }
}

fn validate_shape(record: &Value) -> Result<(), String> {
    let mut fields = vec![
        "schema",
        "id",
        "projectId",
        "subject",
        "adoption",
        "economics",
        "assurance",
        "update",
        "ownedModifications",
        "rollback",
        "exit",
        "retirement",
        "lifecycle",
        "provenance",
    ];
    if record.get("operationTrace").is_some() {
        fields.push("operationTrace");
    }
    if record.get("authorityRequirements").is_some() {
        fields.push("authorityRequirements");
    }
    exact(record, &fields, "internalization record")?;
    if record["schema"] != "code-intel-internalization-record.v1" {
        return Err("internalization record schema is invalid".to_string());
    }
    nonempty(&record["id"], "record id")?;
    nonempty(&record["projectId"], "project id")?;
    validate_subject(&record["subject"])?;
    validate_adoption(&record["adoption"])?;
    if let Some(requirements) = record.get("authorityRequirements") {
        exact(
            requirements,
            &["repositoryGovernedAttestation"],
            "authority requirements",
        )?;
        requirements["repositoryGovernedAttestation"]
            .as_bool()
            .ok_or("repositoryGovernedAttestation is invalid")?;
    }
    if let Some(trace) = record.get("operationTrace") {
        validate_operation_trace(trace)?;
    }
    validate_economics(&record["economics"])?;
    exact(
        &record["assurance"],
        &["maintenanceEvidence", "securityEvidence"],
        "assurance",
    )?;
    validate_evidence_shape(&record["assurance"]["maintenanceEvidence"], "maintenance")?;
    validate_evidence_shape(&record["assurance"]["securityEvidence"], "security")?;
    validate_update(&record["update"])?;
    validate_owned_modifications(&record["ownedModifications"])?;
    validate_rollback(&record["rollback"])?;
    validate_exit(&record["exit"])?;
    validate_retirement(&record["retirement"])?;
    validate_lifecycle(&record["lifecycle"])?;
    exact(
        &record["provenance"],
        &["recordedAt", "recordedBy"],
        "provenance",
    )?;
    record["provenance"]["recordedAt"]
        .as_u64()
        .ok_or("recordedAt is invalid")?;
    nonempty(&record["provenance"]["recordedBy"], "recordedBy")?;
    Ok(())
}

fn validate_operation_trace(value: &Value) -> Result<(), String> {
    let entries = value.as_array().ok_or("operationTrace must be an array")?;
    if entries.is_empty() {
        return Err("operationTrace must not be empty".to_string());
    }
    let mut identities = BTreeSet::new();
    for entry in entries {
        exact(
            entry,
            &[
                "integrationId",
                "operation",
                "command",
                "implementationIdentity",
                "source",
                "conformance",
            ],
            "operation trace entry",
        )?;
        let integration = entry["integrationId"]
            .as_str()
            .ok_or("operation trace integrationId is invalid")?;
        let operation = entry["operation"]
            .as_str()
            .ok_or("operation trace operation is invalid")?;
        if integration.is_empty()
            || operation.is_empty()
            || !identities.insert((integration, operation))
        {
            return Err(
                "operation trace integration/operation is invalid or duplicate".to_string(),
            );
        }
        nonempty(&entry["command"], "operation trace command")?;
        exact(
            &entry["implementationIdentity"],
            &["providerId", "implementationId", "activation"],
            "operation trace implementationIdentity",
        )?;
        for field in ["providerId", "implementationId", "activation"] {
            nonempty(
                &entry["implementationIdentity"][field],
                &format!("operation trace {field}"),
            )?;
        }
        for field in ["source", "conformance"] {
            let expected = if field == "source" {
                vec!["path", "sha256"]
            } else {
                vec!["path", "sha256", "testName"]
            };
            exact(
                &entry[field],
                &expected,
                &format!("operation trace {field}"),
            )?;
            nonempty(
                &entry[field]["path"],
                &format!("operation trace {field} path"),
            )?;
            let digest = entry[field]["sha256"]
                .as_str()
                .ok_or_else(|| format!("operation trace {field} sha256 is invalid"))?;
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(format!("operation trace {field} sha256 is invalid"));
            }
            if field == "conformance" {
                nonempty(
                    &entry[field]["testName"],
                    "operation trace conformance testName",
                )?;
            }
        }
    }
    Ok(())
}

fn validate_subject(value: &Value) -> Result<(), String> {
    exact(value, &["name", "kind", "source", "license"], "subject")?;
    nonempty(&value["name"], "subject name")?;
    let kind = value["kind"].as_str().ok_or("subject kind is invalid")?;
    if !SUBJECT_KINDS.contains(&kind) {
        return Err("subject kind is unknown".to_string());
    }
    exact(&value["source"], &["uri", "revision"], "source")?;
    nonempty(&value["source"]["uri"], "source uri")?;
    nonempty(&value["source"]["revision"], "source revision")?;
    exact(&value["license"], &["id", "obligations"], "license")?;
    nonempty(&value["license"]["id"], "license id")?;
    nonempty_strings(
        &value["license"]["obligations"],
        "license obligations",
        true,
    )?;
    Ok(())
}

fn validate_adoption(value: &Value) -> Result<(), String> {
    exact(
        value,
        &[
            "rung",
            "ownedBoundary",
            "necessityEvidence",
            "compatibilityEvidence",
            "conformanceEvidence",
        ],
        "adoption",
    )?;
    let rung = value["rung"].as_str().ok_or("adoption rung is invalid")?;
    if !ADOPTION_RUNGS.contains(&rung) {
        return Err("adoption rung is unknown".to_string());
    }
    nonempty_strings(&value["ownedBoundary"], "owned boundary", true)?;
    for (field, label) in [
        ("necessityEvidence", "necessity"),
        ("compatibilityEvidence", "compatibility"),
        ("conformanceEvidence", "conformance"),
    ] {
        validate_evidence_shape(&value[field], label)?;
    }
    Ok(())
}

fn validate_economics(value: &Value) -> Result<(), String> {
    exact(
        value,
        &["benefit", "cost", "benefitEvidence", "costEvidence"],
        "economics",
    )?;
    validate_measurement(&value["benefit"], "measured benefit")?;
    validate_measurement(&value["cost"], "measured cost")?;
    validate_evidence_shape(&value["benefitEvidence"], "benefit")?;
    validate_evidence_shape(&value["costEvidence"], "cost")?;
    Ok(())
}

fn validate_measurement(value: &Value, label: &str) -> Result<(), String> {
    exact(value, &["metric", "value", "unit"], label)?;
    nonempty(&value["metric"], &format!("{label} metric"))?;
    let amount = value["value"]
        .as_f64()
        .ok_or_else(|| format!("{label} value is invalid"))?;
    if !amount.is_finite() || amount < 0.0 {
        return Err(format!("{label} value is invalid"));
    }
    nonempty(&value["unit"], &format!("{label} unit"))
}

fn validate_update(value: &Value) -> Result<(), String> {
    exact(value, &["policy", "nextCheckAt", "evidence"], "update")?;
    nonempty(&value["policy"], "update policy")?;
    value["nextCheckAt"]
        .as_u64()
        .ok_or("update nextCheckAt is invalid")?;
    validate_evidence_shape(&value["evidence"], "update")
}

fn validate_owned_modifications(value: &Value) -> Result<(), String> {
    let values = value
        .as_array()
        .ok_or("ownedModifications must be an array")?;
    for modification in values {
        exact(
            modification,
            &["path", "description", "evidenceIds"],
            "owned modification",
        )?;
        nonempty(&modification["path"], "owned modification path")?;
        nonempty(
            &modification["description"],
            "owned modification description",
        )?;
        nonempty_strings(
            &modification["evidenceIds"],
            "owned modification evidenceIds",
            false,
        )?;
    }
    Ok(())
}

fn validate_rollback(value: &Value) -> Result<(), String> {
    exact(value, &["strategy", "evidence"], "rollback")?;
    nonempty(&value["strategy"], "rollback strategy")?;
    validate_evidence_shape(&value["evidence"], "rollback")
}

fn validate_exit(value: &Value) -> Result<(), String> {
    exact(
        value,
        &["strategy", "replacementCriteria", "evidence"],
        "exit",
    )?;
    nonempty(&value["strategy"], "exit strategy")?;
    nonempty_strings(&value["replacementCriteria"], "replacement criteria", true)?;
    validate_evidence_shape(&value["evidence"], "exit")
}

fn validate_retirement(value: &Value) -> Result<(), String> {
    exact(value, &["status", "triggers", "evidence"], "retirement")?;
    if !matches!(
        value["status"].as_str(),
        Some("active" | "candidate" | "approved" | "completed" | "out_of_scope")
    ) {
        return Err("retirement status is invalid".to_string());
    }
    nonempty_strings(&value["triggers"], "retirement triggers", true)?;
    validate_evidence_shape(&value["evidence"], "retirement")
}

fn validate_lifecycle(value: &Value) -> Result<(), String> {
    exact(
        value,
        &[
            "previousStatus",
            "status",
            "effectiveAt",
            "replacementRecordId",
            "evidenceIds",
            "authorityEvent",
        ],
        "lifecycle",
    )?;
    if !value["previousStatus"].is_null()
        && !value["previousStatus"]
            .as_str()
            .is_some_and(|state| LIFECYCLE_STATES.contains(&state))
    {
        return Err("previous lifecycle status is invalid".to_string());
    }
    if !value["status"]
        .as_str()
        .is_some_and(|state| LIFECYCLE_STATES.contains(&state))
    {
        return Err("lifecycle status is invalid".to_string());
    }
    value["effectiveAt"]
        .as_u64()
        .ok_or("lifecycle effectiveAt is invalid")?;
    if !value["replacementRecordId"].is_null() {
        nonempty(&value["replacementRecordId"], "replacement record id")?;
    }
    nonempty_strings(&value["evidenceIds"], "lifecycle evidenceIds", false)?;
    if !value["authorityEvent"].is_null() && !value["authorityEvent"].is_object() {
        return Err("lifecycle authorityEvent is invalid".to_string());
    }
    Ok(())
}

fn validate_evidence_shape(value: &Value, label: &str) -> Result<(), String> {
    exact(
        value,
        &["evidenceIds", "checkedAt", "expiresAt"],
        &format!("{label} evidence"),
    )?;
    nonempty_strings(
        &value["evidenceIds"],
        &format!("{label} evidenceIds"),
        false,
    )?;
    let checked = value["checkedAt"]
        .as_u64()
        .ok_or_else(|| format!("{label} checkedAt is invalid"))?;
    let expires = value["expiresAt"]
        .as_u64()
        .ok_or_else(|| format!("{label} expiresAt is invalid"))?;
    if expires < checked {
        return Err(format!("{label} evidence expiry precedes check"));
    }
    Ok(())
}

fn evidence_classes(record: &Value) -> Vec<(&'static str, &Value)> {
    vec![
        ("necessity", &record["adoption"]["necessityEvidence"]),
        (
            "compatibility",
            &record["adoption"]["compatibilityEvidence"],
        ),
        ("conformance", &record["adoption"]["conformanceEvidence"]),
        ("benefit", &record["economics"]["benefitEvidence"]),
        ("cost", &record["economics"]["costEvidence"]),
        ("maintenance", &record["assurance"]["maintenanceEvidence"]),
        ("security", &record["assurance"]["securityEvidence"]),
        ("update", &record["update"]["evidence"]),
        ("rollback", &record["rollback"]["evidence"]),
        ("exit", &record["exit"]["evidence"]),
        ("retirement", &record["retirement"]["evidence"]),
    ]
}

fn assess_evidence(
    label: &str,
    evidence: &Value,
    evaluated_at: u64,
    known: &BTreeSet<String>,
    diagnostics: &mut Vec<String>,
) -> Result<(), String> {
    assess_ids(label, &evidence["evidenceIds"], known, diagnostics)?;
    let checked = evidence["checkedAt"].as_u64().unwrap();
    let expires = evidence["expiresAt"].as_u64().unwrap();
    if checked > evaluated_at {
        diagnostics.push(format!("{label} evidence is future-dated"));
    }
    if expires < evaluated_at {
        diagnostics.push(format!("{label} evidence is expired"));
    }
    Ok(())
}

fn assess_ids(
    label: &str,
    value: &Value,
    known: &BTreeSet<String>,
    diagnostics: &mut Vec<String>,
) -> Result<(), String> {
    let ids = string_set(value, &format!("{label} evidenceIds"))?;
    if ids.is_empty() {
        diagnostics.push(format!("{label} evidence is missing"));
    } else if !ids.is_subset(known) {
        diagnostics.push(format!("{label} evidence references unknown evidence"));
    }
    Ok(())
}

fn exact(value: &Value, expected: &[&str], label: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label} fields are invalid"))
    }
}

fn nonempty(value: &Value, label: &str) -> Result<(), String> {
    if value.as_str().is_some_and(|value| !value.is_empty()) {
        Ok(())
    } else {
        Err(format!("{label} is invalid"))
    }
}

fn nonempty_strings(value: &Value, label: &str, require_one: bool) -> Result<(), String> {
    let values = value
        .as_array()
        .ok_or_else(|| format!("{label} must be an array"))?;
    if require_one && values.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        let string = value
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{label} contains an invalid value"))?;
        if !seen.insert(string) {
            return Err(format!("{label} contains duplicate values"));
        }
    }
    Ok(())
}

fn string_set(value: &Value, label: &str) -> Result<BTreeSet<String>, String> {
    nonempty_strings(value, label, false)?;
    Ok(value
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect())
}

fn extend_ids(result: &mut BTreeSet<String>, value: &Value, label: &str) -> Result<(), String> {
    result.extend(string_set(value, label)?);
    Ok(())
}

fn unique_input_set(values: &[String], label: &str) -> Result<BTreeSet<String>, String> {
    let result = values.iter().cloned().collect::<BTreeSet<_>>();
    if result.len() != values.len() || result.iter().any(|value| value.is_empty()) {
        Err(format!("{label} contains invalid or duplicate ids"))
    } else {
        Ok(result)
    }
}
