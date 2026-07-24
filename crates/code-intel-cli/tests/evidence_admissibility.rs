use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PAYLOAD: &[u8] = br#"{"schema":"code-intel-evidence-payload.v1","data":{"files":3}}"#;
const PAYLOAD_SHA: &str = "4669d0e1cbe1105783b6eeaaae64e00b392860c50c65cdb2cdd953fa8c2fdbca";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-a04-{}-{nonce}-{}",
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

fn good_request(provider: &str) -> Value {
    json!({
        "schema":"code-intel-evidence-admissibility-request.v1",
        "expectedSnapshotIdentity":SNAPSHOT,
        "policy":{"evaluatedAt":1_700_000_100u64,"maxAgeSeconds":300u64},
        "observation":{
            "schema":"code-intel-observed-evidence.v1",
            "provider":{"id":provider,"implementation":{"id":"synthetic.adapter","version":"1.2.3","digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}},
            "source":{"revision":"fixture-r17"},
            "consumedSnapshotIdentity":SNAPSHOT,
            "observedAt":1_700_000_000u64,
            "completeness":"complete",
            "claimedComplete":true,
            "payload":{"schema":"code-intel-artifact-ref.v1","artifactSchema":"code-intel-evidence-payload.v1","type":"observed.evidence.payload","path":"payload.json","sha256":PAYLOAD_SHA,"consumedSnapshotIdentity":SNAPSHOT},
            "provenance":{"collectionId":"fixture-1","command":"synthetic --emit","startedAt":1_699_999_999u64,"completedAt":1_700_000_000u64},
            "failure":{"kind":"none"}
        }
    })
}

fn run(root: &Path, request: &Value) -> (i32, Value, String) {
    run_with_payload(root, request, PAYLOAD)
}

fn run_with_payload(root: &Path, request: &Value, payload: &[u8]) -> (i32, Value, String) {
    fs::write(root.join("payload.json"), payload).unwrap();
    let request_path = root.join("request.json");
    fs::write(&request_path, serde_json::to_vec(request).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "evidence",
            "validate",
            "--request",
            request_path.to_str().unwrap(),
            "--artifact-root",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout: {} stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (
        output.status.code().unwrap(),
        value,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn unknown_synthetic_provider_is_admitted_without_registry_branching() {
    let temp = Temp::new();
    let (exit, result, stderr) = run(&temp.0, &good_request("provider.never-registered"));
    assert_eq!(exit, 0, "{stderr}");
    assert_eq!(result["status"], "admitted");
    assert_eq!(result["domainVerdict"], "observed");
    assert_eq!(result["engineeringFacts"], json!([]));
    assert_eq!(result["verifiedPayload"]["data"], json!({"files":3}));
    assert_eq!(
        result["evidence"]["provider"]["id"],
        "provider.never-registered"
    );
}

#[test]
fn snapshot_and_digest_mismatch_fail_closed_without_fact() {
    for mutation in ["snapshot", "digest"] {
        let temp = Temp::new();
        let mut request = good_request("synthetic");
        if mutation == "snapshot" {
            request["observation"]["consumedSnapshotIdentity"] = json!("c".repeat(64));
        } else {
            request["observation"]["payload"]["sha256"] = json!("d".repeat(64));
        }
        let (exit, result, _) = run(&temp.0, &request);
        assert_eq!(exit, 65, "mutation={mutation}");
        assert_eq!(result["status"], "rejected");
        assert_eq!(result["domainVerdict"], "unknown");
        assert_eq!(result["engineeringFacts"], json!([]));
    }
}

#[test]
fn stale_malformed_and_incomplete_as_complete_fail_closed() {
    let cases: Vec<Box<dyn Fn(&mut Value)>> = vec![
        Box::new(|v| v["policy"]["evaluatedAt"] = json!(1_700_000_301u64)),
        Box::new(|v| {
            v["observation"]["provider"]
                .as_object_mut()
                .unwrap()
                .remove("implementation");
        }),
        Box::new(|v| {
            v["observation"]["provider"]["implementation"]["digest"] = json!("not-a-digest")
        }),
        Box::new(|v| {
            v["observation"]["completeness"] = json!("partial");
            v["observation"]["claimedComplete"] = json!(true);
        }),
        Box::new(|v| {
            v["observation"]["failure"]["kind"] = json!("provider_unavailable");
        }),
        Box::new(|v| {
            v["observation"]["source"] = json!({});
        }),
        Box::new(|v| {
            v["observation"]["source"] = json!({"revision":"r1","endpointIdentity":"endpoint@1"});
        }),
        Box::new(|v| v["observation"]["provenance"]["collectionId"] = json!("")),
        Box::new(|v| {
            v["observation"]["provenance"]
                .as_object_mut()
                .unwrap()
                .remove("command");
        }),
        Box::new(|v| {
            v["observation"]["provenance"]["completedAt"] = json!(1_699_999_998u64);
        }),
    ];
    for mutate in cases {
        let temp = Temp::new();
        let mut request = good_request("synthetic");
        mutate(&mut request);
        let (exit, result, _) = run(&temp.0, &request);
        assert_eq!(exit, 65);
        assert_eq!(result["domainVerdict"], "unknown");
        assert_eq!(result["engineeringFacts"], json!([]));
    }
}

#[test]
fn freshness_boundaries_clock_types_and_replay_identity_are_deterministic() {
    let temp = Temp::new();
    let mut boundary = good_request("synthetic.clock");
    boundary["policy"]["evaluatedAt"] = json!(1_700_000_300u64);
    let (exit, first, _) = run(&temp.0, &boundary);
    assert_eq!(exit, 0);
    let (exit, replay, _) = run(&temp.0, &boundary);
    assert_eq!(exit, 0);
    assert_eq!(first["admissionIdentity"], replay["admissionIdentity"]);
    assert_eq!(first["engineeringFacts"], json!([]));

    let cases: Vec<Box<dyn Fn(&mut Value)>> = vec![
        Box::new(|v| v["policy"]["evaluatedAt"] = json!(1_700_000_301u64)),
        Box::new(|v| v["policy"]["evaluatedAt"] = json!(1_699_999_999u64)),
        Box::new(|v| v["policy"]["evaluatedAt"] = json!("1700000100")),
        Box::new(|v| v["policy"]["maxAgeSeconds"] = json!(0)),
    ];
    for mutate in cases {
        let temp = Temp::new();
        let mut request = good_request("synthetic.clock");
        mutate(&mut request);
        let (exit, result, _) = run(&temp.0, &request);
        assert_eq!(exit, 65);
        assert_eq!(result["domainVerdict"], "unknown");
        assert_eq!(result["engineeringFacts"], json!([]));
    }
}

#[test]
fn completeness_and_failure_semantics_are_exhaustive() {
    for (completeness, claimed, kind, message, accepted, verdict) in [
        ("complete", true, "none", None, true, "observed"),
        ("partial", false, "none", None, true, "unknown"),
        (
            "partial",
            false,
            "domain_unknown",
            Some("not covered"),
            true,
            "unknown",
        ),
        (
            "partial",
            false,
            "provider_unavailable",
            Some("offline"),
            true,
            "unknown",
        ),
        (
            "partial",
            false,
            "process_failure",
            Some("crashed"),
            false,
            "unknown",
        ),
        (
            "complete",
            true,
            "domain_unknown",
            Some("contradiction"),
            false,
            "unknown",
        ),
    ] {
        let temp = Temp::new();
        let mut request = good_request("synthetic.failure");
        request["observation"]["completeness"] = json!(completeness);
        request["observation"]["claimedComplete"] = json!(claimed);
        request["observation"]["failure"] = match message {
            Some(m) => json!({"kind":kind,"message":m}),
            None => json!({"kind":kind}),
        };
        let (exit, result, _) = run(&temp.0, &request);
        assert_eq!(exit, if accepted { 0 } else { 65 }, "{completeness}/{kind}");
        assert_eq!(result["domainVerdict"], verdict);
        assert_eq!(result["engineeringFacts"], json!([]));
    }
}

#[test]
fn payload_schema_is_checked_after_digest_verification() {
    let temp = Temp::new();
    let payload = br#"{"schema":"wrong.v1","data":{"files":3}}"#;
    let mut request = good_request("synthetic.payload");
    request["observation"]["payload"]["sha256"] =
        json!("cfa6e883b9208641a7de6bd394db4ff63c8950a8712593a704d192cdd71556b9");
    let (exit, result, stderr) = run_with_payload(&temp.0, &request, payload);
    assert_eq!(exit, 65);
    assert!(stderr.contains("schema/data"), "{stderr}");
    assert_eq!(result["domainVerdict"], "unknown");
    assert_eq!(result["engineeringFacts"], json!([]));
}

#[test]
fn endpoint_identity_can_replace_source_revision_and_partial_is_preserved() {
    let temp = Temp::new();
    let mut request = good_request("synthetic.endpoint");
    request["observation"]["source"] =
        json!({"endpointIdentity":"https://fixture.invalid/api@etag-7"});
    request["observation"]["completeness"] = json!("partial");
    request["observation"]["claimedComplete"] = json!(false);
    request["observation"]["failure"] =
        json!({"kind":"domain_unknown","message":"coverage unavailable"});
    let (exit, result, stderr) = run(&temp.0, &request);
    assert_eq!(exit, 0, "{stderr}");
    assert_eq!(result["status"], "admitted");
    assert_eq!(result["evidence"]["completeness"], "partial");
    assert_eq!(result["domainVerdict"], "unknown");
    assert_eq!(result["engineeringFacts"], json!([]));
}

#[test]
fn checked_in_provider_protocol_fixture_is_executable() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/evidence-admissibility/good");
    let request: Value =
        serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
    let request_copy = Temp::new();
    fs::copy(
        fixture.join("payload.json"),
        request_copy.0.join("payload.json"),
    )
    .unwrap();
    let request_path = request_copy.0.join("request.json");
    fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "evidence",
            "validate",
            "--request",
            request_path.to_str().unwrap(),
            "--artifact-root",
            request_copy.0.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn duplicate_key_malformed_request_returns_unknown_without_fact() {
    let temp = Temp::new();
    fs::write(temp.0.join("payload.json"), PAYLOAD).unwrap();
    let request_path = temp.0.join("request.json");
    fs::write(
        &request_path,
        br#"{"schema":"code-intel-evidence-admissibility-request.v1","schema":"duplicate"}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "evidence",
            "validate",
            "--request",
            request_path.to_str().unwrap(),
            "--artifact-root",
            temp.0.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(65));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["domainVerdict"], "unknown");
    assert_eq!(result["engineeringFacts"], json!([]));
}
