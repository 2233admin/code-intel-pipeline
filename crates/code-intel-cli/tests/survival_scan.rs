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
#[path = "../src/survival_scan.rs"]
mod survival_scan;

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const IMPL: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
static NONCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-b05-{}-{now}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
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

fn artifact(root: &Path, path: &str, schema: &str, kind: &str, bytes: &[u8]) -> Value {
    fs::write(root.join(path), bytes).unwrap();
    json!({
        "schema":"code-intel-artifact-ref.v1", "artifactSchema":schema, "type":kind,
        "path":path, "sha256":capability::sha256_hex(bytes), "consumedSnapshotIdentity":SNAPSHOT
    })
}

fn request(root: &Path) -> Value {
    let snapshot = serde_json::to_vec(&json!({
        "schema":"code-intel-repository-snapshot.v1",
        "snapshot":{"identity":SNAPSHOT,"repoIdentity":format!("content-v1:{}", "c".repeat(64)),"head":"unversioned","workingTreePolicy":"explicit_overlay","scope":["."],"inputDigest":"d".repeat(64)},
        "dirtyOverlay":{"present":false,"digest":null,"paths":[],"members":{"trackedModified":[],"trackedDeleted":[],"untracked":[],"renamed":[],"typeChanged":[],"staged":[]},"ignoredPolicy":"excluded_by_git_ignore"},
        "repository":{"kind":"unversioned"}
    })).unwrap();
    let inventory = b"Cargo.toml\0README.md\0src/lib.rs\0";
    let payload = serde_json::to_vec(&json!({
        "schema":"code-intel-evidence-payload.v1",
        "data":{"codenexus":{"schema":"code-intel-codenexus-evidence.v1","snapshotIdentity":SNAPSHOT,
          "provider":{"mode":"full","providerId":"codenexus.full","implementationId":"codenexus.service.v1","activation":"primary"},
          "provenance":{"sourceRevision":"fixture-unavailable","observedAt":1950},"completeness":"partial","availability":"provider_unavailable","providerData":null}}
    })).unwrap();
    let payload_ref = artifact(
        root,
        "provider.json",
        "code-intel-evidence-payload.v1",
        "observed.evidence.payload",
        &payload,
    );
    let native = json!({
        "schema":"code-intel-codenexus-native-result.v1","providerMode":"full","status":"unavailable","providerId":"codenexus.full",
        "implementation":{"id":"codenexus.service.v1","version":"1.0.0","digest":IMPL},"sourceRevision":"fixture-unavailable",
        "expectedSnapshotIdentity":SNAPSHOT,"sourceSnapshotIdentity":SNAPSHOT,"collectedAt":1949,"observedAt":1950,
        "payload":payload_ref,"activation":"primary","effects":["network_provider"]
    });
    let adapter = codenexus_adapter::translate(&native, 2000, 100).unwrap();
    json!({
        "schema":"code-intel-repository-survival-scan-request.v1","snapshotIdentity":SNAPSHOT,
        "inputs":[
          artifact(root, "snapshot.json", "code-intel-repository-snapshot.v1", "repository.snapshot", &snapshot),
          artifact(root, "files.txt", "code-intel-file-inventory.v1", "inventory.files", inventory)
        ],
        "codenexusAdapter":adapter
    })
}

#[test]
fn unavailable_provider_yields_useful_basic_evidence_and_unknown_structure() {
    let root = Temp::new();
    let result = survival_scan::scan(&request(&root.0), &root.0).unwrap();
    let expectations: Value = serde_json::from_str(include_str!(
        "fixtures/survival-scan/unavailable-expectations.json"
    ))
    .unwrap();
    assert_eq!(
        result["schema"],
        "code-intel-repository-survival-scan-result.v1"
    );
    assert_eq!(result["completeness"], expectations["completeness"]);
    assert_eq!(
        result["structuralVerdict"],
        expectations["structuralVerdict"]
    );
    assert_eq!(
        result["providerDiagnosis"]["status"],
        expectations["providerStatus"]
    );
    assert_eq!(result["snapshotIdentity"], SNAPSHOT);
    assert_eq!(result["inventory"]["fileCount"], 3);
    assert_eq!(result["inventory"]["extensions"]["rs"], 1);
    assert_eq!(result["engineeringFacts"].as_array().unwrap().len(), 3);
    let rendered = serde_json::to_string(&result).unwrap().to_ascii_lowercase();
    for forbidden in expectations["forbiddenClaims"].as_array().unwrap() {
        let forbidden = forbidden.as_str().unwrap();
        assert!(
            !rendered.contains(forbidden),
            "fallback overclaims {forbidden}: {rendered}"
        );
    }
}

