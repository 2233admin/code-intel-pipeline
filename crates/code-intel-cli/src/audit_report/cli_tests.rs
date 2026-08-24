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

/// Adversarial regression corpus for ai-safety-006: named fixture files under
/// `tests/fixtures/audit/`, each a full `code-intel-audit-report.v1` document
/// with exactly one deliberate defect. See the three `run_raw_rejects_*`
/// tests below.
fn adversarial_fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/audit/{name}"))
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
fn parse_requires_repo_for_validate_and_for_render() {
    // ai-safety-003 (issue #34): rendering now runs the same fail-closed
    // validate pipeline, so it needs a repository just like `validate` does.
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
    let error = parse(&render_without_repo).unwrap_err();
    assert!(error.contains("requires --repo"), "{error}");

    let render_with_repo = vec![
        "--operation".to_string(),
        "render".to_string(),
        "--repo".to_string(),
        ".".to_string(),
        "--report".to_string(),
        "report.json".to_string(),
    ];
    assert!(matches!(
        parse(&render_with_repo),
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
    // Same fixture `validate_accepts_the_unmodified_fixture_against_the_real_registry`
    // already proves passes the full disk-backed pipeline against the real
    // registry, so `render` (which now runs that same pipeline) can reuse it.
    let markdown = render(&repo_root(), &fixture_path(), RenderFormat::Markdown).unwrap();
    assert!(markdown.contains("## Audit"));
    assert!(markdown.contains("| security |"));
}

#[test]
fn render_with_html_format_produces_a_self_contained_document() {
    let html = render(&repo_root(), &fixture_path(), RenderFormat::Html).unwrap();
    assert!(html.starts_with("<!doctype html>"), "{html}");
    assert!(html.contains("code-intel-pipeline"), "{html}");
}

#[test]
fn render_rejects_a_report_that_fails_validation() {
    // ai-safety-003 (issue #34): render must fail closed exactly like
    // validate does, never printing output for a report that would not
    // pass `--operation validate`.
    let mut value = fixture_value();
    for department in value["departments"].as_array_mut().unwrap() {
        if department["id"] == "ai-safety" {
            department["status"] = json!("disabled");
        }
    }
    let temp = TempReport::write(&value);
    let error = render(&repo_root(), &temp.0, RenderFormat::Markdown).unwrap_err();
    assert!(error.contains("run status is \"disabled\""), "{error}");
}

#[test]
fn parse_format_defaults_to_markdown_and_accepts_html() {
    let without_format = vec![
        "--operation".to_string(),
        "render".to_string(),
        "--repo".to_string(),
        ".".to_string(),
        "--report".to_string(),
        "report.json".to_string(),
    ];
    assert!(matches!(
        parse(&without_format),
        Ok(Operation::Render {
            format: RenderFormat::Markdown,
            ..
        })
    ));

    let with_html = vec![
        "--operation".to_string(),
        "render".to_string(),
        "--repo".to_string(),
        ".".to_string(),
        "--report".to_string(),
        "report.json".to_string(),
        "--format".to_string(),
        "html".to_string(),
    ];
    assert!(matches!(
        parse(&with_html),
        Ok(Operation::Render {
            format: RenderFormat::Html,
            ..
        })
    ));
}

#[test]
fn parse_rejects_unknown_format() {
    let raw = vec![
        "--operation".to_string(),
        "render".to_string(),
        "--report".to_string(),
        "report.json".to_string(),
        "--format".to_string(),
        "pdf".to_string(),
    ];
    let error = parse(&raw).unwrap_err();
    assert!(error.contains("unknown --format"), "{error}");
}

#[test]
fn parse_scope_requires_repo_and_since() {
    let missing_repo = vec![
        "--operation".to_string(),
        "scope".to_string(),
        "--since".to_string(),
        "HEAD~1".to_string(),
    ];
    let error = parse(&missing_repo).unwrap_err();
    assert!(error.contains("requires --repo"), "{error}");

    let missing_since = vec![
        "--operation".to_string(),
        "scope".to_string(),
        "--repo".to_string(),
        ".".to_string(),
    ];
    let error = parse(&missing_since).unwrap_err();
    assert!(error.contains("requires --since"), "{error}");
}

/// Regression for issue #320: the original version of this test ran
/// `scope()` against the *live* checkout (`repo_root()`), which silently
/// assumes `HEAD~1` and `HEAD` always differ on disk. That assumption
/// breaks for delete-only commits and — a second, broader trigger found
/// while investigating — for any merge commit whose tree is identical to
/// its first parent (e.g. a GitHub test-merge commit for a PR whose
/// content was already squash-merged into the base branch). Both are real
/// repository topologies `scope()` must handle without panicking; a test
/// asserting on the ambient repo's current commit shape can't pin that
/// down. Uses a synthetic two-commit fixture repo instead, mirroring
/// `sentrux_evolution::tests::init_repo`.
#[test]
fn scope_against_head_tilde_1_lists_only_existing_changed_files() {
    let repo = init_scope_repo();
    let value = scope(&repo, "HEAD~1").unwrap();
    assert_eq!(value["kind"], "diff");
    assert_eq!(value["since"], "HEAD~1");
    let files = value["files"].as_array().unwrap();
    assert_eq!(
        files,
        &vec![Value::String("second.txt".to_string())],
        "{value}"
    );
    for file in files {
        let relative = file.as_str().unwrap();
        assert!(
            repo.join(relative).is_file(),
            "{relative} should exist on disk"
        );
    }
    fs::remove_dir_all(&repo).ok();
}

/// Two commits: the first adds `first.txt`, the second adds `second.txt`.
/// `HEAD~1...HEAD` is guaranteed non-empty and points at a file that still
/// exists on disk, independent of whatever the ambient checkout's actual
/// commit graph looks like.
fn init_scope_repo() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "code-intel-audit-scope-{nonce}-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("scope fixture dir");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap_or_else(|error| panic!("git {args:?}: {error}"))
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Test"]);
    fs::write(root.join("first.txt"), "first\n").expect("write first.txt");
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "first"]);
    fs::write(root.join("second.txt"), "second\n").expect("write second.txt");
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "second"]);
    root
}

