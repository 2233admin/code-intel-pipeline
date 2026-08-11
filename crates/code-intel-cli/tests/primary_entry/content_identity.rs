use std::process::Command;

use super::run_lite_json;
use crate::common;

#[test]
fn content_identity_repositories_support_run_query_and_unchanged_rerun() {
    for (label, unborn) in [("unversioned", false), ("unborn", true)] {
        let root = std::env::temp_dir().join(format!(
            "code-intel-content-project-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after the epoch")
                .as_nanos()
        ));
        let repo = root.join(format!("{label}-repo"));
        let artifacts = root.join("artifacts");
        std::fs::create_dir_all(repo.join("src")).expect("create content repository");
        std::fs::create_dir_all(repo.join(".sentrux")).expect("create sentrux fixture");
        std::fs::write(repo.join("README.md"), format!("{label} fixture"))
            .expect("write fixture readme");
        std::fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}")
            .expect("write fixture source");
        std::fs::write(
            repo.join(".sentrux/rules.toml"),
            "[constraints]\nmax_cycles = 0\nmax_coupling = \"F\"\nmax_cc = 100\nno_god_files = false\n",
        )
        .expect("write sentrux rules");
        if unborn {
            let output = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["init", "--quiet"])
                .output()
                .expect("initialize unborn repository");
            assert!(
                output.status.success(),
                "git init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        for operation in ["save_baseline", "check"] {
            let output = common::cli()
                .args(["sentrux", "--operation", operation, "--repo"])
                .arg(&repo)
                .output()
                .expect("prepare sentrux baseline");
            assert!(
                output.status.success(),
                "{label} sentrux {operation} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let first = run_lite_json(&repo, &artifacts);
        assert_eq!(first["outcome"], "completed", "{label} first run={first}");

        let query = common::cli()
            .arg("query")
            .arg(&repo)
            .args(["--kind", "evidence", "--limit", "1", "--json"])
            .env("CODE_INTEL_ARTIFACT_ROOT", &artifacts)
            .output()
            .expect("query content-identity repository");
        assert!(
            query.status.success(),
            "{label} query failed: {}",
            String::from_utf8_lossy(&query.stderr)
        );
        let query: serde_json::Value =
            serde_json::from_slice(&query.stdout).expect("content query JSON");
        assert_eq!(query["freshness"]["status"], "current", "{label}: {query}");

        let second = run_lite_json(&repo, &artifacts);
        assert_eq!(
            second["outcome"], "completed",
            "{label} unchanged rerun={second}"
        );
        assert!(
            artifacts.join(format!("{label}-repo")).is_dir(),
            "{label} authority was published under a different repository key"
        );

        std::fs::write(repo.join("README.md"), format!("{label} fixture changed"))
            .expect("change content-identity fixture");
        let stale = common::cli()
            .arg("query")
            .arg(&repo)
            .args(["--kind", "evidence", "--json"])
            .env("CODE_INTEL_ARTIFACT_ROOT", &artifacts)
            .output()
            .expect("refuse changed content-identity repository");
        assert_eq!(
            stale.status.code(),
            Some(65),
            "{label} query was not refused"
        );
        let stale: serde_json::Value =
            serde_json::from_slice(&stale.stdout).expect("content mismatch error JSON");
        assert!(
            stale["diagnostic"]
                .as_str()
                .is_some_and(|message| message.contains("repository key collision")),
            "{label}: {stale}"
        );

        let _ = std::fs::remove_dir_all(root);
    }
}
