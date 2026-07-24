use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[path = "../src/adapter_contract.rs"]
mod adapter_contract;
#[path = "../src/admissibility.rs"]
mod admissibility;
#[path = "../src/artifact_ref.rs"]
mod artifact_ref;
#[path = "../src/capability.rs"]
mod capability;
#[path = "../src/capability_inventory.rs"]
mod capability_inventory;
#[path = "../src/graph_adapter.rs"]
mod graph_adapter;
#[path = "../src/snapshot.rs"]
mod snapshot;
#[path = "../src/stable_artifact.rs"]
mod stable_artifact;

const CURRENT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const WRONG: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const IMPLEMENTATION_DIGEST: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-b02-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
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
    let text = match name {
        "internal-current" => include_str!("fixtures/graph-adapter/internal-current.json"),
        "external-current" => include_str!("fixtures/graph-adapter/external-current.json"),
        "internal-partial" => include_str!("fixtures/graph-adapter/internal-partial.json"),
        "internal-missing" => include_str!("fixtures/graph-adapter/internal-missing.json"),
        _ => panic!("unknown fixture {name}"),
    };
    serde_json::from_str(text).unwrap()
}

fn graph_document() -> Value {
    json!({
        "schema":"code-intel-understand-graph.v1",
        "summary":{"files":2,"symbols":3},
        "nodes":[],
        "edges":[],
        "symbols":[]
    })
}

