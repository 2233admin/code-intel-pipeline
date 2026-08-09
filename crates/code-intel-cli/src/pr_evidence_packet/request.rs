use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::capability::reject_duplicate_json_keys;

use super::validation::{
    array_field, digest_field, enum_field, identifier_field, object_field, positive_integer_field,
    repo_relative_file, require_const, require_object_keys, text_field,
};

const REQUEST_SCHEMA: &str = "code-intel-pr-evidence-request.v1";

pub(super) fn parse_json(bytes: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "PR evidence request must be UTF-8 JSON".to_string())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    reject_duplicate_json_keys(text).map_err(|error| format!("PR evidence request {error}"))?;
    serde_json::from_str(text)
        .map_err(|error| format!("PR evidence request is invalid JSON: {error}"))
}

pub(super) fn normalize(request: &Value) -> Result<Value, String> {
    require_object_keys(request, &["schema", "subject", "claims"], "request")?;
    require_const(request, "schema", REQUEST_SCHEMA, "request")?;
    let subject = validate_subject(object_field(request, "subject", "request")?)?;
    let snapshot = subject["snapshotIdentity"].as_str().expect("validated");
    let claims = array_field(request, "claims", "request")?;

    let mut seen_ids = BTreeSet::new();
    let mut normalized = Vec::with_capacity(claims.len());
    for (index, claim) in claims.iter().enumerate() {
        let claim = validate_claim(claim, index, snapshot)?;
        let id = claim["id"].as_str().expect("validated");
        if !seen_ids.insert(id.to_string()) {
            return Err(format!("request.claims repeats id {id}"));
        }
        normalized.push(claim);
    }
    normalized.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));

    Ok(json!({
        "schema": REQUEST_SCHEMA,
        "subject": subject,
        "claims": normalized,
    }))
}

fn validate_subject(value: &Value) -> Result<Value, String> {
    require_object_keys(
        value,
        &[
            "repository",
            "baseRevision",
            "headRevision",
            "snapshotIdentity",
        ],
        "request.subject",
    )?;
    let repository = text_field(value, "repository", "request.subject")?;
    let base = super::validation::revision_field(value, "baseRevision", "request.subject")?;
    let head = super::validation::revision_field(value, "headRevision", "request.subject")?;
    if base == head {
        return Err("request.subject baseRevision and headRevision must differ".into());
    }
    let snapshot = digest_field(value, "snapshotIdentity", "request.subject")?;
    Ok(json!({
        "repository": repository,
        "baseRevision": base,
        "headRevision": head,
        "snapshotIdentity": snapshot,
    }))
}

fn validate_claim(value: &Value, index: usize, snapshot: &str) -> Result<Value, String> {
    let context = format!("request.claims[{index}]");
    require_object_keys(
        value,
        &[
            "id",
            "authority",
            "status",
            "availability",
            "summary",
            "evidence",
            "locations",
        ],
        &context,
    )?;
    let id = identifier_field(value, "id", &context)?;
    let authority = enum_field(
        value,
        "authority",
        &["gate", "advisory", "observation"],
        &context,
    )?;
    let status = enum_field(value, "status", &["pass", "fail", "unknown"], &context)?;
    let availability = enum_field(
        value,
        "availability",
        &["current", "stale", "unavailable"],
        &context,
    )?;
    if availability != "current" && status != "unknown" {
        return Err(format!(
            "{context} must use status unknown when availability is {availability}"
        ));
    }
    let summary = text_field(value, "summary", &context)?;
    let evidence = validate_evidence(
        object_field(value, "evidence", &context)?,
        snapshot,
        &context,
    )?;
    let locations = validate_locations(array_field(value, "locations", &context)?, &context)?;
    Ok(json!({
        "id": id,
        "authority": authority,
        "status": status,
        "availability": availability,
        "summary": summary,
        "evidence": evidence,
        "locations": locations,
    }))
}

fn validate_evidence(value: &Value, snapshot: &str, claim_context: &str) -> Result<Value, String> {
    let context = format!("{claim_context}.evidence");
    require_object_keys(
        value,
        &["artifactSchema", "type", "sha256", "snapshotIdentity"],
        &context,
    )?;
    let artifact_schema = identifier_field(value, "artifactSchema", &context)?;
    let artifact_type = identifier_field(value, "type", &context)?;
    let sha256 = digest_field(value, "sha256", &context)?;
    let evidence_snapshot = digest_field(value, "snapshotIdentity", &context)?;
    if evidence_snapshot != snapshot {
        return Err(format!(
            "{context}.snapshotIdentity must match request.subject.snapshotIdentity"
        ));
    }
    Ok(json!({
        "artifactSchema": artifact_schema,
        "type": artifact_type,
        "sha256": sha256,
        "snapshotIdentity": evidence_snapshot,
    }))
}

fn validate_locations(values: &[Value], context: &str) -> Result<Vec<Value>, String> {
    let mut locations = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for (index, value) in values.iter().enumerate() {
        let location_context = format!("{context}.locations[{index}]");
        require_object_keys(value, &["file", "line"], &location_context)?;
        let file = repo_relative_file(value, "file", &location_context)?;
        let line = positive_integer_field(value, "line", &location_context)?;
        let key = (file.to_string(), line);
        if !seen.insert(key) {
            return Err(format!(
                "{context}.locations must not repeat file:line entries"
            ));
        }
        locations.push(json!({"file": file, "line": line}));
    }
    Ok(locations)
}
