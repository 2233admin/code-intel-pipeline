#[path = "../src/runtime_ci_evidence.rs"]
mod runtime_ci_evidence;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-runtime-ci-{}-{nonce}",
            std::process::id()
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

fn signal(status: &str, observed: bool) -> Value {
    json!({"status":status,"observed":observed,"summary":format!("fixture {status}")})
}

fn source(completeness: &str, tests: &str, build: &str, runtime: &str) -> Value {
    json!({
        "schema":"code-intel-runtime-ci-observation.v1",
        "provider":{"id":"fixture.local-json","runId":"run-42","sourceRevision":"abc123"},
        "provenance":{
            "collectorId":"fixture-exporter",
            "collectorVersion":"1.0.0",
            "collectionId":"collection-42",
            "collectedAt":1950
        },
        "snapshotIdentity":SNAPSHOT,
        "observedAt":1950,
        "completeness":completeness,
        "signals":{
            "tests":signal(tests, tests != "unknown"),
            "build":signal(build, build != "unknown"),
            "runtime":signal(runtime, runtime != "unknown")
        }
    })
}

fn write_request(root: &Path, source: &Value) -> Value {
    let bytes = serde_json::to_vec(source).unwrap();
    fs::write(root.join("runtime-ci.json"), &bytes).unwrap();
    json!({
        "schema":"code-intel-runtime-ci-ingest-request.v1",
        "expectedSnapshotIdentity":SNAPSHOT,
        "artifact":{"path":"runtime-ci.json","sha256":runtime_ci_evidence::sha256_hex(&bytes)},
        "policy":{"evaluatedAt":2000,"maxAgeSeconds":100}
    })
}

#[test]
fn only_complete_current_fully_positive_observations_are_green() {
    let temp = Temp::new();
    let request = write_request(&temp.0, &source("complete", "passed", "passed", "healthy"));
    let result = runtime_ci_evidence::ingest_request(&temp.0, &request).unwrap();
    assert_eq!(result["admission"], "accepted");
    assert_eq!(result["health"], "green");
    assert_eq!(result["failureKind"], "none");
    assert_eq!(
        result["facts"],
        json!([
            "tests_observed_passed",
            "build_observed_passed",
            "runtime_observed_healthy"
        ])
    );
}

#[test]
fn missing_partial_and_unobserved_are_unknown_not_green() {
    let temp = Temp::new();
    let missing_request = json!({
        "schema":"code-intel-runtime-ci-ingest-request.v1",
        "expectedSnapshotIdentity":SNAPSHOT,
        "artifact":{"path":"missing.json","sha256":"b".repeat(64)},
        "policy":{"evaluatedAt":2000,"maxAgeSeconds":100}
    });
    let missing = runtime_ci_evidence::ingest_request(&temp.0, &missing_request).unwrap();
    assert_eq!(missing["health"], "unknown");
    assert_eq!(missing["failureKind"], "artifact_missing");
    assert_eq!(missing["facts"], json!([]));

    let partial = runtime_ci_evidence::normalize(
        &source("partial", "passed", "unknown", "unknown"),
        SNAPSHOT,
        2000,
        100,
    )
    .unwrap();
    assert_eq!(partial["health"], "unknown");
    assert_eq!(partial["failureKind"], "partial_coverage");
    assert_eq!(partial["facts"], json!([]));
}

#[test]
fn observed_failure_is_red_even_when_other_domains_are_unknown() {
    let result = runtime_ci_evidence::normalize(
        &source("partial", "failed", "unknown", "unknown"),
        SNAPSHOT,
        2000,
        100,
    )
    .unwrap();
    assert_eq!(result["health"], "red");
    assert_eq!(result["failureKind"], "observed_failure");
    assert_eq!(result["facts"], json!(["runtime_ci_observed_failure"]));
}

#[test]
fn stale_snapshot_mismatch_and_digest_forgery_fail_closed() {
    let mut stale_source = source("complete", "passed", "passed", "healthy");
    stale_source["observedAt"] = json!(1800);
    let stale = runtime_ci_evidence::normalize(&stale_source, SNAPSHOT, 2000, 100).unwrap();
    assert_eq!(stale["admission"], "rejected");
    assert_eq!(stale["health"], "unknown");
    assert_eq!(stale["failureKind"], "stale");

    let mut wrong = source("complete", "passed", "passed", "healthy");
    wrong["snapshotIdentity"] = json!("c".repeat(64));
    let wrong = runtime_ci_evidence::normalize(&wrong, SNAPSHOT, 2000, 100).unwrap();
    assert_eq!(wrong["failureKind"], "snapshot_mismatch");
    assert_eq!(wrong["facts"], json!([]));

    let temp = Temp::new();
    let mut request = write_request(&temp.0, &source("complete", "passed", "passed", "healthy"));
    request["artifact"]["sha256"] = json!("d".repeat(64));
    assert!(runtime_ci_evidence::ingest_request(&temp.0, &request)
        .unwrap_err()
        .contains("digest mismatch"));
}

