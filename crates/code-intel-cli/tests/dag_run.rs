use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

fn temp_dir() -> PathBuf {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "code-intel-a09-run-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

fn doctor_tool_fixture(root: &Path, conforming_sentrux: bool) -> PathBuf {
    let bin = root.join(if conforming_sentrux {
        "doctor-tools-ready"
    } else {
        "doctor-tools-nonconforming"
    });
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
        let sentrux = if conforming_sentrux {
            "@echo off\r\necho Enforce architectural rules\r\necho Tier: pro\r\nexit /b 0\r\n"
        } else {
            "@echo off\r\necho fixture nonconforming\r\nexit /b 0\r\n"
        };
        fs::write(bin.join("sentrux.cmd"), sentrux).unwrap();
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
        let sentrux = if conforming_sentrux {
            "#!/bin/sh\necho 'Enforce architectural rules'\necho 'Tier: pro'\nexit 0\n"
        } else {
            "#!/bin/sh\necho 'fixture nonconforming'\nexit 0\n"
        };
        fs::write(&path, sentrux).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

#[test]
fn production_run_route_executes_snapshot_then_inventory() {
    let root = temp_dir();
    let repo = root.join("repo");
    let out = root.join("run");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let doctor_tools = doctor_tool_fixture(&root, true);

    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .arg("--doctor-tool-path-prefix")
        .arg(&doctor_tools)
        .args(["--doctor-require-repowise", "false"])
        .args(["--doctor-require-understand", "true"])
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
    assert_eq!(manifest["schema"], "code-intel-run-manifest.v1");
    assert_eq!(manifest["outcome"], "completed", "manifest={manifest}");
    assert!(out.join("run-manifest.json").is_file());
    assert!(out.join("run-manifest-ref.json").is_file());
    let manifest_ref: Value =
        serde_json::from_slice(&fs::read(out.join("run-manifest-ref.json")).unwrap()).unwrap();
    assert_eq!(manifest_ref["artifactSchema"], "code-intel-run-manifest.v1");
    assert_eq!(manifest_ref["type"], "run.manifest");
    assert_eq!(manifest_ref["path"], "run-manifest.json");
    assert_eq!(
        manifest_ref["consumedSnapshotIdentity"],
        manifest["snapshotIdentity"]
    );
    assert!(out.join("repo.snapshot/snapshot.json").is_file());
    assert!(out.join("inventory.rg/files.txt").is_file());
    assert_eq!(
        fs::read(out.join("inventory.rg/files.txt")).unwrap(),
        b"README.md\nsrc/lib.rs\n",
        "A09 inventory must preserve the A00 normalized rg artifact"
    );

    let snapshot_request: Value =
        serde_json::from_slice(&fs::read(out.join("repo.snapshot.request.json")).unwrap()).unwrap();
    let snapshot_result: Value =
        serde_json::from_slice(&fs::read(out.join("repo.snapshot.result.json")).unwrap()).unwrap();
    let inventory_request: Value =
        serde_json::from_slice(&fs::read(out.join("inventory.rg.request.json")).unwrap()).unwrap();
    let inventory_result: Value =
        serde_json::from_slice(&fs::read(out.join("inventory.rg.result.json")).unwrap()).unwrap();
    let doctor_result: Value =
        serde_json::from_slice(&fs::read(out.join("doctor.result.json")).unwrap()).unwrap();
    let doctor_request: Value =
        serde_json::from_slice(&fs::read(out.join("doctor.request.json")).unwrap()).unwrap();
    let native_result: Value =
        serde_json::from_slice(&fs::read(out.join("evidence.native-code.result.json")).unwrap())
            .unwrap();
    for envelope in [&snapshot_request, &inventory_request] {
        assert_eq!(envelope["schema"], "code-intel-capability-request.v1");
    }
    for envelope in [
        &snapshot_result,
        &doctor_result,
        &inventory_result,
        &native_result,
    ] {
        assert_eq!(envelope["schema"], "code-intel-capability-result.v1");
        assert_eq!(envelope["status"], "completed");
        assert_eq!(envelope["verdict"], "pass");
        assert_eq!(envelope["domainVerdict"], "pass");
    }
    assert_eq!(snapshot_request["capability"], "repo.snapshot");
    assert_eq!(doctor_request["options"]["requireRepowise"], false);
    assert_eq!(doctor_request["options"]["requireUnderstand"], true);
    assert_eq!(inventory_request["capability"], "inventory.rg");
    assert_eq!(inventory_request["inputs"].as_array().unwrap().len(), 1);
    assert_eq!(
        inventory_request["inputs"][0]["artifactSchema"],
        "code-intel-repository-snapshot.v1"
    );
    assert_eq!(
        inventory_request["inputs"][0]["sha256"],
        snapshot_result["artifacts"][0]["sha256"]
    );
    assert_eq!(
        manifest["nodes"]["repo.snapshot"]["artifacts"][0]["path"],
        "repo.snapshot/snapshot.json"
    );
    assert_eq!(
        manifest["nodes"]["inventory.rg"]["artifacts"][0]["path"],
        "inventory.rg/files.txt"
    );
    assert_eq!(
        manifest["nodes"]["doctor"]["artifacts"][0]["path"],
        "doctor/doctor-observation.json"
    );
    assert_eq!(
        manifest["nodes"]["evidence.native-code"]["status"],
        "succeeded"
    );
    assert_eq!(manifest["nodes"]["evidence.graph"]["status"], "succeeded");
    assert_eq!(manifest["nodes"]["evidence.sentrux"]["status"], "succeeded");
    assert_eq!(
        manifest["nodes"]["diagnosis.hospital"]["status"],
        "succeeded"
    );
    for node in ["evidence.graph", "evidence.sentrux", "diagnosis.hospital"] {
        assert_eq!(manifest["nodes"][node]["verdict"], "pass", "node={node}");
    }
    assert!(manifest["nodes"]["evidence.graph"]["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|artifact| artifact["type"] == "observed.evidence.payload"));
    assert!(manifest["nodes"]["evidence.sentrux"]["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|artifact| artifact["type"] == "provider.sentrux.command-observation"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn production_dag_output_commits_and_enters_the_authoritative_index() {
    let root = temp_dir();
    let repo = root.join("repo");
    let source = root.join("a09-source");
    let artifact_root = root.join("artifacts");
    let repo_authority = artifact_root.join("fixture-repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(repo.join("tests")).unwrap();
    fs::create_dir_all(&repo_authority).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    fs::write(
        repo.join("tests/lib_test.rs"),
        "use crate::lib;\n#[test]\nfn covers_fixture() {}\n",
    )
    .unwrap();
    let doctor_tools = doctor_tool_fixture(&root, true);

    let dag = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&source)
        .arg("--doctor-tool-path-prefix")
        .arg(&doctor_tools)
        .output()
        .unwrap();
    assert_eq!(
        dag.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&dag.stdout),
        String::from_utf8_lossy(&dag.stderr)
    );

    let commit = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["run", "commit", "--source-root"])
        .arg(&source)
        .arg("--authority-root")
        .arg(&repo_authority)
        .arg("--manifest-ref")
        .arg(source.join("run-manifest-ref.json"))
        .args(["--final-name", "run-001"])
        .output()
        .unwrap();
    assert_eq!(
        commit.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&commit.stdout),
        String::from_utf8_lossy(&commit.stderr)
    );

    let index = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["artifact", "index", "--artifact-root"])
        .arg(&artifact_root)
        .output()
        .unwrap();
    assert_eq!(
        index.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&index.stdout),
        String::from_utf8_lossy(&index.stderr)
    );
    let index: Value = serde_json::from_slice(&index.stdout).unwrap();
    assert_eq!(index["entries"].as_array().unwrap().len(), 1);
    assert_eq!(index["entries"][0]["repo"], "fixture-repo");
    assert_eq!(index["entries"][0]["run"], "run-001");
    assert!(
        index["entries"][0]["artifactRefs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|artifact| artifact["type"] == "repository.snapshot"),
        "index={index}"
    );

    let query = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["artifact", "query", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--type", "inventory.files", "--contains", "src/lib.rs"])
        .output()
        .unwrap();
    assert_eq!(
        query.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&query.stdout),
        String::from_utf8_lossy(&query.stderr)
    );
    let query: Value = serde_json::from_slice(&query.stdout).unwrap();
    assert_eq!(query["schema"], "code-intel-evidence-query.v1");
    assert_eq!(query["runOutcome"], "completed");
    assert_eq!(query["authority"]["status"], "committed");
    assert_eq!(query["coverage"]["status"], "complete");
    assert_eq!(query["coverage"]["requestedEvidenceStatus"], "available");
    assert_eq!(query["confidence"], "high");
    assert_eq!(query["freshness"]["status"], "current");
    assert_eq!(query["matches"].as_array().unwrap().len(), 1);
    assert_eq!(
        query["matches"][0]["artifactRef"]["type"],
        "inventory.files"
    );

    let freshness_unknown = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["artifact", "query", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--type", "inventory.files"])
        .output()
        .unwrap();
    assert_eq!(freshness_unknown.status.code(), Some(0));
    let freshness_unknown: Value = serde_json::from_slice(&freshness_unknown.stdout).unwrap();
    assert_eq!(freshness_unknown["freshness"]["status"], "unknown");
    assert_eq!(freshness_unknown["confidence"], "limited");

    let impact = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["change", "impact", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--changed", "src/lib.rs"])
        .output()
        .unwrap();
    assert_eq!(
        impact.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&impact.stdout),
        String::from_utf8_lossy(&impact.stderr)
    );
    let impact: Value = serde_json::from_slice(&impact.stdout).unwrap();
    assert_eq!(impact["schema"], "code-intel-change-impact.v1");
    assert_eq!(impact["runOutcome"], "completed");
    assert_eq!(impact["freshness"]["status"], "current");
    assert_eq!(
        impact["testSelection"]["files"],
        json!(["tests/lib_test.rs"])
    );
    assert_eq!(impact["testSelection"]["commands"], json!(["cargo test"]));

    fs::write(repo.join("src/lib.rs"), "pub fn changed() {}\n").unwrap();
    let stale = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["artifact", "query", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--type", "inventory.files"])
        .output()
        .unwrap();
    assert_eq!(stale.status.code(), Some(0));
    let stale: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(stale["freshness"]["status"], "stale");
    assert_eq!(stale["confidence"], "limited");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn production_run_preserves_doctor_domain_failure_and_completes_unrelated_branch() {
    let root = temp_dir();
    let repo = root.join("repo");
    let out = root.join("run");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let doctor_tools = doctor_tool_fixture(&root, false);

    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .arg("--doctor-tool-path-prefix")
        .arg(&doctor_tools)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(10));
    let manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(manifest["outcome"], "domain_failed");
    assert_eq!(manifest["nodes"]["doctor"]["status"], "domain_failed");
    let doctor_artifacts = manifest["nodes"]["doctor"]["artifacts"]
        .as_array()
        .expect("domain failure must retain verified doctor evidence");
    assert!(!doctor_artifacts.is_empty());
    assert!(doctor_artifacts
        .iter()
        .any(|artifact| artifact["type"] == "doctor.observation"));
    assert_eq!(manifest["nodes"]["repo.snapshot"]["status"], "succeeded");
    assert_eq!(manifest["nodes"]["inventory.rg"]["status"], "succeeded");
    assert_eq!(
        manifest["nodes"]["evidence.native-code"]["status"],
        "succeeded"
    );
    assert_eq!(manifest["nodes"]["evidence.graph"]["status"], "succeeded");
    assert_eq!(manifest["nodes"]["evidence.sentrux"]["status"], "succeeded");
    assert_eq!(
        manifest["nodes"]["diagnosis.hospital"]["status"],
        "succeeded"
    );
    let doctor_result: Value =
        serde_json::from_slice(&fs::read(out.join("doctor.result.json")).unwrap()).unwrap();
    assert_eq!(doctor_result["status"], "completed");
    assert_eq!(doctor_result["verdict"], "fail");
    assert_eq!(doctor_result["domainVerdict"], "fail");
    assert_eq!(doctor_result["exitCode"], 10);
    assert!(doctor_result["diagnostics"][0]
        .as_str()
        .unwrap()
        .contains("provider conformance failed"));

    let artifact_root = root.join("artifacts");
    let authority = artifact_root.join("fixture-repo");
    fs::create_dir_all(&authority).unwrap();
    let commit = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["run", "commit", "--source-root"])
        .arg(&out)
        .arg("--authority-root")
        .arg(&authority)
        .arg("--manifest-ref")
        .arg(out.join("run-manifest-ref.json"))
        .args(["--final-name", "failed-001"])
        .output()
        .unwrap();
    assert_eq!(commit.status.code(), Some(0));
    let committed_root = authority.join("failed-001");
    let marker: Value =
        serde_json::from_slice(&fs::read(committed_root.join("run-complete.json")).unwrap())
            .unwrap();
    let committed_manifest: Value = serde_json::from_slice(
        &fs::read(committed_root.join(marker["manifest"]["path"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    let committed_doctor_artifact = committed_manifest["nodes"]["doctor"]["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["type"] == "doctor.observation")
        .unwrap();
    assert!(committed_root
        .join(committed_doctor_artifact["path"].as_str().unwrap())
        .is_file());

    let index = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["artifact", "index", "--artifact-root"])
        .arg(&artifact_root)
        .output()
        .unwrap();
    assert_eq!(index.status.code(), Some(0));
    let index: Value = serde_json::from_slice(&index.stdout).unwrap();
    assert_eq!(index["entries"], json!([]));
    assert!(index["diagnostics"].as_array().unwrap().iter().any(|item| {
        item["run"] == "failed-001"
            && item["classification"] == "non_completed"
            && item["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("domain_failed"))
    }));

    let query = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["artifact", "query", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--type", "code_evidence.files"])
        .output()
        .unwrap();
    assert_eq!(query.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&query.stderr)
        .contains("no committed authoritative run is indexed"));

    let impact = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["change", "impact", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--changed", "src/lib.rs"])
        .output()
        .unwrap();
    assert_eq!(impact.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&impact.stderr)
        .contains("no committed authoritative run is indexed"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn optional_session_evidence_is_snapshot_bound_a03_verified_and_manifested() {
    let root = temp_dir();
    let repo = root.join("repo");
    let out = root.join("run");
    let trace = root.join("trace.json");
    let session = root.join("session-evidence.json");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    fs::write(
        &trace,
        serde_json::to_vec(&json!({
            "version":1,
            "session":{"id":"private-session","harness":"Codex Desktop","cwd":repo},
            "events":[{
                "seq":1,
                "tool":"read_file",
                "action":"read",
                "targets":[{"path":"src/lib.rs","touch":"read"}],
                "outside":[],
                "isError":false
            }],
            "stats":{"observability":{"reads":"exact","errors":"exact"}}
        }))
        .unwrap(),
    )
    .unwrap();
    let adapted = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["provider", "session-adapt", "--repo"])
        .arg(&repo)
        .arg("--trace")
        .arg(&trace)
        .arg("--out")
        .arg(&session)
        .output()
        .unwrap();
    assert_eq!(
        adapted.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&adapted.stdout),
        String::from_utf8_lossy(&adapted.stderr)
    );

    let doctor_tools = doctor_tool_fixture(&root, true);
    let run = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .arg("--session-evidence")
        .arg(&session)
        .arg("--doctor-tool-path-prefix")
        .arg(&doctor_tools)
        .output()
        .unwrap();
    assert_eq!(
        run.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let manifest: Value = serde_json::from_slice(&run.stdout).unwrap();
    let node = &manifest["nodes"]["verification.session-evidence"];
    assert_eq!(node["status"], "succeeded");
    assert_eq!(node["verdict"], "pass");
    assert_eq!(
        node["artifacts"][0]["artifactSchema"],
        "code-intel-session-evidence.v1"
    );
    assert_eq!(
        node["artifacts"][0]["type"],
        "verification.session-evidence"
    );
    assert_eq!(
        node["artifacts"][0]["consumedSnapshotIdentity"],
        manifest["snapshotIdentity"]
    );
    assert!(out
        .join("verification.session-evidence/session-evidence.json")
        .is_file());

    let _ = fs::remove_dir_all(root);
}
