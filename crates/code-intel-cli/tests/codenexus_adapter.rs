use std::collections::BTreeSet;
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
#[path = "../src/codenexus_adapter.rs"]
mod codenexus_adapter;
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
            "code-intel-b04-{}-{nonce}-{}",
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
        "full-current" => include_str!("fixtures/codenexus-adapter/full-current.json"),
        "lite-current" => include_str!("fixtures/codenexus-adapter/lite-current.json"),
        "unavailable" => include_str!("fixtures/codenexus-adapter/unavailable.json"),
        _ => panic!("unknown fixture {name}"),
    };
    serde_json::from_str(text).unwrap()
}

fn build_case(
    root: &Path,
    fixture: &Value,
    expected_snapshot: &str,
    source_snapshot: &str,
    observed_at: u64,
) -> Value {
    let mode = fixture["providerMode"].as_str().unwrap();
    let implementation_id = fixture["implementationId"].as_str().unwrap();
    let completeness = if fixture["status"] == "current" {
        "complete"
    } else {
        "partial"
    };
    let availability = if fixture["status"] == "unavailable" {
        "provider_unavailable"
    } else {
        "available"
    };
    let provider_data = if fixture["status"] == "unavailable" {
        Value::Null
    } else if mode == "full" {
        json!({"opaqueFullResult":{"queryId":"q-7","impactRelationships":[{"providerEdge":"opaque"}]}})
    } else {
        json!({"opaqueLiteContext":{"files":[{"path":"src/lib.rs","reason":"hotspot"}]}})
    };
    let payload = json!({
        "schema":"code-intel-evidence-payload.v1",
        "data":{
            "codenexus":{
                "schema":"code-intel-codenexus-evidence.v1",
                "snapshotIdentity":source_snapshot,
                "provider":{
                    "mode":mode,
                    "providerId":fixture["providerId"],
                    "implementationId":implementation_id,
                    "activation":fixture["activation"]
                },
                "provenance":{
                    "sourceRevision":fixture["sourceRevision"],
                    "observedAt":observed_at
                },
                "completeness":completeness,
                "availability":availability,
                "providerData":provider_data
            }
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    fs::write(root.join("payload.json"), &bytes).unwrap();
    json!({
        "schema":"code-intel-codenexus-native-result.v1",
        "providerMode":mode,
        "status":fixture["status"],
        "providerId":fixture["providerId"],
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
        "activation":fixture["activation"],
        "effects":fixture["effects"]
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

fn route(root: &Path, native: &Value) -> (i32, Vec<u8>, String) {
    let request = root.join("native.json");
    fs::write(&request, serde_json::to_vec(native).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "provider",
            "codenexus-adapt",
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
    (
        output.status.code().unwrap(),
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn full_and_lite_share_one_port_shape_but_keep_distinct_provenance() {
    let full_root = Temp::new();
    let lite_root = Temp::new();
    let full_native = build_case(
        &full_root.0,
        &descriptor("full-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    let lite_native = build_case(
        &lite_root.0,
        &descriptor("lite-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    let full = codenexus_adapter::translate(&full_native, 2_000, 100).unwrap();
    let lite = codenexus_adapter::translate(&lite_native, 2_000, 100).unwrap();

    assert_eq!(keys(&full["port"]), keys(&lite["port"]));
    assert_eq!(
        keys(&full["port"]["provider"]),
        keys(&lite["port"]["provider"])
    );
    assert_eq!(full["port"]["schema"], "code-intel-codenexus-port.v1");
    assert_ne!(
        full["port"]["provider"]["providerId"],
        lite["port"]["provider"]["providerId"]
    );
    assert_ne!(
        full["port"]["provider"]["implementationId"],
        lite["port"]["provider"]["implementationId"]
    );
    assert_eq!(full["port"]["boundary"]["storageOwnership"], "provider");
    assert_eq!(lite["port"]["boundary"]["storageOwnership"], "provider");

    for (root, adapter) in [(&full_root.0, &full), (&lite_root.0, &lite)] {
        let admitted =
            admissibility::validate_for_consumer(&adapter["evidence"]["request"], root).unwrap();
        assert_eq!(admitted.result()["domainVerdict"], "observed");
        codenexus_adapter::validate_admitted_payload(admitted.payload(), adapter).unwrap();
    }
}

#[test]
fn unavailable_is_partial_provider_unavailable_and_never_fabricates_facts() {
    let root = Temp::new();
    let native = build_case(&root.0, &descriptor("unavailable"), CURRENT, CURRENT, 1_950);
    let adapter = codenexus_adapter::translate(&native, 2_000, 100).unwrap();
    assert_eq!(adapter["port"]["status"], "unavailable");
    assert_eq!(adapter["port"]["completeness"], "partial");
    assert_eq!(adapter["port"]["failureKind"], "provider_unavailable");
    assert_eq!(adapter["port"]["perceptionUsable"], false);
    let admitted =
        admissibility::validate_for_consumer(&adapter["evidence"]["request"], &root.0).unwrap();
    assert_eq!(admitted.result()["domainVerdict"], "unknown");
    assert_eq!(admitted.result()["engineeringFacts"], json!([]));
    assert_eq!(
        admitted.payload()["data"]["codenexus"]["providerData"],
        Value::Null
    );
}

#[test]
fn snapshot_mismatch_and_stale_observation_fail_closed_in_a04() {
    let wrong_root = Temp::new();
    let wrong_native = build_case(
        &wrong_root.0,
        &descriptor("full-current"),
        CURRENT,
        WRONG,
        1_950,
    );
    let wrong = codenexus_adapter::translate(&wrong_native, 2_000, 100).unwrap();
    assert_eq!(wrong["port"]["freshness"], "snapshot_mismatch");
    let wrong_error =
        match admissibility::validate_for_consumer(&wrong["evidence"]["request"], &wrong_root.0) {
            Ok(_) => panic!("wrong-snapshot evidence was admitted"),
            Err(error) => error,
        };
    assert!(wrong_error.contains("consumed snapshot mismatch"));

    let stale_root = Temp::new();
    let stale_native = build_case(
        &stale_root.0,
        &descriptor("full-current"),
        CURRENT,
        CURRENT,
        1_800,
    );
    let stale = codenexus_adapter::translate(&stale_native, 2_000, 100).unwrap();
    assert_eq!(stale["port"]["freshness"], "stale");
    let stale_error =
        match admissibility::validate_for_consumer(&stale["evidence"]["request"], &stale_root.0) {
            Ok(_) => panic!("stale evidence was admitted"),
            Err(error) => error,
        };
    assert!(stale_error.contains("stale"));
}

#[test]
fn lite_is_only_explicit_fallback_or_rollback_and_full_is_primary() {
    let root = Temp::new();
    let mut lite = build_case(
        &root.0,
        &descriptor("lite-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    lite["activation"] = json!("automatic_primary");
    assert!(codenexus_adapter::translate(&lite, 2_000, 100)
        .unwrap_err()
        .contains("explicit fallback or legacy rollback"));

    let mut full = build_case(
        &root.0,
        &descriptor("full-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    full["activation"] = json!("explicit_fallback");
    assert!(codenexus_adapter::translate(&full, 2_000, 100)
        .unwrap_err()
        .contains("full provider must be primary"));

    let mut relabelled_lite = build_case(
        &root.0,
        &descriptor("lite-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    relabelled_lite["providerId"] = json!("codenexus.full");
    assert!(codenexus_adapter::translate(&relabelled_lite, 2_000, 100)
        .unwrap_err()
        .contains("lite compatibility identity"));
}

#[test]
fn adapter_declares_effects_rejects_storage_coupling_and_drops_unknown_secrets() {
    let root = Temp::new();
    let mut full = build_case(
        &root.0,
        &descriptor("full-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    full["databasePath"] = json!("C:/provider/private.db");
    full["apiToken"] = json!("B04-SECRET-SENTINEL");
    let error = codenexus_adapter::translate(&full, 2_000, 100).unwrap_err();
    assert!(error.contains("fields are invalid"));
    assert!(!error.contains("B04-SECRET-SENTINEL"));

    let native = build_case(
        &root.0,
        &descriptor("full-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    let adapter = codenexus_adapter::translate(&native, 2_000, 100).unwrap();
    assert_eq!(adapter["port"]["boundary"]["transport"], "artifact_ref");
    assert!(adapter["port"]["effects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|effect| effect == "network_provider"));
    let rendered = serde_json::to_string(&adapter).unwrap();
    assert!(!rendered.contains("private.db"));
    assert!(!rendered.contains("token"));
}

#[test]
fn admitted_payload_identity_cannot_be_relabelled() {
    let root = Temp::new();
    let native = build_case(
        &root.0,
        &descriptor("full-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    let adapter = codenexus_adapter::translate(&native, 2_000, 100).unwrap();
    let admitted =
        admissibility::validate_for_consumer(&adapter["evidence"]["request"], &root.0).unwrap();
    let mut relabelled = admitted.payload().clone();
    relabelled["data"]["codenexus"]["provider"]["implementationId"] =
        json!("pipeline.fake-impact-engine");
    assert!(
        codenexus_adapter::validate_admitted_payload(&relabelled, &adapter)
            .unwrap_err()
            .contains("provider identity mismatch")
    );
}

#[test]
fn port_schema_and_boundary_document_are_closed_and_real() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema: Value = serde_json::from_slice(
        &fs::read(root.join("orchestration/schemas/code-intel-codenexus-port.v1.schema.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["boundary"]["properties"]["storageOwnership"]["const"],
        "provider"
    );
    assert_eq!(
        schema["properties"]["boundary"]["properties"]["impactSemanticsOwnership"]["const"],
        "provider"
    );

    let docs = fs::read_to_string(root.join("docs/codenexus-provider-adapter.md")).unwrap();
    assert!(docs.contains("does not import CodeNexus libraries"));
    assert!(docs.contains("explicit_fallback"));
    assert!(docs.contains("provider_unavailable"));
}

#[test]
fn production_route_runs_full_lite_and_unavailable_through_a04() {
    for (fixture_name, verdict, usable) in [
        ("full-current", "observed", true),
        ("lite-current", "observed", true),
        ("unavailable", "unknown", false),
    ] {
        let root = Temp::new();
        let native = build_case(&root.0, &descriptor(fixture_name), CURRENT, CURRENT, 1_950);
        let (exit, stdout, stderr) = route(&root.0, &native);
        assert_eq!(exit, 0, "{fixture_name}: {stderr}");
        let result: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(result["schema"], "code-intel-codenexus-route-result.v1");
        assert_eq!(result["status"], "completed");
        assert_eq!(result["admission"]["domainVerdict"], verdict);
        assert_eq!(result["adapter"]["port"]["perceptionUsable"], usable);
        assert_eq!(result["engineeringFacts"], json!([]));
    }
}

#[test]
fn production_route_rejects_secret_fields_wrong_snapshot_and_bad_usage() {
    let secret_root = Temp::new();
    let mut secret = build_case(
        &secret_root.0,
        &descriptor("full-current"),
        CURRENT,
        CURRENT,
        1_950,
    );
    secret["apiToken"] = json!("B04-ROUTE-SECRET");
    let (exit, stdout, stderr) = route(&secret_root.0, &secret);
    assert_eq!(exit, 65);
    let rendered = format!("{}{}", String::from_utf8_lossy(&stdout), stderr);
    assert!(!rendered.contains("B04-ROUTE-SECRET"));

    let wrong_root = Temp::new();
    let wrong = build_case(
        &wrong_root.0,
        &descriptor("full-current"),
        CURRENT,
        WRONG,
        1_950,
    );
    let (exit, stdout, _) = route(&wrong_root.0, &wrong);
    assert_eq!(exit, 65);
    let result: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(result["status"], "rejected");
    assert_eq!(result["engineeringFacts"], json!([]));

    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["provider", "codenexus-adapt"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
}

#[test]
fn production_registry_facade_and_route_schema_are_declared() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("orchestration/integrations.json")).unwrap())
            .unwrap();
    let integration = manifest["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "provider.codenexus-adapt")
        .expect("provider.codenexus-adapt registry entry");
    assert_eq!(integration["required"], false);
    assert!(integration["commands"]["adapt"]
        .as_str()
        .unwrap()
        .contains("provider codenexus-adapt"));
    assert!(integration["commands"]["facade"]
        .as_str()
        .unwrap()
        .contains("-CodeNexusAdapterRequest"));

    let facade = fs::read_to_string(root.join("run-code-intel.ps1")).unwrap();
    assert!(facade.contains("provider codenexus-adapt"));
    assert!(facade.contains("CodeNexusAdapterMaxAgeSeconds"));

    let schema: Value = serde_json::from_slice(
        &fs::read(
            root.join("orchestration/schemas/code-intel-codenexus-route-result.v1.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["additionalProperties"], false);
}
