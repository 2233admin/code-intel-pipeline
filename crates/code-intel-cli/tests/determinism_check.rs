//! R1.1 end-to-end: `determinism check` over a tiny fixture repository, using
//! the same fast stub-tool fixture `dag_run.rs`'s production-route tests use
//! (`--doctor-tool-path-prefix`) so this stays a fast `cargo test` case
//! rather than a multi-minute self-scan.

mod common;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn temp_dir() -> PathBuf {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "code-intel-determinism-check-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

/// Fast stand-ins for `rg`/`git`/`python`/`repowise`/`sentrux` so a run
/// completes in well under a second instead of shelling out to (or failing
/// to find) the real toolchain — the same fixture `dag_run.rs`'s
/// `production_run_route_executes_snapshot_then_inventory` uses.
fn doctor_tool_fixture(root: &Path) -> PathBuf {
    let bin = root.join("doctor-tools");
    fs::create_dir_all(&bin).unwrap();
    #[cfg(windows)]
    {
        for name in ["rg", "git", "python", "repowise"] {
            fs::write(
                bin.join(format!("{name}.cmd")),
                "@echo off\r\nexit /b 0\r\n",
            )
            .unwrap();
        }
        fs::write(
            bin.join("sentrux.cmd"),
            "@echo off\r\necho Enforce architectural rules\r\necho Tier: pro\r\nexit /b 0\r\n",
        )
        .unwrap();
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["rg", "git", "python", "repowise"] {
            let path = bin.join(name);
            fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = bin.join("sentrux");
        fs::write(
            &path,
            "#!/bin/sh\necho 'Enforce architectural rules'\necho 'Tier: pro'\nexit 0\n",
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

#[test]
fn three_runs_over_a_stable_fixture_repo_are_byte_identical_after_stripping_timestamps() {
    let root = temp_dir();
    let repo = root.join("repo");
    let out = root.join("determinism-out");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let doctor_tools = doctor_tool_fixture(&root);

    let output = common::cli()
        .args(["determinism", "check", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .arg("--runs")
        .arg("3")
        .arg("--doctor-tool-path-prefix")
        .arg(&doctor_tools)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema"], "code-intel-determinism-report.v1");
    assert_eq!(report["runs"], 3);
    assert_eq!(
        report["consistent"], true,
        "unexpected divergence: {report}"
    );
    assert!(report["firstDivergence"].is_null());
    let outcomes = report["runOutcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), 3);
    for outcome in outcomes {
        assert_eq!(outcome["outcome"], "completed", "report={report}");
    }
}

#[test]
fn a_repository_that_is_not_a_directory_is_a_usage_error() {
    let root = temp_dir();
    fs::create_dir_all(&root).unwrap();
    let output = common::cli()
        .args(["determinism", "check", "--repo"])
        .arg(root.join("does-not-exist"))
        .arg("--out")
        .arg(root.join("out"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
}

#[test]
fn runs_below_two_is_a_usage_error() {
    let root = temp_dir();
    fs::create_dir_all(&root).unwrap();
    let output = common::cli()
        .args(["determinism", "check", "--repo"])
        .arg(&root)
        .arg("--out")
        .arg(root.join("out"))
        .arg("--runs")
        .arg("1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--runs must be at least 2"));
}
