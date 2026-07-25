use super::*;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_bytes() -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audit/audit-report.v1.example.json"),
    )
    .unwrap()
}

fn fixture_value() -> Value {
    serde_json::from_slice(&fixture_bytes()).unwrap()
}

/// The real, on-disk `orchestration/audit/departments.v1.json`. `security`
/// is `enabled: true` (T2: its prompt landed at
/// `orchestration/audit/prompts/security.md`); `ai-safety` and
/// `supply-chain` stay `enabled: false` pending their own department
/// tickets (see FIX 2's enabled-consistency rule). A report validated
/// against this registry must therefore report `security` as `assessed`
/// or `not_assessed` (never `disabled`) and the other two as `disabled`;
/// see `registry_loads_and_validates_the_real_departments_file` and
/// `validates_a_minimal_report_with_security_not_assessed_against_the_real_registry`.
fn real_registry() -> DepartmentRegistry {
    DepartmentRegistry::load(&repo_root()).unwrap()
}

/// A synthetic registry, independent of the real
/// `orchestration/audit/departments.v1.json` (which now has only
/// `security` `enabled: true`), that marks `security`, `ai-safety`, and
/// `supply-chain` all `enabled: true`. The example fixture reports
/// `security` as `assessed` and the other two as `not_assessed` (see FIX
/// 2's enabled-consistency rule (d): `not_assessed` requires `enabled:
/// true`), so every test below that validates the fixture — or a report
/// shaped like it — needs this all-enabled registry, not the real one.
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
fn parses_and_validates_the_example_fixture() {
    let report = AuditReport::parse(&fixture_bytes()).unwrap();
    report.validate(&registry()).unwrap();
}

#[test]
fn rejects_confirmed_finding_without_file_evidence() {
    let mut value = fixture_value();
    value["findings"][0]["evidence"] =
        json!([{"kind": "command", "source": "sentrux check --repo ."}]);
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(error.contains("no file evidence"), "{error}");
}

#[test]
fn rejects_unregistered_department_reference() {
    let mut value = fixture_value();
    value["findings"][0]["department"] = json!("not-a-real-department");
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(error.contains("unregistered department"), "{error}");
}

#[test]
fn rejects_report_missing_a_registered_department() {
    let mut value = fixture_value();
    // Drop the "ai-safety" department run and every entry that
    // references it, so nothing but rule (c)'s exact-membership check
    // can catch the omission.
    value["departments"]
        .as_array_mut()
        .unwrap()
        .retain(|department| department["id"] != "ai-safety");
    value["score_dashboard"]["entries"]
        .as_array_mut()
        .unwrap()
        .retain(|entry| entry["department"] != "ai-safety");
    value["coverage_matrix"]
        .as_array_mut()
        .unwrap()
        .retain(|row| row["department"] != "ai-safety");
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(
        error.contains("missing a department run for registered department \"ai-safety\""),
        "{error}"
    );
}

#[test]
fn rejects_score_entry_for_a_department_absent_from_the_report() {
    let mut value = fixture_value();
    // Drop only the "ai-safety" department run; its score entry (still
    // present) references a *registered* department, so this must be
    // caught by rule (b)'s "present in this report's departments"
    // check, distinctly from an unregistered-department error.
    value["departments"]
        .as_array_mut()
        .unwrap()
        .retain(|department| department["id"] != "ai-safety");
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(
        error.contains("score entry references department \"ai-safety\" that is not present"),
        "{error}"
    );
}

#[test]
fn rejects_assessed_status_when_the_registry_entry_is_disabled() {
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
            {"id":"security","title":"Security","enabled":false,"prompt":"p","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"},
            {"id":"ai-safety","title":"AI Safety","enabled":true,"prompt":"p","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"},
            {"id":"supply-chain","title":"Supply Chain","enabled":true,"prompt":"p","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"}
        ]
    });
    let security_disabled = DepartmentRegistry::from_value(&value).unwrap();
    // The fixture reports "security" as `assessed`, but this registry
    // says "security" is `enabled: false` (so it must be `disabled`).
    let report = AuditReport::parse(&fixture_bytes()).unwrap();
    let error = report.validate(&security_disabled).unwrap_err();
    assert!(
        error.contains(
            "\"security\" is disabled in the registry but its run status is \"assessed\""
        ),
        "{error}"
    );
}

