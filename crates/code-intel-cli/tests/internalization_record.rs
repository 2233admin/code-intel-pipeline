#[path = "../src/authority.rs"]
mod authority;
#[path = "../src/internalization_record.rs"]
mod internalization_record;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const NOW: u64 = 1_700_000_100;
static NEXT_SCHEMA_CHECK: AtomicUsize = AtomicUsize::new(1);
static SCHEMA_CHECK_LOCK: Mutex<()> = Mutex::new(());

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn complete() -> Value {
    serde_json::from_slice(
        &fs::read(root().join("tests/fixtures/internalization/complete.json")).unwrap(),
    )
    .unwrap()
}

fn advisory_candidate(name: &str) -> Value {
    serde_json::from_slice(
        &fs::read(
            root()
                .join("orchestration/internalization")
                .join(format!("{name}.json")),
        )
        .unwrap(),
    )
    .unwrap()
}

fn c03_measurements() -> Value {
    serde_json::from_slice(
        &fs::read(root().join("orchestration/internalization/c03-r05-r12-measurements.json"))
            .unwrap(),
    )
    .unwrap()
}

fn integration_exists(id: &str) -> bool {
    let registry: Value =
        serde_json::from_slice(&fs::read(root().join("orchestration/integrations.json")).unwrap())
            .unwrap();
    registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["id"] == id)
}

fn known(record: &Value) -> Vec<String> {
    internalization_record::record_evidence_ids(record)
        .unwrap()
        .into_iter()
        .collect()
}

