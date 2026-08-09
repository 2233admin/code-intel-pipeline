use std::fs;
use std::path::Path;

use serde_json::json;

use super::super::{assert_success, fixture, run_request, Temp};

#[test]
fn review_ready_packet_sorts_claims_and_keeps_source_locations_visible() {
    let temp = Temp::new();
    let (output, path) = run_request(&temp.0, "ready", &fixture("review-ready.request.json"));
    let packet = assert_success(&output);

    assert_eq!(packet["schema"], "code-intel-pr-evidence-packet.v1");
    assert_eq!(packet["decision"]["authority"], "advisory");
    assert_eq!(packet["decision"]["state"], "ready_for_human_merge_review");
    assert_eq!(packet["decision"]["hardGateStatus"], "passed");
    assert_eq!(
        packet["claims"]
            .as_array()
            .unwrap()
            .iter()
            .map(|claim| claim["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["binary-diff", "change-agenda", "runtime-ci"]
    );
    assert_eq!(
        packet["claims"][1]["locations"][0]["file"],
        "crates/code-intel-cli/src/change_agenda/mod.rs"
    );
    assert_eq!(packet["claims"][1]["locations"][0]["line"], 42);
    assert!(packet["packetId"]
        .as_str()
        .unwrap()
        .starts_with("pr-evidence-packet-v1:"));
    assert_eq!(packet["binding"]["sha256"].as_str().unwrap().len(), 64);

    let rendered = fs::read_to_string(path).unwrap();
    assert_eq!(
        rendered,
        String::from_utf8(output.stdout).unwrap().trim_end()
    );
}

#[test]
fn failed_gate_blocks_but_packet_assembly_succeeds() {
    let temp = Temp::new();
    let (output, _) = run_request(&temp.0, "blocked", &fixture("blocked.request.json"));
    let packet = assert_success(&output);

    assert_eq!(packet["decision"]["state"], "blocked");
    assert_eq!(packet["decision"]["hardGateStatus"], "failed");
    assert_eq!(packet["decision"]["reasons"][0]["code"], "gate_failed");
    assert_eq!(packet["decision"]["reasons"][0]["claimId"], "runtime-ci");
}

#[test]
fn unavailable_or_unknown_evidence_cannot_make_a_packet_review_ready() {
    let temp = Temp::new();
    let request = fixture("manual-review.request.json");
    let (output, _) = run_request(&temp.0, "manual", &request);
    let packet = assert_success(&output);

    assert_eq!(packet["decision"]["state"], "manual_review");
    assert_eq!(packet["decision"]["hardGateStatus"], "passed");
    assert_eq!(
        packet["decision"]["reasons"][0]["code"],
        "evidence_unavailable"
    );
    assert_ne!(packet["decision"]["state"], "ready_for_human_merge_review");

    let mut stale = request.clone();
    stale["claims"][1]["availability"] = json!("stale");
    let (output, _) = run_request(&temp.0, "stale", &stale);
    let packet = assert_success(&output);
    assert_eq!(packet["decision"]["state"], "manual_review");
    assert_eq!(packet["decision"]["reasons"][0]["code"], "evidence_stale");

    let mut missing_gate = request;
    missing_gate["claims"].as_array_mut().unwrap().remove(0);
    let (output, _) = run_request(&temp.0, "missing-gate", &missing_gate);
    let packet = assert_success(&output);
    assert_eq!(packet["decision"]["state"], "manual_review");
    assert_eq!(packet["decision"]["hardGateStatus"], "unknown");
    assert_eq!(
        packet["decision"]["reasons"][0]["code"],
        "missing_gate_evidence"
    );

    let mut unknown_gate = fixture("manual-review.request.json");
    unknown_gate["claims"][0]["status"] = json!("unknown");
    let (output, _) = run_request(&temp.0, "unknown-gate", &unknown_gate);
    let packet = assert_success(&output);
    assert_eq!(packet["decision"]["state"], "manual_review");
    assert_eq!(packet["decision"]["hardGateStatus"], "unknown");
    assert!(packet["decision"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason["code"] == "claim_unknown"));
}

#[test]
fn schemas_docs_and_registry_keep_the_packet_advisory_and_closed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "orchestration/schemas/code-intel-pr-evidence-request.v1.schema.json",
        "orchestration/schemas/code-intel-pr-evidence-packet.v1.schema.json",
    ] {
        let schema: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(relative)).unwrap()).unwrap();
        assert_eq!(schema["additionalProperties"], false, "{relative}");
    }
    let docs = fs::read_to_string(root.join("docs/pr-evidence-packet.md")).unwrap();
    assert!(docs.contains("does not grant merge authority"));
    assert!(docs.contains("never invents a location"));
    assert!(docs.contains("unknown"));

    let registry: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("orchestration/integrations.json")).unwrap())
            .unwrap();
    let entry = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "delivery.pr-evidence-packet")
        .expect("packet adapter is registered");
    assert_eq!(entry["effects"], json!(["repo_read", "local_write"]));
    assert!(entry["extensionPoint"]
        .as_str()
        .unwrap()
        .contains("does not independently verify"));
}
