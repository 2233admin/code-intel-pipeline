use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{commit_fixture, manifest_of, run_lite_json};
use crate::common;

#[path = "../support/sha256.rs"]
mod sha256;

const READABLE_TYPES: [(&str, &str); 3] = [
    ("hospital", "diagnosis.hospital"),
    ("hospitalMarkdown", "diagnosis.hospital-view"),
    ("agentCodeSliceRanking", "code_evidence.agent_slice"),
];

fn fixture_repo(label: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "code-intel-readable-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after the epoch")
            .as_nanos()
    ));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("create fixture repository");
    std::fs::write(repo.join("README.md"), "fixture\n").expect("write readme");
    std::fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").expect("write source");
    let output = common::cli()
        .args(["sentrux", "--operation", "save_baseline", "--repo"])
        .arg(&repo)
        .output()
        .expect("save sentrux baseline");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let artifacts = root.join("artifacts");
    (root, repo, artifacts)
}

/// Every advertised path is a file under the publication whose bytes match the
/// committed manifest's ref of the same type; returns the names checked.
fn assert_readable_artifacts(result: &Value) -> Vec<String> {
    let publication = Path::new(result["publication"]["path"].as_str().expect("publication"));
    let manifest = manifest_of(publication);
    let readable = result["readableArtifacts"]
        .as_object()
        .unwrap_or_else(|| panic!("readableArtifacts is an object: {result}"));
    for (name, value) in readable {
        let path = Path::new(value.as_str().expect("readable path is a string"));
        assert!(path.starts_with(publication), "{name}: {}", path.display());
        assert!(path.is_file(), "{name} is not a file: {}", path.display());
        let bytes = std::fs::read(path).expect("read artifact");
        let (_, artifact_type) = READABLE_TYPES
            .iter()
            .find(|(field, _)| field == name)
            .unwrap_or_else(|| panic!("unexpected readable field {name}"));
        let committed = manifest["nodes"]
            .as_object()
            .expect("manifest nodes")
            .values()
            .flat_map(|node| node["artifacts"].as_array().into_iter().flatten())
            .find(|artifact| artifact["type"] == *artifact_type)
            .unwrap_or_else(|| panic!("no committed ref of type {artifact_type}"));
        assert_eq!(
            Some(sha256::sha256_hex(&bytes).as_str()),
            committed["sha256"].as_str(),
            "{name}"
        );
        if *name == "hospitalMarkdown" {
            String::from_utf8(bytes).expect("markdown is UTF-8");
        } else {
            serde_json::from_slice::<Value>(&bytes).expect("artifact is JSON");
        }
    }
    readable.keys().cloned().collect()
}

#[test]
fn completed_lite_run_advertises_committed_objects() {
    let (root, repo, artifacts) = fixture_repo("lite");
    commit_fixture(&repo, "fixture");

    let result = run_lite_json(&repo, &artifacts);

    assert_eq!(result["outcome"], "completed", "{result}");
    let names = assert_readable_artifacts(&result);
    assert!(
        names.contains(&"agentCodeSliceRanking".to_string()),
        "{names:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn domain_failed_normal_run_advertises_committed_objects() {
    let (root, repo, artifacts) = fixture_repo("normal");
    std::fs::write(repo.join("src/god.rs"), "pub fn wide() {}\n".repeat(900))
        .expect("write god file");
    commit_fixture(&repo, "fixture");
    let tools = root.join("tools");
    std::fs::create_dir_all(&tools).expect("create stub tools");
    for name in ["python", "repowise"] {
        #[cfg(windows)]
        std::fs::write(
            tools.join(format!("{name}.cmd")),
            "@echo off\r\nexit /b 0\r\n",
        )
        .expect("write stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = tools.join(name);
            std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write stub");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod stub");
        }
    }
    let binary_dir = Path::new(env!("CARGO_BIN_EXE_code-intel"))
        .parent()
        .expect("binary dir")
        .to_path_buf();
    let mut paths = vec![tools, binary_dir];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));

    let output = common::cli()
        .arg("run")
        .arg(&repo)
        .args(["--mode", "normal", "--json"])
        .env("CODE_INTEL_ARTIFACT_ROOT", &artifacts)
        .env("PATH", std::env::join_paths(paths).expect("fixture PATH"))
        .output()
        .expect("run normal mode");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "unparsable output ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });

    assert_eq!(output.status.code(), Some(10), "{result}");
    assert_eq!(result["outcome"], "domain_failed", "{result}");
    assert_eq!(result["exitCode"], 10, "{result}");
    assert_eq!(result["failureNode"], "diagnosis.hospital", "{result}");
    let mut names = assert_readable_artifacts(&result);
    names.sort();
    assert_eq!(
        names,
        ["agentCodeSliceRanking", "hospital", "hospitalMarkdown"],
        "{result}"
    );
    let _ = std::fs::remove_dir_all(root);
}
