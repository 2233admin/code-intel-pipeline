use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::{AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;

const CRITICALITY_THRESHOLD: u64 = 50;
const CONFIDENCE_THRESHOLD: u64 = 50;

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
            "understanding.quadrant accepts no options; method selection is a read-only consumer"
                .into(),
        ));
    }
    let [input] = verified_inputs else {
        return Err(AdapterError::Contract(
            "understanding.quadrant requires exactly one A03-verified project.orientation Artifact Ref"
                .into(),
        ));
    };
    if input.artifact_schema() != "code-intel-project-orientation.v1"
        || input.artifact_type() != "project.orientation"
    {
        return Err(AdapterError::Contract(
            "understanding.quadrant consumes only the D01 project.orientation artifact".into(),
        ));
    }
    let orientation: Value = serde_json::from_slice(input.bytes())
        .map_err(|error| AdapterError::Contract(format!("parse D01 orientation: {error}")))?;
    if orientation["snapshotIdentity"] != request["snapshot"]["identity"] {
        return Err(AdapterError::Contract(
            "D01 orientation payload snapshot differs from the A01 request".into(),
        ));
    }

    let document = project(&orientation, input.sha256())?;
    let bytes = serde_json::to_vec(&document).map_err(|error| {
        AdapterError::Internal(format!("serialize understanding quadrant: {error}"))
    })?;
    fs::create_dir(out).map_err(|error| {
        AdapterError::Io(format!(
            "exclusive understanding quadrant staging create {}: {error}",
            out.display()
        ))
    })?;
    fs::write(out.join("understanding-quadrant.json"), &bytes)
        .map_err(|error| AdapterError::Io(format!("write understanding-quadrant.json: {error}")))?;

    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-understanding-quadrant.v1".into(),
            artifact_type: "understanding.quadrant".into(),
            relative_path: "understanding-quadrant.json".into(),
            bytes,
        }],
        observed_effects: vec!["local_write".into()],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn project(orientation: &Value, source_sha256: &str) -> Result<Value, AdapterError> {
    let level = orientation["confidence"]["level"]
        .as_str()
        .ok_or_else(|| AdapterError::Contract("D01 confidence level is missing".into()))?;
    let known_confidence = match level {
        "high" => 100,
        "medium" => 70,
        "low" => 40,
        _ => {
            return Err(AdapterError::Contract(
                "D01 confidence level is outside low/medium/high".into(),
            ));
        }
    };
    let mut items = BTreeMap::<String, Value>::new();

    insert_item(
        &mut items,
        "identity".into(),
        "repository identity".into(),
        100,
        known_confidence,
        "known",
        "Repository identity and revision are established by D01.".into(),
        provenance(&orientation["identity"], "identity")?,
    )?;
    let purpose_known = orientation["purpose"]["status"] == "known";
    insert_item(
        &mut items,
        "purpose".into(),
        "project purpose".into(),
        90,
        if purpose_known { known_confidence } else { 0 },
        if purpose_known { "known" } else { "unknown" },
        orientation["purpose"]["reason"]
            .as_str()
            .unwrap_or("Project purpose remains unknown.")
            .to_string(),
        provenance(&orientation["purpose"], "purpose")?,
    )?;
    for language in array(&orientation["languages"], "languages")? {
        let name = string(language, "name", "language")?;
        insert_item(
            &mut items,
            format!("language:{name}"),
            format!("language {name}"),
            30,
            known_confidence,
            "known",
            format!("D01 observed source files for {name}."),
            provenance(language, "language")?,
        )?;
    }
    for boundary in array(&orientation["boundaries"], "boundaries")? {
        let path = string(boundary, "path", "boundary")?;
        insert_item(
            &mut items,
            format!("boundary:{path}"),
            format!("system boundary {path}"),
            90,
            known_confidence,
            "known",
            format!("D01 identifies {path} as a top-level system boundary."),
            provenance(boundary, "boundary")?,
        )?;
    }
    for entry in array(&orientation["entryPoints"], "entryPoints")? {
        let path = string(entry, "path", "entry point")?;
        insert_item(
            &mut items,
            format!("entry-point:{path}"),
            format!("entry point {path}"),
            80,
            known_confidence,
            "known",
            format!("D01 classifies {path} as an entry point."),
            provenance(entry, "entry point")?,
        )?;
    }
    for command in array(&orientation["commands"], "commands")? {
        let path = string(command, "path", "command")?;
        insert_item(
            &mut items,
            format!("command:{path}"),
            format!("command {path}"),
            75,
            known_confidence,
            "known",
            format!("D01 identifies {path} as an executable project command."),
            provenance(command, "command")?,
        )?;
    }
    insert_item(
        &mut items,
        "active-change".into(),
        "active change".into(),
        70,
        known_confidence,
        "known",
        format!(
            "The D01 working tree status is {}.",
            orientation["activeChange"]["status"]
                .as_str()
                .unwrap_or("unknown")
        ),
        provenance(&orientation["activeChange"], "activeChange")?,
    )?;
    for risk in array(&orientation["risks"], "risks")? {
        let code = string(risk, "code", "risk")?;
        insert_item(
            &mut items,
            format!("risk:{code}"),
            format!("risk {code}"),
            85,
            known_confidence,
            "known",
            string(risk, "statement", "risk")?.to_string(),
            provenance(risk, "risk")?,
        )?;
    }
    for availability in array(&orientation["evidenceAvailability"], "evidenceAvailability")? {
        let evidence = string(availability, "evidence", "evidence availability")?;
        let status = string(availability, "status", "evidence availability")?;
        insert_item(
            &mut items,
            format!("evidence:{evidence}"),
            format!("evidence availability {evidence}"),
            35,
            if status == "unknown" {
                0
            } else {
                known_confidence
            },
            if status == "unknown" {
                "unknown"
            } else {
                "known"
            },
            format!("D01 reports {evidence} evidence as {status}."),
            provenance(availability, "evidence availability")?,
        )?;
    }
    for unknown in array(&orientation["unknowns"], "unknowns")? {
        let field = string(unknown, "field", "unknown")?;
        insert_item(
            &mut items,
            format!("unknown:{field}"),
            field.to_string(),
            unknown_criticality(field),
            0,
            "unknown",
            string(unknown, "reason", "unknown")?.to_string(),
            provenance(unknown, "unknown")?,
        )?;
    }

    let items = items.into_values().collect::<Vec<_>>();
    let visible_unknowns = items
        .iter()
        .filter(|item| item["sourceState"] == "unknown")
        .map(|item| item["id"].clone())
        .collect::<Vec<_>>();
    let mut counts = BTreeMap::<String, u64>::new();
    for item in &items {
        *counts
            .entry(item["quadrant"].as_str().unwrap().to_string())
            .or_default() += 1;
    }

    Ok(json!({
        "schema":"code-intel-understanding-quadrant.v1",
        "snapshotIdentity":orientation["snapshotIdentity"],
        "sourceOrientation":{
            "artifactSchema":"code-intel-project-orientation.v1",
            "artifactType":"project.orientation",
            "sha256":source_sha256
        },
        "classificationPolicy":{
            "schema":"code-intel-understanding-quadrant-policy.v1",
            "scoreRange":{"minimum":0,"maximum":100},
            "systemCriticalityThreshold":CRITICALITY_THRESHOLD,
            "evidenceConfidenceThreshold":CONFIDENCE_THRESHOLD,
            "thresholdRule":"greater_than_or_equal_is_upper_band",
            "unknownCriticalityRule":"critical_by_default_except_declared_supporting_context",
            "methodConsumerPolicy":"C01_cards_and_C02_selection_may_consume_but_cannot_rewrite"
        },
        "items":items,
        "visibleUnknowns":visible_unknowns,
        "counts":{
            "Known Core":counts.get("Known Core").copied().unwrap_or(0),
            "Critical Unknown":counts.get("Critical Unknown").copied().unwrap_or(0),
            "Supporting Context":counts.get("Supporting Context").copied().unwrap_or(0),
            "Deferred Unknown":counts.get("Deferred Unknown").copied().unwrap_or(0)
        }
    }))
}