#[test]
fn complete_record_enables_production_and_projects_reuse_notice_and_provenance() {
    let record = complete();
    let evaluation = internalization_record::evaluate_record(&record, NOW, &known(&record), &[])
        .expect("complete record must validate");
    assert_eq!(evaluation["researchAllowed"], true);
    assert_eq!(evaluation["productionEnabled"], true);
    assert_eq!(evaluation["status"], "production_enabled");
    assert_eq!(evaluation["diagnostics"], json!([]));
    assert_eq!(
        evaluation["consumedAuthorityEventId"],
        "authority-enable-example"
    );

    let reuse = internalization_record::project_reuse_record(&record, &evaluation).unwrap();
    assert_eq!(reuse["schema"], "code-intel-reuse-record.v1");
    assert_eq!(reuse["sourceRevision"], "0123456789abcdef");
    assert_eq!(reuse["adoptionRung"], "adapt");
    assert_eq!(reuse["economics"]["benefit"]["value"], 2);
    assert_eq!(
        reuse["compatibilityEvidence"]["evidenceIds"],
        json!(["ev-compatibility"])
    );
    assert_eq!(
        reuse["conformanceEvidence"]["evidenceIds"],
        json!(["ev-conformance"])
    );
    assert_eq!(
        reuse["assurance"]["securityEvidence"]["evidenceIds"],
        json!(["ev-security"])
    );
    assert_eq!(reuse["productionEnabled"], true);
    assert_eq!(reuse["engineeringFacts"], json!([]));

    let notice = internalization_record::project_notice_provenance(&record, &evaluation).unwrap();
    assert_eq!(notice["schema"], "code-intel-notice-provenance.v1");
    assert!(notice["noticeText"]
        .as_str()
        .unwrap()
        .contains("Apache-2.0"));
    assert_eq!(notice["provenance"]["revision"], "0123456789abcdef");
    assert_eq!(
        notice["provenance"]["ownedModifications"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn each_required_evidence_class_independently_blocks_production_but_not_research() {
    let cases = [
        "/adoption/necessityEvidence",
        "/adoption/compatibilityEvidence",
        "/adoption/conformanceEvidence",
        "/economics/benefitEvidence",
        "/economics/costEvidence",
        "/assurance/maintenanceEvidence",
        "/assurance/securityEvidence",
        "/update/evidence",
        "/rollback/evidence",
        "/exit/evidence",
        "/retirement/evidence",
        "/ownedModifications/0",
        "/lifecycle",
    ];
    for pointer in cases {
        let mut record = complete();
        record
            .pointer_mut(&format!("{pointer}/evidenceIds"))
            .unwrap()
            .as_array_mut()
            .unwrap()
            .clear();
        let evaluation =
            internalization_record::evaluate_record(&record, NOW, &known(&record), &[]).unwrap();
        assert_eq!(
            evaluation["researchAllowed"], true,
            "{pointer}: {evaluation}"
        );
        assert_eq!(
            evaluation["productionEnabled"], false,
            "{pointer}: {evaluation}"
        );
        assert!(
            evaluation["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str().unwrap().contains("evidence")),
            "{pointer}: {evaluation}"
        );
    }
}

#[test]
fn source_license_and_measurements_are_structural_requirements() {
    for (pointer, invalid) in [
        ("/subject/source/revision", json!("")),
        ("/subject/license/obligations", json!([])),
        ("/economics/benefit/value", json!(-1)),
        ("/economics/cost/unit", json!("")),
    ] {
        let mut record = complete();
        *record.pointer_mut(pointer).unwrap() = invalid;
        assert!(
            internalization_record::evaluate_record(&record, NOW, &[], &[]).is_err(),
            "{pointer} must be rejected structurally"
        );
    }
}

#[test]
fn operation_trace_rejects_empty_duplicate_and_malformed_digest_entries() {
    let valid = json!({
        "integrationId": "provider.example",
        "operation": "adapt",
        "command": "code-intel provider example-adapt",
        "implementationIdentity": {
            "providerId": "example.provider",
            "implementationId": "example.v1",
            "activation": "primary"
        },
        "source": {
            "path": "src/example.rs",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "conformance": {
            "path": "tests/example.rs",
            "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "testName": "example_conformance"
        }
    });

    let mut empty = complete();
    empty["operationTrace"] = json!([]);
    assert!(internalization_record::evaluate_record(&empty, NOW, &[], &[]).is_err());

    let mut duplicate = complete();
    duplicate["operationTrace"] = json!([valid.clone(), valid.clone()]);
    assert!(internalization_record::evaluate_record(&duplicate, NOW, &[], &[]).is_err());

    let mut malformed = complete();
    malformed["operationTrace"] = json!([valid]);
    malformed["operationTrace"][0]["source"]["sha256"] = json!("ABC123");
    assert!(internalization_record::evaluate_record(&malformed, NOW, &[], &[]).is_err());
}

#[test]
fn expired_unknown_and_update_due_evidence_fail_closed_for_production() {
    let mut expired = complete();
    expired["assurance"]["securityEvidence"]["expiresAt"] = json!(NOW - 1);
    let expired_eval =
        internalization_record::evaluate_record(&expired, NOW, &known(&expired), &[]).unwrap();
    assert_eq!(expired_eval["productionEnabled"], false);
    assert!(expired_eval["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d.as_str().unwrap().contains("expired")));

    let record = complete();
    let mut incomplete_known = known(&record);
    incomplete_known.retain(|id| id != "ev-conformance");
    let unknown_eval =
        internalization_record::evaluate_record(&record, NOW, &incomplete_known, &[]).unwrap();
    assert_eq!(unknown_eval["productionEnabled"], false);
    assert!(unknown_eval["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d.as_str().unwrap().contains("unknown")));

    let mut due = complete();
    due["update"]["nextCheckAt"] = json!(NOW - 1);
    let due_eval = internalization_record::evaluate_record(&due, NOW, &known(&due), &[]).unwrap();
    assert_eq!(due_eval["productionEnabled"], false);
    assert!(due_eval["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d.as_str().unwrap().contains("update check")));
}

#[test]
fn lifecycle_changes_require_a05_authority_and_reject_replay_or_illegal_edges() {
    let mut missing = complete();
    missing["lifecycle"]["authorityEvent"] = Value::Null;
    let missing_eval =
        internalization_record::evaluate_record(&missing, NOW, &known(&missing), &[]).unwrap();
    assert_eq!(missing_eval["productionEnabled"], false);
    assert!(missing_eval["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d.as_str().unwrap().contains("authority")));

    let record = complete();
    let replay = internalization_record::evaluate_record(
        &record,
        NOW,
        &known(&record),
        &["authority-enable-example".to_string()],
    )
    .unwrap();
    assert_eq!(replay["productionEnabled"], false);
    assert!(replay["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d.as_str().unwrap().contains("replay")));

    let mut expired_authority = complete();
    expired_authority["lifecycle"]["authorityEvent"]["expiresAt"] = json!(NOW - 1);
    let expired_authority_eval = internalization_record::evaluate_record(
        &expired_authority,
        NOW,
        &known(&expired_authority),
        &[],
    )
    .unwrap();
    assert_eq!(expired_authority_eval["productionEnabled"], false);
    assert!(expired_authority_eval["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d.as_str().unwrap().contains("expired")));

    let mut illegal = complete();
    illegal["lifecycle"]["previousStatus"] = json!("retired");
    let illegal_eval =
        internalization_record::evaluate_record(&illegal, NOW, &known(&illegal), &[]).unwrap();
    assert_eq!(illegal_eval["productionEnabled"], false);
    assert!(illegal_eval["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d.as_str().unwrap().contains("transition")));
}

#[test]
fn generic_v1_out_of_scope_event_remains_compatible_but_declared_repository_sign_off_is_required() {
    let mut generic = complete();
    generic["lifecycle"]["status"] = json!("out_of_scope");
    let accepted =
        internalization_record::evaluate_record(&generic, NOW, &known(&generic), &[]).unwrap();
    assert_eq!(accepted["lifecycleAccepted"], true);
    assert_eq!(
        accepted["consumedAuthorityEventId"],
        "authority-enable-example"
    );

    generic["authorityRequirements"] = json!({"repositoryGovernedAttestation":true});
    let rejected =
        internalization_record::evaluate_record(&generic, NOW, &known(&generic), &[]).unwrap();
    assert_eq!(rejected["lifecycleAccepted"], false);
    assert!(rejected["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str().unwrap().contains("attestation is required")));
}

#[test]
fn rejected_incomplete_and_expired_lifecycle_attempts_do_not_consume_authority() {
    let mut rejected = complete();
    rejected["lifecycle"]["previousStatus"] = json!("retired");

    let mut incomplete = complete();
    incomplete["adoption"]["conformanceEvidence"]["evidenceIds"] = json!([]);

    let mut expired = complete();
    expired["update"]["evidence"]["expiresAt"] = json!(NOW - 1);

    for (label, record) in [
        ("rejected", rejected),
        ("incomplete", incomplete),
        ("expired", expired),
    ] {
        let evaluation =
            internalization_record::evaluate_record(&record, NOW, &known(&record), &[]).unwrap();
        assert_eq!(evaluation["productionEnabled"], false, "{label}");
        assert_eq!(
            evaluation["consumedAuthorityEventId"],
            Value::Null,
            "{label}: {evaluation}"
        );
    }
}

#[test]
fn candidate_retirement_and_expired_update_or_retirement_are_retryable() {
    let mut cases = Vec::new();

    let mut candidate = complete();
    candidate["lifecycle"]["previousStatus"] = json!("production_enabled");
    candidate["lifecycle"]["status"] = json!("retired");
    candidate["retirement"]["status"] = json!("candidate");
    let mut completed = candidate.clone();
    completed["retirement"]["status"] = json!("completed");
    cases.push(("candidate retirement", candidate, completed));

    let mut expired_update = complete();
    expired_update["update"]["evidence"]["expiresAt"] = json!(NOW - 1);
    let mut renewed_update = expired_update.clone();
    renewed_update["update"]["evidence"]["expiresAt"] = json!(NOW + 100);
    cases.push(("expired update", expired_update, renewed_update));

    let mut expired_retirement = complete();
    expired_retirement["retirement"]["evidence"]["expiresAt"] = json!(NOW - 1);
    let mut renewed_retirement = expired_retirement.clone();
    renewed_retirement["retirement"]["evidence"]["expiresAt"] = json!(NOW + 100);
    cases.push(("expired retirement", expired_retirement, renewed_retirement));

    for (label, first, repaired) in cases {
        let first_evaluation =
            internalization_record::evaluate_record(&first, NOW, &known(&first), &[]).unwrap();
        assert_eq!(first_evaluation["productionEnabled"], false, "{label}");
        assert_eq!(
            first_evaluation["consumedAuthorityEventId"],
            Value::Null,
            "{label}: {first_evaluation}"
        );

        let retry = internalization_record::evaluate_record(&repaired, NOW, &known(&repaired), &[])
            .unwrap();
        assert_eq!(
            retry["consumedAuthorityEventId"], "authority-enable-example",
            "{label}: {retry}"
        );
        assert_eq!(retry["lifecycleAccepted"], true, "{label}: {retry}");
    }
}

#[test]
fn replacement_rollback_and_retirement_require_their_specific_closure() {
    let mut replacement = complete();
    replacement["lifecycle"]["previousStatus"] = json!("production_enabled");
    replacement["lifecycle"]["status"] = json!("replaced");
    replacement["lifecycle"]["replacementRecordId"] = Value::Null;
    let replacement_eval =
        internalization_record::evaluate_record(&replacement, NOW, &known(&replacement), &[])
            .unwrap();
    assert_eq!(replacement_eval["lifecycleAccepted"], false);

    let mut rollback = complete();
    rollback["lifecycle"]["previousStatus"] = json!("production_enabled");
    rollback["lifecycle"]["status"] = json!("rollback");
    rollback["rollback"]["evidence"]["evidenceIds"] = json!([]);
    let rollback_eval =
        internalization_record::evaluate_record(&rollback, NOW, &known(&rollback), &[]).unwrap();
    assert_eq!(rollback_eval["lifecycleAccepted"], false);

    let mut retired = complete();
    retired["lifecycle"]["previousStatus"] = json!("production_enabled");
    retired["lifecycle"]["status"] = json!("retired");
    retired["retirement"]["status"] = json!("candidate");
    let retired_eval =
        internalization_record::evaluate_record(&retired, NOW, &known(&retired), &[]).unwrap();
    assert_eq!(retired_eval["lifecycleAccepted"], false);
}

#[test]
fn rollback_replacement_retirement_and_audit_only_research_have_positive_paths() {
    for (status, replacement, retirement) in [
        ("rollback", Value::Null, "active"),
        ("replaced", json!("reuse-replacement"), "active"),
        ("retired", Value::Null, "completed"),
    ] {
        let mut record = complete();
        record["lifecycle"]["previousStatus"] = json!("production_enabled");
        record["lifecycle"]["status"] = json!(status);
        record["lifecycle"]["replacementRecordId"] = replacement;
        record["retirement"]["status"] = json!(retirement);
        let evaluation =
            internalization_record::evaluate_record(&record, NOW, &known(&record), &[]).unwrap();
        assert_eq!(
            evaluation["lifecycleAccepted"], true,
            "{status}: {evaluation}"
        );
        assert_eq!(evaluation["productionEnabled"], false);
    }

    let mut research = complete();
    research["lifecycle"]["previousStatus"] = Value::Null;
    research["lifecycle"]["status"] = json!("research");
    research["lifecycle"]["authorityEvent"] = Value::Null;
    research["adoption"]["conformanceEvidence"]["evidenceIds"] = json!([]);
    let evaluation =
        internalization_record::evaluate_record(&research, NOW, &known(&research), &[]).unwrap();
    assert_eq!(evaluation["researchAllowed"], true);
    assert_eq!(evaluation["lifecycleAccepted"], true);
    assert_eq!(evaluation["productionEnabled"], false);
}

#[test]
fn deterministic_store_rejects_duplicates_and_does_not_make_adoption_decisions() {
    let record = complete();
    let mut store = internalization_record::RecordStore::default();
    store.insert(record.clone()).unwrap();
    assert!(store
        .insert(record.clone())
        .unwrap_err()
        .contains("duplicate"));
    let projected = store
        .project_reuse_records(NOW, &known(&record), &[])
        .unwrap();
    assert_eq!(projected.len(), 1);
    let text = serde_json::to_string(&projected).unwrap();
    assert!(!text.contains("adoption_decision"));
    assert!(!text.contains("install"));
}

#[test]
fn projections_use_sealed_evaluations_bound_to_the_exact_record() {
    let record = complete();
    let evaluation =
        internalization_record::evaluate_record(&record, NOW, &known(&record), &[]).unwrap();
    assert_ne!(
        std::any::type_name_of_val(&evaluation),
        std::any::type_name::<Value>()
    );

    let mut changed_after_evaluation = record.clone();
    changed_after_evaluation["lifecycle"]["authorityEvent"]["id"] = json!("forged-authority-event");
    assert!(
        internalization_record::project_reuse_record(&changed_after_evaluation, &evaluation)
            .is_err()
    );
    assert!(internalization_record::project_notice_provenance(
        &changed_after_evaluation,
        &evaluation
    )
    .is_err());
}

fn assert_research_candidate(record: &Value, expected_id: &str) {
    assert_eq!(record["id"], expected_id);
    assert_eq!(record["lifecycle"]["status"], "research");
    assert!(record["subject"]["source"]["revision"]
        .as_str()
        .unwrap()
        .contains("unverified-upstream"));
    assert_eq!(record["subject"]["license"]["id"], "UNKNOWN-RESEARCH-ONLY");

    let evidence_ids = known(record);
    let gaps = evidence_ids
        .iter()
        .filter(|id| id.starts_with("gap:"))
        .cloned()
        .collect::<Vec<_>>();
    assert!(!gaps.is_empty());
    let admitted = evidence_ids
        .into_iter()
        .filter(|id| !id.starts_with("gap:"))
        .collect::<Vec<_>>();
    let evaluation =
        internalization_record::evaluate_record(record, 1_783_900_800, &admitted, &[]).unwrap();
    assert_eq!(evaluation["researchAllowed"], true);
    assert_eq!(evaluation["productionEnabled"], false);
    assert_eq!(evaluation["consumedAuthorityEventId"], Value::Null);
    let diagnostics = evaluation["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(diagnostics.contains("unknown evidence"));
    assert!(gaps.iter().any(|gap| gap.contains(":license")));
    assert!(gaps.iter().any(|gap| gap.contains(":upstream-revision")));

    let reuse = internalization_record::project_reuse_record(record, &evaluation).unwrap();
    let notice = internalization_record::project_notice_provenance(record, &evaluation).unwrap();
    assert_eq!(reuse["productionEnabled"], false);
    assert!(notice["noticeText"]
        .as_str()
        .unwrap()
        .contains("UNKNOWN-RESEARCH-ONLY"));
    assert_checked_schema(record, "code-intel-internalization-record.v1.schema.json");
    assert_checked_schema(&reuse, "code-intel-reuse-record.v1.schema.json");
    assert_checked_schema(&notice, "code-intel-notice-provenance.v1.schema.json");
}

fn integration(id: &str) -> Value {
    let registry: Value =
        serde_json::from_slice(&fs::read(root().join("orchestration/integrations.json")).unwrap())
            .unwrap();
    registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap_or_else(|| panic!("missing integration {id}"))
        .clone()
}

fn production_participant(id: &str) -> Value {
    let registry: Value =
        serde_json::from_slice(&fs::read(root().join("orchestration/integrations.json")).unwrap())
            .unwrap();
    registry["productionRegistry"]["participants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["capabilityId"] == id)
        .unwrap_or_else(|| panic!("missing production participant {id}"))
        .clone()
}

fn assert_recomputable_sha(record: &Value, relative: &str, label: &str) {
    let digest = recompute_sha(relative);
    assert!(
        record["subject"]["source"]["revision"]
            .as_str()
            .unwrap()
            .contains(&format!("{label}:{digest}")),
        "{relative} digest is not bound into the record"
    );
}

fn assert_research_record_projects(record: &Value, expected_id: &str) {
    assert_eq!(record["id"], expected_id);
    assert_eq!(record["lifecycle"]["status"], "research");
    assert_eq!(record["lifecycle"]["authorityEvent"], Value::Null);
    let evidence = known(record);
    let admitted = evidence
        .iter()
        .filter(|id| !id.starts_with("gap:"))
        .cloned()
        .collect::<Vec<_>>();
    assert!(evidence.iter().any(|id| id.starts_with("gap:")));
    let evaluation =
        internalization_record::evaluate_record(record, 1_783_900_800, &admitted, &[]).unwrap();
    assert_eq!(evaluation["researchAllowed"], true);
    assert_eq!(evaluation["productionEnabled"], false);
    assert_eq!(evaluation["consumedAuthorityEventId"], Value::Null);
    let reuse = internalization_record::project_reuse_record(record, &evaluation).unwrap();
    let notice = internalization_record::project_notice_provenance(record, &evaluation).unwrap();
    assert_eq!(reuse["productionEnabled"], false);
    assert_checked_schema(record, "code-intel-internalization-record.v1.schema.json");
    assert_checked_schema(&reuse, "code-intel-reuse-record.v1.schema.json");
    assert_checked_schema(&notice, "code-intel-notice-provenance.v1.schema.json");
}

fn assert_signed_out_of_scope_record_projects(record: &Value, expected_id: &str) {
    assert_eq!(record["id"], expected_id);
    assert_eq!(record["lifecycle"]["status"], "out_of_scope");
    assert_eq!(
        record["lifecycle"]["authorityEvent"]["approver"],
        json!({"id":"code-intel-maintainers","role":"repository_governance"})
    );
    assert_eq!(
        record["lifecycle"]["authorityEvent"]["attestation"]["scheme"],
        "repository-governed-sha256-v1"
    );
    assert!(
        record["lifecycle"]["authorityEvent"]["expiresAt"]
            .as_u64()
            .unwrap()
            > 1_783_900_800
    );

    let evidence = known(record);
    let evaluation =
        internalization_record::evaluate_record(record, 1_783_900_800, &evidence, &[]).unwrap();
    assert_eq!(evaluation["lifecycleAccepted"], true);
    assert_eq!(evaluation["productionEnabled"], false);
    assert_eq!(
        evaluation["consumedAuthorityEventId"],
        record["lifecycle"]["authorityEvent"]["id"]
    );

    let mut unsigned = record.clone();
    unsigned["lifecycle"]["authorityEvent"]
        .as_object_mut()
        .unwrap()
        .remove("attestation");
    let rejected =
        internalization_record::evaluate_record(&unsigned, 1_783_900_800, &known(&unsigned), &[])
            .unwrap();
    assert_eq!(rejected["lifecycleAccepted"], false);
    assert!(rejected["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str().unwrap().contains("attestation is required")));
    assert_checked_schema(record, "code-intel-internalization-record.v1.schema.json");
}

fn recompute_sha(relative: &str) -> String {
    let path = root().join(relative);
    let mut command = Command::new("pwsh");
    command.args([
        "-NoProfile",
        "-CommandWithArgs",
        "(Get-FileHash -LiteralPath $args[0] -Algorithm SHA256).Hash.ToLowerInvariant()",
        path.to_str().unwrap(),
    ]);
    let output = run_command_with_timeout(&mut command);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let digest = String::from_utf8(output.stdout).unwrap();
    let digest = digest.trim().to_string();
    assert_eq!(digest.len(), 64);
    digest
}

fn assert_operation_trace_exact(record: &Value, integration_ids: &[&str]) {
    let mut expected = BTreeMap::new();
    for integration_id in integration_ids {
        let entry = integration(integration_id);
        for (operation, command) in entry["commands"].as_object().unwrap() {
            expected.insert(
                ((*integration_id).to_string(), operation.clone()),
                command.as_str().unwrap().to_string(),
            );
        }
    }

    let traces = record["operationTrace"].as_array().unwrap();
    let mut actual = BTreeMap::new();
    for trace in traces {
        let integration_id = trace["integrationId"].as_str().unwrap();
        let operation = trace["operation"].as_str().unwrap();
        let key = (integration_id.to_string(), operation.to_string());
        assert!(
            actual.insert(key.clone(), trace).is_none(),
            "duplicate operation trace {key:?}"
        );

        let source_path = trace["source"]["path"].as_str().unwrap();
        assert_eq!(trace["source"]["sha256"], recompute_sha(source_path));
        let test_path = trace["conformance"]["path"].as_str().unwrap();
        assert_eq!(trace["conformance"]["sha256"], recompute_sha(test_path));
        let test_name = trace["conformance"]["testName"].as_str().unwrap();
        assert!(
            fs::read_to_string(root().join(test_path))
                .unwrap()
                .contains(test_name),
            "{test_path} does not contain named conformance {test_name}"
        );
    }

    assert_eq!(
        actual.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "operation trace coverage must exactly match registry commands"
    );
    for (key, command) in expected {
        assert_eq!(
            actual[&key]["command"].as_str().unwrap(),
            command,
            "trace command differs from registry for {key:?}"
        );
    }
}

#[test]
fn ticket_r01_repowise_record_traces_b01_operations_and_stays_research_only() {
    let record = advisory_candidate("repowise");
    assert_research_candidate(&record, "internalization.repowise-record");
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/src/repowise_adapter.rs",
        "local-adapter-sha256",
    );
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/tests/repowise_route.rs",
        "local-conformance-sha256",
    );
    assert_operation_trace_exact(&record, &["provider.repowise-adapt", "memory.repowise"]);
    let route = integration("provider.repowise-adapt");
    assert!(route["commands"]["adapt"]
        .as_str()
        .unwrap()
        .contains("provider repowise-adapt"));
    assert!(route["commands"]["facade"]
        .as_str()
        .unwrap()
        .contains("-RepowiseAdapterRequest"));
    assert_eq!(record["economics"]["benefit"]["value"], 2);
    let evidence = serde_json::to_string(&record).unwrap();
    assert!(evidence.contains("local:b01:production-operation-trace"));
    assert!(evidence.contains("gap:repowise:representative-value-measurement"));
    assert!(evidence.contains("gap:repowise:quota-data-handling-review"));
}

#[test]
fn ticket_r02_graph_record_keeps_internal_and_external_evidence_separate() {
    let record = advisory_candidate("graph");
    assert_research_candidate(&record, "internalization.graph-record");
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/src/graph_adapter.rs",
        "local-adapter-sha256",
    );
    assert_operation_trace_exact(
        &record,
        &[
            "provider.graph-adapt",
            "graph.code-intel-understand",
            "graph.understand-external",
        ],
    );
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/tests/graph_adapter.rs",
        "local-conformance-sha256",
    );
    let route = integration("provider.graph-adapt");
    assert!(route["commands"]["adapt"]
        .as_str()
        .unwrap()
        .contains("provider graph-adapt"));
    assert!(route["commands"]["facade"]
        .as_str()
        .unwrap()
        .contains("-GraphAdapterRequest"));
    let internal = integration("graph.code-intel-understand");
    let external = integration("graph.understand-external");
    assert_eq!(internal["kind"], "internal-rust-provider");
    assert_eq!(external["kind"], "compatibility-fallback");
    assert!(internal["commands"]["refresh"]
        .as_str()
        .unwrap()
        .contains("code-intel.exe graph"));
    assert!(external["commands"]["refresh"]
        .as_str()
        .unwrap()
        .starts_with("/understand"));
    let evidence = serde_json::to_string(&record).unwrap();
    assert!(evidence.contains("local:b02:internal-operation-trace"));
    assert!(evidence.contains("local:b02:external-fallback-operation-trace"));
    assert!(evidence.contains("gap:graph:external-upstream-conformance"));
}

#[test]
fn ticket_r03_sentrux_record_blocks_shim_retirement_on_windows_and_plugin_gaps() {
    let record = advisory_candidate("sentrux");
    assert_research_candidate(&record, "internalization.sentrux-record");
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/src/sentrux_adapter.rs",
        "local-adapter-sha256",
    );
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/tests/sentrux_adapter.rs",
        "local-conformance-sha256",
    );
    assert_operation_trace_exact(&record, &["provider.sentrux-adapt", "structure.sentrux"]);
    let route = integration("provider.sentrux-adapt");
    assert!(route["commands"]["adapt"]
        .as_str()
        .unwrap()
        .contains("provider sentrux-adapt"));
    assert!(route["commands"]["facade"]
        .as_str()
        .unwrap()
        .contains("-SentruxAdapterRequest"));
    let runtime = integration("structure.sentrux");
    assert!(runtime["commands"]["rustScan"]
        .as_str()
        .unwrap()
        .contains("code-intel.exe sentrux"));
    assert_eq!(
        runtime["commands"]["rustDsm"],
        "target/debug/code-intel.exe sentrux dsm <repo-path>"
    );
    assert_eq!(record["retirement"]["status"], "candidate");
    let retirement = serde_json::to_string(&record["retirement"]).unwrap();
    assert!(retirement.contains("gap:sentrux:upstream-windows-conformance"));
    assert!(retirement.contains("gap:sentrux:upstream-plugin-conformance"));
    let owned = record["ownedModifications"].as_array().unwrap();
    assert_eq!(owned.len(), 3);
    assert!(owned
        .iter()
        .any(|entry| { entry["path"] == "crates/code-intel-cli/src/sentrux_analysis.rs" }));
}

#[test]
fn ticket_r04_codenexus_record_traces_full_lite_swap_without_vendoring_semantics() {
    let record = advisory_candidate("codenexus");
    assert_research_candidate(&record, "internalization.codenexus-record");
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/src/codenexus_adapter.rs",
        "local-adapter-sha256",
    );
    assert_operation_trace_exact(
        &record,
        &[
            "provider.codenexus-adapt",
            "runtime.code-nexus-lite",
            "localization.codenexus-lite",
        ],
    );
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/tests/codenexus_adapter.rs",
        "local-conformance-sha256",
    );
    let route = integration("provider.codenexus-adapt");
    assert!(route["commands"]["adapt"]
        .as_str()
        .unwrap()
        .contains("provider codenexus-adapt"));
    assert!(route["commands"]["facade"]
        .as_str()
        .unwrap()
        .contains("-CodeNexusAdapterRequest"));
    assert_eq!(integration("runtime.code-nexus-lite")["required"], false);
    assert_eq!(
        integration("localization.codenexus-lite")["kind"],
        "compatibility-adapter"
    );
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    for provider_owned in [
        "process",
        "indexing",
        "storage",
        "retrieval",
        "impact semantics",
    ] {
        assert!(
            boundary.contains(provider_owned),
            "missing {provider_owned}"
        );
    }
    let evidence = serde_json::to_string(&record).unwrap();
    assert!(evidence.contains("local:b04:full-lite-swap-trace"));
    assert!(evidence.contains("gap:codenexus:measured-localization-value"));
    assert!(evidence.contains("gap:codenexus:full-provider-security-review"));
}

#[test]
fn ticket_r04_codenexus_compat_command_executes_and_writes_context() {
    let temp = std::env::temp_dir().join(format!(
        "code-intel-r04-compat-{}-{}",
        std::process::id(),
        NEXT_SCHEMA_CHECK.fetch_add(1, Ordering::Relaxed)
    ));
    let repo = temp.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("README.md"), "# R04 compat fixture\n").unwrap();

    let mut command = Command::new("pwsh");
    command.args([
        "-NoProfile",
        "-File",
        root().join("Invoke-CodeNexusLite.ps1").to_str().unwrap(),
        "-RepoPath",
        repo.to_str().unwrap(),
        "-Quiet",
    ]);
    let output = run_command_with_timeout(&mut command);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let context_path = repo.join(".code-intel/codenexus-context.json");
    let context: Value = serde_json::from_slice(&fs::read(&context_path).unwrap()).unwrap();
    assert_eq!(context["tool"], "codenexus-lite");
    let emitted_repo = PathBuf::from(context["repo"].as_str().unwrap());
    assert_eq!(
        fs::canonicalize(emitted_repo).unwrap(),
        fs::canonicalize(&repo).unwrap(),
        "compatibility context must identify the same repository across Windows short/long path aliases"
    );
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn ticket_r05_repomix_is_a_measured_reviewed_deletion_not_fake_production() {
    let record = advisory_candidate("repomix");
    assert_research_record_projects(&record, "internalization.repomix-record");
    assert_recomputable_sha(
        &record,
        "orchestration/internalization/c03-r05-r12-measurements.json",
        "measurement-sha256",
    );
    let participant = production_participant("pack.repomix");
    assert_eq!(participant["status"], "deleted");
    assert!(participant["reviewedDeletion"]["evidence"]
        .as_str()
        .unwrap()
        .contains("no Repomix executable"));
    let registry: Value =
        serde_json::from_slice(&fs::read(root().join("orchestration/integrations.json")).unwrap())
            .unwrap();
    assert!(!registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["id"] == "pack.repomix"));
    assert!(record.get("operationTrace").is_none());
    assert_eq!(record["retirement"]["status"], "completed");
    assert_eq!(record["economics"]["benefit"]["value"], 0);
    assert_eq!(record["economics"]["cost"]["value"], 0);
    let measurements = c03_measurements();
    assert_eq!(
        measurements["observations"]["r05Repomix"]["npmRegistryMetadataEntries"],
        3
    );
    assert_eq!(
        measurements["observations"]["r05Repomix"]["npmPackageTarballEntries"],
        3
    );
    assert_eq!(
        measurements["observations"]["r05Repomix"]["npmInstalledOrExtractedExecutableEntries"],
        0
    );
    let source = fs::read_to_string(root().join("run-code-intel.ps1")).unwrap();
    assert!(!source.contains("$repomixTool = Join-Path"));
    assert!(source.contains("Repomix production participation was reviewed and removed"));
}

#[test]
fn ticket_r06_native_record_binds_b08_parity_size_coverage_and_latency() {
    let record = advisory_candidate("native-code-evidence");
    assert_research_record_projects(&record, "internalization.native-code-evidence-record");
    assert_recomputable_sha(
        &record,
        "orchestration/internalization/c03-r05-r12-measurements.json",
        "measurement-sha256",
    );
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/src/native_code_evidence.rs",
        "local-runtime-sha256",
    );
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/tests/native_code_evidence.rs",
        "local-conformance-sha256",
    );
    assert_operation_trace_exact(&record, &["evidence.native-code"]);
    assert_eq!(record["economics"]["benefit"]["value"], 6);
    assert_eq!(record["economics"]["cost"]["value"], 27110);
    let text = serde_json::to_string(&record).unwrap();
    for fact in [
        "normalized-parity-artifacts-6",
        "artifact-refs-8",
        "fixture-files-2",
    ] {
        assert!(text.contains(fact), "missing measured fact {fact}");
    }
    let measurements = c03_measurements();
    let labeled = &measurements["observations"]["r06NativeCodeEvidence"]["labeledCorpus"];
    assert_eq!(labeled["samples"], 12);
    assert_eq!(labeled["truePositives"], 6);
    assert_eq!(labeled["falsePositives"], 2);
    assert_eq!(labeled["falseNegatives"], 2);
    assert_eq!(labeled["precision"], 0.75);
    assert_eq!(labeled["recall"], 0.75);
    assert_eq!(labeled["supportedCoverage"], 0.833333);
    assert!(labeled["elapsedMs"]
        .as_f64()
        .is_some_and(|value| value >= 0.0));
    assert!(text.contains("labeled-corpus-samples-12"));
    assert!(!text.contains("gap:native-code:representative-precision-recall"));
    assert!(text.contains("line-heuristic"));
}

#[test]
fn ticket_r07_cocoindex_is_a_reviewed_retirement_with_no_production_path() {
    let record = advisory_candidate("cocoindex");
    assert_research_record_projects(&record, "internalization.cocoindex-record");
    assert_recomputable_sha(
        &record,
        "orchestration/internalization/c03-r05-r12-measurements.json",
        "measurement-sha256",
    );
    assert!(record.get("operationTrace").is_none());
    assert_eq!(
        production_participant("evidence.cocoindex-code")["status"],
        "deleted"
    );
    assert!(!integration_exists("evidence.cocoindex-code"));
    assert_eq!(record["retirement"]["status"], "completed");
    assert_eq!(record["economics"]["benefit"]["value"], 0);
    assert_eq!(record["economics"]["cost"]["value"], 0);
    let text = serde_json::to_string(&record).unwrap();
    for boundary in [
        "production-semantic-invocations:0",
        "reviewed-deletion",
        "native-baseline-independent",
    ] {
        assert!(text.contains(boundary), "missing {boundary}");
    }
    let source = fs::read_to_string(root().join("run-code-intel.ps1")).unwrap();
    assert!(!source.contains("Get-JsonProperty $adapters \"cocoindex-code\""));
    assert!(!source.contains("Test-CommandAvailable $cocoCommand"));
}

#[test]
fn ticket_r08_github_research_failed_live_reproduction_is_reviewed_out() {
    let record = advisory_candidate("github-research");
    assert_research_record_projects(&record, "internalization.github-research-record");
    assert_recomputable_sha(
        &record,
        "orchestration/internalization/c03-r05-r12-measurements.json",
        "measurement-sha256",
    );
    assert!(record.get("operationTrace").is_none());
    assert_eq!(
        production_participant("research.github-solution")["status"],
        "deleted"
    );
    assert!(!integration_exists("research.github-solution"));
    assert_eq!(record["retirement"]["status"], "completed");
    assert_eq!(record["economics"]["benefit"]["value"], 0);
    let measurements = c03_measurements();
    let live = &measurements["observations"]["r08GitHubResearch"]["representativeLiveRun"];
    assert_eq!(live["status"], "manual_required");
    assert_eq!(live["candidates"], 0);
    assert_eq!(live["reproducible"], false);
    assert_eq!(live["externalWrites"], 0);
    assert_eq!(live["invocationExitCodes"], serde_json::json!([1, 1, 0, 0]));
    assert_eq!(live["returnedUrls"], serde_json::json!([]));
    assert_eq!(live["returnedSourceRevisions"], serde_json::json!([]));
    assert_recomputable_sha(
        &record,
        "orchestration/internalization/evidence/r08-live-20260714/github-solution-research.json",
        "run-artifact-sha256",
    );
    let text = serde_json::to_string(&record).unwrap();
    assert!(text.contains("invalid-query"));
    let source = fs::read_to_string(root().join("run-code-intel.ps1")).unwrap();
    assert!(!source.contains("$githubResearchScript"));
}

#[test]
fn ticket_r10_git_is_versioned_measured_and_read_only() {
    let record = advisory_candidate("git");
    assert_research_record_projects(&record, "internalization.git-record");
    assert_recomputable_sha(
        &record,
        "orchestration/internalization/c03-r05-r12-measurements.json",
        "measurement-sha256",
    );
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/src/snapshot.rs",
        "local-snapshot-source-sha256",
    );
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/tests/snapshot_identity.rs",
        "local-conformance-sha256",
    );
    assert_operation_trace_exact(&record, &["repository.snapshot-identity"]);
    assert_eq!(record["economics"]["benefit"]["value"], 12);
    assert_eq!(record["economics"]["cost"]["value"], 422.294);
    let text = serde_json::to_string(&record).unwrap();
    assert!(text.contains("installed-version:2.54.0.windows.1"));
    assert!(text.contains("rev-parse-samples-20"));
    assert!(text.contains("mutation-commands-0"));
    assert!(text.contains("local-license-sha256"));
    assert!(text.contains("alternate-vcs-contract-fixture"));
    assert!(text.contains("alternate-vcs-mismatch-fail-closed"));
    assert!(text.contains("alternate-vcs-adapter-actually-executed"));
    assert!(text.contains("rollback-to-git-or-unversioned"));
    assert!(!text.contains("gap:git:local-license-copy"));
    assert!(!text.contains("gap:git:alternate-vcs-port"));
}

#[test]
fn ticket_r12_unverified_greenfield_plugin_is_retired_from_production() {
    let record = advisory_candidate("greenfield");
    assert_research_record_projects(&record, "internalization.greenfield-record");
    assert_recomputable_sha(
        &record,
        "orchestration/internalization/c03-r05-r12-measurements.json",
        "measurement-sha256",
    );
    assert!(record.get("operationTrace").is_none());
    assert!(!integration_exists("spec.greenfield"));
    let registry = fs::read_to_string(root().join("orchestration/integrations.json")).unwrap();
    assert!(!registry.contains("\"capabilityId\": \"spec.greenfield\""));
    assert_eq!(record["retirement"]["status"], "completed");
    assert_eq!(record["economics"]["benefit"]["value"], 2);
    assert_eq!(record["economics"]["cost"]["value"], 2127.4285);
    let text = serde_json::to_string(&record).unwrap();
    assert!(text.contains("auto-analyze-without-flag-0"));
    assert!(text.contains("explicit-analyze-fixture-1"));
    assert!(text.contains("external-plugin-retired"));
    assert!(text.contains("pipeline-owned-plan-only"));
    assert!(text.contains("never grant specification or implementation authority"));
}

#[test]
fn ticket_r13_openspec_record_is_measured_removable_and_fail_closed() {
    let record = advisory_candidate("openspec");
    assert_research_candidate(&record, "internalization.openspec-record");
    let atom = fs::read_to_string(root().join("OpenSpec-Detector.ps1")).unwrap();
    assert_eq!(atom.matches("openspec-opsx").count(), 5);
    assert_eq!(record["economics"]["benefit"]["value"], 5);
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    assert!(boundary.contains("no openspec init"));
    assert!(record["rollback"]["strategy"]
        .as_str()
        .unwrap()
        .contains("delete this candidate record"));
}

#[test]
fn ticket_r14_spec_kit_record_requires_facts_and_has_no_auto_init_authority() {
    let record = advisory_candidate("spec-kit");
    assert_research_candidate(&record, "internalization.spec-kit-record");
    let atom = fs::read_to_string(root().join("OpenSpec-Detector.ps1")).unwrap();
    assert_eq!(atom.matches("spec-kit").count(), 8);
    assert_eq!(record["economics"]["benefit"]["value"], 8);
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    assert!(boundary.contains("no specify init"));
    assert!(record["exit"]["replacementCriteria"][0]
        .as_str()
        .unwrap()
        .contains("repository facts rather than tool name alone"));
}

#[test]
fn ticket_r15_matt_flow_record_traces_one_owned_branch_without_external_effects() {
    let record = advisory_candidate("matt-flow");
    assert_research_candidate(&record, "internalization.matt-flow-record");
    let atom = fs::read_to_string(root().join("OpenSpec-Detector.ps1")).unwrap();
    assert_eq!(atom.matches("stack = \"matt-flow\"").count(), 1);
    assert_eq!(record["economics"]["benefit"]["value"], 1);
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    assert!(boundary.contains("no issue creation"));
    assert!(boundary.contains("external-write authority"));
}

#[test]
fn ticket_r16_gstack_record_exposes_source_gap_and_no_execution_authority() {
    let record = advisory_candidate("gstack");
    assert_research_candidate(&record, "internalization.gstack-record");
    let atom = fs::read_to_string(root().join("OpenSpec-Detector.ps1")).unwrap();
    assert_eq!(atom.matches("stack = \"gstack\"").count(), 1);
    assert_eq!(record["economics"]["benefit"]["value"], 1);
    assert!(record["subject"]["source"]["uri"]
        .as_str()
        .unwrap()
        .starts_with("unverified-upstream:"));
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    assert!(boundary.contains("no skill execution"));
    assert!(boundary.contains("external-write authority"));
}

#[test]
fn ticket_r17_qiaomu_goal_record_traces_contract_and_stays_outside_scanner() {
    let record = advisory_candidate("qiaomu-goal");
    assert_research_candidate(&record, "internalization.qiaomu-goal-record");
    assert_eq!(record["economics"]["benefit"]["value"], 7);
    let intake = fs::read_to_string(root().join("docs/agent-goal-intake.md")).unwrap();
    for semantic in [
        "outcome",
        "verification",
        "constraints",
        "boundaries",
        "iteration policy",
        "completion evidence",
        "pause conditions",
    ] {
        assert!(intake.contains(semantic), "missing {semantic}");
    }
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    assert!(boundary.contains("no qiaomu runtime dependency"));
    assert!(boundary.contains("scanner invocation"));
    assert!(record["rollback"]["strategy"]
        .as_str()
        .unwrap()
        .contains("retaining the versioned pipeline-owned Agent Goal Intake contract"));
}

#[test]
fn ticket_r18_agent_loops_record_is_pinned_locally_and_removable_from_runtime() {
    let record = advisory_candidate("agent-loops");
    assert_research_candidate(&record, "internalization.agent-loops-record");
    assert_eq!(record["economics"]["benefit"]["value"], 3);
    let intake = fs::read_to_string(root().join("docs/agent-goal-intake.md")).unwrap();
    for pattern in ["`/goal`", "`/loop`", "`/schedule`"] {
        assert!(intake.contains(pattern), "missing {pattern}");
    }
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    assert!(boundary.contains("no awesome-agent-loops runtime dependency"));
    assert!(boundary.contains("scheduling authority"));
    assert!(record["exit"]["replacementCriteria"][2]
        .as_str()
        .unwrap()
        .contains("no scanner contract"));
}

#[test]
fn ticket_r19_metaharness_record_has_expiring_authority_gap_and_no_runtime() {
    let record = advisory_candidate("metaharness");
    assert_research_candidate(&record, "internalization.metaharness-record");
    assert_eq!(record["economics"]["benefit"]["value"], 6);
    let harness = fs::read_to_string(root().join("docs/harness-factory-reference.md")).unwrap();
    for concern in [
        "branded `npx code-intel`",
        "host configuration bundles",
        "release-gate command shapes",
        "SBOM, provenance",
        "static repository analysis",
        "package layout",
    ] {
        assert!(harness.contains(concern), "missing {concern}");
    }
    let lifecycle = serde_json::to_string(&record["lifecycle"]["evidenceIds"]).unwrap();
    assert!(lifecycle.contains("gap:metaharness:retention-authority"));
    assert_eq!(record["update"]["nextCheckAt"], 1_791_676_800u64);
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    assert!(boundary.contains("no MetaHarness runtime dependency"));
    assert!(record["retirement"]["triggers"][0]
        .as_str()
        .unwrap()
        .contains("no authority-approved retention decision"));
}

#[test]
fn ticket_r20_yao_record_rejects_local_doc_test_as_upstream_execution_proof() {
    let record = advisory_candidate("yao-meta-skill");
    assert_research_candidate(&record, "internalization.yao-meta-skill-record");
    assert_eq!(record["economics"]["benefit"]["value"], 8);
    let benchmark = fs::read_to_string(root().join("docs/skill-development-benchmark.md")).unwrap();
    for criterion in [
        "lean `SKILL.md` entrypoint",
        "near-neighbor exclusions",
        "multiple agent hosts",
        "eval fixtures",
        "failure library",
        "review reports",
        "release gates",
        "adoption drift",
    ] {
        assert!(benchmark.contains(criterion), "missing {criterion}");
    }
    assert!(benchmark.contains("This check does not run `yao-meta-skill`"));
    let evidence = serde_json::to_string(&record).unwrap();
    assert!(evidence.contains("gap:yao-meta-skill:behavioral-measurement"));
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    assert!(boundary.contains("no yao-meta-skill runtime dependency"));
    assert!(record["exit"]["replacementCriteria"][2]
        .as_str()
        .unwrap()
        .contains("beyond a local documentation test pass"));
}

#[test]
fn ticket_r22_mattpocock_skills_record_traces_each_retained_concept_and_exit() {
    let record = advisory_candidate("mattpocock-skills");
    assert_research_candidate(&record, "internalization.mattpocock-skills-record");
    assert_eq!(record["economics"]["benefit"]["value"], 4);
    assert_eq!(record["ownedModifications"].as_array().unwrap().len(), 4);
    let project_management =
        fs::read_to_string(root().join("docs/project-management-support.md")).unwrap();
    for concept in ["issue tracker", "triage", "domain documentation"] {
        assert!(project_management.contains(concept), "missing {concept}");
    }
    let workflow = fs::read_to_string(root().join("docs/openspec-detector.md")).unwrap();
    assert!(workflow.contains("idea→ship"));
    assert!(record["exit"]["strategy"]
        .as_str()
        .unwrap()
        .contains("retire the source reference independently"));
}

#[test]
fn ticket_r09_rg_record_traces_every_registered_production_operation() {
    let record = advisory_candidate("rg");
    assert_research_record_projects(&record, "internalization.rg-record");
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/src/capability_inventory.rs",
        "local-native-source-sha256",
    );
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/tests/capability_exec.rs",
        "local-conformance-sha256",
    );
    assert_operation_trace_exact(&record, &["inventory.rg"]);
    let exact = BTreeMap::from([
        (
            ("inventory.rg", "run"),
            "normalized_inventory_matches_real_legacy_runner_with_custom_exclude",
        ),
        (
            ("inventory.rg", "capabilityExec"),
            "inventory_rg_exec_emits_one_result_and_stable_real_rg_artifact",
        ),
    ]);
    for trace in record["operationTrace"].as_array().unwrap() {
        let key = (
            trace["integrationId"].as_str().unwrap(),
            trace["operation"].as_str().unwrap(),
        );
        assert_eq!(trace["conformance"]["testName"], exact[&key]);
    }
    assert_eq!(record["adoption"]["rung"], "invoke");
    assert!(serde_json::to_string(&record)
        .unwrap()
        .contains("gap:rg:replacement-command-drill"));
    assert!(serde_json::to_string(&record)
        .unwrap()
        .contains("gap:rg:latency-p50-p95-measurement"));
}

#[test]
fn ticket_r11_tree_sitter_v_record_does_not_claim_sentrux_or_grammar_ownership() {
    let record = advisory_candidate("tree-sitter-v");
    assert_research_record_projects(&record, "internalization.tree-sitter-v-record");
    assert_recomputable_sha(
        &record,
        "Install-SentruxVlangOverlay.ps1",
        "local-installer-sha256",
    );
    assert_recomputable_sha(
        &record,
        "overlays/sentrux/vlang/grammars/windows-x86_64.dll",
        "local-compiled-windows-x86_64-sha256",
    );
    assert_recomputable_sha(
        &record,
        "overlays/sentrux/vlang/src/sentrux_vlang_alias.c",
        "local-abi-alias-source-sha256",
    );
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    assert!(boundary.contains("Sentrux owns parser/plugin loading"));
    assert!(boundary.contains("upstream owns grammar implementation"));
    assert!(boundary.contains("neither ownership is transferred"));
    let record_text = serde_json::to_string(&record).unwrap();
    for gap in [
        "gap:tree-sitter-v:pinned-upstream-revision",
        "gap:tree-sitter-v:reproducible-build",
        "gap:tree-sitter-v:v-fixture-conformance",
        "gap:tree-sitter-v:abi-symbol-verification",
    ] {
        assert!(record_text.contains(gap), "missing {gap}");
    }
    assert!(!record_text.contains("gap:tree-sitter-v:compiled-artifact-digest"));
    assert!(record["ownedModifications"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["path"] == "overlays/sentrux/vlang/src/sentrux_vlang_alias.c"));
}

#[test]
fn ticket_r21_ponytail_record_binds_behavior_without_external_runtime_claim() {
    let record = advisory_candidate("ponytail");
    assert_research_record_projects(&record, "internalization.ponytail-record");
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/src/ponytail_gate.rs",
        "local-runtime-sha256",
    );
    assert_recomputable_sha(
        &record,
        "crates/code-intel-cli/tests/ponytail_gate.rs",
        "local-conformance-sha256",
    );
    let boundary = serde_json::to_string(&record["adoption"]["ownedBoundary"]).unwrap();
    assert!(boundary.contains("Value Filter"));
    assert!(boundary.contains("Necessity Trace"));
    assert!(boundary.contains("not an external runtime"));
    assert!(
        boundary.contains("not an external runtime")
            || boundary.contains("not an external Ponytail runtime")
    );
    assert!(record["rollback"]["strategy"]
        .as_str()
        .unwrap()
        .contains("report_only"));
    let test_count =
        fs::read_to_string(root().join("crates/code-intel-cli/tests/ponytail_gate.rs"))
            .unwrap()
            .lines()
            .filter(|line| line.trim() == "#[test]")
            .count();
    assert_eq!(test_count, 12);
    assert_eq!(record["economics"]["benefit"]["value"], test_count);
}

#[test]
fn ticket_r23_r24_r25_reference_records_add_no_external_write_ui_or_model_runtime() {
    for (name, expected_id) in [
        ("linear", "internalization.linear-record"),
        ("obsidian", "internalization.obsidian-record"),
        ("llm-wiki", "internalization.llm-wiki-record"),
    ] {
        let record = advisory_candidate(name);
        assert_signed_out_of_scope_record_projects(&record, expected_id);
        assert_recomputable_sha(
            &record,
            "docs/project-management-support.md",
            "local-policy-sha256",
        );
        assert_recomputable_sha(
            &record,
            "scripts/tests/test-project-management-support.ps1",
            "local-boundary-test-sha256",
        );
        assert_eq!(record["economics"]["benefit"]["value"], 0);
        let text = serde_json::to_string(&record).unwrap();
        assert!(text.contains("no-current-use"));
        assert!(text.contains("scope-authority"));
        assert!(text.contains("code-intel-authority-event.v1"));
        assert!(text.contains("repository-governed-sha256-v1"));
    }
    let linear = serde_json::to_string(&advisory_candidate("linear")).unwrap();
    assert!(linear.contains("no connector install"));
    assert!(linear.contains("no-external-write"));
    let obsidian = serde_json::to_string(&advisory_candidate("obsidian")).unwrap();
    assert!(
        obsidian.contains("no UI/plugin/vault dependency") || obsidian.contains("no-ui-runtime")
    );
    assert!(obsidian.contains("cannot replace scanner artifacts"));
    let wiki = serde_json::to_string(&advisory_candidate("llm-wiki")).unwrap();
    assert!(wiki.contains("no model/provider call"));
    assert!(wiki.contains("never Observed Evidence or Engineering Fact"));
}

#[test]
fn ticket_r26_my_code_machine_record_defers_big_bang_and_host_effects() {
    let record = advisory_candidate("my-code-machine");
    assert_signed_out_of_scope_record_projects(&record, "internalization.my-code-machine-record");
    assert_recomputable_sha(
        &record,
        "docs/adr/0001-merge-mcm-rust-unified.md",
        "local-merge-proposal-sha256",
    );
    assert_recomputable_sha(
        &record,
        "docs/adr/0010-tool-neutral-engineering-intelligence-core.md",
        "local-no-big-bang-adr-sha256",
    );
    assert!(!root().join("crates/machine").exists());
    assert!(!root().join("crates/sync").exists());
    let text = serde_json::to_string(&record).unwrap();
    for boundary in [
        "no-big-bang",
        "no-host-mutation-authority",
        "gap:my-code-machine:atom-parity",
        "gap:my-code-machine:effect-contracts",
        "gap:my-code-machine:adoption-authority",
    ] {
        assert!(text.contains(boundary), "missing {boundary}");
    }
}

#[test]
fn checked_schemas_accept_complete_record_and_both_projections() {
    let record = complete();
    let evaluation =
        internalization_record::evaluate_record(&record, NOW, &known(&record), &[]).unwrap();
    assert_checked_schema(&record, "code-intel-internalization-record.v1.schema.json");
    assert_checked_schema(
        &internalization_record::project_reuse_record(&record, &evaluation).unwrap(),
        "code-intel-reuse-record.v1.schema.json",
    );
    assert_checked_schema(
        &internalization_record::project_notice_provenance(&record, &evaluation).unwrap(),
        "code-intel-notice-provenance.v1.schema.json",
    );
}

#[test]
fn claude_code_merge_queue_record_traces_optional_adapter_and_keeps_promotion_human_only() {
    let record = advisory_candidate("claude-code-merge-queue");
    assert_research_record_projects(&record, "internalization.claude-code-merge-queue-record");
    assert_eq!(record["subject"]["license"]["id"], "MIT");
    assert!(record["subject"]["source"]["revision"]
        .as_str()
        .unwrap()
        .contains("e7a76958dbd3953b84f12abbc2e6bd755aafce53"));
    assert_recomputable_sha(
        &record,
        "Invoke-MultiAgentMergeQueue.ps1",
        "local-adapter-sha256",
    );
    assert_recomputable_sha(
        &record,
        "scripts/tests/test-multi-agent-merge-queue.ps1",
        "local-conformance-sha256",
    );
    assert_recomputable_sha(
        &record,
        "orchestration/multi-agent-merge-queue-policy.v1.json",
        "local-policy-sha256",
    );
    assert_recomputable_sha(
        &record,
        "docs/multi-agent-merge-queue.md",
        "local-doc-sha256",
    );
    assert_operation_trace_exact(&record, &["delivery.multi-agent-merge-queue"]);
    let integration = integration("delivery.multi-agent-merge-queue");
    assert_eq!(integration["required"], false);
    assert_eq!(integration["stage"], "landing_coordination");
    assert!(integration["commands"].get("promote").is_none());
    assert!(integration["commands"]["land"]
        .as_str()
        .unwrap()
        .contains("-AllowNetworkPush"));
    let evidence = serde_json::to_string(&record).unwrap();
    assert!(evidence.contains("production promotion remains human-only"));
    assert!(evidence.contains("gap:merge-queue:representative-cluster-throughput"));
}

fn assert_checked_schema(value: &Value, schema: &str) {
    // PowerShell's Test-Json schema engine is not reliable under concurrent
    // invocations from this parallel test binary. Keep the external validator
    // serialized while preserving unique payload paths for failure diagnosis.
    let _guard = SCHEMA_CHECK_LOCK.lock().unwrap();
    let path = std::env::temp_dir().join(format!(
        "code-intel-c03-schema-{}-{}.json",
        std::process::id(),
        NEXT_SCHEMA_CHECK.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    let schema_path = root().join("orchestration/schemas").join(schema);
    let mut command = Command::new("pwsh");
    command.args([
        "-NoProfile",
        "-CommandWithArgs",
        "if (-not ((Get-Content -Raw -LiteralPath $args[0]) | Test-Json -SchemaFile $args[1])) { exit 1 }",
        path.to_str().unwrap(),
        schema_path.to_str().unwrap(),
    ]);
    let output = run_command_with_timeout(&mut command);
    fs::remove_file(path).unwrap();
    assert!(
        output.status.success(),
        "{schema}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_command_with_timeout(command: &mut Command) -> std::process::Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "subprocess timed out; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
