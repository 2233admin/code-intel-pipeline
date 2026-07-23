use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[path = "../src/artifact_ref.rs"]
mod artifact_ref;
#[path = "../src/capability.rs"]
mod capability;
#[path = "../src/capability_inventory.rs"]
mod capability_inventory;
#[path = "../src/decision_port.rs"]
mod decision_port;
#[path = "../src/snapshot.rs"]
mod snapshot;
#[path = "../src/stable_artifact.rs"]
mod stable_artifact;

use decision_port::{
    DecisionExchange, DecisionRequestResponsePort, FileDecisionPort, InMemoryDecisionPort,
    NativeStructuredDecisionPort, PlainTextDecisionPort,
};

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/decision-port")
        .join(name);
    serde_json::from_slice(&fs::read(path).expect("fixture should be readable"))
        .expect("fixture should be valid JSON")
}

fn branches(result: &Value) -> Vec<(&str, &str)> {
    result["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| {
            (
                branch["branchId"].as_str().unwrap(),
                branch["status"].as_str().unwrap(),
            )
        })
        .collect()
}

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("code-intel-c06-{label}-{stamp}"))
}

#[test]
fn in_memory_port_correlates_one_gap_and_preserves_branch_locality() {
    let request = fixture("request.json");
    let response = fixture("valid-response.json");
    let mut port = InMemoryDecisionPort::default();
    port.supply_response(response).unwrap();
    let mut exchange = DecisionExchange::default();

    let result = exchange
        .advance(
            &request,
            &mut port,
            1_700_000_020,
            &["inventory", "publication"],
        )
        .unwrap();

    assert_eq!(result["status"], "resolved");
    assert_eq!(result["acceptedAnswer"]["optionId"], "accept-risk");
    assert_eq!(result["authorityProvenance"]["actorId"], "release-owner");
    assert_eq!(
        branches(&result),
        vec![("inventory", "continues"), ("publication", "ready")]
    );
    assert_eq!(result["effects"], json!([]));
}

#[test]
fn in_memory_port_resolves_after_an_asynchronous_pending_poll() {
    let request = fixture("request.json");
    let mut port = InMemoryDecisionPort::default();
    let mut exchange = DecisionExchange::default();
    let pending = exchange
        .advance(
            &request,
            &mut port,
            1_700_000_015,
            &["inventory", "publication"],
        )
        .unwrap();
    assert_eq!(pending["status"], "pending");

    port.supply_response(fixture("valid-response.json"))
        .unwrap();
    let resolved = exchange
        .advance(
            &request,
            &mut port,
            1_700_000_020,
            &["inventory", "publication"],
        )
        .unwrap();
    assert_eq!(resolved["status"], "resolved");
}

#[test]
fn pending_and_timeout_block_only_the_dependent_branch() {
    let request = fixture("request.json");
    let mut port = InMemoryDecisionPort::default();
    let mut exchange = DecisionExchange::default();
    let pending = exchange
        .advance(
            &request,
            &mut port,
            1_700_000_020,
            &["inventory", "publication"],
        )
        .unwrap();
    assert_eq!(pending["status"], "pending");
    assert_eq!(
        branches(&pending),
        vec![
            ("inventory", "continues"),
            ("publication", "blocked_pending_response")
        ]
    );

    let timeout = exchange
        .advance(
            &request,
            &mut port,
            1_700_000_101,
            &["inventory", "publication"],
        )
        .unwrap();
    assert_eq!(timeout["status"], "timeout");
    assert_eq!(
        branches(&timeout),
        vec![
            ("inventory", "continues"),
            ("publication", "blocked_timeout")
        ]
    );
}