#[allow(clippy::too_many_arguments)]
fn insert_item(
    items: &mut BTreeMap<String, Value>,
    id: String,
    subject: String,
    criticality: u64,
    confidence: u64,
    source_state: &str,
    statement: String,
    provenance: Value,
) -> Result<(), AdapterError> {
    let (criticality_band, confidence_band, quadrant) = classify(criticality, confidence);
    let prior = items.insert(
        id.clone(),
        json!({
            "id":id,
            "subject":subject,
            "sourceState":source_state,
            "systemCriticality":{"score":criticality,"band":criticality_band},
            "evidenceConfidence":{"score":confidence,"band":confidence_band},
            "quadrant":quadrant,
            "statement":statement,
            "provenance":provenance
        }),
    );
    if prior.is_some() {
        return Err(AdapterError::Contract(format!(
            "D01 projects duplicate understanding item id {id}"
        )));
    }
    Ok(())
}

fn classify(criticality: u64, confidence: u64) -> (&'static str, &'static str, &'static str) {
    let critical = criticality >= CRITICALITY_THRESHOLD;
    let confident = confidence >= CONFIDENCE_THRESHOLD;
    match (critical, confident) {
        (true, true) => ("critical", "high", "Known Core"),
        (true, false) => ("critical", "low", "Critical Unknown"),
        (false, true) => ("supporting", "high", "Supporting Context"),
        (false, false) => ("supporting", "low", "Deferred Unknown"),
    }
}

