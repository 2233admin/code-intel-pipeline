use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::authority;
#[cfg(not(test))]
use crate::capability::reject_duplicate_json_keys;

#[cfg(test)]
fn reject_duplicate_json_keys(_: &str) -> Result<(), String> {
    Ok(())
}

const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

const CHANGE_KINDS: [&str; 7] = [
    "artifact",
    "dependency",
    "abstraction",
    "file",
    "test",
    "documentation",
    "process",
];
const OPERATIONS: [&str; 3] = ["add", "delete", "reuse"];
const CURRENT_VALUE_SOURCES: [&str; 6] = [
    "operator_requested_outcome",
    "committed_engineering_plan_deliverable",
    "verified_defect_or_risk",
    "required_contract_or_gate",
    "evidence_closing_spike",
    "approved_debt_reduction",
];
const RUNGS: [&str; 7] = [
    "do_nothing",
    "repository_reuse",
    "standard_library",
    "platform_native",
    "installed_dependency",
    "one_liner",
    "smallest_local_implementation",
];
const NON_FILTERABLE: [&str; 7] = [
    "verification",
    "evidence",
    "safety",
    "error_handling",
    "accessibility",
    "data_loss_prevention",
    "artifact_contract",
];

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let request_path = match parse_cli(raw) {
        Ok(path) => path,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let request = match read_request(&request_path) {
        Ok(request) => request,
        Err((exit, message)) => {
            eprintln!("{message}");
            return exit;
        }
    };
    match evaluate(&request) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string(&result).expect("Ponytail result serializes")
            );
            if result["enforcedBlock"] == true {
                2
            } else {
                0
            }
        }
        Err(message) => {
            eprintln!("{message}");
            65
        }
    }
}

fn parse_cli(raw: &[String]) -> Result<PathBuf, String> {
    if raw.first().map(String::as_str) != Some("ponytail-gate") {
        return Err("usage: governance ponytail-gate --request <request.json|->".to_string());
    }
    if raw.len() != 3 || raw[1] != "--request" || raw[2].is_empty() {
        return Err("usage: governance ponytail-gate --request <request.json|->".to_string());
    }
    Ok(PathBuf::from(&raw[2]))
}

fn read_request(path: &Path) -> Result<Value, (i32, String)> {
    let bytes = if path == Path::new("-") {
        let mut bytes = Vec::new();
        io::stdin()
            .take(MAX_REQUEST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| (74, format!("read Ponytail request stdin: {error}")))?;
        bytes
    } else {
        let metadata = fs::metadata(path)
            .map_err(|error| (74, format!("inspect Ponytail request: {error}")))?;
        if !metadata.is_file() {
            return Err((65, "Ponytail request must be a regular file".to_string()));
        }
        if metadata.len() > MAX_REQUEST_BYTES {
            return Err((65, "Ponytail request exceeds size limit".to_string()));
        }
        fs::read(path).map_err(|error| (74, format!("read Ponytail request: {error}")))?
    };
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err((65, "Ponytail request exceeds size limit".to_string()));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| (65, format!("Ponytail request is not UTF-8: {error}")))?;
    reject_duplicate_json_keys(text).map_err(|error| (65, error))?;
    serde_json::from_str(text)
        .map_err(|error| (65, format!("invalid Ponytail request JSON: {error}")))
}

pub(crate) fn policy_document() -> Value {
    json!({
        "schema":"code-intel-ponytail-gate-policy.v1",
        "changeKinds":CHANGE_KINDS,
        "operations":OPERATIONS,
        "allowedCurrentValueSources":CURRENT_VALUE_SOURCES,
        "forbiddenValueSources":["future_maybe"],
        "firstSufficientSolutionRungs":RUNGS,
        "nonFilterableRequirements":NON_FILTERABLE,
        "bypassAuthoritySchema":"code-intel-authority-event.v1",
        "rules":{
            "currentValue":"every change must name one allowed current value source backed by known evidence",
            "firstSufficient":"every rung below the selected rung must be rejected once with known evidence",
            "protectedRequirements":"verification, evidence, safety, and the documented engineering boundaries cannot be filtered out",
            "bypass":"only a scoped, approved, unexpired, unreplayed A05 authority event covering value-source, lower-rung, and all required trace evidence may bypass a value or rung rejection",
            "modes":"report_only retains rejection traces without blocking; enforce blocks when any rejection remains"
        }
    })
}

