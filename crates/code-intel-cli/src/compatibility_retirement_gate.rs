use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::{AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::{
    normalized_retirement_call_path, retirement_portable_paths, VerifiedArtifact,
};
use crate::capability::sha256_hex;

#[path = "authority.rs"]
mod authority;

const MANIFEST_SCHEMA: &str = "code-intel-compatibility-retirement-manifest.v1";
const EVIDENCE_SCHEMA: &str = "code-intel-compatibility-retirement-evidence.v1";

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let evaluated_at = evaluated_at(request)?;
    let (manifest, evidence) = load_inputs(request, verified_inputs)?;
    let decision = evaluate(&manifest, &evidence, evaluated_at)?;
    publish_decision(out, decision)
}

fn evaluated_at(request: &Value) -> Result<u64, AdapterError> {
    let options = request["options"].as_object().ok_or_else(|| {
        AdapterError::InvalidOptions("retirement gate options must be an object".into())
    })?;
    if options.len() != 1 || !options.contains_key("evaluatedAt") {
        return Err(AdapterError::InvalidOptions(
            "compatibility.retirement-gate requires exactly options.evaluatedAt".into(),
        ));
    }
    request["options"]["evaluatedAt"].as_u64().ok_or_else(|| {
        AdapterError::InvalidOptions("retirement gate evaluatedAt must be an integer".into())
    })
}

fn load_inputs(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
) -> Result<(Value, BTreeMap<String, Value>), AdapterError> {
    let manifest_input = verified_inputs
        .iter()
        .filter(|input| input.artifact_schema() == MANIFEST_SCHEMA)
        .collect::<Vec<_>>();
    if manifest_input.len() != 1 {
        return Err(AdapterError::Contract(
            "retirement gate requires exactly one A03-verified retirement manifest".into(),
        ));
    }
    let manifest: Value = serde_json::from_slice(manifest_input[0].bytes())
        .map_err(|e| AdapterError::Contract(format!("parse retirement manifest: {e}")))?;
    if manifest["snapshotIdentity"] != request["snapshot"]["identity"] {
        return Err(AdapterError::Contract(
            "retirement manifest snapshot differs from the A01 request".into(),
        ));
    }
    let mut evidence = BTreeMap::<String, Value>::new();
    for input in verified_inputs {
        if input.artifact_schema() == MANIFEST_SCHEMA {
            continue;
        }
        if input.artifact_schema() != EVIDENCE_SCHEMA
            || input.artifact_type() != "compatibility.retirement-evidence"
        {
            return Err(AdapterError::Contract(
                "retirement gate accepts only its closed manifest and evidence contracts".into(),
            ));
        }
        let value: Value = serde_json::from_slice(input.bytes())
            .map_err(|e| AdapterError::Contract(format!("parse retirement evidence: {e}")))?;
        if value["snapshotIdentity"] != request["snapshot"]["identity"] {
            return Err(AdapterError::Contract(
                "retirement evidence payload snapshot differs from the A01 request".into(),
            ));
        }
        if evidence.insert(input.sha256().to_string(), value).is_some() {
            return Err(AdapterError::Contract(
                "duplicate retirement evidence digest".into(),
            ));
        }
    }
    Ok((manifest, evidence))
}