#[test]
fn scope_rejects_a_bogus_since_ref() {
    let error = scope(&repo_root(), "not-a-real-ref-xyz").unwrap_err();
    assert!(!error.is_empty());
}

#[test]
fn run_raw_exits_nonzero_on_a_bogus_scope_ref() {
    let raw = vec![
        "--operation".to_string(),
        "scope".to_string(),
        "--repo".to_string(),
        repo_root().to_string_lossy().into_owned(),
        "--since".to_string(),
        "not-a-real-ref-xyz".to_string(),
    ];
    assert_eq!(run_raw(&raw), 65);
}

// ai-safety-006: adversarial fixture corpus for the audit validate pipeline.
// Each test below runs the exact `code-intel audit --operation validate`
// entry point (`run_raw`) against a fixture that looks superficially like a
// valid report but carries one fabricated or internally inconsistent claim —
// the failure mode a compromised or careless department-agent run would need
// to slip past to report a false clean bill of health (see ai-safety-002 and
// docs/audit-report.md's Untrusted Content Boundary). Before this corpus
// existed, nothing in the test suite exercised fabrication/inconsistency
// resistance for `--operation validate`.
//
// `fail()` maps every validate failure to exit code 65, so each test also
// runs the underlying `validate` directly and pins the rejection reason to
// the one deliberate defect its fixture carries — otherwise an incidental
// earlier rejection (a fixture drifting out of shape, say) would still pass.

#[test]
fn run_raw_rejects_a_report_citing_a_nonexistent_evidence_file() {
    let report = adversarial_fixture_path("invalid-evidence-path.json");
    let error = validate(&repo_root(), &report).unwrap_err();
    assert!(
        error.contains(
            "cites evidence that does not resolve under the repository: \
             crates/code-intel-cli/src/audit_report/does-not-exist-ai-safety-006.rs"
        ),
        "{error}"
    );
    let raw = vec![
        "--operation".to_string(),
        "validate".to_string(),
        "--repo".to_string(),
        repo_root().to_string_lossy().into_owned(),
        "--report".to_string(),
        report.to_string_lossy().into_owned(),
    ];
    assert_eq!(run_raw(&raw), 65);
}

#[test]
fn run_raw_rejects_a_report_with_a_reversed_line_range() {
    let report = adversarial_fixture_path("invalid-line-range.json");
    let error = validate(&repo_root(), &report).unwrap_err();
    assert!(
        error.contains("with line_end 10 before line_start 50"),
        "{error}"
    );
    let raw = vec![
        "--operation".to_string(),
        "validate".to_string(),
        "--repo".to_string(),
        repo_root().to_string_lossy().into_owned(),
        "--report".to_string(),
        report.to_string_lossy().into_owned(),
    ];
    assert_eq!(run_raw(&raw), 65);
}

#[test]
fn run_raw_rejects_a_report_fabricating_high_coverage_for_unassessed_departments() {
    let report = adversarial_fixture_path("fabricated-coverage-claim.json");
    let error = validate(&repo_root(), &report).unwrap_err();
    assert!(
        error.contains("is not_assessed but its coverage row is \"high\""),
        "{error}"
    );
    let raw = vec![
        "--operation".to_string(),
        "validate".to_string(),
        "--repo".to_string(),
        repo_root().to_string_lossy().into_owned(),
        "--report".to_string(),
        report.to_string_lossy().into_owned(),
    ];
    assert_eq!(run_raw(&raw), 65);
}