#[test]
fn contracts_reject_unknown_fields_false_observation_claims_and_path_escape() {
    let mut extra = source("complete", "passed", "passed", "healthy");
    extra["provider"]["secret"] = json!("must-not-be-accepted");
    assert!(runtime_ci_evidence::normalize(&extra, SNAPSHOT, 2000, 100)
        .unwrap_err()
        .contains("fields are invalid"));

    let mut false_claim = source("complete", "passed", "passed", "healthy");
    false_claim["signals"]["tests"]["observed"] = json!(false);
    assert!(
        runtime_ci_evidence::normalize(&false_claim, SNAPSHOT, 2000, 100)
            .unwrap_err()
            .contains("observed is false")
    );

    let temp = Temp::new();
    let request = json!({
        "schema":"code-intel-runtime-ci-ingest-request.v1",
        "expectedSnapshotIdentity":SNAPSHOT,
        "artifact":{"path":"../escape.json","sha256":"e".repeat(64)},
        "policy":{"evaluatedAt":2000,"maxAgeSeconds":100}
    });
    assert!(runtime_ci_evidence::ingest_request(&temp.0, &request)
        .unwrap_err()
        .contains("without '..'"));
}

#[test]
fn schemas_and_documentation_are_closed_and_provider_neutral() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "orchestration/schemas/code-intel-runtime-ci-ingest-request.v1.schema.json",
        "orchestration/schemas/code-intel-runtime-ci-observation.v1.schema.json",
        "orchestration/schemas/code-intel-runtime-ci-summary.v1.schema.json",
    ] {
        let schema: Value =
            serde_json::from_slice(&fs::read(root.join(relative)).unwrap()).unwrap();
        assert_eq!(schema["additionalProperties"], false, "{relative}");
    }
    let docs = fs::read_to_string(root.join("docs/runtime-ci-evidence.md")).unwrap();
    assert!(docs.contains("does not call a CI provider"));
    assert!(docs.contains("Missing is not green"));
    assert!(docs.contains("Hospital/PET"));
}

#[test]
fn production_cli_reads_only_the_pinned_local_artifact() {
    let temp = Temp::new();
    let request = write_request(&temp.0, &source("complete", "passed", "passed", "healthy"));
    let request_path = temp.0.join("request.json");
    let output_path = temp.0.join("summary.json");
    fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["provider", "runtime-ci-evidence", "--artifact-root"])
        .arg(&temp.0)
        .arg("--request")
        .arg(&request_path)
        .arg("--out")
        .arg(&output_path)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value = serde_json::from_slice(&fs::read(output_path).unwrap()).unwrap();
    assert_eq!(summary["schema"], "code-intel-runtime-ci-summary.v1");
    assert_eq!(summary["health"], "green");
}

#[test]
fn production_cli_rejects_duplicate_source_keys() {
    let temp = Temp::new();
    let valid = serde_json::to_string(&source("complete", "passed", "passed", "healthy")).unwrap();
    let duplicate = valid.replacen(
        "\"completeness\":\"complete\"",
        "\"completeness\":\"complete\",\"completeness\":\"partial\"",
        1,
    );
    assert!(duplicate.matches("\"completeness\"").count() >= 2);
    fs::write(temp.0.join("runtime-ci.json"), duplicate.as_bytes()).unwrap();
    let request = json!({
        "schema":"code-intel-runtime-ci-ingest-request.v1",
        "expectedSnapshotIdentity":SNAPSHOT,
        "artifact":{"path":"runtime-ci.json","sha256":runtime_ci_evidence::sha256_hex(duplicate.as_bytes())},
        "policy":{"evaluatedAt":2000,"maxAgeSeconds":100}
    });
    let request_path = temp.0.join("request.json");
    let output_path = temp.0.join("summary.json");
    fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["provider", "runtime-ci-evidence", "--artifact-root"])
        .arg(&temp.0)
        .arg("--request")
        .arg(&request_path)
        .arg("--out")
        .arg(&output_path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(65));
    assert!(!output_path.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate JSON object key"));
}
