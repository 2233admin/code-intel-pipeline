use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[path = "../src/admissibility.rs"]
mod admissibility;
#[path = "../src/artifact_ref.rs"]
mod artifact_ref;
#[path = "../src/capability.rs"]
mod capability;
#[path = "../src/capability_inventory.rs"]
mod capability_inventory;
#[path = "../src/sentrux_adapter.rs"]
mod sentrux_adapter;
#[path = "../src/snapshot.rs"]
mod snapshot;
#[path = "../src/stable_artifact.rs"]
mod stable_artifact;

const CURRENT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const IMPL: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
static SEQ: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-b03-{}-{nonce}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn descriptor(name: &str) -> Value {
    serde_json::from_str(match name {
        "complete" => include_str!("fixtures/sentrux-adapter/complete.json"),
        "partial" => include_str!("fixtures/sentrux-adapter/partial.json"),
        "unknown" => include_str!("fixtures/sentrux-adapter/unknown-kind.json"),
        "crashed" => include_str!("fixtures/sentrux-adapter/crashed.json"),
        _ => panic!("unknown fixture"),
    })
    .unwrap()
}

fn build_case(root: &Path, fixture: &Value) -> Value {
    let mut native = fixture.clone();
    native["schema"] = json!("code-intel-sentrux-provider-native.v1");
    native["implementation"] = json!({"id":"sentrux.shim.compat","version":"1.0.0","digest":IMPL});
    native["rollbackIdentity"] = json!("Invoke-SentruxAgentTool.ps1");
    native["sourceRevision"] = json!("revision-b03");
    native["expectedSnapshotIdentity"] = json!(CURRENT);
    native["sourceSnapshotIdentity"] = json!(CURRENT);
    native["collectedAt"] = json!(1940);
    native["observedAt"] = json!(1950);
    native["declaredEffects"] = json!(["repo_read", "local_write", "process_spawn"]);
    native["observedEffects"] = native["declaredEffects"].clone();
    native["payload"] = json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":"code-intel-evidence-payload.v1","type":"observed.evidence.payload","path":"payload.json","sha256":CURRENT,"consumedSnapshotIdentity":CURRENT});
    let first = sentrux_adapter::translate(&native, 2000, 100).unwrap();
    let payload = json!({"schema":"code-intel-evidence-payload.v1","data":{"structuralEvidence":{
        "schema":"code-intel-structural-evidence-payload.v1",
        "snapshotIdentity":CURRENT,
        "provider":first["port"]["provider"],
        "provenance":first["port"]["provenance"],
        "effects":first["port"]["effects"],
        "completeness":first["port"]["completeness"],
        "rules":first["port"]["rules"]
    }}});
    let bytes = serde_json::to_vec(&payload).unwrap();
    fs::write(root.join("payload.json"), &bytes).unwrap();
    native["payload"]["sha256"] = json!(capability::sha256_hex(&bytes));
    native
}