#[test]
fn validates_a_minimal_report_with_security_not_assessed_against_the_real_registry() {
    // The real orchestration/audit/departments.v1.json now registers
    // `security` with `enabled: true` (T2: its prompt landed) while
    // `ai-safety` and `supply-chain` stay `enabled: false` pending their
    // own department tickets. A report that honestly reflects that —
    // `security` `not_assessed` (rule (d) forbids `disabled` once a
    // department is enabled), `ai-safety`/`supply-chain` `disabled`, every
    // score null, every coverage row `not_assessed`, `overall` null — must
    // validate cleanly against it.
    let value = json!({
        "schema": "code-intel-audit-report.v1",
        "generatedAt": null,
        "repo": "code-intel-pipeline",
        "rubric_version": "v1",
        "departments": [
            {
                "id": "security",
                "status": "not_assessed",
                "applicability": {"applicable": "unknown", "reason": "This minimal fixture does not run the security department; no evidence was gathered."}
            },
            {
                "id": "ai-safety",
                "status": "disabled",
                "applicability": {"applicable": "unknown", "reason": "ai-safety is disabled in the registry for this run."}
            },
            {
                "id": "supply-chain",
                "status": "disabled",
                "applicability": {"applicable": "unknown", "reason": "supply-chain is disabled in the registry for this run."}
            }
        ],
        "findings": [],
        "score_dashboard": {
            "entries": [
                {"department": "security", "score": null, "justification": "Not assessed by this minimal fixture."},
                {"department": "ai-safety", "score": null, "justification": "Disabled in the registry for this run."},
                {"department": "supply-chain", "score": null, "justification": "Disabled in the registry for this run."}
            ],
            "overall": null
        },
        "coverage_matrix": [
            {"department": "security", "coverage": "not_assessed", "inspected_evidence": [], "exclusions": ["security was not run by this minimal fixture."]},
            {"department": "ai-safety", "coverage": "not_assessed", "inspected_evidence": [], "exclusions": ["ai-safety is disabled in the registry for this run."]},
            {"department": "supply-chain", "coverage": "not_assessed", "inspected_evidence": [], "exclusions": ["supply-chain is disabled in the registry for this run."]}
        ]
    });
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    report.validate(&real_registry()).unwrap();
}

#[test]
fn rejects_not_assessed_department_with_present_coverage() {
    let mut value = fixture_value();
    let index = value["coverage_matrix"]
        .as_array()
        .unwrap()
        .iter()
        .position(|row| row["department"] == "ai-safety")
        .unwrap();
    value["coverage_matrix"][index]["coverage"] = json!("high");
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(error.contains("expected \"not_assessed\""), "{error}");
}

#[test]
fn rejects_not_assessed_department_with_a_non_null_score() {
    let mut value = fixture_value();
    let index = value["score_dashboard"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .position(|entry| entry["department"] == "ai-safety")
        .unwrap();
    value["score_dashboard"]["entries"][index]["score"] = json!(5.0);
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(error.contains("score entry is not null"), "{error}");
}

#[test]
fn rejects_overall_score_mismatch() {
    let mut value = fixture_value();
    value["score_dashboard"]["overall"] = json!(5.0);
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(error.contains("recomputed mean"), "{error}");
}

#[test]
fn rejects_perfect_score_with_zero_findings_and_non_high_coverage() {
    let mut value = fixture_value();
    value["findings"] = json!([]);
    value["score_dashboard"]["entries"][0]["score"] = json!(10.0);
    value["score_dashboard"]["overall"] = json!(10.0);
    value["coverage_matrix"][0]["coverage"] = json!("medium");
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(error.contains("scored 10.0 with zero findings"), "{error}");
}

#[test]
fn rejects_duplicate_coverage_row_for_a_department() {
    let mut value = fixture_value();
    let duplicate = value["coverage_matrix"][0].clone();
    value["coverage_matrix"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(
        error.contains("coverage rows, expected exactly 1"),
        "{error}"
    );
}

#[test]
fn rejects_duplicate_score_entry_for_a_department() {
    let mut value = fixture_value();
    let duplicate = value["score_dashboard"]["entries"][0].clone();
    value["score_dashboard"]["entries"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(
        error.contains("score entries, expected at most 1"),
        "{error}"
    );
}

#[test]
fn rejects_assessed_department_with_a_missing_score_entry() {
    let mut value = fixture_value();
    let entries = value["score_dashboard"]["entries"].as_array_mut().unwrap();
    entries.retain(|entry| entry["department"] != "security");
    value["score_dashboard"]["overall"] = json!(null);
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(
        error.contains("assessed but has no non-null score entry"),
        "{error}"
    );
}

#[test]
fn rejects_assessed_department_with_a_null_score() {
    let mut value = fixture_value();
    for entry in value["score_dashboard"]["entries"].as_array_mut().unwrap() {
        if entry["department"] == "security" {
            entry["score"] = json!(null);
        }
    }
    value["score_dashboard"]["overall"] = json!(null);
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(
        error.contains("assessed but has no non-null score entry"),
        "{error}"
    );
}

#[test]
fn rejects_assessed_department_with_a_not_assessed_coverage_row() {
    let mut value = fixture_value();
    for row in value["coverage_matrix"].as_array_mut().unwrap() {
        if row["department"] == "security" {
            row["coverage"] = json!("not_assessed");
        }
    }
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = report.validate(&registry()).unwrap_err();
    assert!(
        error.contains("assessed but its coverage row is \"not_assessed\""),
        "{error}"
    );
}