fn build_case(
    root: &Path,
    fixture: &Value,
    expected_snapshot: &str,
    source_snapshot: &str,
    observed_at: u64,
) -> Value {
    let mode = fixture["providerMode"].as_str().unwrap();
    let implementation_id = if mode == "internal" {
        "architecture-graph.internal-rust"
    } else {
        "architecture-graph.understand-compat"
    };
    let fallback_identity = fixture["fallback"]["identity"].clone();
    let graph = if fixture["graphPresent"] == true {
        graph_document()
    } else {
        Value::Null
    };
    let completeness = if fixture["status"] == "current" {
        "complete"
    } else {
        "partial"
    };
    let payload = json!({
        "schema":"code-intel-evidence-payload.v1",
        "data":{
            "architectureGraph":{
                "schema":"code-intel-architecture-graph-evidence.v1",
                "snapshotIdentity":source_snapshot,
                "provider":{
                    "mode":mode,
                    "implementationId":implementation_id,
                    "fallbackIdentity":fallback_identity
                },
                "provenance":{
                    "sourceRevision":fixture["sourceRevision"],
                    "observedAt":observed_at
                },
                "completeness":completeness,
                "graph":graph
            }
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    fs::write(root.join("payload.json"), &bytes).unwrap();
    json!({
        "schema":"code-intel-graph-provider-native.v1",
        "providerMode":mode,
        "status":fixture["status"],
        "implementation":{
            "id":implementation_id,
            "version":"1.0.0",
            "digest":IMPLEMENTATION_DIGEST
        },
        "sourceRevision":fixture["sourceRevision"],
        "expectedSnapshotIdentity":expected_snapshot,
        "sourceSnapshotIdentity":source_snapshot,
        "collectedAt":observed_at - 1,
        "observedAt":observed_at,
        "payload":{
            "schema":"code-intel-artifact-ref.v1",
            "artifactSchema":"code-intel-evidence-payload.v1",
            "type":"observed.evidence.payload",
            "path":"payload.json",
            "sha256":capability::sha256_hex(&bytes),
            "consumedSnapshotIdentity":source_snapshot
        },
        "fallback":fixture["fallback"]
    })
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

fn route(root: &Path, native: &Value) -> (i32, Value, String) {
    let request = root.join("native.json");
    fs::write(&request, serde_json::to_vec(native).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "provider",
            "graph-adapt",
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
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (
        output.status.code().unwrap(),
        stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn internal_and_external_current_outputs_share_one_port_and_provenance_schema() {
    let internal_root = Temp::new();
    let external_root = Temp::new();
    let internal_native = build_case(
        &internal_root.0,
        &descriptor("internal-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    let external_native = build_case(
        &external_root.0,
        &descriptor("external-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    let internal = graph_adapter::translate(&internal_native, 2_000, 100).unwrap();
    let external = graph_adapter::translate(&external_native, 2_000, 100).unwrap();

    assert_eq!(keys(&internal["port"]), keys(&external["port"]));
    assert_eq!(
        keys(&internal["port"]["provenance"]),
        keys(&external["port"]["provenance"])
    );
    assert_eq!(
        internal["port"]["schema"],
        "code-intel-architecture-graph-port.v1"
    );
    assert_eq!(internal["port"]["provider"]["mode"], "internal");
    assert!(internal["port"]["provider"]["fallbackIdentity"].is_null());
    assert_eq!(external["port"]["provider"]["mode"], "external");
    assert_eq!(
        external["port"]["provider"]["fallbackIdentity"],
        "understand-anything.compat.v1"
    );

    for (root, adapter) in [(&internal_root.0, &internal), (&external_root.0, &external)] {
        let admitted =
            admissibility::validate_for_consumer(&adapter["evidence"]["request"], root).unwrap();
        assert_eq!(admitted.result()["domainVerdict"], "observed");
        graph_adapter::validate_admitted_payload(admitted.payload(), adapter).unwrap();
    }
}

#[test]
fn wrong_head_and_stale_graphs_are_rejected_by_a04() {
    let wrong_root = Temp::new();
    let wrong_native = build_case(
        &wrong_root.0,
        &descriptor("internal-current"),
        CURRENT,
        WRONG,
        1_950,
    );
    let wrong = graph_adapter::translate(&wrong_native, 2_000, 100).unwrap();
    assert_eq!(wrong["port"]["freshness"], "snapshot_mismatch");
    assert_eq!(wrong["port"]["anatomyUsable"], false);
    assert!(
        admissibility::validate_for_consumer(&wrong["evidence"]["request"], &wrong_root.0)
            .err()
            .unwrap()
            .contains("consumed snapshot mismatch")
    );

    let stale_root = Temp::new();
    let stale_native = build_case(
        &stale_root.0,
        &descriptor("internal-current"),
        CURRENT,
        CURRENT,
        1_800,
    );
    let stale = graph_adapter::translate(&stale_native, 2_000, 100).unwrap();
    assert_eq!(stale["port"]["freshness"], "stale");
    assert!(
        admissibility::validate_for_consumer(&stale["evidence"]["request"], &stale_root.0)
            .err()
            .unwrap()
            .contains("stale")
    );
}

#[test]
fn missing_partial_and_current_matrix_preserves_unknown_without_facts() {
    for (name, verdict, completeness) in [
        ("internal-missing", "unknown", "partial"),
        ("internal-partial", "unknown", "partial"),
        ("internal-current", "observed", "complete"),
    ] {
        let root = Temp::new();
        let native = build_case(&root.0, &descriptor(name), CURRENT, CURRENT, 1_950);
        let adapter = graph_adapter::translate(&native, 2_000, 100).unwrap();
        let admitted =
            admissibility::validate_for_consumer(&adapter["evidence"]["request"], &root.0).unwrap();
        graph_adapter::validate_admitted_payload(admitted.payload(), &adapter).unwrap();
        assert_eq!(adapter["port"]["completeness"], completeness, "{name}");
        assert_eq!(adapter["port"]["anatomyUsable"], false, "{name}");
        assert_eq!(admitted.result()["domainVerdict"], verdict, "{name}");
        assert_eq!(admitted.result()["engineeringFacts"], json!([]), "{name}");
        assert_eq!(
            adapter["factPromotion"]["engineeringFacts"],
            json!([]),
            "{name}"
        );
    }
}

#[test]
fn fallback_identity_and_payload_identity_cannot_be_relabelled() {
    let root = Temp::new();
    let mut external = build_case(
        &root.0,
        &descriptor("external-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    external["fallback"]["activation"] = json!("legacy_rollback");
    let rollback = graph_adapter::translate(&external, 2_000, 100).unwrap();
    assert_eq!(
        rollback["port"]["provider"]["fallbackIdentity"],
        "understand-anything.compat.v1"
    );
    external["fallback"]["activation"] = json!("automatic_primary");
    assert!(graph_adapter::translate(&external, 2_000, 100)
        .unwrap_err()
        .contains("explicit fallback/rollback identity"));
    external["fallback"] = Value::Null;
    assert!(graph_adapter::translate(&external, 2_000, 100)
        .unwrap_err()
        .contains("external graph fallback"));

    let mut internal = build_case(
        &root.0,
        &descriptor("internal-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    internal["fallback"] = json!({
        "identity":"illegal",
        "activation":"explicit_fallback",
        "reason":"illegal internal fallback"
    });
    assert!(graph_adapter::translate(&internal, 2_000, 100)
        .unwrap_err()
        .contains("internal graph provider"));

    let native = build_case(
        &root.0,
        &descriptor("external-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    let adapter = graph_adapter::translate(&native, 2_000, 100).unwrap();
    let admitted =
        admissibility::validate_for_consumer(&adapter["evidence"]["request"], &root.0).unwrap();
    let mut relabelled = admitted.payload().clone();
    relabelled["data"]["architectureGraph"]["provider"]["fallbackIdentity"] =
        json!("different-fallback");
    assert!(
        graph_adapter::validate_admitted_payload(&relabelled, &adapter)
            .unwrap_err()
            .contains("provider/fallback identity mismatch")
    );
}

#[test]
fn public_route_accepts_current_and_rejects_wrong_head_without_secret_or_fact() {
    let current_root = Temp::new();
    let current_native = build_case(
        &current_root.0,
        &descriptor("internal-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    let (exit, result, stderr) = route(&current_root.0, &current_native);
    assert_eq!(exit, 0, "{stderr}");
    assert_eq!(result["schema"], "code-intel-graph-route-result.v1");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["adapter"]["port"]["anatomyUsable"], true);
    assert_eq!(result["engineeringFacts"], json!([]));

    let wrong_root = Temp::new();
    let wrong_native = build_case(
        &wrong_root.0,
        &descriptor("internal-current"),
        CURRENT,
        WRONG,
        1_950,
    );
    let (exit, result, _) = route(&wrong_root.0, &wrong_native);
    assert_eq!(exit, 65);
    assert_eq!(result["status"], "rejected");
    assert_eq!(result["engineeringFacts"], json!([]));

    let secret_root = Temp::new();
    let mut secret_native = build_case(
        &secret_root.0,
        &descriptor("internal-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    secret_native["apiToken"] = json!("B02-SECRET-SENTINEL");
    let (exit, result, stderr) = route(&secret_root.0, &secret_native);
    assert_eq!(exit, 65);
    let rendered = format!("{result}{stderr}");
    assert!(!rendered.contains("B02-SECRET-SENTINEL"));
}

#[test]
fn public_route_usage_registry_facade_and_schemas_are_real() {
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["provider", "graph-adapt"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());

    let validation = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["provider", "--action", "Validate", "--json"])
        .output()
        .unwrap();
    let validation_json: Value = serde_json::from_slice(&validation.stdout).unwrap();
    assert_eq!(validation.status.code(), Some(0));
    assert_eq!(validation_json["ok"], true, "{validation_json}");

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("orchestration/integrations.json")).unwrap())
            .unwrap();
    let integration = manifest["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "provider.graph-adapt")
        .expect("provider.graph-adapt registry entry");
    assert_eq!(integration["required"], true);
    assert!(integration["commands"]["adapt"]
        .as_str()
        .unwrap()
        .contains("provider graph-adapt"));
    assert!(integration["commands"]["facade"]
        .as_str()
        .unwrap()
        .contains("-GraphAdapterRequest"));

    let facade = fs::read_to_string(root.join("run-code-intel.ps1")).unwrap();
    assert!(facade.contains("provider graph-adapt"));
    assert!(facade.contains("GraphAdapterMaxAgeSeconds"));

    for schema in [
        "code-intel-architecture-graph-port.v1.schema.json",
        "code-intel-graph-route-result.v1.schema.json",
    ] {
        let value: Value = serde_json::from_slice(
            &fs::read(root.join("orchestration/schemas").join(schema)).unwrap(),
        )
        .unwrap();
        assert_eq!(value["additionalProperties"], false, "{schema}");
    }
}
