//! End-to-end coverage for `code-intel perf-optimize run` (#301): the CLI
//! plumbing (arg parsing, budget/steps derivation, weco presence/BYOK+account
//! check, baseline denoising, weco invocation, `run status`/`run diff`
//! queries, report assembly) against a fake `weco` binary. The denoising
//! median itself and the `denoise-eval` subcommand are covered in isolation
//! by `src/perf_optimize/denoise_tests.rs` and `denoise_eval_cli_tests.rs`;
//! this test's job is proving the glue between those pieces and a real weco
//! subprocess is correct, not re-proving denoising works.
//!
//! The fixture's `status`/`diff` responses are shaped to match weco-cli's
//! real output (confirmed by reading its source, #301 research), not
//! guessed: `run status <id>` prints `{"current_step",...,"best_metric",...}`
//! JSON unconditionally, `run diff <id> --step best --against baseline`
//! prints a raw unified diff on stdout.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

fn temp_dir(tag: &str) -> PathBuf {
    let unique = format!(
        "code-intel-perf-optimize-e2e-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixture_source_file(dir: &Path) -> PathBuf {
    let path = dir.join("fixture_target.rs");
    fs::write(&path, "pub fn fixture() {}\n").unwrap();
    path
}

#[cfg(windows)]
fn write_weco_fixture(
    dir: &Path,
    run_id: &str,
    steps_run: u64,
    best_metric: f64,
    then_hang: bool,
) -> PathBuf {
    let script = dir.join("weco.cmd");
    let run_body = if then_hang {
        "ping -n 60 127.0.0.1 >nul\r\n"
    } else {
        ""
    };
    let body = format!(
        "@echo off\r\n\
         if \"%2\"==\"status\" (\r\n\
         \x20 echo {{\"current_step\": {steps_run}, \"total_steps\": 10, \"best_metric\": {best_metric}, \"status\": \"completed\"}}\r\n\
         \x20 exit /b 0\r\n\
         )\r\n\
         if \"%2\"==\"diff\" (\r\n\
         \x20 echo --- a/fixture_target.rs\r\n\
         \x20 echo +++ b/fixture_target.rs\r\n\
         \x20 exit /b 0\r\n\
         )\r\n\
         if \"%2\"==\"stop\" (\r\n\
         \x20 exit /b 0\r\n\
         )\r\n\
         echo Run ID: {run_id}\r\n\
         {run_body}"
    );
    fs::write(&script, body).unwrap();
    script
}

#[cfg(not(windows))]
fn write_weco_fixture(
    dir: &Path,
    run_id: &str,
    steps_run: u64,
    best_metric: f64,
    then_hang: bool,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("weco");
    let run_body = if then_hang { "sleep 60\n" } else { "" };
    let body = format!(
        "#!/bin/sh\n\
         if [ \"$2\" = \"status\" ]; then\n\
         \x20 echo '{{\"current_step\": {steps_run}, \"total_steps\": 10, \"best_metric\": {best_metric}, \"status\": \"completed\"}}'\n\
         \x20 exit 0\n\
         fi\n\
         if [ \"$2\" = \"diff\" ]; then\n\
         \x20 echo '--- a/fixture_target.rs'\n\
         \x20 echo '+++ b/fixture_target.rs'\n\
         \x20 exit 0\n\
         fi\n\
         if [ \"$2\" = \"stop\" ]; then\n\
         \x20 exit 0\n\
         fi\n\
         echo \"Run ID: {run_id}\"\n\
         {run_body}"
    );
    fs::write(&script, body).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn baseline_eval_command(value: &str) -> String {
    if cfg!(windows) {
        format!("echo latency_ms: {value}")
    } else {
        format!("echo 'latency_ms: {value}'")
    }
}

fn run(dir: &Path, source: &Path, extra_args: &[&str]) -> serde_json::Value {
    let mut command = common::cli();
    command
        .args([
            "perf-optimize",
            "run",
            "--repo",
            ".",
            "--target",
            "fixture::target",
            "--source",
        ])
        .arg(source)
        .args([
            "--metric",
            "latency_ms",
            "--goal",
            "minimize",
            "--doctor-tool-path-prefix",
        ])
        .arg(dir)
        .env("OPENAI_API_KEY", "test-key-not-a-real-credential")
        .env("WECO_API_KEY", "test-account-token-not-a-real-credential")
        .args(extra_args);
    let output = command.output().expect("spawn perf-optimize run");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "perf-optimize run did not print JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn a_candidate_clearing_the_threshold_reports_no_stopped_by() {
    let dir = temp_dir("happy-path");
    let source = fixture_source_file(&dir);
    write_weco_fixture(&dir, "e2e-fixture-run", 3, 80.0, false);

    let report = run(
        &dir,
        &source,
        &[
            "--eval-command",
            &baseline_eval_command("100"),
            "--seconds-per-step",
            "1",
        ],
    );

    assert_eq!(report["schema"], "code-intel-perf-optimize-run.v1");
    assert_eq!(report["baseline"], 100.0);
    assert_eq!(report["bestCandidate"], 80.0);
    assert_eq!(report["stepsRun"], 3);
    assert_eq!(report["metThreshold"], true);
    assert_eq!(report["improvementPercent"], 20.0);
    // A threshold-clearing candidate doesn't need a "why did it stop" story.
    assert_eq!(report["stoppedBy"], serde_json::Value::Null);
    assert!(report["bestCandidateDiff"]
        .as_str()
        .unwrap()
        .contains("fixture_target.rs"));

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_candidate_that_never_clears_the_threshold_reports_steps_exhausted() {
    let dir = temp_dir("steps-exhausted-path");
    let source = fixture_source_file(&dir);
    // weco runs every step it was given and exits cleanly (no timeout), but
    // never finds a candidate good enough to clear the default 5% bar --
    // distinct from the timeout case: nothing here cut the search short.
    write_weco_fixture(&dir, "e2e-fixture-run-exhausted", 3, 97.0, false);

    let report = run(
        &dir,
        &source,
        &[
            "--eval-command",
            &baseline_eval_command("100"),
            "--seconds-per-step",
            "1",
        ],
    );

    assert_eq!(report["baseline"], 100.0);
    assert_eq!(report["bestCandidate"], 97.0);
    assert_eq!(report["stepsRun"], 3);
    // 3% improvement is under the 5% default threshold.
    assert_eq!(report["metThreshold"], false);
    assert_eq!(report["stoppedBy"], "steps_exhausted");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_wall_clock_timeout_reports_budget_exhausted_with_whatever_was_captured() {
    let dir = temp_dir("timeout-path");
    let source = fixture_source_file(&dir);
    write_weco_fixture(&dir, "e2e-fixture-run-timeout", 1, 98.0, true);

    let report = run(
        &dir,
        &source,
        &[
            "--eval-command",
            &baseline_eval_command("100"),
            "--budget-wall-clock",
            "1",
            "--seconds-per-step",
            "1",
            "--grace-period-seconds",
            "1",
        ],
    );

    assert_eq!(report["baseline"], 100.0);
    assert_eq!(report["bestCandidate"], 98.0);
    assert_eq!(report["stepsRun"], 1);
    // 2% improvement is under the 5% default threshold.
    assert_eq!(report["metThreshold"], false);
    assert_eq!(report["stoppedBy"], "budget_exhausted");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_missing_eval_command_reports_unavailable_without_spawning_weco() {
    let dir = temp_dir("no-eval-command");
    let source = fixture_source_file(&dir);
    // Deliberately empty: if the CLI reached weco detection despite having
    // no eval-command, this would still report unavailable (no weco found
    // here either), so the assertion on `reason` is what actually proves
    // the eval-command check fired first.
    let report = run(&dir, &source, &[]);

    assert_eq!(report["status"], "unavailable");
    assert!(report["reason"]
        .as_str()
        .unwrap()
        .contains("no --eval-command"));

    fs::remove_dir_all(&dir).ok();
}

/// #301 correction: `--source` is required just as much as `--eval-command`
/// -- weco can't run at all without a file to mutate.
#[test]
fn a_missing_source_reports_unavailable_without_spawning_weco() {
    let dir = temp_dir("no-source");
    let mut command = common::cli();
    command
        .args([
            "perf-optimize",
            "run",
            "--repo",
            ".",
            "--target",
            "fixture::target",
            "--metric",
            "latency_ms",
            "--goal",
            "minimize",
            "--eval-command",
            &baseline_eval_command("100"),
            "--doctor-tool-path-prefix",
        ])
        .arg(&dir)
        .env("OPENAI_API_KEY", "test-key-not-a-real-credential")
        .env("WECO_API_KEY", "test-account-token-not-a-real-credential");
    let output = command.output().expect("spawn perf-optimize run");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(report["status"], "unavailable");
    assert!(report["reason"].as_str().unwrap().contains("no --source"));

    fs::remove_dir_all(&dir).ok();
}

/// #301 correction: BYOK alone isn't enough -- weco's own account token
/// (`WECO_API_KEY`) is a second, independent gate.
#[test]
fn byok_without_a_weco_account_token_reports_unavailable() {
    let dir = temp_dir("byok-no-account");
    let source = fixture_source_file(&dir);
    write_weco_fixture(&dir, "unused-run-id", 0, 0.0, false);

    let mut command = common::cli();
    command
        .args([
            "perf-optimize",
            "run",
            "--repo",
            ".",
            "--target",
            "fixture::target",
            "--source",
        ])
        .arg(&source)
        .args([
            "--metric",
            "latency_ms",
            "--goal",
            "minimize",
            "--eval-command",
            &baseline_eval_command("100"),
            "--doctor-tool-path-prefix",
        ])
        .arg(&dir)
        .env("OPENAI_API_KEY", "test-key-not-a-real-credential")
        .env_remove("WECO_API_KEY");
    let output = command.output().expect("spawn perf-optimize run");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(report["status"], "unavailable");
    assert!(report["reason"].as_str().unwrap().contains("WECO_API_KEY"));

    fs::remove_dir_all(&dir).ok();
}