#[test]
fn fallback_rejects_observed_provider_and_forged_admission() {
    let root = Temp::new();
    let mut observed = request(&root.0);
    observed["codenexusAdapter"]["port"]["status"] = json!("current");
    assert!(survival_scan::scan(&observed, &root.0).is_err());

    let mut forged = request(&root.0);
    forged["codenexusAdapter"]["evidence"]["request"]["observation"]["failure"] =
        json!({"kind":"none"});
    assert!(survival_scan::scan(&forged, &root.0).is_err());
}

#[test]
fn fallback_rejects_undeclared_codenexus_port_fields() {
    let root = Temp::new();
    let mut forged = request(&root.0);
    forged["codenexusAdapter"]["port"]["graph"] = json!({"nodes": []});
    assert!(survival_scan::scan(&forged, &root.0)
        .unwrap_err()
        .contains("port fields are invalid"));
}

#[test]
fn fallback_rejects_undeclared_fact_promotion_fields() {
    let root = Temp::new();
    let mut forged = request(&root.0);
    forged["codenexusAdapter"]["factPromotion"]["secret"] = json!("not-authority");
    assert!(survival_scan::scan(&forged, &root.0)
        .unwrap_err()
        .contains("fact promotion fields are invalid"));
}

#[test]
fn snapshot_and_inventory_are_a03_verified_and_snapshot_bound() {
    let root = Temp::new();
    let mut tampered = request(&root.0);
    fs::write(
        root.0.join("files.txt"),
        b"Cargo.lock\0README.md\0src/lib.rs\0",
    )
    .unwrap();
    assert!(survival_scan::scan(&tampered, &root.0)
        .unwrap_err()
        .contains("SHA-256"));
    tampered = request(&root.0);
    tampered["inputs"][1]["consumedSnapshotIdentity"] = json!("e".repeat(64));
    assert!(survival_scan::scan(&tampered, &root.0)
        .unwrap_err()
        .contains("snapshot"));
}

#[test]
fn production_cli_registry_facade_schema_and_docs_are_closed() {
    let root = Temp::new();
    let request_path = root.0.join("request.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&request(&root.0)).unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "repository",
            "survival-scan",
            "--request",
            request_path.to_str().unwrap(),
            "--artifact-root",
            root.0.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["structuralVerdict"], "unknown");

    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: Value =
        serde_json::from_slice(&fs::read(repo.join("orchestration/integrations.json")).unwrap())
            .unwrap();
    let entry = manifest["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == "repository.survival-scan")
        .unwrap();
    assert_eq!(entry["required"], true);
    assert!(entry["commands"]["scan"]
        .as_str()
        .unwrap()
        .contains("repository survival-scan"));
    assert!(entry["commands"]["facade"]
        .as_str()
        .unwrap()
        .contains("SurvivalScanRequest"));
    let schema: Value = serde_json::from_slice(
        &fs::read(repo.join(
            "orchestration/schemas/code-intel-repository-survival-scan-result.v1.schema.json",
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["additionalProperties"], false);
    let docs = fs::read_to_string(repo.join("docs/repository-survival-scan.md")).unwrap();
    assert!(docs.contains("reduced"));
    assert!(docs.contains("structuralVerdict = unknown"));
    let facade = fs::read_to_string(repo.join("run-code-intel.ps1")).unwrap();
    assert!(facade.contains("repository survival-scan"));
}

#[test]
fn result_has_exact_top_level_contract() {
    let root = Temp::new();
    let result = survival_scan::scan(&request(&root.0), &root.0).unwrap();
    let keys = result
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        keys,
        [
            "schema",
            "status",
            "snapshotIdentity",
            "repository",
            "inventory",
            "providerDiagnosis",
            "completeness",
            "structuralVerdict",
            "limitations",
            "engineeringFacts"
        ]
        .into_iter()
        .collect()
    );
}
