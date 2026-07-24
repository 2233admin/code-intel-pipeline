use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};

const EVALUATED_AT: &str = "1700000100";
const MAX_AGE: &str = "300";
static NEXT_FILE: AtomicUsize = AtomicUsize::new(1);

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    root().join("tests/fixtures/repowise-adapter")
}

fn run(request: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "provider",
            "repowise-adapt",
            "--request",
            request.to_str().unwrap(),
            "--artifact-root",
            fixtures().to_str().unwrap(),
            "--evaluated-at",
            EVALUATED_AT,
            "--max-age-seconds",
            MAX_AGE,
        ])
        .output()
        .unwrap()
}

fn result(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout must be JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn with_fixture(name: &str, edit: impl FnOnce(&mut Value)) -> PathBuf {
    let mut value: Value =
        serde_json::from_slice(&fs::read(fixtures().join(name)).unwrap()).unwrap();
    edit(&mut value);
    let path = std::env::temp_dir().join(format!(
        "code-intel-repowise-route-{}-{}.json",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    path
}

fn assert_admission(result: &Value, channel: &str, status: &str, verdict: &str) {
    let admission = result["admissions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["channel"] == channel)
        .unwrap();
    assert_eq!(admission["result"]["status"], status);
    assert_eq!(admission["result"]["domainVerdict"], verdict);
}

#[test]
fn public_route_translates_and_a04_validates_success_quota_and_index_only() {
    for (fixture, count) in [
        ("success.json", 2),
        ("quota.json", 2),
        ("index-only.json", 1),
    ] {
        let output = run(&fixtures().join(fixture));
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value = result(&output);
        assert_eq!(value["schema"], "code-intel-repowise-route-result.v1");
        assert_eq!(
            value["adapter"]["schema"],
            "code-intel-repowise-adapter-result.v1"
        );
        assert_eq!(value["admissions"].as_array().unwrap().len(), count);
        assert_admission(&value, "index", "admitted", "observed");
        if fixture == "quota.json" {
            assert_eq!(value["adapter"]["index"]["status"], "current");
            assert_admission(&value, "docs", "admitted", "unknown");
        }
    }
}

#[test]
fn public_route_preserves_missing_cli_and_incomplete_docs_without_fake_facts() {
    let missing = with_fixture("success.json", |native| {
        native["cli"]["status"] = json!("missing");
        native["health"]["status"] = json!("unavailable");
    });
    let incomplete = with_fixture("success.json", |native| {
        native["docs"]["status"] = json!("partial");
        native["docs"]["payload"]["path"] = json!("partial-docs-payload.json");
        native["docs"]["payload"]["sha256"] =
            json!("d7677aecabbe555f5aebe19cc411ac3cadb5bd77dbfe33dc9c402f55b1385ffa");
    });
    let missing_output = run(&missing);
    assert_eq!(missing_output.status.code(), Some(0));
    let missing_result = result(&missing_output);
    assert_eq!(missing_result["adapter"]["health"]["status"], "unavailable");
    assert_eq!(missing_result["admissions"], json!([]));
    assert_eq!(
        missing_result["adapter"]["factPromotion"]["eligible"],
        false
    );

    let incomplete_output = run(&incomplete);
    assert_eq!(incomplete_output.status.code(), Some(0));
    let incomplete_result = result(&incomplete_output);
    assert_admission(&incomplete_result, "docs", "admitted", "unknown");
    assert_eq!(
        incomplete_result["adapter"]["factPromotion"]["engineeringFacts"],
        json!([])
    );
    fs::remove_file(missing).unwrap();
    fs::remove_file(incomplete).unwrap();
}

#[test]
fn public_route_reports_stale_as_a04_rejection_and_never_leaks_native_diagnostics() {
    let stale = with_fixture("index-only.json", |native| {
        native["collectedAt"] = json!(1_699_999_000u64);
        native["index"]["observedAt"] = json!(1_699_999_000u64);
    });
    let output = run(&stale);
    assert_eq!(output.status.code(), Some(65));
    let value = result(&output);
    assert_admission(&value, "index", "rejected", "unknown");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("super-secret"));
    assert!(!text.contains("sk-live-123"));
    assert!(!text.contains("ANTHROPIC_API_KEY"));
    fs::remove_file(stale).unwrap();
}

#[test]
fn registry_declares_the_exact_public_route() {
    let registry: Value =
        serde_json::from_slice(&fs::read(root().join("orchestration/integrations.json")).unwrap())
            .unwrap();
    let entry = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "provider.repowise-adapt")
        .expect("provider.repowise-adapt must be registered");
    assert_eq!(
        entry["commands"]["adapt"],
        "target/debug/code-intel.exe provider repowise-adapt --request <native.json|-> --artifact-root <artifact-directory> --evaluated-at <unix-seconds> --max-age-seconds <seconds>"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "route",
            "--action",
            "List",
            "--provider",
            "repowise",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let public_registry: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(public_registry["routes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|route| route["operation"] == "adapt"));
}
