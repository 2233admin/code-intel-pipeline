//! End-to-end coverage for `code-intel perf-optimize run` (#301): the CLI
//! plumbing (arg parsing, budget/steps derivation, weco presence/BYOK check,
//! baseline denoising, weco invocation, stdout scanning, report assembly)
//! against a fake `weco` binary. The denoising median itself and the
//! `denoise-eval` subcommand are covered in isolation by
//! `src/perf_optimize/denoise_tests.rs` and `denoise_eval_cli_tests.rs`; this
//! test's job is proving the glue between those pieces and a real weco
//! subprocess is correct, not re-proving denoising works.

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

#[cfg(windows)]
fn write_weco_fixture(dir: &Path, run_id: &str, metric_lines: &[&str], then_hang: bool) -> PathBuf {
    let script = dir.join("weco.cmd");
    let mut body = String::from("@echo off\r\nif \"%2\"==\"stop\" (\r\n  exit /b 0\r\n)\r\n");
    body.push_str(&format!("echo Run ID: {run_id}\r\n"));
    for line in metric_lines {
        body.push_str(&format!("echo {line}\r\n"));
    }
    if then_hang {
        body.push_str("ping -n 60 127.0.0.1 >nul\r\n");
    }
    fs::write(&script, body).unwrap();
    script
}

#[cfg(not(windows))]
fn write_weco_fixture(dir: &Path, run_id: &str, metric_lines: &[&str], then_hang: bool) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("weco");
    let mut body = String::from("#!/bin/sh\nif [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\n");
    body.push_str(&format!("echo \"Run ID: {run_id}\"\n"));
    for line in metric_lines {
        body.push_str(&format!("echo \"{line}\"\n"));
    }
    if then_hang {
        body.push_str("sleep 60\n");
    }
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

fn run(dir: &Path, extra_args: &[&str]) -> serde_json::Value {
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
            "--doctor-tool-path-prefix",
        ])
        .arg(dir)
        .env("OPENAI_API_KEY", "test-key-not-a-real-credential")
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
    write_weco_fixture(
        &dir,
        "e2e-fixture-run",
        &["latency_ms: 100", "latency_ms: 80", "latency_ms: 90"],
        false,
    );

    let report = run(
        &dir,
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

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_wall_clock_timeout_reports_budget_exhausted_with_whatever_was_captured() {
    let dir = temp_dir("timeout-path");
    write_weco_fixture(&dir, "e2e-fixture-run-timeout", &["latency_ms: 98"], true);

    let report = run(
        &dir,
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
    // Deliberately empty: if the CLI reached weco detection despite having
    // no eval-command, this would still report unavailable (no weco found
    // here either), so the assertion on `reason` is what actually proves
    // the eval-command check fired first.
    let report = run(&dir, &[]);

    assert_eq!(report["status"], "unavailable");
    assert!(report["reason"]
        .as_str()
        .unwrap()
        .contains("no --eval-command"));

    fs::remove_dir_all(&dir).ok();
}
