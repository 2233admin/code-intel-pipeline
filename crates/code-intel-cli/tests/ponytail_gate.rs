#[path = "../src/authority.rs"]
mod authority;
#[path = "../src/ponytail_gate.rs"]
mod ponytail_gate;

use std::collections::BTreeSet;
use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const NOW: u64 = 2_000;

fn lower(rung: &str) -> Value {
    json!({
        "rung": rung,
        "reason": format!("{rung} cannot satisfy the current contract"),
        "evidenceIds": ["ev-plan"]
    })
}

fn change(id: &str, kind: &str, operation: &str, source: &str, rung: &str) -> Value {
    let ladder = [
        "do_nothing",
        "repository_reuse",
        "standard_library",
        "platform_native",
        "installed_dependency",
        "one_liner",
        "smallest_local_implementation",
    ];
    let selected = ladder
        .iter()
        .position(|candidate| *candidate == rung)
        .unwrap();
    json!({
        "id": id,
        "kind": kind,
        "operation": operation,
        "valueSource": {
            "kind": source,
            "id": "C00",
            "evidenceIds": ["ev-plan"]
        },
        "requiredEvidenceIds": ["ev-protection-boundary"],
        "firstSufficientRung": rung,
        "lowerRungs": ladder[..selected].iter().map(|rung| lower(rung)).collect::<Vec<_>>(),
        "removedProtections": []
    })
}

fn request(mode: &str, changes: Vec<Value>) -> Value {
    json!({
        "schema": "code-intel-ponytail-gate-request.v1",
        "mode": mode,
        "evaluatedAt": NOW,
        "knownEvidenceIds": ["ev-plan", "ev-risk", "ev-approval", "ev-protection-boundary"],
        "consumedAuthorityEventIds": [],
        "changes": changes
    })
}

fn bypass(id: &str, expires_at: u64, evidence: &[&str]) -> Value {
    json!({
        "changeId": id,
        "authorityEvent": {
            "schema": "code-intel-authority-event.v1",
            "id": format!("authority-{id}"),
            "decision": "approved",
            "approver": {"id": "architect-1", "role": "engineering_authority"},
            "evidenceIds": evidence,
            "issuedAt": 1_900,
            "expiresAt": expires_at
        }
    })
}

#[test]
fn future_maybe_dependency_is_rejected_but_committed_repository_reuse_is_accepted() {
    let speculative = change(
        "dependency-cache",
        "dependency",
        "add",
        "future_maybe",
        "installed_dependency",
    );
    let reuse = change(
        "reuse-authority-validator",
        "abstraction",
        "reuse",
        "committed_engineering_plan_deliverable",
        "repository_reuse",
    );
    let result = ponytail_gate::evaluate(&request("enforce", vec![speculative, reuse])).unwrap();
    assert_eq!(result["wouldReject"], 1);
    assert_eq!(result["enforcedBlock"], true);
    assert_eq!(result["changes"][0]["status"], "rejected");
    assert_eq!(result["changes"][1]["status"], "accepted");
}

#[test]
fn report_only_retains_the_same_trace_without_blocking() {
    let speculative = change(
        "unused-process",
        "process",
        "add",
        "future_maybe",
        "smallest_local_implementation",
    );
    let result = ponytail_gate::evaluate(&request("report_only", vec![speculative])).unwrap();
    assert_eq!(result["wouldReject"], 1);
    assert_eq!(result["enforcedBlock"], false);
    assert_eq!(result["changes"][0]["status"], "rejected");
    assert_eq!(result["changes"][0]["change"]["id"], "unused-process");
}

#[test]
fn valid_current_sources_cover_addition_deletion_reuse_test_doc_and_process() {
    let cases = [
        (
            "artifact-add",
            "artifact",
            "add",
            "operator_requested_outcome",
        ),
        ("file-delete", "file", "delete", "approved_debt_reduction"),
        (
            "utility-reuse",
            "abstraction",
            "reuse",
            "committed_engineering_plan_deliverable",
        ),
        ("test-add", "test", "add", "verified_defect_or_risk"),
        (
            "doc-add",
            "documentation",
            "add",
            "required_contract_or_gate",
        ),
        ("process-add", "process", "add", "evidence_closing_spike"),
    ];
    for (id, kind, operation, source) in cases {
        let result = ponytail_gate::evaluate(&request(
            "enforce",
            vec![change(id, kind, operation, source, "repository_reuse")],
        ))
        .unwrap();
        assert_eq!(result["wouldReject"], 0, "{id}: {result}");
        assert_eq!(result["changes"][0]["status"], "accepted");
    }
}