fn unknown_criticality(field: &str) -> u64 {
    let field = field.to_ascii_lowercase();
    if [
        "language",
        "documentation",
        "example",
        "context",
        "metadata",
        "style",
    ]
    .iter()
    .any(|token| field.contains(token))
    {
        25
    } else {
        90
    }
}

fn provenance(value: &Value, label: &str) -> Result<Value, AdapterError> {
    let provenance = value["provenance"]
        .as_array()
        .filter(|values| !values.is_empty())
        .ok_or_else(|| AdapterError::Contract(format!("D01 {label} provenance is missing")))?;
    for entry in provenance {
        let object = entry.as_object().ok_or_else(|| {
            AdapterError::Contract(format!("D01 {label} provenance entry must be an object"))
        })?;
        let exact = object.len() == 3
            && object.contains_key("artifactType")
            && object.contains_key("artifactSha256")
            && object.contains_key("jsonPointer");
        let valid = entry["artifactType"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
            && entry["artifactSha256"].as_str().is_some_and(|value| {
                value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            && entry["jsonPointer"]
                .as_str()
                .is_some_and(|value| value.starts_with('/'));
        if !exact || !valid {
            return Err(AdapterError::Contract(format!(
                "D01 {label} provenance entry is invalid"
            )));
        }
    }
    Ok(Value::Array(provenance.clone()))
}

fn array<'a>(value: &'a Value, label: &str) -> Result<&'a Vec<Value>, AdapterError> {
    value
        .as_array()
        .ok_or_else(|| AdapterError::Contract(format!("D01 {label} must be an array")))
}

fn string<'a>(value: &'a Value, field: &str, label: &str) -> Result<&'a str, AdapterError> {
    value[field]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AdapterError::Contract(format!("D01 {label}.{field} is missing")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_are_inclusive_and_cover_all_four_quadrants() {
        assert_eq!(classify(50, 50).2, "Known Core");
        assert_eq!(classify(50, 49).2, "Critical Unknown");
        assert_eq!(classify(49, 50).2, "Supporting Context");
        assert_eq!(classify(49, 49).2, "Deferred Unknown");
    }

    #[test]
    fn unknown_criticality_is_conservative_with_explicit_supporting_exceptions() {
        assert_eq!(unknown_criticality("dependencies.runtime"), 90);
        assert_eq!(unknown_criticality("documentation.examples"), 25);
        assert_eq!(unknown_criticality("unrecognized.future.field"), 90);
    }

    #[test]
    fn duplicate_projected_ids_fail_closed() {
        let mut items = BTreeMap::new();
        let provenance = json!([{"artifactType":"project.orientation","sha256":"a".repeat(64),"jsonPointer":"/languages/0"}]);
        insert_item(
            &mut items,
            "language:Rust".into(),
            "language Rust".into(),
            30,
            100,
            "known",
            "first".into(),
            provenance.clone(),
        )
        .unwrap();
        let error = insert_item(
            &mut items,
            "language:Rust".into(),
            "language Rust".into(),
            30,
            100,
            "known",
            "second".into(),
            provenance,
        )
        .unwrap_err();
        assert!(
            matches!(error, AdapterError::Contract(message) if message.contains("duplicate understanding item id"))
        );
    }

    #[test]
    fn provenance_entries_must_be_closed_nonempty_claim_objects() {
        for invalid in [
            json!([null]),
            json!([{}]),
            json!([{"artifactType":"","artifactSha256":"a".repeat(64),"jsonPointer":"/x"}]),
            json!([{"artifactType":"project.orientation","artifactSha256":"a".repeat(64),"jsonPointer":"x"}]),
            json!([{"artifactType":"project.orientation","artifactSha256":"a".repeat(64),"jsonPointer":"/x","extra":true}]),
        ] {
            assert!(provenance(&json!({"provenance":invalid}), "fixture").is_err());
        }
    }
}
