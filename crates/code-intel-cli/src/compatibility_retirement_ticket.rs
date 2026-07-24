use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::adapter_contract::{AdapterArtifact, AdapterDomainVerdict, AdapterError, AdapterOutput};
use crate::artifact_ref::{
    normalized_retirement_call_path, retirement_portable_paths,
    validate_retirement_deletion_diff_value, VerifiedArtifact,
};
use crate::capability::{reject_duplicate_json_keys, sha256_hex};

const TEMPLATE_SCHEMA: &str = "code-intel-compatibility-retirement-ticket-template.v1";

pub(crate) fn execute(
    request: &Value,
    inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let options = request["options"]
        .as_object()
        .ok_or_else(|| AdapterError::InvalidOptions("ticket options must be an object".into()))?;
    if options.len() != 1 || !options.contains_key("evaluatedAt") {
        return Err(AdapterError::InvalidOptions(
            "ticket template requires exactly options.evaluatedAt".into(),
        ));
    }
    let evaluated_at = options["evaluatedAt"]
        .as_u64()
        .ok_or_else(|| AdapterError::InvalidOptions("evaluatedAt must be an integer".into()))?;
    let one = |schema: &str| -> Result<&VerifiedArtifact, AdapterError> {
        let found = inputs
            .iter()
            .filter(|v| v.artifact_schema() == schema)
            .collect::<Vec<_>>();
        if found.len() == 1 {
            Ok(found[0])
        } else {
            Err(AdapterError::Contract(format!(
                "ticket requires exactly one {schema}"
            )))
        }
    };
    if inputs.len() != 4 {
        return Err(AdapterError::Contract(
            "ticket requires exactly template, E00 manifest/decision, and deletion diff inputs"
                .into(),
        ));
    }
    let template = one(TEMPLATE_SCHEMA)?;
    let manifest = one("code-intel-compatibility-retirement-manifest.v1")?;
    let decision = one("code-intel-compatibility-retirement-decision.v1")?;
    let deletion = one("code-intel-compatibility-retirement-deletion-diff.v1")?;
    validate_template(template.bytes(), evaluated_at).map_err(AdapterError::Contract)?;
    let ticket: Value = serde_json::from_slice(template.bytes())
        .map_err(|e| AdapterError::Contract(format!("parse ticket: {e}")))?;
    let manifest_value: Value = serde_json::from_slice(manifest.bytes())
        .map_err(|e| AdapterError::Contract(format!("parse manifest: {e}")))?;
    let decision_value: Value = serde_json::from_slice(decision.bytes())
        .map_err(|e| AdapterError::Contract(format!("parse decision: {e}")))?;
    let deletion_value: Value = serde_json::from_slice(deletion.bytes())
        .map_err(|e| AdapterError::Contract(format!("parse deletion diff: {e}")))?;
    validate_retirement_deletion_diff_value(&deletion_value).map_err(AdapterError::Contract)?;
    let snapshot = &request["snapshot"]["identity"];
    if [
        &ticket["snapshotIdentity"],
        &manifest_value["snapshotIdentity"],
        &decision_value["snapshotIdentity"],
        &deletion_value["snapshotIdentity"],
    ]
    .iter()
    .any(|v| *v != snapshot)
    {
        return Err(AdapterError::Contract(
            "ticket inputs must share the A01 snapshot".into(),
        ));
    }
    if decision_value["decision"] != "approved"
        || decision_value["authorityBoundary"] != "approval_only_no_deletion_authority"
    {
        return Err(AdapterError::Contract(
            "ticket requires an approved E00 decision".into(),
        ));
    }
    ref_digest(
        &ticket["source"]["retirementDecision"],
        decision.sha256(),
        "E00 decision",
    )?;
    ref_digest(
        &ticket["source"]["retirementManifest"],
        manifest.sha256(),
        "E00 manifest",
    )?;
    ref_digest(
        &ticket["evidence"]["deletionDiff"],
        deletion.sha256(),
        "deletion diff",
    )?;
    let subject = &manifest_value["approvalSubject"];
    let subject_sha = sha256_hex(&serde_json::to_vec(subject).expect("JSON subject serializes"));
    if decision_value["approvalSubjectSha256"] != subject_sha {
        return Err(AdapterError::Contract(
            "E00 decision is not content-bound to the consumed manifest".into(),
        ));
    }
    let ticket_call_path = normalized_retirement_call_path(
        &ticket["legacyBranch"]["callPath"],
        ticket["legacyBranch"]["branchId"].as_str().unwrap_or(""),
    )
    .map_err(AdapterError::Contract)?;
    let approved_call_path = normalized_retirement_call_path(
        &subject["legacyBranch"]["callPath"],
        subject["legacyBranch"]["branchId"].as_str().unwrap_or(""),
    )
    .map_err(AdapterError::Contract)?;
    let ticket_files = retirement_portable_paths(&ticket["affectedFiles"], "ticket affectedFiles")
        .map_err(AdapterError::Contract)?;
    let approved_files = retirement_portable_paths(
        &subject["legacyBranch"]["affectedFiles"],
        "approved affectedFiles",
    )
    .map_err(AdapterError::Contract)?;
    if ticket["retirementId"] != manifest_value["retirementId"]
        || ticket["retirementId"] != decision_value["retirementId"]
        || ticket["legacyBranch"]["capabilityId"] != subject["legacyBranch"]["capabilityId"]
        || ticket["legacyBranch"]["branchId"] != subject["legacyBranch"]["branchId"]
        || ticket_call_path != approved_call_path
        || ticket_files != approved_files
        || ticket["replacement"]
            != serde_json::json!({"capabilityId":subject["replacement"]["capabilityId"],"dependencies":subject["replacement"]["dependencies"]})
    {
        return Err(AdapterError::Contract(
            "ticket branch/replacement differs from the approved E00 subject".into(),
        ));
    }
    for (ticket_field, subject_ref) in [
        ("golden", &subject["parity"]["golden"]),
        ("contract", &subject["parity"]["contract"]),
        ("effects", &subject["parity"]["effects"]),
        ("usage", &subject["usageObservation"]),
        (
            "rollbackRehearsal",
            &subject["rollback"]["executionEvidence"],
        ),
    ] {
        if ticket["evidence"][ticket_field] != *subject_ref {
            return Err(AdapterError::Contract(format!(
                "ticket {ticket_field} evidence differs from E00"
            )));
        }
    }
    if deletion_value["retirementId"] != ticket["retirementId"]
        || deletion_value["legacyBranchId"] != ticket["legacyBranch"]["branchId"]
        || retirement_portable_paths(&deletion_value["affectedFiles"], "deletion affectedFiles")
            .map_err(AdapterError::Contract)?
            != approved_files
        || deletion_value["deletionsOnly"] != true
    {
        return Err(AdapterError::Contract(
            "deletion diff is not scoped to the ticket".into(),
        ));
    }
    fs::create_dir(out).map_err(|e| AdapterError::Io(format!("create ticket staging: {e}")))?;
    fs::write(
        out.join("compatibility-retirement-ticket.json"),
        template.bytes(),
    )
    .map_err(|e| AdapterError::Io(format!("write ticket: {e}")))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: TEMPLATE_SCHEMA.into(),
            artifact_type: "compatibility.retirement-ticket-template".into(),
            relative_path: "compatibility-retirement-ticket.json".into(),
            bytes: template.bytes().to_vec(),
        }],
        observed_effects: vec!["local_write".into()],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn ref_digest(reference: &Value, actual: &str, label: &str) -> Result<(), AdapterError> {
    if reference["sha256"] == actual {
        Ok(())
    } else {
        Err(AdapterError::Contract(format!(
            "ticket {label} reference SHA-256 mismatch"
        )))
    }
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match lint_raw(raw) {
        Ok(()) => {
            println!("{{\"ok\":true}}");
            0
        }
        Err(message) => {
            eprintln!("error: {message}");
            65
        }
    }
}

