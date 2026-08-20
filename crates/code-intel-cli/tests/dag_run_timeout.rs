use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{doctor_tool_fixture, temp_dir};

fn doctor_tool_timeout_fixture(root: &Path) -> PathBuf {
    let bin = doctor_tool_fixture(root, true);
    #[cfg(windows)]
    fs::write(
        bin.join("sentrux.cmd"),
        "@echo off\r\nfor /L %%i in (1,1,50000000) do @rem\r\necho Enforce architectural rules\r\necho Tier: pro\r\nexit /b 0\r\n",
    )
    .unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = bin.join("sentrux");
        fs::write(&path, "#!/bin/sh\nsleep 5\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

pub(super) fn production_run_timeout_keeps_sibling_artifact_and_commits_manifest() {
    let root = temp_dir();
    let repo = root.join("fixture-repo");
    let source = root.join("run-source");
    let authority = root.join("authority");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(&authority).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let doctor_tools = doctor_tool_timeout_fixture(&root);
    let started = Instant::now();

    let output = crate::common::cli()
        .args(["run", "execute", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&source)
        .arg("--authority-root")
        .arg(&authority)
        .args([
            "--final-name",
            "timeout-run",
            "--profile",
            "offline",
            "--skip-sentrux",
            "true",
            "--max-concurrency",
            "2",
            "--node-timeout",
            "1",
        ])
        .arg("--doctor-tool-path-prefix")
        .arg(&doctor_tools)
        .output()
        .unwrap();
    let elapsed = started.elapsed();

    assert_eq!(
        output.status.code(),
        Some(71),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "timed-out run waited for the sleeping child: {elapsed:?}"
    );
    let execution: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(execution["outcome"], "timeout");
    assert_eq!(execution["failures"]["timeout"][0]["node"], "doctor");
    assert!(execution["failures"]["process"]
        .as_array()
        .unwrap()
        .is_empty());
    let manifest = &execution["manifest"];
    let timeout = &manifest["nodes"]["doctor"];
    assert_eq!(timeout["status"], "timeout");
    let diagnostic = timeout["diagnostic"].as_str().unwrap();
    assert!(diagnostic.contains("doctor"));
    assert!(diagnostic.contains("configured timeout limit: 1000ms"));
    assert!(diagnostic.contains("actual elapsed"));

    assert_eq!(manifest["nodes"]["repo.snapshot"]["status"], "succeeded");
    assert!(source.join("repo.snapshot/snapshot.json").is_file());
    let publication = PathBuf::from(execution["publication"]["path"].as_str().unwrap());
    assert!(publication.join("run-complete.json").is_file());
    let marker: Value =
        serde_json::from_slice(&fs::read(publication.join("run-complete.json")).unwrap()).unwrap();
    let published_manifest = publication.join(marker["manifest"]["path"].as_str().unwrap());
    assert!(published_manifest.is_file());

    let _ = fs::remove_dir_all(root);
}