#[test]
fn expired_processing_future_timestamp_and_wrong_correlation_are_rejected() {
    let request = fixture("request.json");

    let mut expired_port = InMemoryDecisionPort::default();
    expired_port
        .supply_response(fixture("valid-response.json"))
        .unwrap();
    assert!(DecisionExchange::default()
        .advance(
            &request,
            &mut expired_port,
            1_700_000_100,
            &["inventory", "publication"]
        )
        .unwrap_err()
        .to_string()
        .contains("expired"));

    let mut future = fixture("valid-response.json");
    future["timestamp"] = json!(1_700_000_021_u64);
    let mut future_port = InMemoryDecisionPort::default();
    future_port.supply_response(future).unwrap();
    assert!(DecisionExchange::default()
        .advance(
            &request,
            &mut future_port,
            1_700_000_020,
            &["inventory", "publication"]
        )
        .unwrap_err()
        .to_string()
        .contains("future"));

    let mut wrong = fixture("valid-response.json");
    wrong["correlationId"] = json!("other-correlation");
    let mut wrong_port = InMemoryDecisionPort::default();
    wrong_port.supply_response(wrong).unwrap();
    assert!(DecisionExchange::default()
        .advance(
            &request,
            &mut wrong_port,
            1_700_000_020,
            &["inventory", "publication"]
        )
        .unwrap_err()
        .to_string()
        .contains("correlation"));
}

#[test]
fn wrong_gap_actor_and_stale_evidence_fail_closed_without_consuming_request() {
    for fixture_name in [
        "wrong-gap-response.json",
        "wrong-actor-response.json",
        "stale-response.json",
    ] {
        let request = fixture("request.json");
        let mut port = InMemoryDecisionPort::default();
        port.supply_response(fixture(fixture_name)).unwrap();
        let mut exchange = DecisionExchange::default();
        assert!(exchange
            .advance(
                &request,
                &mut port,
                1_700_000_020,
                &["inventory", "publication"]
            )
            .is_err());

        port.supply_response(fixture("valid-response.json"))
            .unwrap();
        let recovered = exchange
            .advance(
                &request,
                &mut port,
                1_700_000_020,
                &["inventory", "publication"],
            )
            .unwrap();
        assert_eq!(recovered["status"], "resolved");
    }
}

#[test]
fn replay_and_cancellation_fail_closed() {
    let request = fixture("request.json");
    let mut port = InMemoryDecisionPort::default();
    port.supply_response(fixture("valid-response.json"))
        .unwrap();
    let mut exchange = DecisionExchange::default();
    exchange
        .advance(
            &request,
            &mut port,
            1_700_000_020,
            &["inventory", "publication"],
        )
        .unwrap();
    assert!(exchange
        .advance(
            &request,
            &mut port,
            1_700_000_021,
            &["inventory", "publication"]
        )
        .unwrap_err()
        .to_string()
        .contains("replay"));

    let mut cancel_port = InMemoryDecisionPort::default();
    cancel_port
        .supply_cancellation(fixture("cancellation.json"))
        .unwrap();
    let mut cancel_exchange = DecisionExchange::default();
    let cancelled = cancel_exchange
        .advance(
            &request,
            &mut cancel_port,
            1_700_000_020,
            &["inventory", "publication"],
        )
        .unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(
        branches(&cancelled),
        vec![
            ("inventory", "continues"),
            ("publication", "blocked_cancelled")
        ]
    );
    assert_eq!(cancelled["effects"], json!([]));
}

fn resolve_with<P: DecisionRequestResponsePort>(mut port: P) -> Value {
    let request = fixture("request.json");
    let mut exchange = DecisionExchange::default();
    exchange
        .advance(
            &request,
            &mut port,
            1_700_000_020,
            &["inventory", "publication"],
        )
        .unwrap()
}