pub(crate) fn evaluate(request: &Value) -> Result<Value, String> {
    validate_request(request)?;
    let mode = request["mode"].as_str().unwrap();
    let evaluated_at = request["evaluatedAt"].as_u64().unwrap();
    let known = string_set(&request["knownEvidenceIds"], "knownEvidenceIds")?;
    let consumed = string_set(
        &request["consumedAuthorityEventIds"],
        "consumedAuthorityEventIds",
    )?;
    let changes = request["changes"].as_array().unwrap();
    let duplicate_changes = duplicates(changes.iter().filter_map(|change| change["id"].as_str()));
    let duplicate_events = duplicates(changes.iter().filter_map(|change| {
        change
            .pointer("/bypass/authorityEvent/id")
            .and_then(Value::as_str)
    }));

    let mut newly_consumed = BTreeSet::new();
    let results = changes
        .iter()
        .enumerate()
        .map(|(index, change)| {
            evaluate_change(
                change,
                index,
                evaluated_at,
                &known,
                &consumed,
                &duplicate_changes,
                &duplicate_events,
                &mut newly_consumed,
            )
        })
        .collect::<Vec<_>>();
    let would_reject = results
        .iter()
        .filter(|result| result["status"] == "rejected")
        .count();
    let mut all_consumed = consumed;
    all_consumed.extend(newly_consumed);
    Ok(json!({
        "schema":"code-intel-ponytail-gate-result.v1",
        "status":"completed",
        "mode":mode,
        "wouldReject":would_reject,
        "enforcedBlock":mode == "enforce" && would_reject > 0,
        "traceRetained":true,
        "consumedAuthorityEventIds":all_consumed,
        "changes":results
    }))
}

#[allow(clippy::too_many_arguments)]
fn evaluate_change(
    change: &Value,
    index: usize,
    evaluated_at: u64,
    known: &BTreeSet<String>,
    consumed: &BTreeSet<String>,
    duplicate_changes: &BTreeSet<String>,
    duplicate_events: &BTreeSet<String>,
    newly_consumed: &mut BTreeSet<String>,
) -> Value {
    let id = change["id"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("invalid-change-{index}"));
    let hard_rejection = validate_change_shape(change).and_then(|_| {
        let removed = removed_protections(change)?;
        if !removed.is_empty() {
            Err(format!(
                "non-filterable requirements would be removed: {}",
                removed.into_iter().collect::<Vec<_>>().join(", ")
            ))
        } else {
            Ok(())
        }
    });
    let hard_rejection = if duplicate_changes.contains(&id) {
        Err("duplicate change id".to_string())
    } else {
        hard_rejection
    };
    let policy_rejection = hard_rejection
        .as_ref()
        .map(|_| validate_policy(change, known))
        .unwrap_or(Ok(()));
    let bypass = change.get("bypass");

    let (status, authority_event_id, diagnostics) = match (hard_rejection, policy_rejection) {
        (Err(message), _) => ("rejected", None, vec![message]),
        (Ok(()), Ok(())) if bypass.is_none() => ("accepted", None, Vec::new()),
        (Ok(()), Ok(())) => (
            "rejected",
            None,
            vec!["bypass is not allowed without a policy rejection".to_string()],
        ),
        (Ok(()), Err(message)) => match validate_bypass(
            bypass,
            &id,
            change,
            evaluated_at,
            known,
            consumed,
            duplicate_events,
            newly_consumed,
        ) {
            Ok(event_id) => ("bypassed", Some(event_id), vec![message]),
            Err(bypass_error) => (
                "rejected",
                None,
                vec![message, format!("bypass rejected: {bypass_error}")],
            ),
        },
    };
    json!({
        "change":change,
        "status":status,
        "authorityEventId":authority_event_id,
        "diagnostics":diagnostics
    })
}

fn validate_request(request: &Value) -> Result<(), String> {
    exact(
        request,
        &[
            "schema",
            "mode",
            "evaluatedAt",
            "knownEvidenceIds",
            "consumedAuthorityEventIds",
            "changes",
        ],
        "Ponytail gate request",
    )?;
    if request["schema"] != "code-intel-ponytail-gate-request.v1" {
        return Err("Ponytail gate request schema is invalid".to_string());
    }
    if !matches!(request["mode"].as_str(), Some("report_only" | "enforce")) {
        return Err("Ponytail gate mode is invalid".to_string());
    }
    request["evaluatedAt"]
        .as_u64()
        .ok_or("evaluatedAt must be a non-negative integer")?;
    string_set(&request["knownEvidenceIds"], "knownEvidenceIds")?;
    string_set(
        &request["consumedAuthorityEventIds"],
        "consumedAuthorityEventIds",
    )?;
    let changes = request["changes"]
        .as_array()
        .ok_or("changes must be an array")?;
    if changes.is_empty() {
        return Err("changes must not be empty".to_string());
    }
    for (index, change) in changes.iter().enumerate() {
        validate_change_shape(change)
            .map_err(|error| format!("changes[{index}] is schema-invalid: {error}"))?;
    }
    Ok(())
}

