//! Binary-level coverage for `code-intel verify <path> [--json]` (issue
//! #367): the aggregating gate that composes `lint hardcoded-paths`,
//! `sentrux gate`, and `repin`'s check-only scan into one pass/fail verdict.
mod common;
#[path = "../src/content_contract.rs"]
mod content_contract;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("code-intel-verify-{label}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(repo: &Path) {
    git(repo, &["init", "--quiet"]);
    git(repo, &["config", "user.name", "Verify Test"]);
    git(repo, &["config", "user.email", "verify@example.invalid"]);
    git(repo, &["config", "core.autocrlf", "false"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
    git(repo, &["config", "core.hooksPath", ""]);
}

fn commit_all(repo: &Path, message: &str) {
    git(repo, &["add", "."]);
    git(repo, &["commit", "--quiet", "-m", message]);
}

fn sha256_of(path: &Path) -> String {
    content_contract::sha256_hex(&fs::read(path).unwrap())
}

/// A minimal clean fixture: one tracked source file, nothing that trips
/// `lint hardcoded-paths` (it only scans `.ps1`/`.psm1`/`.md`/`.yml`) or
/// `repin` (no 64-hex pin tokens anywhere).
fn clean_fixture(label: &str) -> TempTree {
    let tree = TempTree::new(label);
    init_repo(&tree.0);
    fs::write(tree.0.join("lib.rs"), "pub fn entry() {}\n").unwrap();
    commit_all(&tree.0, "init");
    tree
}

fn save_baseline(repo: &Path) {
    let output = common::cli()
        .args(["sentrux", "--operation", "save_baseline", "--repo"])
        .arg(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "save_baseline: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn verify_json(repo: &Path) -> Output {
    common::cli()
        .arg("verify")
        .arg(repo)
        .arg("--json")
        .output()
        .unwrap()
}

fn verify_text(repo: &Path) -> Output {
    common::cli().arg("verify").arg(repo).output().unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "verify output is not JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn check_by_name<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .find(|check| check["name"] == name)
        .unwrap_or_else(|| panic!("no sub-check named {name} in {report}"))
}

#[test]
fn clean_repo_reports_ok_true_with_all_three_check_results() {
    let tree = clean_fixture("clean");
    save_baseline(&tree.0);

    let output = verify_json(&tree.0);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["schema"], "code-intel-verify-report.v1");
    assert_eq!(report["ok"], true);
    let checks = report["checks"].as_array().expect("checks array");
    assert_eq!(checks.len(), 3);
    for name in ["lint hardcoded-paths", "sentrux gate", "repin (check-only)"] {
        let check = check_by_name(&report, name);
        assert_eq!(check["status"], "pass", "check {name}: {check}");
    }
}

#[test]
fn missing_sentrux_baseline_fails_the_gate_subcheck_and_the_overall_verdict() {
    // No save_baseline call: `sentrux gate` treats an ungoverned repository
    // as a real failure, and so must `verify`.
    let tree = clean_fixture("missing-baseline");

    let output = verify_json(&tree.0);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["ok"], false);
    assert_eq!(check_by_name(&report, "sentrux gate")["status"], "fail");
    // The other two sub-checks are unrelated to the missing baseline and
    // must still report their own honest result, not fail closed.
    assert_eq!(
        check_by_name(&report, "lint hardcoded-paths")["status"],
        "pass"
    );
    assert_eq!(
        check_by_name(&report, "repin (check-only)")["status"],
        "pass"
    );
}

#[test]
fn stale_declared_pin_fails_the_repin_subcheck_without_mutating_the_repo() {
    let tree = TempTree::new("stale-pin");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init source");
    save_baseline(&tree.0);
    commit_all(&tree.0, "save baseline");

    let head_digest = sha256_of(&tree.0.join("source.rs"));
    fs::write(
        tree.0.join("record.json"),
        format!(r#"{{"source":{{"path":"source.rs","sha256":"{head_digest}"}}}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "pin record");

    // Edit the source; record.json's pin is now stale relative to the
    // worktree. Snapshot every tracked file's bytes first so we can prove
    // `verify` never touches any of them (issue #367's "never --write"
    // requirement).
    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();
    let before: Vec<(PathBuf, Vec<u8>)> = ["source.rs", "record.json"]
        .iter()
        .map(|name| {
            let path = tree.0.join(name);
            (path.clone(), fs::read(&path).unwrap())
        })
        .collect();

    let output = verify_json(&tree.0);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["ok"], false);
    let repin_check = check_by_name(&report, "repin (check-only)");
    assert_eq!(repin_check["status"], "fail");
    assert_eq!(repin_check["detail"]["clean"], false);
    assert_eq!(repin_check["detail"]["stalePins"][0]["file"], "record.json");

    // Never `--write`: every file byte-identical to before the run.
    for (path, contents) in &before {
        assert_eq!(
            &fs::read(path).unwrap(),
            contents,
            "verify must never mutate {}",
            path.display()
        );
    }
}

#[test]
fn human_output_reports_a_short_pass_fail_summary_per_subcheck() {
    let tree = clean_fixture("human-missing-baseline");
    // No baseline saved: forces a mixed pass/fail summary.
    let output = verify_text(&tree.0);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("code-intel verify: FAILED\n"),
        "{stdout}"
    );
    assert!(stdout.contains("[PASS] lint hardcoded-paths"), "{stdout}");
    assert!(stdout.contains("[FAIL] sentrux gate"), "{stdout}");
    assert!(stdout.contains("[PASS] repin (check-only)"), "{stdout}");
}

#[test]
fn missing_path_argument_is_a_usage_error_not_a_false_ok() {
    let output = common::cli().arg("verify").output().unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a <path>"));
}

#[test]
fn verify_never_passes_a_mutating_flag_to_its_sub_checks() {
    // Adversarial code-review-style assertion (AGENTS.md #7's "check anyway"
    // instruction): the compiled binary's argv contract for `verify` must
    // not accept or forward `--write`. A stray `--write` on the `verify`
    // invocation itself is simply an unrecognized positional/ignored flag,
    // not a mutating switch, because `run_raw` never reads it.
    let tree = clean_fixture("no-write-flag");
    save_baseline(&tree.0);
    let before = fs::read(tree.0.join("lib.rs")).unwrap();

    let output = common::cli()
        .arg("verify")
        .arg(&tree.0)
        .arg("--write")
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");

    assert_eq!(fs::read(tree.0.join("lib.rs")).unwrap(), before);
}
