use super::*;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audit/audit-report.v1.example.json")
}

fn fixture_bytes() -> Vec<u8> {
    fs::read(fixture_path()).unwrap()
}

fn fixture_value() -> Value {
    serde_json::from_slice(&fixture_bytes()).unwrap()
}

/// Mirrors `audit_report::validate_tests::registry()`: a fixed in-memory
/// registry with all three departments `enabled: true`, so the tests that
/// exercise the report-shape rules stay independent of whatever the on-disk
/// registry grows next. The disk-backed path is covered separately by
/// `validate_accepts_the_unmodified_fixture_against_the_real_registry`.
fn synthetic_enabled_registry() -> DepartmentRegistry {
    let value = json!({
        "schema": "code-intel-audit-departments.v1",
        "catalogVersion": "1.0.0",
        "rubrics": {
            "severity": "orchestration/audit/rubrics/severity.md",
            "confidence": "orchestration/audit/rubrics/confidence.md",
            "evidence": "orchestration/audit/rubrics/evidence.md",
            "coverage": "orchestration/audit/rubrics/coverage.md",
            "scoring": "orchestration/audit/rubrics/scoring.md"
        },
        "findingContract": "docs/audit-report.md",
        "departments": [
            {"id":"security","title":"Security","enabled":true,"prompt":"orchestration/audit/prompts/security.md","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"},
            {"id":"ai-safety","title":"AI Safety","enabled":true,"prompt":"orchestration/audit/prompts/ai-safety.md","consumes":[],"applicabilityCheck":"ai-surface","trackingIssue":"t"},
            {"id":"supply-chain","title":"Supply Chain","enabled":true,"prompt":"orchestration/audit/prompts/supply-chain.md","consumes":[],"applicabilityCheck":"manifests-present","trackingIssue":"t"}
        ]
    });
    DepartmentRegistry::from_value(&value).unwrap()
}

struct TempReport(PathBuf);

impl TempReport {
    fn write(value: &Value) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-audit-cli-test-{}-{nonce}.json",
            std::process::id()
        ));
        fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        Self(path)
    }
}

impl Drop for TempReport {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn parse_requires_repo_for_validate_but_not_for_render() {
    let validate_without_repo = vec![
        "--operation".to_string(),
        "validate".to_string(),
        "--report".to_string(),
        "report.json".to_string(),
    ];
    let error = parse(&validate_without_repo).unwrap_err();
    assert!(error.contains("requires --repo"), "{error}");

    let render_without_repo = vec![
        "--operation".to_string(),
        "render".to_string(),
        "--report".to_string(),
        "report.json".to_string(),
    ];
    assert!(matches!(
        parse(&render_without_repo),
        Ok(Operation::Render { .. })
    ));
}

#[test]
fn validate_report_accepts_the_example_fixture_against_a_synthetic_enabled_registry() {
    let report = AuditReport::parse(&fixture_bytes()).unwrap();
    let summary = validate_report(&report, &synthetic_enabled_registry()).unwrap();
    assert_eq!(summary["ok"], true);
    assert_eq!(summary["findings_total"], 1);
    assert_eq!(summary["overall"], 7.0);
    assert_eq!(summary["departments_assessed"], 1);
}

#[test]
fn validate_accepts_the_unmodified_fixture_against_the_real_registry() {
    // All three departments are now enabled on disk, and the fixture reports
    // `security` assessed with the other two `not_assessed` — the shape rule
    // (d) requires of an enabled department that did not run. Exercise the
    // full disk-backed pipeline (load + registry.validate + report.validate)
    // against the REAL repository registry, no adjustment needed.
    let summary = validate(&repo_root(), &fixture_path()).unwrap();
    assert_eq!(summary["ok"], true);
    assert_eq!(summary["findings_total"], 1);
    assert_eq!(summary["overall"], 7.0);
    assert_eq!(summary["departments_assessed"], 1);
}

#[test]
fn validate_rejects_a_disabled_run_status_against_the_real_registry() {
    // Every registry entry is enabled, so rule (d) rejects a report that
    // reports any department as `disabled`.
    let mut value = fixture_value();
    for department in value["departments"].as_array_mut().unwrap() {
        if department["id"] == "ai-safety" {
            department["status"] = json!("disabled");
        }
    }
    let temp = TempReport::write(&value);
    let error = validate(&repo_root(), &temp.0).unwrap_err();
    assert!(error.contains("run status is \"disabled\""), "{error}");
}

#[test]
fn run_raw_exits_nonzero_on_a_failing_validate() {
    let mut value = fixture_value();
    value["score_dashboard"]["overall"] = json!(1.0);
    let temp = TempReport::write(&value);
    let raw = vec![
        "--operation".to_string(),
        "validate".to_string(),
        "--repo".to_string(),
        repo_root().to_string_lossy().into_owned(),
        "--report".to_string(),
        temp.0.to_string_lossy().into_owned(),
    ];
    assert_eq!(run_raw(&raw), 65);
}

#[test]
fn render_prints_the_audit_markdown_section() {
    let markdown = render(&fixture_path()).unwrap();
    assert!(markdown.contains("## Audit"));
    assert!(markdown.contains("| security |"));
}