#[test]
fn native_plain_text_and_file_adapters_are_substitutable() {
    let response = fixture("valid-response.json");

    let mut native = NativeStructuredDecisionPort::default();
    native.supply_response(response.clone()).unwrap();
    assert_eq!(resolve_with(native)["status"], "resolved");

    let mut plain = PlainTextDecisionPort::default();
    plain
        .supply_line("choice\tdecision-42\trisk-gap-7\taccept-risk\trelease-owner\trisk_acceptance\tlocal-cli\t1700000020")
        .unwrap();
    let plain_result = resolve_with(plain);
    assert_eq!(plain_result["status"], "resolved");

    let root = temp_root("file");
    fs::create_dir_all(&root).unwrap();
    let response_path = root.join("decision-42.response.json");
    fs::write(
        &response_path,
        serde_json::to_vec_pretty(&response).unwrap(),
    )
    .unwrap();
    let file_result = resolve_with(FileDecisionPort::new(root.clone()));
    fs::remove_dir_all(root).unwrap();
    assert_eq!(file_result["status"], "resolved");
}

#[test]
fn plain_text_request_is_lossless_for_every_authority_decision_field() {
    let request = fixture("request.json");
    let mut port = PlainTextDecisionPort::default();
    port.submit(&request).unwrap();
    let rendered = port.outbox().join("\n");
    for required in [
        "recommendation",
        "accept-risk",
        "rollback is proven",
        "options",
        "consequence",
        "evidenceRefs",
        "risk-report",
        "authorityNeeded",
        "risk_acceptance",
        "expiresAt",
        "1700000100",
    ] {
        assert!(
            rendered.contains(required),
            "plain-text request dropped {required}: {rendered}"
        );
    }
}

#[test]
fn malformed_request_and_free_form_response_are_closed_and_supported() {
    let mut invalid = fixture("request.json");
    invalid["questions"] = json!(["second question"]);
    let mut exchange = DecisionExchange::default();
    let mut port = InMemoryDecisionPort::default();
    assert!(exchange
        .advance(
            &invalid,
            &mut port,
            1_700_000_020,
            &["inventory", "publication"]
        )
        .is_err());

    let mut free_form = fixture("valid-response.json");
    free_form["answer"] = json!({"kind": "free-form", "text": "accept with a seven-day review"});
    let mut free_port = InMemoryDecisionPort::default();
    free_port.supply_response(free_form).unwrap();
    let result = resolve_with(free_port);
    assert_eq!(result["acceptedAnswer"]["kind"], "free-form");
}

#[test]
fn schemas_docs_and_registry_binding_exist() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in [
        "orchestration/schemas/code-intel-decision-request.v1.schema.json",
        "orchestration/schemas/code-intel-decision-response.v1.schema.json",
        "orchestration/schemas/code-intel-decision-cancellation.v1.schema.json",
        "orchestration/schemas/code-intel-decision-exchange-result.v1.schema.json",
        "docs/decision-request-response-port.md",
    ] {
        assert!(root.join(path).is_file(), "missing {path}");
    }
    let registry: Value =
        serde_json::from_slice(&fs::read(root.join("orchestration/integrations.json")).unwrap())
            .unwrap();
    let integration = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "decision.request-response-port")
        .expect("C06 production registry binding");
    assert_eq!(
        integration["entrypoint"],
        "crates/code-intel-cli/src/decision_port.rs"
    );
    assert_eq!(integration["kind"], "internal-port");
    for contract in [
        "orchestration/schemas/code-intel-decision-cancellation.v1.schema.json",
        "orchestration/schemas/code-intel-decision-exchange-result.v1.schema.json",
    ] {
        assert!(integration["artifactContract"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == contract));
    }
}

#[test]
fn production_cli_routes_through_the_native_structured_adapter() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixtures = root.join("tests/fixtures/decision-port");
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "decision",
            "request-response",
            "--request",
            fixtures.join("request.json").to_str().unwrap(),
            "--response",
            fixtures.join("valid-response.json").to_str().unwrap(),
            "--now",
            "1700000020",
            "--branch",
            "inventory",
            "--branch",
            "publication",
        ])
        .output()
        .expect("production CLI should execute");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "resolved");
    assert_eq!(result["correlationId"], "decision-42");
}