fn validate_change_shape(change: &Value) -> Result<(), String> {
    exact_optional(
        change,
        &[
            "id",
            "kind",
            "operation",
            "valueSource",
            "requiredEvidenceIds",
            "firstSufficientRung",
            "lowerRungs",
            "removedProtections",
        ],
        &["bypass"],
        "change",
    )?;
    nonempty(&change["id"], "change id")?;
    let kind = change["kind"].as_str().ok_or("change kind is invalid")?;
    if !CHANGE_KINDS.contains(&kind) {
        return Err("change kind is unknown".to_string());
    }
    let operation = change["operation"]
        .as_str()
        .ok_or("change operation is invalid")?;
    if !OPERATIONS.contains(&operation) {
        return Err("change operation is unknown".to_string());
    }
    exact(
        &change["valueSource"],
        &["kind", "id", "evidenceIds"],
        "value source",
    )?;
    let source_kind = change["valueSource"]["kind"]
        .as_str()
        .ok_or("value source kind is invalid")?;
    if source_kind != "future_maybe" && !CURRENT_VALUE_SOURCES.contains(&source_kind) {
        return Err("value source kind is not in the request schema".to_string());
    }
    nonempty(&change["valueSource"]["id"], "value source id")?;
    if string_set(
        &change["valueSource"]["evidenceIds"],
        "value source evidenceIds",
    )?
    .is_empty()
    {
        return Err("value source evidenceIds must not be empty".to_string());
    }
    if string_set(&change["requiredEvidenceIds"], "requiredEvidenceIds")?.is_empty() {
        return Err("requiredEvidenceIds must not be empty".to_string());
    }
    let selected_rung = change["firstSufficientRung"]
        .as_str()
        .ok_or("first sufficient rung is invalid")?;
    if !RUNGS.contains(&selected_rung) {
        return Err("first sufficient rung is not in the request schema".to_string());
    }
    let lower_rungs = change["lowerRungs"]
        .as_array()
        .ok_or("lowerRungs must be an array")?;
    let mut unique_lower_rungs = BTreeSet::new();
    for (index, lower_rung) in lower_rungs.iter().enumerate() {
        let identity = serde_json::to_string(lower_rung)
            .map_err(|error| format!("serialize lowerRungs[{index}]: {error}"))?;
        if !unique_lower_rungs.insert(identity) {
            return Err("duplicate lowerRungs entries are schema-invalid".to_string());
        }
        exact(
            lower_rung,
            &["rung", "reason", "evidenceIds"],
            &format!("lowerRungs[{index}]"),
        )?;
        let rung = lower_rung["rung"].as_str().ok_or("lower rung is invalid")?;
        if !RUNGS.contains(&rung) {
            return Err("lower rung is not in the request schema".to_string());
        }
        nonempty(&lower_rung["reason"], "lower rung reason")?;
        if string_set(&lower_rung["evidenceIds"], "lower rung evidenceIds")?.is_empty() {
            return Err("lower rung evidenceIds must not be empty".to_string());
        }
    }
    removed_protections(change)
        .map_err(|error| format!("removedProtections is schema-invalid: {error}"))?;
    if let Some(bypass) = change.get("bypass") {
        validate_bypass_shape(bypass)?;
    }
    Ok(())
}

fn validate_bypass_shape(bypass: &Value) -> Result<(), String> {
    exact(bypass, &["changeId", "authorityEvent"], "bypass")?;
    nonempty(&bypass["changeId"], "bypass changeId")?;
    let event = &bypass["authorityEvent"];
    exact(
        event,
        &[
            "schema",
            "id",
            "decision",
            "approver",
            "evidenceIds",
            "issuedAt",
            "expiresAt",
        ],
        "authority event",
    )?;
    if event["schema"] != "code-intel-authority-event.v1" || event["decision"] != "approved" {
        return Err("authority event constants are invalid".to_string());
    }
    nonempty(&event["id"], "authority event id")?;
    exact(&event["approver"], &["id", "role"], "authority approver")?;
    nonempty(&event["approver"]["id"], "authority approver id")?;
    nonempty(&event["approver"]["role"], "authority approver role")?;
    if string_set(&event["evidenceIds"], "authority event evidenceIds")?.is_empty() {
        return Err("authority event evidenceIds must not be empty".to_string());
    }
    event["issuedAt"]
        .as_u64()
        .ok_or("authority event issuedAt is invalid")?;
    event["expiresAt"]
        .as_u64()
        .ok_or("authority event expiresAt is invalid")?;
    Ok(())
}

