use super::*;
use serde_json::{json, Value};
use std::{fs, path::Path};

/// The path this fixture's only finding cites as `file` evidence
/// (`tests/fixtures/audit/audit-report.v1.example.json`).
const EVIDENCE_PATH: &str = "crates/code-intel-cli/src/capability_inventory.rs";

fn fixture_value() -> Value {
    serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audit/audit-report.v1.example.json"),
        )
        .unwrap(),
    )
    .unwrap()
}

/// A synthetic all-enabled registry, independent of whatever
/// `orchestration/audit/departments.v1.json` grows next. Duplicated from
/// `validate_tests::registry()` rather than shared: sibling test modules
/// (`tests` and this one, both attached to `validate.rs`) do not see each
/// other's private items, only their common ancestor's.
fn registry() -> DepartmentRegistry {
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

#[test]
fn rejects_diff_scope_missing_since() {
    let mut value = fixture_value();
    value["scope"] = json!({"kind": "diff", "files": [EVIDENCE_PATH]});
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(error.contains("scope.since must be present"), "{error}");
}

#[test]
fn rejects_diff_scope_with_empty_since() {
    let mut value = fixture_value();
    value["scope"] = json!({"kind": "diff", "since": "", "files": [EVIDENCE_PATH]});
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(error.contains("scope.since must be present"), "{error}");
}

#[test]
fn rejects_diff_scope_missing_files() {
    let mut value = fixture_value();
    value["scope"] = json!({"kind": "diff", "since": "HEAD~1"});
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(error.contains("scope.files must be non-empty"), "{error}");
}

#[test]
fn rejects_finding_evidence_path_outside_diff_scope() {
    let mut value = fixture_value();
    value["scope"] = json!({
        "kind": "diff",
        "since": "HEAD~1",
        "files": ["some/other/file.rs"]
    });
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(error.contains("outside the declared diff scope"), "{error}");
}

#[test]
fn accepts_finding_evidence_path_inside_diff_scope_with_backslash_normalization() {
    let mut value = fixture_value();
    // scope.files spelled with backslashes; the fixture's evidence path
    // already uses forward slashes — rule (j) must normalise both sides,
    // not just the scope side or just the evidence side.
    let windows_style = EVIDENCE_PATH.replace('/', "\\");
    value["scope"] = json!({
        "kind": "diff",
        "since": "HEAD~1",
        "files": [windows_style]
    });
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    report.validate(&registry()).unwrap();
}

#[test]
fn full_scope_carries_no_path_restriction() {
    let mut value = fixture_value();
    value["scope"] = json!({"kind": "full"});
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    report.validate(&registry()).unwrap();
}