fn route(root: &Path, native: &Value) -> (i32, Value, String) {
    let request = root.join("native.json");
    fs::write(&request, serde_json::to_vec(native).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "provider",
            "sentrux-adapt",
            "--request",
            request.to_str().unwrap(),
            "--artifact-root",
            root.to_str().unwrap(),
            "--evaluated-at",
            "2000",
            "--max-age-seconds",
            "100",
        ])
        .output()
        .unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap();
    (
        output.status.code().unwrap(),
        value,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn complete_normalizes_every_authoritative_kind_and_passes_a04() {
    let root = Temp::new();
    let native = build_case(&root.0, &descriptor("complete"));
    let adapter = sentrux_adapter::translate(&native, 2000, 100).unwrap();
    assert_eq!(adapter["port"]["completeness"], "complete");
    assert_eq!(
        adapter["port"]["rules"].as_array().unwrap().len(),
        sentrux_adapter::AUTHORITATIVE_RULE_KINDS.len()
    );
    for kind in sentrux_adapter::AUTHORITATIVE_RULE_KINDS {
        assert!(adapter["port"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == kind));
    }
    let admitted =
        admissibility::validate_for_consumer(&adapter["evidence"]["request"], &root.0).unwrap();
    assert_eq!(admitted.result()["domainVerdict"], "observed");
    assert_eq!(admitted.result()["engineeringFacts"], json!([]));
    sentrux_adapter::validate_admitted_payload(admitted.payload(), &adapter).unwrap();
}

#[test]
fn partial_unknown_and_crashed_never_become_diagnosis() {
    for name in ["partial", "unknown", "crashed"] {
        let root = Temp::new();
        let native = build_case(&root.0, &descriptor(name));
        let adapter = sentrux_adapter::translate(&native, 2000, 100).unwrap();
        assert_eq!(adapter["port"]["completeness"], "partial", "{name}");
        let admitted =
            admissibility::validate_for_consumer(&adapter["evidence"]["request"], &root.0).unwrap();
        assert_eq!(admitted.result()["domainVerdict"], "unknown", "{name}");
        let (code, result, _) = route(&root.0, &native);
        assert_eq!(code, 0, "{name}");
        assert_eq!(
            result["adapter"]["port"]["diagnosisEligible"], false,
            "{name}"
        );
        assert_eq!(result["engineeringFacts"], json!([]), "{name}");
    }
}

#[test]
fn unknown_kind_is_fail_closed_even_when_provider_labels_it_pass() {
    let root = Temp::new();
    let native = build_case(&root.0, &descriptor("unknown"));
    let adapter = sentrux_adapter::translate(&native, 2000, 100).unwrap();
    let rule = &adapter["port"]["rules"][0];
    assert_eq!(rule["status"], "unsupported");
    assert_eq!(rule["verdict"], "unknown");
    assert_eq!(rule["failure"]["kind"], "domain_unknown");
}

#[test]
fn effect_mismatch_and_inconsistent_rule_are_rejected() {
    let root = Temp::new();
    let mut native = build_case(&root.0, &descriptor("complete"));
    native["observedEffects"] = json!(["repo_read"]);
    assert!(sentrux_adapter::translate(&native, 2000, 100)
        .unwrap_err()
        .contains("effects do not match"));
    native["observedEffects"] = native["declaredEffects"].clone();
    native["authoritativeRules"][0]["failure"] =
        json!({"kind":"domain_unknown","message":"bad relabel"});
    assert!(sentrux_adapter::translate(&native, 2000, 100)
        .unwrap_err()
        .contains("inconsistent"));
}

#[test]
fn complete_missing_known_kind_is_downgraded_and_payload_relabel_is_rejected() {
    let root = Temp::new();
    let mut fixture = descriptor("complete");
    fixture["authoritativeRules"].as_array_mut().unwrap().pop();
    let native = build_case(&root.0, &fixture);
    let adapter = sentrux_adapter::translate(&native, 2000, 100).unwrap();
    assert_eq!(adapter["port"]["completeness"], "partial");
    let admitted =
        admissibility::validate_for_consumer(&adapter["evidence"]["request"], &root.0).unwrap();
    assert_eq!(admitted.result()["domainVerdict"], "unknown");
    let mut payload = admitted.payload().clone();
    payload["data"]["structuralEvidence"]["completeness"] = json!("complete");
    assert!(sentrux_adapter::validate_admitted_payload(&payload, &adapter).is_err());
}

#[test]
fn complete_label_with_known_not_evaluated_rule_is_downgraded_before_diagnosis() {
    let root = Temp::new();
    let mut fixture = descriptor("complete");
    fixture["authoritativeRules"][0]["status"] = json!("not_evaluated");
    fixture["authoritativeRules"][0]["verdict"] = json!("unknown");
    fixture["authoritativeRules"][0]["failure"] =
        json!({"kind":"domain_unknown","message":"provider did not evaluate this rule"});
    let native = build_case(&root.0, &fixture);
    let adapter = sentrux_adapter::translate(&native, 2000, 100).unwrap();
    assert_eq!(adapter["port"]["completeness"], "partial");
    assert_eq!(
        adapter["evidence"]["request"]["observation"]["claimedComplete"],
        false
    );
    let (code, result, stderr) = route(&root.0, &native);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(result["admission"]["domainVerdict"], "unknown");
    assert_eq!(result["adapter"]["port"]["diagnosisEligible"], false);
}

#[test]
fn public_route_complete_is_eligible_but_never_emits_facts() {
    let root = Temp::new();
    let native = build_case(&root.0, &descriptor("complete"));
    let (code, result, stderr) = route(&root.0, &native);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(result["schema"], "code-intel-sentrux-route-result.v1");
    assert_eq!(result["adapter"]["port"]["diagnosisEligible"], true);
    assert_eq!(result["engineeringFacts"], json!([]));
    assert_eq!(
        result["admission"]["schema"],
        "code-intel-evidence-admissibility-result.v1"
    );
}

#[test]
fn secret_shaped_extra_input_is_rejected_without_echo() {
    let root = Temp::new();
    let mut native = build_case(&root.0, &descriptor("complete"));
    native["apiToken"] = json!("SENTINEL_DO_NOT_ECHO");
    let (code, result, stderr) = route(&root.0, &native);
    assert_eq!(code, 65);
    let rendered = format!("{result}{stderr}");
    assert!(!rendered.contains("SENTINEL_DO_NOT_ECHO"));
}
