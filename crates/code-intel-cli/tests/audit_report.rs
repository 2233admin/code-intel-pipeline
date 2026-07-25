use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

// This crate has no lib target (see Cargo.toml — `code-intel` is bin-only),
// so this black-box test cannot call into `audit_report::validate()`
// directly. Structural (deny_unknown_fields) and cross-artifact (registry)
// contract checks that are meaningfully black-box testable live here,
// re-implemented against the tracked files the same way the rest of this
// crate's other tests/*.rs files already do (see sha256_hex in
// hospital_diagnosis.rs). The domain invariants a-f from
// `docs/audit-report.md` (confirmed-needs-file-evidence, unregistered
// department, not_assessed consistency, overall-score recomputation,
// perfect-score-needs-high-coverage, one-row-per-department) exercise the
// real `validate()` logic and live as `#[cfg(test)]` unit tests inside
// `src/audit_report.rs` instead.

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-audit-report-test-{label}-{}-{nonce}-{}.json",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audit/audit-report.v1.example.json")
}

fn fixture_value() -> Value {
    serde_json::from_slice(&fs::read(fixture_path()).unwrap()).unwrap()
}

fn registry_value() -> Value {
    serde_json::from_slice(
        &fs::read(repo_root().join("orchestration/audit/departments.v1.json")).unwrap(),
    )
    .unwrap()
}

/// Mirrors the `Test-Json` shell-out already used by
/// `tests/hospital_diagnosis.rs` for `code-intel-hospital.v1`, but passes the
/// paths through the environment instead of trailing `-Command` arguments:
/// `pwsh -Command "<script>" <arg1> <arg2>` does not reliably bind trailing
/// positional arguments to `$args`/`param()` when launched as a subprocess
/// (verified empirically — they arrive as `$null`), so a schema-rejection
/// case would silently no-op instead of exercising `Test-Json`.
fn validates_against_schema(document: &Path, schema: &Path) -> bool {
    Command::new("pwsh")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "if (-not (Get-Content -Raw -LiteralPath $env:CIP_TEST_JSON_DOCUMENT | Test-Json -SchemaFile $env:CIP_TEST_JSON_SCHEMA -ErrorAction Stop)) { exit 1 }",
        ])
        .env("CIP_TEST_JSON_DOCUMENT", document)
        .env("CIP_TEST_JSON_SCHEMA", schema)
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn fixture_is_valid_json_matching_the_expected_shape() {
    let value = fixture_value();
    assert_eq!(value["schema"], "code-intel-audit-report.v1");
    assert_eq!(value["rubric_version"], "v1");
    assert!(value["generatedAt"].is_null());
    let ids = value["departments"]
        .as_array()
        .expect("departments must be an array")
        .iter()
        .map(|department| department["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["security", "ai-safety", "supply-chain"]);
    assert_eq!(value["findings"].as_array().unwrap().len(), 1);
    assert_eq!(value["score_dashboard"]["overall"], 7.0);
}

#[test]
fn fixture_validates_against_the_audit_report_schema() {
    let schema = repo_root().join("orchestration/schemas/code-intel-audit-report.v1.schema.json");
    assert!(
        validates_against_schema(&fixture_path(), &schema),
        "the example fixture must validate against code-intel-audit-report.v1"
    );
}

#[test]
fn unknown_top_level_field_fails_schema_validation() {
    let mut value = fixture_value();
    value["bogus"] = json!(true);
    let temp = Temp::new("unknown-field");
    fs::write(&temp.0, serde_json::to_vec(&value).unwrap()).unwrap();
    let schema = repo_root().join("orchestration/schemas/code-intel-audit-report.v1.schema.json");
    assert!(
        !validates_against_schema(&temp.0, &schema),
        "additionalProperties: false must reject an unknown top-level field"
    );
}

#[test]
fn registry_file_has_unique_department_ids_and_existing_rubric_and_finding_contract_files() {
    let root = repo_root();
    let registry = registry_value();
    assert_eq!(registry["schema"], "code-intel-audit-departments.v1");
    let ids = registry["departments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|department| department["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "department ids must be unique");
    for label in ["severity", "confidence", "evidence", "coverage", "scoring"] {
        let path = registry["rubrics"][label].as_str().unwrap();
        assert!(
            root.join(path).is_file(),
            "rubrics.{label} file must exist on disk: {path}"
        );
    }
    let finding_contract = registry["findingContract"].as_str().unwrap();
    assert!(
        root.join(finding_contract).is_file(),
        "findingContract file must exist on disk: {finding_contract}"
    );
    // T1 registers exactly these three departments and does not yet run any
    // of them (see docs/audit-report.md and CHANGELOG.md). Check each
    // required slot by id rather than the full list, so a future PR can add
    // more departments without this test needing to change.
    for id in ["security", "ai-safety", "supply-chain"] {
        let department = registry["departments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|department| department["id"] == id)
            .unwrap_or_else(|| panic!("registry must contain a department with id \"{id}\""));
        assert_eq!(
            department["enabled"],
            json!(false),
            "department \"{id}\" must currently be enabled: false"
        );
    }
}

#[test]
fn enabled_departments_in_the_registry_have_an_existing_prompt_file() {
    let root = repo_root();
    let registry = registry_value();
    for department in registry["departments"].as_array().unwrap() {
        if department["enabled"] == json!(true) {
            let prompt = department["prompt"].as_str().unwrap();
            assert!(
                root.join(prompt).is_file(),
                "enabled department \"{}\" must have an existing prompt file: {prompt}",
                department["id"]
            );
        }
    }
}
