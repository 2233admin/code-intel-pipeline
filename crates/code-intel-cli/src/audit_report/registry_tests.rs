use super::*;
use serde_json::json;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn real_registry() -> DepartmentRegistry {
    DepartmentRegistry::load(&repo_root()).unwrap()
}

#[test]
fn registry_loads_and_validates_the_real_departments_file() {
    let registry = real_registry();
    assert_eq!(registry.departments.len(), 3);
    registry.validate(&repo_root()).unwrap();
}

#[test]
fn registry_rejects_duplicate_department_ids() {
    let value = json!({
        "schema": "code-intel-audit-departments.v1",
        "catalogVersion": "1.0.0",
        "rubrics": {
            "severity": "a", "confidence": "b", "evidence": "c",
            "coverage": "d", "scoring": "e"
        },
        "findingContract": "docs/audit-report.md",
        "departments": [
            {"id":"security","title":"Security","enabled":false,"prompt":"p1","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"},
            {"id":"security","title":"Security Again","enabled":false,"prompt":"p2","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"}
        ]
    });
    let registry = DepartmentRegistry::from_value(&value).unwrap();
    let error = registry.validate(&repo_root()).unwrap_err();
    assert!(error.contains("duplicate department id"), "{error}");
}

#[test]
fn registry_rejects_missing_rubric_file() {
    let value = json!({
        "schema": "code-intel-audit-departments.v1",
        "catalogVersion": "1.0.0",
        "rubrics": {
            "severity": "orchestration/audit/rubrics/does-not-exist.md",
            "confidence": "orchestration/audit/rubrics/confidence.md",
            "evidence": "orchestration/audit/rubrics/evidence.md",
            "coverage": "orchestration/audit/rubrics/coverage.md",
            "scoring": "orchestration/audit/rubrics/scoring.md"
        },
        "findingContract": "docs/audit-report.md",
        "departments": []
    });
    let registry = DepartmentRegistry::from_value(&value).unwrap();
    let error = registry.validate(&repo_root()).unwrap_err();
    assert!(error.contains("does not exist"), "{error}");
}

#[test]
fn registry_requires_prompt_file_when_enabled() {
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
            {"id":"synthetic","title":"Synthetic","enabled":true,"prompt":"orchestration/audit/prompts/does-not-exist.md","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"}
        ]
    });
    let registry = DepartmentRegistry::from_value(&value).unwrap();
    let error = registry.validate(&repo_root()).unwrap_err();
    assert!(
        error.contains("enabled but its prompt file does not exist"),
        "{error}"
    );
}

#[test]
fn registry_allows_a_disabled_department_with_a_missing_prompt_file() {
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
            {"id":"synthetic","title":"Synthetic","enabled":false,"prompt":"orchestration/audit/prompts/does-not-exist.md","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"}
        ]
    });
    let registry = DepartmentRegistry::from_value(&value).unwrap();
    registry.validate(&repo_root()).unwrap();
}

#[test]
fn registry_rejects_an_unknown_consumed_modality() {
    let mut value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(repo_root().join("orchestration/audit/departments.v1.json")).unwrap(),
    )
    .unwrap();
    value["departments"][0]["consumes"] = json!(["xray", "holograph"]);
    let error = DepartmentRegistry::from_value(&value).err().unwrap();
    assert!(error.contains("unknown modality \"holograph\""), "{error}");
}