fn lint_raw(raw: &[String]) -> Result<(), String> {
    if raw.first().map(String::as_str) != Some("lint") {
        return Err("expected compatibility retirement-ticket lint".into());
    }
    let mut ticket = None;
    let mut evaluated_at = None;
    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "--ticket" if i + 1 < raw.len() => ticket = Some(raw[i + 1].clone()),
            "--evaluated-at" if i + 1 < raw.len() => {
                evaluated_at = Some(
                    raw[i + 1]
                        .parse::<u64>()
                        .map_err(|_| "--evaluated-at must be an integer")?,
                )
            }
            other => return Err(format!("unknown retirement-ticket lint argument: {other}")),
        }
        i += 2;
    }
    let path = ticket.ok_or("--ticket is required")?;
    let evaluated_at = evaluated_at.ok_or("--evaluated-at is required")?;
    let bytes = fs::read(Path::new(&path)).map_err(|e| format!("read ticket: {e}"))?;
    validate_template(&bytes, evaluated_at)
}

pub(crate) fn validate_template(bytes: &[u8], evaluated_at: u64) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("ticket is not UTF-8: {e}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("ticket is not JSON: {e}"))?;
    validate_template_value(&value, evaluated_at)
}

pub(crate) fn validate_template_value(value: &Value, evaluated_at: u64) -> Result<(), String> {
    exact(
        value,
        &[
            "schema",
            "snapshotIdentity",
            "ticketId",
            "retirementId",
            "legacyBranch",
            "replacement",
            "affectedFiles",
            "evidence",
            "source",
            "owner",
            "verifier",
            "observationExpiry",
            "status",
            "authorityBoundary",
        ],
        "ticket",
    )?;
    if value["schema"] != TEMPLATE_SCHEMA
        || value["status"] != "draft"
        || value["authorityBoundary"] != "template_only_no_approval_or_deletion_authority"
    {
        return Err("ticket schema/status/authority boundary is invalid".into());
    }
    for key in [
        "snapshotIdentity",
        "ticketId",
        "retirementId",
        "owner",
        "verifier",
    ] {
        nonempty(&value[key], key)?;
    }
    if value["owner"] == value["verifier"] {
        return Err("owner and verifier must be independent".into());
    }
    if value["observationExpiry"]
        .as_u64()
        .is_none_or(|expiry| expiry < evaluated_at)
    {
        return Err("ticket observation evidence is expired".into());
    }
    exact(
        &value["legacyBranch"],
        &["capabilityId", "branchId", "callPath"],
        "legacyBranch",
    )?;
    for key in ["capabilityId", "branchId", "callPath"] {
        nonempty(&value["legacyBranch"][key], key)?;
    }
    normalized_retirement_call_path(
        &value["legacyBranch"]["callPath"],
        value["legacyBranch"]["branchId"].as_str().unwrap_or(""),
    )?;
    exact(
        &value["replacement"],
        &["capabilityId", "dependencies"],
        "replacement",
    )?;
    nonempty(
        &value["replacement"]["capabilityId"],
        "replacement.capabilityId",
    )?;
    unique_strings(
        &value["replacement"]["dependencies"],
        false,
        "replacement.dependencies",
    )?;
    retirement_portable_paths(&value["affectedFiles"], "affectedFiles")?;
    exact(
        &value["evidence"],
        &[
            "golden",
            "contract",
            "effects",
            "usage",
            "rollbackRehearsal",
            "deletionDiff",
        ],
        "evidence",
    )?;
    for key in [
        "golden",
        "contract",
        "effects",
        "usage",
        "rollbackRehearsal",
    ] {
        artifact_ref(
            &value["evidence"][key],
            "code-intel-compatibility-retirement-evidence.v1",
            "compatibility.retirement-evidence",
        )?;
    }
    artifact_ref(
        &value["evidence"]["deletionDiff"],
        "code-intel-compatibility-retirement-deletion-diff.v1",
        "compatibility.retirement-deletion-diff",
    )?;
    exact(
        &value["source"],
        &["retirementDecision", "retirementManifest"],
        "source",
    )?;
    artifact_ref(
        &value["source"]["retirementDecision"],
        "code-intel-compatibility-retirement-decision.v1",
        "compatibility.retirement-decision",
    )?;
    artifact_ref(
        &value["source"]["retirementManifest"],
        "code-intel-compatibility-retirement-manifest.v1",
        "compatibility.retirement-manifest",
    )?;
    Ok(())
}