#[test]
fn deleting_safety_evidence_or_verification_is_never_filterable_even_with_bypass() {
    for protection in ["safety", "evidence", "verification"] {
        let mut deletion = change(
            &format!("delete-{protection}"),
            "test",
            "delete",
            "approved_debt_reduction",
            "repository_reuse",
        );
        deletion["removedProtections"] = json!([protection]);
        deletion["bypass"] = bypass(
            deletion["id"].as_str().unwrap(),
            2_100,
            &["ev-plan", "ev-approval"],
        );
        let result = ponytail_gate::evaluate(&request("enforce", vec![deletion])).unwrap();
        assert_eq!(result["changes"][0]["status"], "rejected");
        assert_eq!(result["changes"][0]["authorityEventId"], Value::Null);
    }
}

#[test]
fn bypass_requires_a05_evidence_expiry_scope_and_single_use() {
    let mut allowed = change(
        "temporary-local-code",
        "file",
        "add",
        "future_maybe",
        "smallest_local_implementation",
    );
    allowed["bypass"] = bypass(
        "temporary-local-code",
        2_100,
        &["ev-plan", "ev-approval", "ev-protection-boundary"],
    );
    let result = ponytail_gate::evaluate(&request("enforce", vec![allowed.clone()])).unwrap();
    assert_eq!(result["changes"][0]["status"], "bypassed");
    assert_eq!(result["enforcedBlock"], false);
    assert_eq!(
        result["consumedAuthorityEventIds"],
        json!(["authority-temporary-local-code"])
    );

    let mut expired = allowed.clone();
    expired["bypass"] = bypass("temporary-local-code", 1_999, &["ev-plan", "ev-approval"]);
    assert_eq!(
        ponytail_gate::evaluate(&request("enforce", vec![expired])).unwrap()["changes"][0]
            ["status"],
        "rejected"
    );

    let mut missing_evidence = allowed.clone();
    missing_evidence["bypass"] = bypass("temporary-local-code", 2_100, &["ev-approval"]);
    assert_eq!(
        ponytail_gate::evaluate(&request("enforce", vec![missing_evidence])).unwrap()["changes"][0]
            ["status"],
        "rejected"
    );

    let mut replay_request = request("enforce", vec![allowed]);
    replay_request["consumedAuthorityEventIds"] = json!(["authority-temporary-local-code"]);
    assert_eq!(
        ponytail_gate::evaluate(&replay_request).unwrap()["changes"][0]["status"],
        "rejected"
    );
}

#[test]
fn bypass_must_cover_value_lower_rung_and_all_required_trace_evidence() {
    let mut missing_required = change(
        "missing-protection-evidence",
        "file",
        "add",
        "future_maybe",
        "repository_reuse",
    );
    missing_required["bypass"] = bypass(
        "missing-protection-evidence",
        2_100,
        &["ev-plan", "ev-approval"],
    );
    let result = ponytail_gate::evaluate(&request("enforce", vec![missing_required])).unwrap();
    assert_eq!(result["changes"][0]["status"], "rejected");
    assert_eq!(result["consumedAuthorityEventIds"], json!([]));

    let mut unknown_rung_evidence = change(
        "unknown-rung-evidence",
        "file",
        "add",
        "future_maybe",
        "repository_reuse",
    );
    unknown_rung_evidence["lowerRungs"][0]["evidenceIds"] = json!(["ev-rung-unknown"]);
    unknown_rung_evidence["bypass"] = bypass(
        "unknown-rung-evidence",
        2_100,
        &["ev-plan", "ev-approval", "ev-protection-boundary"],
    );
    let result = ponytail_gate::evaluate(&request("enforce", vec![unknown_rung_evidence])).unwrap();
    assert_eq!(result["changes"][0]["status"], "rejected");
    assert_eq!(result["consumedAuthorityEventIds"], json!([]));
    assert!(result["changes"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message
            .as_str()
            .unwrap()
            .contains("authority event evidence")));
}

#[test]
fn authority_cannot_bypass_a_malformed_or_unknown_protection_declaration() {
    let mut malformed = change(
        "malformed",
        "file",
        "add",
        "future_maybe",
        "repository_reuse",
    );
    malformed["removedProtections"] = json!(["unknown_guard"]);
    malformed["bypass"] = bypass("malformed", 2_100, &["ev-plan", "ev-approval"]);
    let error = ponytail_gate::evaluate(&request("enforce", vec![malformed])).unwrap_err();
    assert!(error.contains("removedProtections"), "{error}");
}

#[test]
fn schema_invalid_value_source_kind_and_first_rung_fail_before_bypass() {
    let mut invalid_source = change(
        "invalid-source",
        "file",
        "add",
        "not-a-schema-value-source",
        "repository_reuse",
    );
    invalid_source["bypass"] = bypass("invalid-source", 2_100, &["ev-plan", "ev-approval"]);
    let source_error =
        ponytail_gate::evaluate(&request("enforce", vec![invalid_source])).unwrap_err();
    assert!(source_error.contains("value source kind"), "{source_error}");

    let mut invalid_rung = change(
        "invalid-rung",
        "file",
        "add",
        "required_contract_or_gate",
        "repository_reuse",
    );
    invalid_rung["firstSufficientRung"] = json!("not-a-schema-rung");
    invalid_rung["bypass"] = bypass("invalid-rung", 2_100, &["ev-plan", "ev-approval"]);
    let rung_error = ponytail_gate::evaluate(&request("enforce", vec![invalid_rung])).unwrap_err();
    assert!(rung_error.contains("first sufficient rung"), "{rung_error}");
}

