mod common;
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

    let output = common::cli()
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
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
        manifest["nodes"]["evidence.codenexus"]["status"],
        "succeeded"
    );
    assert_eq!(
        manifest["nodes"]["diagnosis.hospital"]["status"],
        "succeeded"
    );
    for node in [
        "evidence.graph",
        "evidence.sentrux",
        "evidence.codenexus",
        "diagnosis.hospital",
    ] {
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
    assert!(manifest["nodes"]["evidence.codenexus"]["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|artifact| artifact["type"] == "observed.evidence.payload"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn production_dag_output_commits_and_enters_the_authoritative_index() {
    let root = temp_dir();
    // Named to match the "fixture-repo" identity used below for the
    // authority subdirectory, the index entry, and every `--repo` query
    // flag: `run execute` (A08 F2 fix) derives the published repo key from
    // this directory's real basename, so a mismatched fixture name would
    // silently double-nest instead of exercising the intended layout.
    let repo = root.join("fixture-repo");
    let source = root.join("a09-source");
    let artifact_root = root.join("artifacts");
    let repo_authority = artifact_root.join("fixture-repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(repo.join("tests")).unwrap();
    fs::create_dir_all(&repo_authority).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"fixture-repo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    fs::write(
        repo.join("tests/lib_test.rs"),
        "use crate::lib;\n#[test]\nfn covers_fixture() {}\n",
    )
    .unwrap();
    let doctor_tools = doctor_tool_fixture(&root, true);

    let execution = common::cli()
        .args(["run", "execute", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&source)
        .arg("--authority-root")
        .arg(&repo_authority)
        .args(["--final-name", "run-001"])
        .arg("--doctor-tool-path-prefix")
        .arg(&doctor_tools)
        .output()
        .unwrap();
    assert_eq!(
        execution.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&execution.stdout),
        String::from_utf8_lossy(&execution.stderr)
    );
    let execution: Value = serde_json::from_slice(&execution.stdout).unwrap();
    assert_eq!(execution["schema"], "code-intel-execution-result.v1");
    assert_eq!(execution["outcome"], "completed");
    assert_eq!(execution["exitCode"], 0);
    assert_eq!(execution["manifest"]["outcome"], "completed");
    assert_eq!(execution["publication"]["status"], "committed");
    assert_eq!(execution["publication"]["name"], "run-001");
    assert_eq!(execution["publication"]["marker"], "run-complete.json");
    assert_eq!(
        execution["publication"]["path"],
        repo_authority.join("run-001").to_string_lossy().as_ref()
    );
    let doctor_request: Value =
        serde_json::from_slice(&fs::read(source.join("doctor.request.json")).unwrap()).unwrap();
    assert_eq!(doctor_request["options"]["requireRepowise"], false);
    assert_eq!(doctor_request["options"]["requireUnderstand"], false);

    let index: Value =
        serde_json::from_slice(&fs::read(artifact_root.join("index.json")).unwrap()).unwrap();
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

    let query = common::cli()
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
    assert_eq!(query["run"], "run-001");
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

    let freshness_unknown = common::cli()
        .args(["artifact", "query", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--type", "inventory.files"])
        .output()
        .unwrap();
    assert_eq!(freshness_unknown.status.code(), Some(0));
    let freshness_unknown: Value = serde_json::from_slice(&freshness_unknown.stdout).unwrap();
    assert_eq!(freshness_unknown["freshness"]["status"], "unknown");
    assert_eq!(freshness_unknown["confidence"], "limited");

    let impact = common::cli()
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
    assert_eq!(
        impact["testSelection"]["commands"],
        json!(["cargo test --manifest-path Cargo.toml --test lib_test"])
    );

    let advisory_fresh = common::cli()
        .args(["change", "impact", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--changed", "src/lib.rs", "--staleness", "advisory"])
        .output()
        .unwrap();
    assert_eq!(
        advisory_fresh.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&advisory_fresh.stdout),
        String::from_utf8_lossy(&advisory_fresh.stderr)
    );
    let advisory_fresh: Value = serde_json::from_slice(&advisory_fresh.stdout).unwrap();
    assert_eq!(advisory_fresh["freshness"]["status"], "current");
    assert_eq!(
        advisory_fresh, impact,
        "advisory staleness must not change a current-snapshot answer"
    );

    fs::write(repo.join("src/lib.rs"), "pub fn changed() {}\n").unwrap();
    let stale = common::cli()
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

    let stale_impact = common::cli()
        .args(["change", "impact", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--changed", "src/lib.rs"])
        .output()
        .unwrap();
    assert_eq!(stale_impact.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&stale_impact.stderr)
        .contains("change impact requires the committed snapshot to be current"));

    let stale_explicit = common::cli()
        .args(["change", "impact", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--changed", "src/lib.rs", "--staleness", "current"])
        .output()
        .unwrap();
    assert_eq!(stale_explicit.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&stale_explicit.stderr)
        .contains("change impact requires the committed snapshot to be current"));

    let stale_advisory = common::cli()
        .args(["change", "impact", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--changed", "src/lib.rs", "--staleness", "advisory"])
        .output()
        .unwrap();
    assert_eq!(
        stale_advisory.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&stale_advisory.stdout),
        String::from_utf8_lossy(&stale_advisory.stderr)
    );
    let stale_advisory: Value = serde_json::from_slice(&stale_advisory.stdout).unwrap();
    assert_eq!(stale_advisory["freshness"]["status"], "stale-advisory");
    assert!(stale_advisory["recordedSnapshotIdentity"].is_string());
    assert!(stale_advisory["currentSnapshotIdentity"].is_string());
    assert_eq!(
        stale_advisory["recordedSnapshotIdentity"],
        stale_advisory["freshness"]["recordedIdentity"]
    );
    assert_eq!(
        stale_advisory["currentSnapshotIdentity"],
        stale_advisory["freshness"]["currentIdentity"]
    );
    assert_ne!(
        stale_advisory["recordedSnapshotIdentity"],
        stale_advisory["currentSnapshotIdentity"]
    );
    assert_eq!(
        stale_advisory["testSelection"]["files"],
        json!(["tests/lib_test.rs"])
    );
    assert!(stale_advisory["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item
            .as_str()
            .is_some_and(|text| text.contains("stale-advisory"))));

    let staleness_rejected = common::cli()
        .args(["change", "impact", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--changed", "src/lib.rs", "--staleness", "eventual"])
        .output()
        .unwrap();
    assert_eq!(staleness_rejected.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&staleness_rejected.stderr)
        .contains("--staleness must be current or advisory"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn production_run_preserves_doctor_domain_failure_and_completes_unrelated_branch() {
    let root = temp_dir();
    // See production_dag_output_commits_and_enters_the_authoritative_index:
    // must match the "fixture-repo" authority/query identity used below.
    let repo = root.join("fixture-repo");
    let completed_out = root.join("completed-run");
    let out = root.join("failed-run");
    let artifact_root = root.join("artifacts");
    let authority = artifact_root.join("fixture-repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(&authority).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let completed_doctor_tools = doctor_tool_fixture(&root, true);
    let completed = common::cli()
        .args(["run", "execute", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&completed_out)
        .arg("--authority-root")
        .arg(&authority)
        .args(["--final-name", "completed-001"])
        .arg("--doctor-tool-path-prefix")
        .arg(&completed_doctor_tools)
        .output()
        .unwrap();
    assert_eq!(
        completed.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&completed.stdout),
        String::from_utf8_lossy(&completed.stderr)
    );

    let doctor_tools = doctor_tool_fixture(&root, false);

    let output = common::cli()
        .args(["run", "execute", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .arg("--authority-root")
        .arg(&authority)
        .args(["--final-name", "failed-001"])
        .arg("--doctor-tool-path-prefix")
        .arg(&doctor_tools)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(10));
    let execution: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(execution["schema"], "code-intel-execution-result.v1");
    assert_eq!(execution["outcome"], "domain_failed");
    assert_eq!(execution["exitCode"], 10);
    assert_eq!(execution["publication"]["status"], "committed");
    assert_eq!(execution["publication"]["name"], "failed-001");
    let manifest = &execution["manifest"];
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
        manifest["nodes"]["evidence.codenexus"]["status"],
        "succeeded"
    );
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

    let index: Value =
        serde_json::from_slice(&fs::read(artifact_root.join("index.json")).unwrap()).unwrap();
    assert_eq!(index["entries"].as_array().unwrap().len(), 1);
    assert_eq!(index["entries"][0]["run"], "completed-001");
    assert!(index["diagnostics"].as_array().unwrap().iter().any(|item| {
        item["run"] == "failed-001"
            && item["classification"] == "non_completed"
            && item["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("domain_failed"))
    }));

    let query = common::cli()
        .args(["artifact", "query", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--type", "code_evidence.files"])
        .output()
        .unwrap();
    assert_eq!(query.status.code(), Some(0));
    let query: Value = serde_json::from_slice(&query.stdout).unwrap();
    assert_eq!(query["run"], "completed-001");

    let impact = common::cli()
        .args(["change", "impact", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--changed", "src/lib.rs"])
        .output()
        .unwrap();
    assert_eq!(impact.status.code(), Some(0));
    let impact: Value = serde_json::from_slice(&impact.stdout).unwrap();
    assert_eq!(impact["run"], "completed-001");

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
    let adapted = common::cli()
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
    let run = common::cli()
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

#[test]
fn checked_in_execution_result_schema_is_closed_and_binds_outcomes_to_exit_codes() {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../orchestration/schemas/code-intel-execution-result.v1.schema.json"
    ))
    .unwrap();
    assert_eq!(schema["$id"], "code-intel-execution-result.v1.schema.json");
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["schema"]["const"],
        "code-intel-execution-result.v1"
    );
    assert_eq!(
        schema["properties"]["publication"]["additionalProperties"],
        false
    );
    assert_eq!(
        schema["properties"]["publication"]["properties"]["marker"]["type"],
        "string"
    );

    let pairs = schema["oneOf"].as_array().unwrap();
    assert_eq!(pairs.len(), 4);
    assert!(pairs.iter().any(|pair| {
        pair["properties"]["outcome"]["const"] == "completed"
            && pair["properties"]["exitCode"]["const"] == 0
    }));
    assert!(pairs.iter().any(|pair| {
        pair["properties"]["outcome"]["const"] == "domain_failed"
            && pair["properties"]["exitCode"]["const"] == 10
    }));
    assert!(pairs.iter().any(|pair| {
        pair["properties"]["outcome"]["const"] == "domain_unknown"
            && pair["properties"]["exitCode"]["const"] == 20
    }));
    assert!(pairs.iter().any(|pair| {
        pair["properties"]["outcome"]["enum"] == json!(["process_failed", "incomplete"])
            && pair["properties"]["exitCode"]["const"] == 70
    }));

    // #168: `to_execution_json()` emits `failures` on every run; a closed
    // schema that omits it declares its own output invalid.
    assert!(schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|key| key == "failures"));
    let failures = &schema["properties"]["failures"];
    assert_eq!(failures["additionalProperties"], false);
    assert_eq!(failures["required"], json!(["process", "domain"]));
    assert_eq!(
        failures["properties"]["process"]["items"]["required"],
        json!(["node", "diagnostic"])
    );
    assert_eq!(
        failures["properties"]["domain"]["items"]["required"],
        json!(["node", "verdict"])
    );
}

/// Collects instance keys that a closed (`additionalProperties: false`)
/// schema node does not declare. Recurses through declared objects and
/// array items; stops at `$ref` boundaries (those subtrees carry their own
/// schema files). #168: the pwsh `Test-Json` step does not enforce the
/// closed-world bit, so the strict half of the contract is asserted here.
fn undeclared_keys(instance: &Value, schema_node: &Value, at: &str, out: &mut Vec<String>) {
    let Some(fields) = instance.as_object() else {
        return;
    };
    if schema_node["additionalProperties"] != Value::Bool(false) {
        return;
    }
    let declared = &schema_node["properties"];
    for (key, child) in fields {
        let child_schema = &declared[key];
        if child_schema.is_null() {
            out.push(format!("{at}/{key}"));
            continue;
        }
        if let Some(items) = child.as_array() {
            let item_schema = &child_schema["items"];
            for (index, item) in items.iter().enumerate() {
                undeclared_keys(item, item_schema, &format!("{at}/{key}/{index}"), out);
            }
        } else {
            undeclared_keys(child, child_schema, &format!("{at}/{key}"), out);
        }
    }
}

#[test]
fn strict_key_check_rejects_fields_the_schema_does_not_declare() {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../orchestration/schemas/code-intel-execution-result.v1.schema.json"
    ))
    .unwrap();
    let mut doc = json!({
        "schema": "code-intel-execution-result.v1",
        "outcome": "completed",
        "exitCode": 0,
        "failures": {"process": [], "domain": []},
        "manifest": {},
        "publication": {
            "status": "committed",
            "name": "run-001",
            "repo": "fixture",
            "path": "authority/fixture/run-001",
            "marker": "run-complete.json",
        },
    });
    let mut extras = Vec::new();
    undeclared_keys(&doc, &schema, "", &mut extras);
    assert!(extras.is_empty(), "baseline must be clean: {extras:?}");

    doc["sneaky"] = json!(true);
    doc["failures"]["domain"] = json!([{"node": "evidence.rg", "verdict": "fail", "extra": 1}]);
    let mut extras = Vec::new();
    undeclared_keys(&doc, &schema, "", &mut extras);
    extras.sort();
    assert_eq!(extras, vec!["/failures/domain/0/extra", "/sneaky"]);
}

#[test]
fn offline_profile_omits_provider_and_provider_diagnosis_nodes() {
    let root = temp_dir();
    let repo = root.join("repo");
    let out = root.join("offline-staging");
    let authority = root.join("authority");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(&authority).unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let doctor_tools = doctor_tool_fixture(&root, true);

    let output = common::cli()
        .args(["run", "execute", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .arg("--authority-root")
        .arg(&authority)
        .args(["--final-name", "offline-001", "--profile", "offline"])
        .args(["--doctor-require-repowise", "true"])
        .args(["--doctor-require-understand", "true"])
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
    let execution: Value = serde_json::from_slice(&output.stdout).unwrap();

    // #168: a real run's output must not carry keys its own closed schema
    // omits — `Test-Json` below is blind to `additionalProperties: false`.
    let execution_schema: Value = serde_json::from_str(include_str!(
        "../../../orchestration/schemas/code-intel-execution-result.v1.schema.json"
    ))
    .unwrap();
    let mut extras = Vec::new();
    undeclared_keys(&execution, &execution_schema, "", &mut extras);
    assert!(
        extras.is_empty(),
        "execution result emits keys its schema does not declare: {extras:?}"
    );

    let nodes = execution["manifest"]["nodes"].as_object().unwrap();
    assert!(!nodes.contains_key("evidence.graph"));
    assert!(!nodes.contains_key("evidence.sentrux"));
    assert!(!nodes.contains_key("evidence.codenexus"));
    assert!(!nodes.contains_key("diagnosis.hospital"));
    assert!(nodes.contains_key("repo.snapshot"));
    assert!(nodes.contains_key("inventory.rg"));
    assert!(nodes.contains_key("evidence.native-code"));

    assert!(!out.join("evidence.graph.request.json").exists());
    assert!(!out.join("evidence.sentrux.request.json").exists());
    assert!(!out.join("evidence.codenexus.request.json").exists());
    let doctor_request: Value =
        serde_json::from_slice(&fs::read(out.join("doctor.request.json")).unwrap()).unwrap();
    assert_eq!(doctor_request["options"]["requireRepowise"], false);
    assert_eq!(doctor_request["options"]["requireUnderstand"], false);

    let result_path = root.join("execution-result.json");
    fs::write(&result_path, &output.stdout).unwrap();
    let schema = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../orchestration/schemas/code-intel-execution-result.v1.schema.json");
    let validated = Command::new("pwsh")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "param($Document,$Schema); if (-not (Get-Content -Raw -LiteralPath $Document | Test-Json -SchemaFile $Schema -ErrorAction Stop)) { exit 1 }",
        ])
        .arg(&result_path)
        .arg(&schema)
        .output()
        .unwrap();
    assert!(
        validated.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&validated.stdout),
        String::from_utf8_lossy(&validated.stderr)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn authoritative_execute_rejects_diagnosis_only_runs_before_staging_or_publication() {
    let root = temp_dir();
    let repo = root.join("repo");
    let out = root.join("staging");
    let authority = root.join("authority");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&authority).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();

    let output = common::cli()
        .args(["run", "execute", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .arg("--authority-root")
        .arg(&authority)
        .args(["--final-name", "diagnosis-only"])
        .arg("--diagnosis-inputs")
        .arg(root.join("inputs.json"))
        .arg("--seed-artifact-root")
        .arg(root.join("seed"))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(64));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("does not accept diagnosis-only inputs")
    );
    assert!(!out.exists());
    assert!(!authority.join("diagnosis-only").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn strict_profile_cannot_be_weakened_and_keeps_all_provider_nodes_required() {
    let root = temp_dir();
    let repo = root.join("repo");
    let out = root.join("strict-staging");
    let authority = root.join("authority");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(&authority).unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let doctor_tools = doctor_tool_fixture(&root, true);

    let output = common::cli()
        .args(["run", "execute", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .arg("--authority-root")
        .arg(&authority)
        .args(["--final-name", "strict-001", "--profile", "strict"])
        .args(["--doctor-require-repowise", "false"])
        .args(["--doctor-require-understand", "false"])
        .arg("--doctor-tool-path-prefix")
        .arg(&doctor_tools)
        .output()
        .unwrap();

    assert!(
        matches!(output.status.code(), Some(0) | Some(10)),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let execution: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        execution["exitCode"].as_i64(),
        output.status.code().map(i64::from)
    );
    assert_eq!(
        execution["manifest"]["outcome"], execution["outcome"],
        "typed result and terminal manifest must agree"
    );
    let nodes = execution["manifest"]["nodes"].as_object().unwrap();
    assert!(nodes.contains_key("evidence.graph"));
    assert!(nodes.contains_key("evidence.sentrux"));
    assert!(nodes.contains_key("evidence.codenexus"));
    assert!(nodes.contains_key("diagnosis.hospital"));

    let doctor_request: Value =
        serde_json::from_slice(&fs::read(out.join("doctor.request.json")).unwrap()).unwrap();
    assert_eq!(doctor_request["options"]["requireRepowise"], true);
    assert_eq!(doctor_request["options"]["requireUnderstand"], true);

    let _ = fs::remove_dir_all(root);
}

/// Reads the single admitted structural rule of one kind out of a completed
/// `evidence.sentrux` payload.
fn sentrux_rule(out: &Path, kind: &str) -> Value {
    let payload: Value = serde_json::from_slice(
        &fs::read(out.join("evidence.sentrux/sentrux-payload.json")).unwrap(),
    )
    .unwrap();
    payload["data"]["structuralEvidence"]["rules"]
        .as_array()
        .expect("structural evidence must carry rules")
        .iter()
        .find(|rule| rule["kind"] == kind)
        .unwrap_or_else(|| panic!("structural evidence is missing rule {kind}"))
        .clone()
}

fn sentrux_command(out: &Path, id: &str) -> Value {
    let observation: Value = serde_json::from_slice(
        &fs::read(out.join("evidence.sentrux/sentrux-command-observation.json")).unwrap(),
    )
    .unwrap();
    observation["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["id"] == id)
        .unwrap_or_else(|| panic!("command observation is missing {id}"))
        .clone()
}

/// Neither of the two tests below can stub the `doctor` node, because the same
/// `--doctor-tool-path-prefix` also feeds `provider.sentrux-adapt` — pointing it
/// at a stub takes the external Sentrux path and bypasses the built-in engine
/// these tests exist to exercise. So the doctor probes the host's real
/// toolchain, and on a runner without `repowise` it domain-fails and takes the
/// run outcome with it, independently of anything Sentrux reported.
///
/// The gate claim is therefore asserted on the `diagnosis.hospital` node, which
/// is not downstream of `doctor` and runs either way. The whole-run outcome is
/// only asserted when the doctor agreed. CI's `Authoritative self-scan (release
/// gate parity)` step is what covers run-level exit 0, because it passes the
/// doctor requirements explicitly.
fn doctor_succeeded(manifest: &Value) -> bool {
    manifest["nodes"]["doctor"]["status"] == "succeeded"
}

/// A repository that never ran `save_baseline` has no prior measurement, so the
/// built-in gate cannot detect a regression against one. That absence of
/// governance is not a structural violation: reporting it as a failing rule made
/// the hospital diagnose "architecture gate failure" and the whole run exit 10
/// on any never-baselined repository, including a fixture holding one README.
#[test]
fn ungoverned_repository_completes_instead_of_failing_the_architecture_gate() {
    let root = temp_dir();
    let repo = root.join("repo");
    let out = root.join("run");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();

    let output = common::cli()
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    let manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        manifest["nodes"]["diagnosis.hospital"]["status"], "succeeded",
        "manifest={manifest}"
    );
    assert_eq!(
        manifest["nodes"]["diagnosis.hospital"]["verdict"], "pass",
        "manifest={manifest}"
    );
    if doctor_succeeded(&manifest) {
        assert_eq!(
            output.status.code(),
            Some(0),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(manifest["outcome"], "completed", "manifest={manifest}");
    }

    let gate = sentrux_rule(&out, "sentrux_gate");
    assert_eq!(gate["status"], "evaluated");
    assert_eq!(gate["verdict"], "pass");
    assert!(
        gate.get("details").is_none(),
        "an ungoverned gate must not publish violation details: {gate}"
    );
    assert_eq!(sentrux_rule(&out, "sentrux_check")["verdict"], "pass");

    let hospital: Value = serde_json::from_slice(
        &fs::read(out.join("diagnosis.hospital/hospital-report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(hospital["triage"]["primary_diagnosis"], "clean snapshot");
    assert_eq!(hospital["triage"]["failing_rules"], json!([]));

    // The ungoverned state stays auditable: the command observation keeps the
    // engine's real exit code and operator instruction verbatim.
    let observed_gate = sentrux_command(&out, "gate");
    assert_eq!(observed_gate["success"], false);
    assert_eq!(observed_gate["exitCode"], 1);
    assert!(observed_gate["stdout"]
        .as_str()
        .unwrap()
        .contains("Sentrux baseline missing"));

    let _ = fs::remove_dir_all(root);
}

/// The counterpart of the case above: once a baseline exists, a real regression
/// against it must still stop the run. This is the assertion that keeps the
/// ungoverned exemption from becoming a hole in the gate.
#[test]
fn baselined_repository_that_regresses_still_fails_the_architecture_gate() {
    let root = temp_dir();
    let repo = root.join("repo");
    let out = root.join("run");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();

    let baseline = common::cli()
        .args(["sentrux", "--operation", "save_baseline", "--repo"])
        .arg(&repo)
        .output()
        .unwrap();
    assert_eq!(
        baseline.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&baseline.stdout),
        String::from_utf8_lossy(&baseline.stderr)
    );
    assert!(repo.join(".sentrux/baseline.json").is_file());

    // A god file is any source file over 800 lines, so this regresses the gated
    // god_file_count metric from 0 to 1.
    fs::write(repo.join("src/god.rs"), "pub fn wide() {}\n".repeat(900)).unwrap();

    let output = common::cli()
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    let manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        manifest["nodes"]["diagnosis.hospital"]["status"], "domain_failed",
        "manifest={manifest}"
    );
    assert_eq!(
        manifest["nodes"]["diagnosis.hospital"]["diagnostic"], "architecture gate failure",
        "manifest={manifest}"
    );
    if doctor_succeeded(&manifest) {
        assert_eq!(
            output.status.code(),
            Some(10),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(manifest["outcome"], "domain_failed", "manifest={manifest}");
    }

    let gate = sentrux_rule(&out, "sentrux_gate");
    assert_eq!(gate["verdict"], "fail");
    assert!(
        gate["details"]["violations"]
            .as_array()
            .expect("a failing gate must publish violation details")
            .iter()
            .any(|violation| violation["rule"] == "god_files_increased"),
        "gate={gate}"
    );

    let hospital: Value = serde_json::from_slice(
        &fs::read(out.join("diagnosis.hospital/hospital-report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        hospital["triage"]["primary_diagnosis"],
        "architecture gate failure"
    );

    let _ = fs::remove_dir_all(root);
}

/// `test-dag-facade.ps1` guarded the artifact-path composition the PowerShell
/// launcher owned: a run lands directly under `<artifact root>/<repo name>`,
/// the repository name never repeats inside the path, and the explicit route
/// and the environment default produce the same evidence. `run dag-coordinate`
/// owns that composition now, so the guard lives here. The run outcome is
/// deliberately not asserted — the host toolchain decides it, and
/// `production_run_route_executes_snapshot_then_inventory` covers the outcome
/// with the doctor configured.
#[test]
fn artifact_root_routes_runs_where_readers_look_and_matches_the_environment_default() {
    let root = temp_dir();
    // The launcher fixture carried a space, an ampersand and non-ASCII, because
    // each of those has broken a path round trip in this pipeline before.
    let repo_name = "repo & 文";
    let repo = root.join(repo_name);
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("README & 文.md"), "fixture").unwrap();

    let run = |base: &Path, explicit: bool| -> PathBuf {
        let mut command = common::cli();
        command.args(["run", "dag-coordinate", "--repo"]).arg(&repo);
        if explicit {
            command.arg("--artifact-root").arg(base);
        } else {
            command.env("CODE_INTEL_ARTIFACT_ROOT", base);
        }
        // A nonzero exit means a node failed on this host, not that the path
        // composition failed, so the run is judged from disk below.
        let output = command.output().unwrap();
        let repo_artifacts = base.join(repo_name);
        let runs: Vec<PathBuf> = fs::read_dir(&repo_artifacts)
            .unwrap_or_else(|error| {
                panic!(
                    "no artifacts under {}: {error}; stderr={}",
                    repo_artifacts.display(),
                    String::from_utf8_lossy(&output.stderr)
                )
            })
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(
            runs.len(),
            1,
            "a run must be a direct child of the repository artifact root: {runs:?}"
        );
        assert!(
            !repo_artifacts.join(repo_name).exists(),
            "the repository name must not repeat inside the artifact path"
        );
        assert!(
            runs[0]
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".dag-staging-")),
            "run directory={:?}",
            runs[0]
        );
        runs[0].clone()
    };

    let explicit = run(&root.join("explicit artifacts"), true);
    let default = run(&root.join("default artifacts"), false);

    let manifest: Value =
        serde_json::from_slice(&fs::read(explicit.join("run-manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema"], "code-intel-run-manifest.v1");
    assert_eq!(
        fs::read(explicit.join("inventory.rg/files.txt")).unwrap(),
        fs::read(default.join("inventory.rg/files.txt")).unwrap(),
        "the explicit and default artifact roots disagreed about the inventory"
    );

    let _ = fs::remove_dir_all(root);
}

/// The two routes disagree about who owns the staging path, so accepting both
/// would silently honour one and drop the other.
#[test]
fn out_and_artifact_root_cannot_both_name_the_staging_directory() {
    let root = temp_dir();
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();

    let output = common::cli()
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(root.join("run"))
        .arg("--artifact-root")
        .arg(root.join("artifacts"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("mutually exclusive"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(root);
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Regression for issue #123: the production DAG must complete when the
/// repository is a linked worktree, where `.git` is a pointer file rather
/// than a directory. `evidence.sentrux` is the interesting node — it
/// re-derives the snapshot (`begin_consumption`, untracked enumeration)
/// inside its own capability process, so a worktree-hostile Git invocation
/// surfaces here while `repo.snapshot` still passes in the parent.
#[test]
fn production_run_completes_on_a_linked_worktree_checkout() {
    let root = temp_dir();
    let primary = root.join("primary");
    let linked = root.join("linked");
    let out = root.join("run");
    fs::create_dir_all(primary.join("src")).unwrap();
    fs::write(primary.join("README.md"), "fixture\n").unwrap();
    fs::write(primary.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    git(&primary, &["init", "--quiet"]);
    git(&primary, &["config", "user.name", "Worktree Fixture"]);
    git(
        &primary,
        &["config", "user.email", "worktree@example.invalid"],
    );
    git(&primary, &["add", "."]);
    git(&primary, &["commit", "--quiet", "-m", "baseline"]);
    git(
        &primary,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            linked.to_str().expect("fixture path is UTF-8"),
            "HEAD",
        ],
    );
    assert!(
        linked.join(".git").is_file(),
        "linked worktree must expose .git as a pointer file"
    );
    // Untracked content in the worktree keeps the enumeration path
    // (`ls-files --others`) load-bearing rather than trivially empty.
    fs::write(linked.join("untracked.txt"), "scratch\n").unwrap();
    let doctor_tools = doctor_tool_fixture(&root, true);

    let output = common::cli()
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&linked)
        .arg("--out")
        .arg(&out)
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
    let manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(manifest["outcome"], "completed", "manifest={manifest}");
    assert_eq!(
        manifest["nodes"]["evidence.sentrux"]["status"], "succeeded",
        "manifest={manifest}"
    );
    assert_eq!(manifest["nodes"]["evidence.sentrux"]["verdict"], "pass");
    assert_eq!(manifest["nodes"]["repo.snapshot"]["status"], "succeeded");

    // Release the worktree registration before deleting the fixture so the
    // primary repository's admin area never points at a vanished directory.
    git(
        &primary,
        &[
            "worktree",
            "remove",
            "--force",
            linked.to_str().expect("fixture path is UTF-8"),
        ],
    );
    let _ = fs::remove_dir_all(root);
}