fn validate_policy(change: &Value, known: &BTreeSet<String>) -> Result<(), String> {
    let source = &change["valueSource"];
    let evidence = string_set(&source["evidenceIds"], "value source evidenceIds")?;
    if evidence.is_empty() || !evidence.is_subset(known) {
        return Err("value source evidence is missing or unknown".to_string());
    }
    let required = string_set(&change["requiredEvidenceIds"], "requiredEvidenceIds")?;
    if required.is_empty() || !required.is_subset(known) {
        return Err("required trace evidence is missing or unknown".to_string());
    }
    let source_kind = source["kind"]
        .as_str()
        .ok_or("value source kind is invalid")?;
    if !CURRENT_VALUE_SOURCES.contains(&source_kind) {
        return Err(format!("value source is not current: {source_kind}"));
    }
    let selected = change["firstSufficientRung"]
        .as_str()
        .and_then(|rung| RUNGS.iter().position(|candidate| *candidate == rung))
        .ok_or("first sufficient rung is invalid")?;
    let lower = change["lowerRungs"].as_array().unwrap();
    if lower.len() != selected {
        return Err("lower rung trace is incomplete or duplicated".to_string());
    }
    for (index, entry) in lower.iter().enumerate() {
        exact(entry, &["rung", "reason", "evidenceIds"], "lower rung")?;
        if entry["rung"] != RUNGS[index] {
            return Err("lower rungs must enumerate every earlier rung in order".to_string());
        }
        nonempty(&entry["reason"], "lower rung reason")?;
        let rung_evidence = string_set(&entry["evidenceIds"], "lower rung evidenceIds")?;
        if rung_evidence.is_empty() || !rung_evidence.is_subset(known) {
            return Err("lower rung evidence is missing or unknown".to_string());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_bypass(
    bypass: Option<&Value>,
    change_id: &str,
    change: &Value,
    evaluated_at: u64,
    known: &BTreeSet<String>,
    consumed: &BTreeSet<String>,
    duplicate_events: &BTreeSet<String>,
    newly_consumed: &mut BTreeSet<String>,
) -> Result<String, String> {
    let bypass = bypass.ok_or("policy rejection has no authority bypass")?;
    exact(bypass, &["changeId", "authorityEvent"], "bypass")?;
    if bypass["changeId"] != change_id {
        return Err("authority bypass is scoped to another change".to_string());
    }
    let event = &bypass["authorityEvent"];
    let event_id = event["id"]
        .as_str()
        .ok_or("authority event id is invalid")?;
    if duplicate_events.contains(event_id) || newly_consumed.contains(event_id) {
        return Err("authority event duplicate use is rejected".to_string());
    }
    let mut required = string_set(
        &change["valueSource"]["evidenceIds"],
        "value source evidenceIds",
    )?;
    required.extend(string_set(
        &change["requiredEvidenceIds"],
        "requiredEvidenceIds",
    )?);
    for lower_rung in change["lowerRungs"].as_array().unwrap() {
        required.extend(string_set(
            &lower_rung["evidenceIds"],
            "lower rung evidenceIds",
        )?);
    }
    let event_id =
        authority::validate_authority_event(event, evaluated_at, known, &required, consumed)?;
    newly_consumed.insert(event_id.clone());
    Ok(event_id)
}

fn removed_protections(change: &Value) -> Result<BTreeSet<String>, String> {
    let removed = string_set(&change["removedProtections"], "removedProtections")?;
    if removed
        .iter()
        .any(|protection| !NON_FILTERABLE.contains(&protection.as_str()))
    {
        return Err("removedProtections contains an unknown requirement".to_string());
    }
    Ok(removed)
}

fn exact(value: &Value, expected: &[&str], label: &str) -> Result<(), String> {
    exact_optional(value, expected, &[], label)
}

fn exact_optional(
    value: &Value,
    required: &[&str],
    optional: &[&str],
    label: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let required = required.iter().copied().collect::<BTreeSet<_>>();
    let allowed = required
        .iter()
        .copied()
        .chain(optional.iter().copied())
        .collect::<BTreeSet<_>>();
    if required.is_subset(&actual) && actual.is_subset(&allowed) {
        Ok(())
    } else {
        Err(format!("{label} fields are invalid"))
    }
}

fn string_set(value: &Value, label: &str) -> Result<BTreeSet<String>, String> {
    let values = value
        .as_array()
        .ok_or_else(|| format!("{label} must be an array"))?;
    let mut result = BTreeSet::new();
    for value in values {
        let item = value
            .as_str()
            .filter(|item| !item.is_empty())
            .ok_or_else(|| format!("{label} contains an invalid id"))?;
        if !result.insert(item.to_string()) {
            return Err(format!("{label} contains duplicate ids"));
        }
    }
    Ok(result)
}

fn duplicates<'a>(values: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value.to_string()).or_insert(0usize) += 1;
    }
    counts
        .into_iter()
        .filter_map(|(value, count)| (count > 1).then_some(value))
        .collect()
}

fn nonempty(value: &Value, label: &str) -> Result<(), String> {
    if value.as_str().is_some_and(|value| !value.is_empty()) {
        Ok(())
    } else {
        Err(format!("{label} is invalid"))
    }
}