fn artifact_ref(value: &Value, schema: &str, kind: &str) -> Result<(), String> {
    exact(
        value,
        &[
            "schema",
            "artifactSchema",
            "type",
            "path",
            "sha256",
            "consumedSnapshotIdentity",
        ],
        "Artifact Ref",
    )?;
    if value["schema"] != "code-intel-artifact-ref.v1"
        || value["artifactSchema"] != schema
        || value["type"] != kind
        || !value["path"]
            .as_str()
            .is_some_and(|v| !v.is_empty() && !v.contains('\\'))
        || !value["consumedSnapshotIdentity"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
        || !value["sha256"]
            .as_str()
            .is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("ticket Artifact Ref is invalid".into());
    }
    Ok(())
}

fn unique_strings(value: &Value, paths: bool, label: &str) -> Result<(), String> {
    let values = value
        .as_array()
        .ok_or_else(|| format!("{label} must be an array"))?;
    if values.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        let text = value
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("{label} contains an invalid value"))?;
        if paths
            && (text.contains('\\') || text.starts_with('/') || text.split('/').any(|p| p == ".."))
        {
            return Err(format!("{label} contains a non-portable path"));
        }
        if !seen.insert(text) {
            return Err(format!("{label} contains duplicates"));
        }
    }
    Ok(())
}

fn nonempty(value: &Value, label: &str) -> Result<(), String> {
    if value.as_str().is_some_and(|v| !v.trim().is_empty()) {
        Ok(())
    } else {
        Err(format!("{label} must be non-empty"))
    }
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
        Err(format!("{label} must use the closed contract"))
    }
}
