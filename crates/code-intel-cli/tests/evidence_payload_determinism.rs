//! An `observed.evidence.payload` is content-addressed: `payload.sha256` is
//! the identity the admission envelope references and the publication layer
//! dedupes on. It must therefore be a pure function of the snapshot.
//!
//! Two `run dag-coordinate` executions over one unchanged tree used to emit
//! two different payload digests, because the provider stamped its collection
//! wall-clock into the hashed bytes (`provenance.observedAt`, and
//! `graph.generatedAtUnix` inside the embedded architecture graph). A 1.9 MB
//! graph payload was re-published on every run, and the digest churn
//! propagated into `admissionIdentity`.
//!
//! Freshness is not lost: `observedAt` still rides in the native result, the
//! provider port and the A04 observation, which is where
//! `admissibility::validate_sealed` reads it.
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn temp_dir() -> PathBuf {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "code-intel-f3-payload-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

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

/// Name the manifest this test means. Without it the spawned binary resolves
/// whatever `CODE_INTEL_HOME` points at, which on a developer machine is
/// usually a different checkout than the one being tested.
fn manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("orchestration")
        .join("integrations.json")
}

fn dag_run(repo: &Path, out: &Path, doctor_tools: &Path) -> Value {
    let output = common::cli()
        .args(["run", "dag-coordinate", "--repo"])
        .arg(repo)
        .arg("--out")
        .arg(out)
        .arg("--doctor-tool-path-prefix")
        .arg(doctor_tools)
        .env("CODE_INTEL_INTEGRATIONS_MANIFEST", manifest_path())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(manifest["outcome"], "completed", "manifest={manifest}");
    manifest
}

/// The digest the run manifest publishes for one node artifact.
fn published_digest(manifest: &Value, node: &str, path: &str) -> String {
    manifest["nodes"][node]["artifacts"]
        .as_array()
        .unwrap_or_else(|| panic!("node {node} has no artifacts: {manifest}"))
        .iter()
        .find(|artifact| artifact["path"] == path)
        .unwrap_or_else(|| panic!("node {node} did not publish {path}: {manifest}"))["sha256"]
        .as_str()
        .expect("published artifact digest")
        .to_string()
}

#[test]
fn evidence_payloads_are_byte_identical_across_runs_of_one_unchanged_tree() {
    let root = temp_dir();
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let doctor_tools = doctor_tool_fixture(&root);

    let first_out = root.join("run-1");
    let first = dag_run(&repo, &first_out, &doctor_tools);

    // The provider clock has one-second resolution, so the two runs have to
    // straddle a second boundary or a wall-clock stamp could not show up at
    // all and this test would pass against the very defect it guards.
    thread::sleep(Duration::from_millis(1_100));

    let second_out = root.join("run-2");
    let second = dag_run(&repo, &second_out, &doctor_tools);

    for (node, relative) in [
        ("evidence.graph", "evidence.graph/graph-payload.json"),
        ("evidence.sentrux", "evidence.sentrux/sentrux-payload.json"),
    ] {
        let first_bytes = fs::read(first_out.join(relative)).unwrap();
        let second_bytes = fs::read(second_out.join(relative)).unwrap();
        let first_payload: Value = serde_json::from_slice(&first_bytes).unwrap();
        let second_payload: Value = serde_json::from_slice(&second_bytes).unwrap();
        assert_eq!(
            first_payload, second_payload,
            "{node} payload changed on an unchanged tree"
        );
        assert_eq!(
            first_bytes, second_bytes,
            "{node} payload bytes changed on an unchanged tree"
        );

        // The published identity is what dedupe and caching key on, so assert
        // it directly rather than trusting that equal bytes were hashed.
        assert_eq!(
            published_digest(&first, node, relative),
            published_digest(&second, node, relative),
            "{node} republished a new content-addressed object for an unchanged tree"
        );
    }

    // The snapshot really was identical: without this the assertions above
    // would be vacuous if the runs had somehow scanned different trees.
    assert_eq!(first["snapshotIdentity"], second["snapshotIdentity"]);

    let _ = fs::remove_dir_all(root);
}