fn publish_decision(out: &Path, decision: Value) -> Result<AdapterOutput, AdapterError> {
    let bytes = serde_json::to_vec(&decision)
        .map_err(|e| AdapterError::Internal(format!("serialize retirement decision: {e}")))?;
    fs::create_dir(out).map_err(|e| AdapterError::Io(format!("create retirement staging: {e}")))?;
    fs::write(out.join("compatibility-retirement-decision.json"), &bytes)
        .map_err(|e| AdapterError::Io(format!("write retirement decision: {e}")))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-compatibility-retirement-decision.v1".into(),
            artifact_type: "compatibility.retirement-decision".into(),
            relative_path: "compatibility-retirement-decision.json".into(),
            bytes,
        }],
        observed_effects: vec!["local_write".into()],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn evaluate(
    manifest: &Value,
    evidence: &BTreeMap<String, Value>,
    evaluated_at: u64,
) -> Result<Value, AdapterError> {
    let subject = &manifest["approvalSubject"];
    let retirement_id = text(manifest, "retirementId", "manifest")?;
    let legacy = &subject["legacyBranch"];
    let legacy_capability = text(legacy, "capabilityId", "legacyBranch")?;
    let legacy_branch = text(legacy, "branchId", "legacyBranch")?;
    normalized_retirement_call_path(&legacy["callPath"], legacy_branch)
        .map_err(AdapterError::Contract)?;
    retirement_portable_paths(&legacy["affectedFiles"], "legacyBranch.affectedFiles")
        .map_err(AdapterError::Contract)?;
    let replacement = &subject["replacement"];
    let replacement_capability = text(replacement, "capabilityId", "replacement")?;
    let dependencies = strings(&replacement["dependencies"], "replacement.dependencies")?;
    let mut blockers = structural_blockers(
        subject,
        legacy_capability,
        replacement_capability,
        &dependencies,
    );
    let mut referenced = BTreeSet::new();
    check_core_evidence(
        subject,
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        &mut referenced,
        &mut blockers,
    )?;
    check_compatibility_and_usage(
        subject,
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        evaluated_at,
        &mut referenced,
        &mut blockers,
    )?;
    check_necessity_and_dependencies(
        subject,
        retirement_id,
        legacy_branch,
        replacement_capability,
        &dependencies,
        evidence,
        &mut referenced,
        &mut blockers,
    )?;
    let subject_sha = check_independent_approval(
        manifest,
        subject,
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        evaluated_at,
        &mut referenced,
        &mut blockers,
    )?;
    ensure_all_evidence_referenced(&referenced, evidence)?;
    Ok(decision_document(
        manifest,
        retirement_id,
        legacy,
        replacement,
        legacy_branch,
        subject_sha,
        referenced.len(),
        blockers,
    ))
}

fn structural_blockers(
    subject: &Value,
    legacy_capability: &str,
    replacement_capability: &str,
    dependencies: &BTreeSet<String>,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if legacy_capability == replacement_capability {
        blockers.push("replacement_is_legacy_capability".into());
    }
    if dependencies.contains(legacy_capability) || dependencies.contains(replacement_capability) {
        blockers.push("cyclic_replacement".into());
    }
    if subject["lineReductionEvidence"] != false {
        blockers.push("line_reduction_is_not_correctness_evidence".into());
    }
    blockers
}