#[test]
fn first_sufficient_rung_requires_every_lower_rung_once_with_known_evidence() {
    let mut missing = change(
        "new-local-code",
        "file",
        "add",
        "required_contract_or_gate",
        "smallest_local_implementation",
    );
    missing["lowerRungs"].as_array_mut().unwrap().remove(2);
    let result = ponytail_gate::evaluate(&request("enforce", vec![missing])).unwrap();
    assert_eq!(result["changes"][0]["status"], "rejected");

    let mut duplicate = change(
        "duplicate-rung",
        "file",
        "add",
        "required_contract_or_gate",
        "repository_reuse",
    );
    duplicate["lowerRungs"]
        .as_array_mut()
        .unwrap()
        .push(lower("do_nothing"));
    let error = ponytail_gate::evaluate(&request("enforce", vec![duplicate])).unwrap_err();
    assert!(error.contains("duplicate lowerRungs"), "{error}");
}

#[test]
fn checked_contracts_are_closed_and_runtime_policy_matches_the_rule_table() {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../orchestration/schemas/code-intel-ponytail-gate.v1.schema.json"
    ))
    .unwrap();
    assert_eq!(schema["$defs"]["request"]["additionalProperties"], false);
    assert_eq!(schema["$defs"]["change"]["additionalProperties"], false);
    assert_eq!(schema["$defs"]["result"]["additionalProperties"], false);
    assert!(schema["$defs"]["change"]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("requiredEvidenceIds")));

    let checked: Value = serde_json::from_str(include_str!(
        "../../../orchestration/ponytail-gate-policy.v1.json"
    ))
    .unwrap();
    assert_eq!(checked, ponytail_gate::policy_document());
    assert_eq!(
        checked["allowedCurrentValueSources"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    let protected = checked["nonFilterableRequirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert!(BTreeSet::from(["safety", "evidence", "verification"]).is_subset(&protected));
}

#[test]
fn c00_self_trace_is_admitted_and_declares_no_dependency_addition() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/ponytail/c00-necessity-trace.json"
    ))
    .unwrap();
    let result = ponytail_gate::evaluate(&fixture).unwrap();
    assert_eq!(result["wouldReject"], 0, "{result}");
    assert!(fixture["changes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|change| change["kind"] != "dependency" || change["operation"] != "add"));
}

#[test]
fn production_cli_consumes_stdin_and_registry_declares_the_gate() {
    let mut speculative = change(
        "cli-speculative-file",
        "file",
        "add",
        "future_maybe",
        "repository_reuse",
    );
    speculative.as_object_mut().unwrap().remove("bypass");
    let input = serde_json::to_vec(&request("enforce", vec![speculative])).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["governance", "ponytail-gate", "--request", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{:?}", output);
    assert!(output.stderr.is_empty(), "{:?}", output);
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["schema"], "code-intel-ponytail-gate-result.v1");
    assert_eq!(result["enforcedBlock"], true);
    assert_eq!(result["changes"][0]["status"], "rejected");

    let registry: Value =
        serde_json::from_str(include_str!("../../../orchestration/integrations.json")).unwrap();
    let gate = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "governance.ponytail-gate")
        .expect("production registry must declare governance.ponytail-gate");
    assert_eq!(gate["entrypoint"], "crates/code-intel-cli/Cargo.toml");
    assert_eq!(
        gate["commands"]["evaluate"],
        "target/debug/code-intel.exe governance ponytail-gate --request <request.json|->"
    );

    let mut invalid = change(
        "invalid-cli-shape",
        "file",
        "add",
        "not-a-schema-value-source",
        "repository_reuse",
    );
    invalid["bypass"] = bypass(
        "invalid-cli-shape",
        2_100,
        &["ev-plan", "ev-approval", "ev-protection-boundary"],
    );
    let invalid_input = serde_json::to_vec(&request("enforce", vec![invalid])).unwrap();
    let mut invalid_child = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["governance", "ponytail-gate", "--request", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    invalid_child
        .stdin
        .take()
        .unwrap()
        .write_all(&invalid_input)
        .unwrap();
    let invalid_output = invalid_child.wait_with_output().unwrap();
    assert_eq!(invalid_output.status.code(), Some(65));
    assert!(invalid_output.stdout.is_empty());
    assert!(String::from_utf8(invalid_output.stderr)
        .unwrap()
        .contains("schema-invalid"));
}
