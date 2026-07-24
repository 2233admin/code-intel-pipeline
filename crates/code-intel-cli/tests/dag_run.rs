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

    let execution = Command::new(env!("CARGO_BIN_EXE_code-intel"))
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
    let artifact_root = root.join("artifacts");
    let authority = artifact_root.join("fixture-repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(&authority).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let doctor_tools = doctor_tool_fixture(&root, false);

    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
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

    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
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
    let nodes = execution["manifest"]["nodes"].as_object().unwrap();
    assert!(!nodes.contains_key("evidence.graph"));
    assert!(!nodes.contains_key("evidence.sentrux"));
    assert!(!nodes.contains_key("diagnosis.hospital"));
    assert!(nodes.contains_key("repo.snapshot"));
    assert!(nodes.contains_key("inventory.rg"));
    assert!(nodes.contains_key("evidence.native-code"));

    assert!(!out.join("evidence.graph.request.json").exists());
    assert!(!out.join("evidence.sentrux.request.json").exists());
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

    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
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

    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
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
    assert!(nodes.contains_key("diagnosis.hospital"));

    let doctor_request: Value =
        serde_json::from_slice(&fs::read(out.join("doctor.request.json")).unwrap()).unwrap();
    assert_eq!(doctor_request["options"]["requireRepowise"], true);
    assert_eq!(doctor_request["options"]["requireUnderstand"], true);

    let _ = fs::remove_dir_all(root);
}
