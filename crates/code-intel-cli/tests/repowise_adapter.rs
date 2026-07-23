#[path = "../src/repowise_adapter.rs"]
mod repowise_adapter;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};

const EVALUATED_AT: u64 = 1_700_000_100;
const MAX_AGE: u64 = 300;
static NEXT_REQUEST: AtomicUsize = AtomicUsize::new(1);

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/repowise-adapter")
}

fn fixture(name: &str) -> Value {
    serde_json::from_slice(&fs::read(fixture_root().join(name)).unwrap()).unwrap()
}

fn evidence<'a>(translated: &'a Value, channel: &str) -> &'a Value {
    translated["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["channel"] == channel)
        .unwrap()
}

fn admit(entry: &Value, artifact_root: &Path) -> (i32, Value) {
    let stamp = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let request_path = std::env::temp_dir().join(format!(
        "code-intel-repowise-admission-{}-{stamp}.json",
        std::process::id()
    ));
    fs::write(
        &request_path,
        serde_json::to_vec(&entry["request"]).unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "evidence",
            "validate",
            "--request",
            request_path.to_str().unwrap(),
            "--artifact-root",
            artifact_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    fs::remove_file(request_path).unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    (output.status.code().unwrap(), result)
}

#[test]
fn repowise_success_keeps_health_separate_and_a04_admits_index_and_docs() {
    let translated =
        repowise_adapter::translate(&fixture("success.json"), EVALUATED_AT, MAX_AGE).unwrap();
    assert_eq!(
        translated["schema"],
        "code-intel-repowise-adapter-result.v1"
    );
    assert_eq!(translated["health"]["kind"], "health");
    assert_eq!(translated["health"]["evidence"], false);
    assert_eq!(translated["evidence"].as_array().unwrap().len(), 2);
    assert_ne!(
        translated["index"]["effects"],
        translated["docs"]["effects"]
    );
    assert_eq!(translated["factPromotion"]["eligible"], false);

    for channel in ["index", "docs"] {
        let (exit, admitted) = admit(evidence(&translated, channel), &fixture_root());
        assert_eq!(exit, 0, "{channel}: {admitted}");
        assert_eq!(admitted["status"], "admitted");
        assert_eq!(admitted["domainVerdict"], "observed");
        assert_eq!(admitted["engineeringFacts"], json!([]));
    }
}

#[test]
fn docs_quota_is_partial_provider_unavailable_without_erasing_current_index() {
    let translated =
        repowise_adapter::translate(&fixture("quota.json"), EVALUATED_AT, MAX_AGE).unwrap();
    assert_eq!(translated["index"]["status"], "current");
    assert_eq!(translated["index"]["completeness"], "complete");
    assert_eq!(translated["docs"]["status"], "quota");
    assert_eq!(translated["docs"]["completeness"], "partial");
    assert_eq!(translated["docs"]["failureKind"], "provider_unavailable");

    let (index_exit, index) = admit(evidence(&translated, "index"), &fixture_root());
    let (docs_exit, docs) = admit(evidence(&translated, "docs"), &fixture_root());
    assert_eq!(index_exit, 0, "{index}");
    assert_eq!(index["domainVerdict"], "observed");
    assert_eq!(docs_exit, 0, "{docs}");
    assert_eq!(docs["domainVerdict"], "unknown");
    assert_eq!(docs["evidence"]["failure"]["kind"], "provider_unavailable");
}

#[test]
fn index_only_emits_one_a04_request_and_docs_are_explicitly_not_requested() {
    let translated =
        repowise_adapter::translate(&fixture("index-only.json"), EVALUATED_AT, MAX_AGE).unwrap();
    assert_eq!(translated["evidence"].as_array().unwrap().len(), 1);
    assert_eq!(translated["evidence"][0]["channel"], "index");
    assert_eq!(translated["docs"]["status"], "not_requested");
    assert_eq!(translated["docs"]["completeness"], "none");
}

#[test]
fn missing_cli_is_health_and_local_tool_diagnosis_not_fake_evidence() {
    let mut native = fixture("success.json");
    native["cli"]["status"] = json!("missing");
    native["health"]["status"] = json!("unavailable");
    let translated = repowise_adapter::translate(&native, EVALUATED_AT, MAX_AGE).unwrap();
    assert_eq!(translated["health"]["status"], "unavailable");
    assert_eq!(translated["index"]["failureKind"], "local_tool_error");
    assert_eq!(translated["docs"]["failureKind"], "local_tool_error");
    assert!(translated["evidence"].as_array().unwrap().is_empty());
}

#[test]
fn stale_index_is_translated_but_a04_rejects_fact_promotion() {
    let mut native = fixture("index-only.json");
    native["collectedAt"] = json!(1_699_999_000u64);
    native["index"]["observedAt"] = json!(1_699_999_000u64);
    let translated = repowise_adapter::translate(&native, EVALUATED_AT, MAX_AGE).unwrap();
    assert_eq!(translated["index"]["freshness"], "stale");
    let (exit, result) = admit(evidence(&translated, "index"), &fixture_root());
    assert_eq!(exit, 65, "{result}");
    assert_eq!(result["status"], "rejected");
    assert_eq!(result["engineeringFacts"], json!([]));
}

#[test]
fn successful_but_incomplete_docs_remain_partial_and_domain_unknown() {
    let mut native = fixture("success.json");
    native["docs"]["status"] = json!("partial");
    native["docs"]["payload"]["path"] = json!("partial-docs-payload.json");
    native["docs"]["payload"]["sha256"] =
        json!("d7677aecabbe555f5aebe19cc411ac3cadb5bd77dbfe33dc9c402f55b1385ffa");
    let translated = repowise_adapter::translate(&native, EVALUATED_AT, MAX_AGE).unwrap();
    assert_eq!(translated["docs"]["completeness"], "partial");
    let (exit, result) = admit(evidence(&translated, "docs"), &fixture_root());
    assert_eq!(exit, 0, "{result}");
    assert_eq!(result["domainVerdict"], "unknown");
    assert_eq!(result["evidence"]["failure"]["kind"], "domain_unknown");
}

#[test]
fn translated_output_never_copies_native_secret_bearing_diagnostics() {
    let native = fixture("success.json");
    let translated = repowise_adapter::translate(&native, EVALUATED_AT, MAX_AGE).unwrap();
    let text = serde_json::to_string(&translated).unwrap();
    assert!(!text.contains("super-secret"));
    assert!(!text.contains("sk-live-123"));
    assert!(!text.contains("ANTHROPIC_API_KEY"));
}