#[test]
fn production_cli_rejects_duplicate_keys_with_a_machine_envelope() {
    let root = temp_root("duplicate-json");
    fs::create_dir_all(&root).unwrap();
    let fixture_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/decision-port");
    let request_text = fs::read_to_string(fixture_root.join("request.json")).unwrap();
    let duplicate = request_text.replacen(
        "\"question\":",
        "\"question\": \"forged hidden question\",\n  \"question\":",
        1,
    );
    let request_path = root.join("duplicate-request.json");
    fs::write(&request_path, duplicate).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "decision",
            "request-response",
            "--request",
            request_path.to_str().unwrap(),
            "--response",
            fixture_root.join("valid-response.json").to_str().unwrap(),
            "--now",
            "1700000020",
            "--branch",
            "inventory",
            "--branch",
            "publication",
        ])
        .output()
        .unwrap();
    fs::remove_dir_all(root).unwrap();
    assert_eq!(output.status.code(), Some(65));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["status"], "rejected");
    assert!(envelope["diagnostics"][0]
        .as_str()
        .unwrap()
        .contains("duplicate"));
}

#[test]
fn file_response_and_cancellation_ingress_reject_duplicate_keys() {
    let request = fixture("request.json");
    let fixture_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/decision-port");

    let response_root = temp_root("duplicate-response");
    fs::create_dir_all(&response_root).unwrap();
    let response = fs::read_to_string(fixture_root.join("valid-response.json"))
        .unwrap()
        .replacen(
            "\"answer\":",
            "\"answer\": {\"kind\":\"choice\",\"optionId\":\"defer\"},\n  \"answer\":",
            1,
        );
    fs::write(response_root.join("decision-42.response.json"), response).unwrap();
    let mut response_port = FileDecisionPort::new(response_root.clone());
    assert!(DecisionExchange::default()
        .advance(
            &request,
            &mut response_port,
            1_700_000_020,
            &["inventory", "publication"]
        )
        .unwrap_err()
        .to_string()
        .contains("duplicate"));
    fs::remove_dir_all(response_root).unwrap();

    let cancel_root = temp_root("duplicate-cancel");
    fs::create_dir_all(&cancel_root).unwrap();
    let cancellation = fs::read_to_string(fixture_root.join("cancellation.json"))
        .unwrap()
        .replacen("\"reason\":", "\"reason\": \"forged\",\n  \"reason\":", 1);
    fs::write(cancel_root.join("decision-42.cancel.json"), cancellation).unwrap();
    let mut cancel_port = FileDecisionPort::new(cancel_root.clone());
    assert!(DecisionExchange::default()
        .advance(
            &request,
            &mut cancel_port,
            1_700_000_020,
            &["inventory", "publication"]
        )
        .unwrap_err()
        .to_string()
        .contains("duplicate"));
    fs::remove_dir_all(cancel_root).unwrap();
}

#[test]
fn production_cli_exit_policy_distinguishes_pending_timeout_and_cancelled() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixtures = root.join("tests/fixtures/decision-port");
    let request_path = fixtures.join("request.json").to_string_lossy().into_owned();
    let cancellation_path = fixtures
        .join("cancellation.json")
        .to_string_lossy()
        .into_owned();
    let cases = [
        (Vec::<String>::new(), "1700000020", 10, "pending"),
        (Vec::<String>::new(), "1700000100", 11, "timeout"),
        (
            vec!["--cancel".to_string(), cancellation_path],
            "1700000020",
            12,
            "cancelled",
        ),
    ];
    for (extra, now, exit, status) in cases {
        let mut args = vec![
            "decision".to_string(),
            "request-response".to_string(),
            "--request".to_string(),
            request_path.clone(),
        ];
        args.extend(extra);
        args.extend([
            "--now".to_string(),
            now.to_string(),
            "--branch".to_string(),
            "inventory".to_string(),
            "--branch".to_string(),
            "publication".to_string(),
        ]);
        let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(exit));
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["status"], status);
    }
}