#[allow(clippy::too_many_arguments)]
fn check_core_evidence(
    subject: &Value,
    retirement_id: &str,
    legacy_branch: &str,
    replacement_capability: &str,
    evidence: &BTreeMap<String, Value>,
    referenced: &mut BTreeSet<String>,
    blockers: &mut Vec<String>,
) -> Result<(), AdapterError> {
    let replacement = &subject["replacement"];
    let legacy = &subject["legacyBranch"];
    check(
        ref_at(replacement, "atomEvidence", "replacement")?,
        "replacement_atom",
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        referenced,
        blockers,
        |details| details["status"] == "production_ready" && details["outcome"] == "passed",
    )?;
    for (field, class) in [
        ("golden", "golden_parity"),
        ("contract", "contract_parity"),
        ("effects", "effect_parity"),
    ] {
        check(
            ref_at(&subject["parity"], field, "parity")?,
            class,
            retirement_id,
            legacy_branch,
            replacement_capability,
            evidence,
            referenced,
            blockers,
            |details| {
                details["outcome"] == "passed"
                    && details["assertionCount"].as_u64().unwrap_or(0) > 0
            },
        )?;
    }
    check(
        ref_at(subject, "registryReconciliation", "approvalSubject")?,
        "registry_reconciliation",
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        referenced,
        blockers,
        |d| {
            d["outcome"] == "passed"
                && d["registryParticipantId"] == legacy["registryParticipantId"]
                && d["replacementCapabilityId"] == replacement["capabilityId"]
                && matches!(d["status"].as_str(), Some("declared" | "deleted"))
        },
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_compatibility_and_usage(
    subject: &Value,
    retirement_id: &str,
    legacy_branch: &str,
    replacement_capability: &str,
    evidence: &BTreeMap<String, Value>,
    evaluated_at: u64,
    referenced: &mut BTreeSet<String>,
    blockers: &mut Vec<String>,
) -> Result<(), AdapterError> {
    let (compatibility_start, compatibility_end) = check_compatibility_window(
        subject,
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        evaluated_at,
        referenced,
        blockers,
    )?;
    check_rollback_and_usage(
        subject,
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        compatibility_start,
        compatibility_end,
        referenced,
        blockers,
    )
}

#[allow(clippy::too_many_arguments)]
fn check_compatibility_window(
    subject: &Value,
    retirement_id: &str,
    legacy_branch: &str,
    replacement_capability: &str,
    evidence: &BTreeMap<String, Value>,
    evaluated_at: u64,
    referenced: &mut BTreeSet<String>,
    blockers: &mut Vec<String>,
) -> Result<(u64, u64), AdapterError> {
    let compatibility_ref = ref_at(subject, "compatibilityWindow", "approvalSubject")?;
    check(
        compatibility_ref,
        "compatibility_window",
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        referenced,
        blockers,
        |d| {
            let start = d["startedAt"].as_u64().unwrap_or(u64::MAX);
            let end = d["observedThrough"].as_u64().unwrap_or(0);
            let days = d["minimumDays"].as_u64().unwrap_or(u64::MAX);
            let checked = d["checkedAt"].as_u64().unwrap_or(u64::MAX);
            let expires = d["expiresAt"].as_u64().unwrap_or(0);
            d["outcome"] == "passed"
                && end >= start
                && end - start >= days.saturating_mul(86_400)
                && checked <= evaluated_at
                && expires >= evaluated_at
                && expires >= checked
        },
    )?;
    let compatibility = evidence
        .get(text(
            compatibility_ref,
            "sha256",
            "compatibilityWindow ref",
        )?)
        .ok_or_else(|| AdapterError::Contract("compatibility window evidence is absent".into()))?;
    let compatibility_start = compatibility["details"]["startedAt"]
        .as_u64()
        .unwrap_or(u64::MAX);
    let compatibility_end = compatibility["details"]["observedThrough"]
        .as_u64()
        .unwrap_or(0);
    Ok((compatibility_start, compatibility_end))
}

#[allow(clippy::too_many_arguments)]
fn check_rollback_and_usage(
    subject: &Value,
    retirement_id: &str,
    legacy_branch: &str,
    replacement_capability: &str,
    evidence: &BTreeMap<String, Value>,
    compatibility_start: u64,
    compatibility_end: u64,
    referenced: &mut BTreeSet<String>,
    blockers: &mut Vec<String>,
) -> Result<(), AdapterError> {
    let rollback = &subject["rollback"];
    let rollback_command = text(rollback, "command", "rollback")?;
    check(
        ref_at(rollback, "executionEvidence", "rollback")?,
        "rollback_execution",
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        referenced,
        blockers,
        |d| {
            d["outcome"] == "passed"
                && d["exitCode"] == 0
                && d["command"] == rollback_command
                && d["executedAt"].as_u64().is_some()
        },
    )?;
    check(
        ref_at(subject, "usageObservation", "approvalSubject")?,
        "usage_observation",
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        referenced,
        blockers,
        |d| {
            let start = d["startedAt"].as_u64().unwrap_or(u64::MAX);
            let end = d["endedAt"].as_u64().unwrap_or(0);
            let total = d["totalInvocations"].as_u64().unwrap_or(0);
            let legacy_count = d["legacyInvocations"].as_u64().unwrap_or(u64::MAX);
            let replacement_count = d["replacementInvocations"].as_u64().unwrap_or(0);
            d["outcome"] == "passed"
                && start == compatibility_start
                && end == compatibility_end
                && end >= start
                && replacement_count > 0
                && legacy_count == 0
                && total == legacy_count.saturating_add(replacement_count)
        },
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_necessity_and_dependencies(
    subject: &Value,
    retirement_id: &str,
    legacy_branch: &str,
    replacement_capability: &str,
    dependencies: &BTreeSet<String>,
    evidence: &BTreeMap<String, Value>,
    referenced: &mut BTreeSet<String>,
    blockers: &mut Vec<String>,
) -> Result<(), AdapterError> {
    let necessity_trace_sha = sha256_hex(
        &serde_json::to_vec(&json!({
            "retirementId":retirement_id,
            "legacyBranchId":legacy_branch,
            "replacementCapabilityId":replacement_capability
        }))
        .expect("retirement trace identity serializes"),
    );
    check(
        ref_at(subject, "necessityEvidence", "approvalSubject")?,
        "c00_necessity",
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        referenced,
        blockers,
        |d| {
            d["outcome"] == "passed"
                && d["decision"] == "admit"
                && d["changeId"] == retirement_id
                && d["necessityTraceSha256"] == necessity_trace_sha
        },
    )?;
    let mut approved_dependencies = BTreeSet::new();
    for state in subject["dependencyStates"]
        .as_array()
        .ok_or_else(|| AdapterError::Contract("dependencyStates must be an array".into()))?
    {
        check(
            state,
            "dependency_approval",
            retirement_id,
            legacy_branch,
            replacement_capability,
            evidence,
            referenced,
            blockers,
            |d| {
                let dependency = d["dependencyId"].as_str().unwrap_or("");
                let valid = d["outcome"] == "passed"
                    && d["status"] == "approved"
                    && dependencies.contains(dependency);
                if valid {
                    approved_dependencies.insert(dependency.to_string());
                }
                valid
            },
        )?;
    }
    if approved_dependencies != *dependencies {
        blockers.push("dependency_approval_set_mismatch".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_independent_approval(
    manifest: &Value,
    subject: &Value,
    retirement_id: &str,
    legacy_branch: &str,
    replacement_capability: &str,
    evidence: &BTreeMap<String, Value>,
    evaluated_at: u64,
    referenced: &mut BTreeSet<String>,
    blockers: &mut Vec<String>,
) -> Result<String, AdapterError> {
    let legacy = &subject["legacyBranch"];
    let subject_bytes = serde_json::to_vec(subject)
        .map_err(|e| AdapterError::Internal(format!("serialize approval subject: {e}")))?;
    let subject_sha = sha256_hex(&subject_bytes);
    check(
        ref_at(manifest, "independentApproval", "manifest")?,
        "independent_approval",
        retirement_id,
        legacy_branch,
        replacement_capability,
        evidence,
        referenced,
        blockers,
        |d| {
            let reviewer = d["reviewer"].as_str().unwrap_or("");
            let event = &d["authorityEvent"];
            let known = BTreeSet::from([subject_sha.clone()]);
            let required = known.clone();
            let consumed = BTreeSet::new();
            d["outcome"] == "passed"
                && d["approved"] == true
                && d["subjectSha256"] == subject_sha
                && !reviewer.is_empty()
                && reviewer != legacy["owner"].as_str().unwrap_or("")
                && event.pointer("/approver/id").and_then(Value::as_str) == Some(reviewer)
                && authority::validate_signed_authority_event(
                    event,
                    evaluated_at,
                    &known,
                    &required,
                    &consumed,
                )
                .is_ok()
        },
    )?;
    Ok(subject_sha)
}

fn ensure_all_evidence_referenced(
    referenced: &BTreeSet<String>,
    evidence: &BTreeMap<String, Value>,
) -> Result<(), AdapterError> {
    if referenced.len() != evidence.len() {
        return Err(AdapterError::Contract(
            "retirement inputs contain unreferenced evidence".into(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decision_document(
    manifest: &Value,
    retirement_id: &str,
    legacy: &Value,
    replacement: &Value,
    legacy_branch: &str,
    subject_sha: String,
    evidence_count: usize,
    mut blockers: Vec<String>,
) -> Value {
    blockers.sort();
    blockers.dedup();
    let approved = blockers.is_empty();
    json!({
        "schema":"code-intel-compatibility-retirement-decision.v1",
        "snapshotIdentity":manifest["snapshotIdentity"],
        "retirementId":retirement_id,
        "legacyBranch":legacy,
        "replacement":replacement,
        "approvalSubjectSha256":subject_sha,
        "decision":if approved {"approved"} else {"blocked"},
        "blockers":blockers,
        "authorityBoundary":"approval_only_no_deletion_authority",
        "gainLedgerProjection":{
            "id":format!("retirement:{retirement_id}"),
            "status":if approved {"approved-for-ticket"} else {"blocked"},
            "gain":format!("Retire legacy branch {legacy_branch} only through a separate deletion ticket"),
            "evidenceCount":evidence_count
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn check<F: FnOnce(&Value) -> bool>(
    reference: &Value,
    class: &str,
    retirement_id: &str,
    legacy_branch: &str,
    replacement: &str,
    evidence: &BTreeMap<String, Value>,
    referenced: &mut BTreeSet<String>,
    blockers: &mut Vec<String>,
    predicate: F,
) -> Result<(), AdapterError> {
    let digest = text(reference, "sha256", "evidence ref")?;
    if !referenced.insert(digest.to_string()) {
        return Err(AdapterError::Contract(
            "one evidence artifact cannot satisfy multiple retirement requirements".into(),
        ));
    }
    let Some(value) = evidence.get(digest) else {
        return Err(AdapterError::Contract(format!(
            "referenced retirement evidence is absent: {digest}"
        )));
    };
    if reference["artifactSchema"] != EVIDENCE_SCHEMA
        || reference["type"] != "compatibility.retirement-evidence"
        || value["evidenceClass"] != class
        || value["retirementId"] != retirement_id
        || value["legacyBranchId"] != legacy_branch
        || value["replacementCapabilityId"] != replacement
    {
        blockers.push(format!("invalid_{class}"));
    } else if !predicate(&value["details"]) {
        blockers.push(format!("unproven_{class}"));
    }
    Ok(())
}

fn ref_at<'a>(value: &'a Value, field: &str, label: &str) -> Result<&'a Value, AdapterError> {
    let reference = &value[field];
    if !reference.is_object() {
        return Err(AdapterError::Contract(format!(
            "{label}.{field} is missing"
        )));
    }
    Ok(reference)
}

fn text<'a>(value: &'a Value, field: &str, label: &str) -> Result<&'a str, AdapterError> {
    value[field]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AdapterError::Contract(format!("{label}.{field} is missing")))
}

fn strings(value: &Value, label: &str) -> Result<BTreeSet<String>, AdapterError> {
    let values = value
        .as_array()
        .ok_or_else(|| AdapterError::Contract(format!("{label} must be an array")))?;
    let mut result = BTreeSet::new();
    for value in values {
        let id = value
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AdapterError::Contract(format!("{label} contains invalid id")))?;
        if !result.insert(id.to_string()) {
            return Err(AdapterError::Contract(format!(
                "{label} contains duplicate ids"
            )));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 3_000_000;

    fn signed_event(subject_sha: &str) -> Value {
        let mut event = json!({
            "schema":"code-intel-authority-event.v1",
            "id":"authority.retirement.ret-1",
            "decision":"approved",
            "approver":{"id":"code-intel-maintainers","role":"repository_governance"},
            "evidenceIds":[subject_sha],
            "issuedAt":NOW - 10,
            "expiresAt":NOW + 10
        });
        let digest = authority::authority_event_digest(&event).unwrap();
        event["attestation"] = json!({
            "scheme":"repository-governed-sha256-v1",
            "digest":digest
        });
        event
    }

    fn evidence(class: &str, details: Value) -> (Value, String, Value) {
        let value = json!({
            "schema":EVIDENCE_SCHEMA,"snapshotIdentity":"snap","id":format!("ev-{class}"),
            "evidenceClass":class,"retirementId":"ret-1","legacyBranchId":"legacy.branch",
            "replacementCapabilityId":"replacement.atom","details":details
        });
        let bytes = serde_json::to_vec(&value).unwrap();
        let sha = sha256_hex(&bytes);
        let reference = json!({
            "artifactSchema":EVIDENCE_SCHEMA,"type":"compatibility.retirement-evidence",
            "path":format!("evidence/{class}.json"),"sha256":sha,
            "consumedSnapshotIdentity":"snap"
        });
        (reference, sha, value)
    }

    fn fixture() -> (Value, BTreeMap<String, Value>) {
        let mut values = BTreeMap::new();
        let mut add = |class: &str, details: Value| {
            let (reference, sha, value) = evidence(class, details);
            values.insert(sha, value);
            reference
        };
        let atom = add(
            "replacement_atom",
            json!({"status":"production_ready","outcome":"passed"}),
        );
        let golden = add(
            "golden_parity",
            json!({"outcome":"passed","assertionCount":4}),
        );
        let contract = add(
            "contract_parity",
            json!({"outcome":"passed","assertionCount":3}),
        );
        let effects = add(
            "effect_parity",
            json!({"outcome":"passed","assertionCount":2}),
        );
        let registry = add(
            "registry_reconciliation",
            json!({"outcome":"passed","registryParticipantId":"legacy.registry","replacementCapabilityId":"replacement.atom","status":"declared"}),
        );
        let window = add(
            "compatibility_window",
            json!({"outcome":"passed","startedAt":1_000,"observedThrough":1_000+30*86_400,"minimumDays":30,"checkedAt":2_600_000,"expiresAt":NOW+100}),
        );
        let rollback = add(
            "rollback_execution",
            json!({"outcome":"passed","command":"restore legacy.branch","executedAt":9_000,"exitCode":0}),
        );
        let usage = add(
            "usage_observation",
            json!({"outcome":"passed","startedAt":1_000,"endedAt":1_000+30*86_400,"totalInvocations":20,"legacyInvocations":0,"replacementInvocations":20}),
        );
        let trace_sha = sha256_hex(
            &serde_json::to_vec(&json!({"retirementId":"ret-1","legacyBranchId":"legacy.branch","replacementCapabilityId":"replacement.atom"})).unwrap(),
        );
        let necessity = add(
            "c00_necessity",
            json!({"outcome":"passed","decision":"admit","changeId":"ret-1","necessityTraceSha256":trace_sha}),
        );
        let dependency = add(
            "dependency_approval",
            json!({"outcome":"passed","dependencyId":"D02","status":"approved","reviewer":"d02-reviewer"}),
        );
        let subject = json!({
            "legacyBranch":{"capabilityId":"legacy.capability","branchId":"legacy.branch","callPath":"run-code-intel.ps1::legacy.branch","affectedFiles":["run-code-intel.ps1"],"owner":"owner-team","registryParticipantId":"legacy.registry"},
            "replacement":{"capabilityId":"replacement.atom","implementationId":"replacement.atom.compat","dependencies":["D02"],"atomEvidence":atom},
            "parity":{"golden":golden,"contract":contract,"effects":effects},
            "registryReconciliation":registry,"compatibilityWindow":window,
            "rollback":{"command":"restore legacy.branch","executionEvidence":rollback},
            "usageObservation":usage,"necessityEvidence":necessity,
            "dependencyStates":[dependency],"lineReductionEvidence":false
        });
        let subject_sha = sha256_hex(&serde_json::to_vec(&subject).unwrap());
        let approval = add(
            "independent_approval",
            json!({"outcome":"passed","approved":true,"authorIndependent":true,"subjectSha256":subject_sha,"reviewer":"code-intel-maintainers","authorityEvent":signed_event(&subject_sha)}),
        );
        (
            json!({"schema":MANIFEST_SCHEMA,"snapshotIdentity":"snap","retirementId":"ret-1","approvalSubject":subject,"independentApproval":approval}),
            values,
        )
    }

    #[test]
    fn complete_content_bound_manifest_is_approved_without_deletion_authority() {
        let (manifest, evidence) = fixture();
        let decision = evaluate(&manifest, &evidence, NOW).unwrap();
        assert_eq!(decision["decision"], "approved");
        assert_eq!(decision["blockers"], json!([]));
        assert_eq!(
            decision["authorityBoundary"],
            "approval_only_no_deletion_authority"
        );
        assert_eq!(
            decision["gainLedgerProjection"]["status"],
            "approved-for-ticket"
        );
    }

    #[test]
    fn missing_rollback_execution_evidence_fails_closed() {
        let (manifest, mut evidence) = fixture();
        let digest = manifest["approvalSubject"]["rollback"]["executionEvidence"]["sha256"]
            .as_str()
            .unwrap()
            .to_string();
        evidence.remove(&digest);
        assert!(
            matches!(evaluate(&manifest, &evidence, NOW), Err(AdapterError::Contract(message)) if message.contains("evidence is absent"))
        );
    }

    #[test]
    fn pending_d02_and_cyclic_replacement_are_visible_blockers() {
        let (mut manifest, mut evidence) = fixture();
        let dependency = manifest["approvalSubject"]["dependencyStates"][0]["sha256"]
            .as_str()
            .unwrap()
            .to_string();
        evidence.get_mut(&dependency).unwrap()["details"]["status"] = json!("pending");
        manifest["approvalSubject"]["replacement"]["dependencies"] = json!(["legacy.capability"]);
        let decision = evaluate(&manifest, &evidence, NOW).unwrap();
        assert_eq!(decision["decision"], "blocked");
        assert!(decision["blockers"]
            .as_array()
            .unwrap()
            .contains(&json!("cyclic_replacement")));
        assert!(decision["blockers"]
            .as_array()
            .unwrap()
            .contains(&json!("unproven_dependency_approval")));
    }

    #[test]
    fn tampering_or_line_count_claim_cannot_reuse_prior_approval() {
        let (mut manifest, evidence) = fixture();
        manifest["approvalSubject"]["lineReductionEvidence"] = json!(true);
        let decision = evaluate(&manifest, &evidence, NOW).unwrap();
        assert_eq!(decision["decision"], "blocked");
        let blockers = decision["blockers"].as_array().unwrap();
        assert!(blockers.contains(&json!("line_reduction_is_not_correctness_evidence")));
        assert!(blockers.contains(&json!("unproven_independent_approval")));
    }

    fn evidence_digest(manifest: &Value, pointer: &str) -> String {
        manifest
            .pointer(pointer)
            .and_then(|value| value["sha256"].as_str())
            .unwrap()
            .to_string()
    }

    #[test]
    fn expired_compatibility_evidence_is_blocked_at_evaluation_time() {
        let (manifest, mut evidence) = fixture();
        let digest = evidence_digest(&manifest, "/approvalSubject/compatibilityWindow");
        evidence.get_mut(&digest).unwrap()["details"]["expiresAt"] = json!(NOW - 1);
        let decision = evaluate(&manifest, &evidence, NOW).unwrap();
        assert!(decision["blockers"]
            .as_array()
            .unwrap()
            .contains(&json!("unproven_compatibility_window")));
    }

    #[test]
    fn dependency_approval_must_match_every_replacement_dependency() {
        let (manifest, mut evidence) = fixture();
        let digest = evidence_digest(&manifest, "/approvalSubject/dependencyStates/0");
        evidence.get_mut(&digest).unwrap()["details"]["dependencyId"] = json!("unrelated");
        let decision = evaluate(&manifest, &evidence, NOW).unwrap();
        let blockers = decision["blockers"].as_array().unwrap();
        assert!(blockers.contains(&json!("unproven_dependency_approval")));
        assert!(blockers.contains(&json!("dependency_approval_set_mismatch")));
    }

    #[test]
    fn usage_requires_observed_replacement_calls_over_the_compatibility_window() {
        let (manifest, mut evidence) = fixture();
        let digest = evidence_digest(&manifest, "/approvalSubject/usageObservation");
        evidence.get_mut(&digest).unwrap()["details"]["replacementInvocations"] = json!(0);
        evidence.get_mut(&digest).unwrap()["details"]["totalInvocations"] = json!(0);
        let decision = evaluate(&manifest, &evidence, NOW).unwrap();
        assert!(decision["blockers"]
            .as_array()
            .unwrap()
            .contains(&json!("unproven_usage_observation")));
    }

    #[test]
    fn c00_necessity_trace_must_bind_the_retirement_record() {
        let (manifest, mut evidence) = fixture();
        let digest = evidence_digest(&manifest, "/approvalSubject/necessityEvidence");
        evidence.get_mut(&digest).unwrap()["details"]["changeId"] = json!("other-retirement");
        let decision = evaluate(&manifest, &evidence, NOW).unwrap();
        assert!(decision["blockers"]
            .as_array()
            .unwrap()
            .contains(&json!("unproven_c00_necessity")));
    }

    #[test]
    fn self_reported_independence_cannot_override_owner_or_authority_policy() {
        let (mut manifest, mut evidence) = fixture();
        manifest["approvalSubject"]["legacyBranch"]["owner"] = json!("code-intel-maintainers");
        let subject_sha = sha256_hex(&serde_json::to_vec(&manifest["approvalSubject"]).unwrap());
        let digest = evidence_digest(&manifest, "/independentApproval");
        let details = &mut evidence.get_mut(&digest).unwrap()["details"];
        details["subjectSha256"] = json!(subject_sha);
        details["authorIndependent"] = json!(true);
        details["authorityEvent"] = signed_event(details["subjectSha256"].as_str().unwrap());
        let decision = evaluate(&manifest, &evidence, NOW).unwrap();
        assert!(decision["blockers"]
            .as_array()
            .unwrap()
            .contains(&json!("unproven_independent_approval")));
    }
}
