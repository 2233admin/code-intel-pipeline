mod common;
#[path = "primary_entry/content_identity.rs"]
mod content_identity;
#[path = "primary_entry/session_gate.rs"]
mod session_gate;
use std::io::Write;
#[path = "primary_entry/legacy_session.rs"]
mod legacy_session;

use std::path::PathBuf;
use std::process::{Command, Stdio};

#[test]
fn root_help_leads_with_the_compiled_primary_entry() {
    let output = common::cli()
        .arg("--help")
        .output()
        .expect("run code-intel --help");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("code-intel ."));
    assert!(stdout.contains("code-intel <path> --mode lite|normal|full"));
    assert!(!stdout.contains("legacy/invoke-code-intel.ps1"));
}

#[test]
fn root_entry_rejects_a_missing_repository_with_usage_exit_code() {
    let missing =
        std::env::temp_dir().join(format!("code-intel-missing-repo-{}", std::process::id()));
    let output = common::cli()
        .arg(&missing)
        .arg("--mode")
        .arg("lite")
        .output()
        .expect("run code-intel with a missing repository");

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr).expect("error is UTF-8");
    assert!(stderr.contains("repository path is not a directory:"));
    assert!(!stderr.contains("unknown command"));
    // A usage error is the user's mistake, not ours, so it carries no source
    // location.
    assert!(!stderr.contains("main.rs:"), "{stderr}");
}

#[test]
fn root_entry_keeps_json_machine_readable_on_usage_errors() {
    let missing = std::env::temp_dir().join(format!(
        "code-intel-json-missing-repo-{}",
        std::process::id()
    ));
    let output = common::cli()
        .arg(&missing)
        .args(["--mode", "lite", "--json"])
        .output()
        .expect("run code-intel JSON error path");

    assert_eq!(output.status.code(), Some(64));
    assert!(output.stderr.is_empty());
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("error output is JSON");
    assert_eq!(result["schema"], "code-intel-primary-result.v1");
    assert_eq!(result["outcome"], "error");
    assert_eq!(result["exitCode"], 64);
    assert!(result["diagnostic"]
        .as_str()
        .is_some_and(|message| message.contains("repository path is not a directory:")));
}

#[test]
fn run_alias_uses_the_same_primary_error_contract() {
    let missing = std::env::temp_dir().join(format!(
        "code-intel-run-alias-missing-repo-{}",
        std::process::id()
    ));
    let output = common::cli()
        .arg("run")
        .arg(&missing)
        .args(["--mode", "lite", "--json"])
        .output()
        .expect("run code-intel run alias");

    assert_eq!(output.status.code(), Some(64));
    assert!(output.stderr.is_empty());
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("run alias error output is JSON");
    assert_eq!(result["schema"], "code-intel-primary-result.v1");
    assert_eq!(result["outcome"], "error");
    assert!(result["diagnostic"]
        .as_str()
        .is_some_and(|message| message.contains("repository path is not a directory:")));
}

#[test]
fn project_query_resolves_repository_context_before_loading_evidence() {
    let root = std::env::temp_dir().join(format!(
        "code-intel-project-query-empty-{}",
        std::process::id()
    ));
    let repo = root.join("fixture-repo");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&repo).expect("create repository fixture");
    std::fs::create_dir_all(&artifacts).expect("create artifact root fixture");

    let output = common::cli()
        .arg("query")
        .arg(&repo)
        .args(["--kind", "evidence", "--json"])
        .env("CODE_INTEL_ARTIFACT_ROOT", &artifacts)
        .output()
        .expect("run project query");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stderr.is_empty());
    let error: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("query error is JSON");
    assert_eq!(error["schema"], "code-intel-project-error.v1");
    assert_eq!(error["kind"], "contract");
    assert!(
        error["diagnostic"]
            .as_str()
            .is_some_and(|message| message.contains("no committed authoritative run")),
        "{error}"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// Issue #279: `code-intel .` (this file's
/// `root_help_leads_with_the_compiled_primary_entry` confirms it's the
/// documented quick start) used to fold the authority-root bootstrap's IO
/// error — including a Windows-only false
/// positive from a symlinked artifact root — into a bare "create repository
/// authority root: <raw OS error>" with no path at all. Drives the exact
/// call site (`project_context.rs::run`) end to end through the compiled
/// binary. A file occupying the per-repo authority directory is a portable,
/// deterministic way to force a genuine block; the symlink false positive
/// itself needs a real second drive, so it's covered directly against the
/// shared helper in `artifacts_tests.rs` instead.
#[test]
fn root_entry_names_the_authority_root_path_when_a_file_blocks_it() {
    let root = std::env::temp_dir().join(format!(
        "code-intel-primary-entry-authority-blocked-{}",
        std::process::id()
    ));
    let repo = root.join("fixture-repo");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&repo).expect("create repository fixture");
    std::fs::create_dir_all(&artifacts).expect("create artifact root fixture");
    let blocked = artifacts.join("fixture-repo");
    std::fs::write(&blocked, b"not a directory").expect("fixture blocking file");

    let output = common::cli()
        .arg(&repo)
        .args(["--mode", "lite", "--json"])
        .env("CODE_INTEL_ARTIFACT_ROOT", &artifacts)
        .output()
        .expect("run code-intel against a blocked authority root");

    assert_eq!(output.status.code(), Some(74));
    assert!(output.stderr.is_empty());
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("error output is JSON");
    assert_eq!(result["schema"], "code-intel-primary-result.v1");
    assert_eq!(result["outcome"], "error");
    assert_eq!(result["exitCode"], 74);
    let diagnostic = result["diagnostic"]
        .as_str()
        .expect("diagnostic is a string");
    assert!(
        diagnostic.contains(&blocked.display().to_string()),
        "diagnostic omits the blocked path: {diagnostic}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn project_status_turns_an_unindexed_project_into_a_guided_first_step() {
    let root = std::env::temp_dir().join(format!(
        "code-intel-project-status-empty-{}",
        std::process::id()
    ));
    let repo = root.join("fixture-repo");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&repo).expect("create repository fixture");
    std::fs::create_dir_all(&artifacts).expect("create artifact root fixture");

    let output = common::cli()
        .arg("status")
        .arg(&repo)
        .arg("--json")
        .env("CODE_INTEL_ARTIFACT_ROOT", &artifacts)
        .output()
        .expect("run project status JSON");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let status: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("project status is JSON");
    assert_eq!(status["schema"], "code-intel-project-status.v1");
    assert_eq!(status["status"], "needs_run");
    assert_eq!(status["freshness"]["status"], "unavailable");
    assert_eq!(status["nextActions"][0]["id"], "analyze");
    assert_eq!(status["nextActions"][0]["argv"][0], "code-intel");

    let output = common::cli()
        .arg("status")
        .arg(&repo)
        .env("CODE_INTEL_ARTIFACT_ROOT", &artifacts)
        .output()
        .expect("run human project status");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let summary = String::from_utf8(output.stdout).expect("status summary is UTF-8");
    assert!(summary.contains("[NEEDS_RUN] fixture-repo"), "{summary}");
    assert!(summary.contains("Next actions:"), "{summary}");
    assert!(summary.contains("analyze:"), "{summary}");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn named_commands_are_not_misclassified_as_repository_paths() {
    let output = common::cli()
        .args(["orchestrate", "--action", "List", "--json"])
        .output()
        .expect("run an existing named command");

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("error is UTF-8");
    assert!(!stderr.contains("unknown primary entry argument"));
}

#[test]
fn unknown_subcommand_points_to_help() {
    let output = common::cli()
        .arg("init")
        .output()
        .expect("run an unknown subcommand");

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr).expect("error is UTF-8");
    assert!(stderr.contains("unknown command: init"));
    assert!(stderr.contains("code-intel --help"));
    assert!(!stderr.contains("repository path is not a directory"));
}

/// End-to-end cover for the stable wrapper: the default route with no
/// subcommand, the summary it prints, and what the authoritative index does
/// with a run that failed.
///
/// This was `legacy/scripts/tests/test-stable-wrapper-e2e.ps1`. It never tested
/// PowerShell — every assertion was already about the compiled binary — but it
/// was the only cover for the summary lines `main.rs` prints.
#[test]
fn stable_wrapper_publishes_a_completed_run_then_keeps_a_failed_one_out_of_the_index() {
    let root = std::env::temp_dir().join(format!(
        "code-intel-wrapper-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after the epoch")
            .as_nanos()
    ));
    let repo = root.join("fixture-repo");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(repo.join("assets")).expect("fixture assets");
    std::fs::create_dir_all(repo.join("src")).expect("fixture src");
    std::fs::create_dir_all(repo.join(".sentrux")).expect("fixture sentrux");
    std::fs::write(repo.join("README.md"), "stable wrapper fixture").expect("fixture readme");
    std::fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}").expect("fixture source");
    std::fs::write(
        repo.join(".sentrux/rules.toml"),
        "[constraints]\nmax_cycles = 0\nmax_coupling = \"F\"\nmax_cc = 100\nno_god_files = false\n",
    )
    .expect("fixture rules");
    // An unsupported binary file must not be grounds for rejecting the run.
    std::fs::write(repo.join("assets/logo.png"), [0x89, 0x50, 0x4e, 0x47, 0xff])
        .expect("fixture binary asset");

    // Provision the baseline with the built-in engine the authoritative run
    // gates with; a PATH-resolved external Sentrux writes a foreign baseline
    // identity and trips the engine-mismatch check.
    for operation in ["save_baseline", "check"] {
        let output = common::cli()
            .args(["sentrux", "--operation", operation, "--repo"])
            .arg(&repo)
            .output()
            .expect("run sentrux");
        assert!(
            output.status.success(),
            "sentrux {operation} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    commit_fixture(&repo, "baseline");

    let (code, output) = run_wrapper(&repo, &artifacts, true);
    // The doctor reports three distinct causes (`doctor_adapter.rs::diagnosis`).
    // Two of them — bootstrap readiness and provider conformance — describe the
    // machine's tools, and this route passes the doctor no flags to fix that up,
    // so on a host that is missing or mismatching a pinned tool they say nothing
    // about the wrapper. The third, manifest reconciliation, is about this
    // repository's own orchestration manifest and is never tolerated here, nor
    // is any second domain failure. CI installs the pinned tools, so everything
    // below still runs where it counts.
    let host_toolchain_gap = code != Some(0)
        && output.matches("Domain failure:").count() == 1
        && output.contains("Domain failure: doctor")
        && !output.contains("manifest reconciliation failed")
        && ["bootstrap readiness failed", "provider conformance failed"]
            .iter()
            .any(|cause| output.contains(cause));
    if host_toolchain_gap {
        eprintln!(
            "host toolchain is incomplete, so the authoritative route is not asserted:\n{output}"
        );
        let _ = std::fs::remove_dir_all(root);
        return;
    }
    assert_eq!(
        code,
        Some(0),
        "wrapper rejected a clean repository: {output}"
    );
    assert!(
        !output.contains("legacy compatibility pipeline"),
        "the default route still executed the legacy pipeline: {output}"
    );
    for marker in ["[PASS]", "Run evidence:", "Outcome:", "completed"] {
        assert!(output.contains(marker), "summary lacks {marker}: {output}");
    }

    let authority = artifacts.join("fixture-repo");
    let completed_run = latest_core_run(&authority);
    let completed = manifest_of(&completed_run);
    assert_eq!(completed["outcome"], "completed", "manifest={completed}");
    for node in ["evidence.graph", "evidence.sentrux", "diagnosis.hospital"] {
        assert_eq!(
            completed["nodes"][node]["status"], "succeeded",
            "default spine did not complete {node}: {completed}"
        );
        assert_eq!(
            completed["nodes"][node]["verdict"], "pass",
            "default spine did not pass {node}: {completed}"
        );
    }

    let doctor = completed["nodes"]["doctor"]["artifacts"]
        .as_array()
        .expect("doctor artifacts")
        .iter()
        .find(|artifact| artifact["type"] == "doctor.observation")
        .unwrap_or_else(|| panic!("no authoritative doctor observation: {completed}"))
        .clone();
    let observation = read_json(&completed_run.join(doctor["path"].as_str().expect("path")));
    assert_eq!(
        observation["environmentPolicy"]["policy"]["requireRepowise"], false,
        "the default route repowise skip did not reach the authoritative doctor policy"
    );

    let name = completed_run
        .file_name()
        .and_then(|value| value.to_str())
        .expect("run name")
        .to_string();
    assert_single_index_entry(&artifacts, &name);

    let report = common::cli()
        .args(["report", "--repo"])
        .arg(&repo)
        .args(["--artifact-root"])
        .arg(&artifacts)
        .output()
        .expect("run committed report reader");
    assert!(
        report.status.success(),
        "report failed: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    let report_text = String::from_utf8_lossy(&report.stdout);
    assert!(
        report_text.contains("hospitalMarkdown:"),
        "report={report_text}"
    );
    assert!(
        report_text.contains("--- hospital.md ---"),
        "report={report_text}"
    );
    let report_json = common::cli()
        .args(["report", "--repo"])
        .arg(&repo)
        .args(["--artifact-root"])
        .arg(&artifacts)
        .arg("--json")
        .output()
        .expect("run committed report reader JSON");
    assert!(report_json.status.success());
    let report_json: serde_json::Value =
        serde_json::from_slice(&report_json.stdout).expect("report JSON");
    assert_eq!(report_json["schema"], "code-intel-report.v1");
    assert!(
        report_json["hospitalMarkdown"]["path"]
            .as_str()
            .is_some_and(|path| std::path::Path::new(path).is_file()),
        "report={report_json}"
    );

    let resume = common::cli()
        .args(["resume", "--repo"])
        .arg(&repo)
        .args(["--artifact-root"])
        .arg(&artifacts)
        .output()
        .expect("run legacy resume against committed layout");
    assert!(!resume.status.success());
    let resume_error = String::from_utf8_lossy(&resume.stderr);
    assert!(
        resume_error.contains("report.json"),
        "resume={resume_error}"
    );
    assert!(
        resume_error.contains("code-intel report --repo"),
        "resume={resume_error}"
    );

    let query = common::cli()
        .args(["artifact", "query", "--artifact-root"])
        .arg(&artifacts)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--type", "observed.evidence.payload"])
        .output()
        .expect("run artifact query");
    assert!(
        query.status.success(),
        "provider payload query failed: {}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query: serde_json::Value = serde_json::from_slice(&query.stdout).expect("query is JSON");
    assert_eq!(query["freshness"]["status"], "current", "query={query}");
    assert!(
        query["matches"].as_array().expect("matches").len() >= 2,
        "query={query}"
    );

    let divergent = root.join("divergent linked worktree");
    let worktree = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args([
            "worktree",
            "add",
            "--quiet",
            "-b",
            "divergent-evidence-test",
        ])
        .arg(&divergent)
        .output()
        .expect("create divergent linked worktree");
    assert!(
        worktree.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&worktree.stderr)
    );
    std::fs::write(
        divergent.join("src/lib.rs"),
        "pub fn fixture() {}\npub fn divergent() {}\n",
    )
    .expect("diverge linked worktree");
    commit_fixture(&divergent, "divergent-linked-worktree");

    let divergent_query = common::cli()
        .arg("query")
        .arg(&divergent)
        .args(["--kind", "evidence", "--json"])
        .env("CODE_INTEL_ARTIFACT_ROOT", &artifacts)
        .output()
        .expect("query divergent linked worktree");
    assert_eq!(divergent_query.status.code(), Some(65));
    assert!(divergent_query.stderr.is_empty());
    let divergent_error: serde_json::Value =
        serde_json::from_slice(&divergent_query.stdout).expect("divergent query refusal is JSON");
    assert_eq!(divergent_error["kind"], "contract");
    assert!(
        divergent_error["diagnostic"]
            .as_str()
            .is_some_and(|message| message.contains("repository identity mismatch")),
        "divergent query did not fail closed: {divergent_error}"
    );

    let divergent_mcp = common::cli()
        .args(["serve", "--mcp", "--repo-path"])
        .arg(&divergent)
        .args(["--artifact-root"])
        .arg(&artifacts)
        .output()
        .expect("start MCP against divergent linked worktree");
    assert_eq!(divergent_mcp.status.code(), Some(65));
    assert!(divergent_mcp.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&divergent_mcp.stderr).contains("repository identity mismatch"),
        "MCP did not fail closed: {}",
        String::from_utf8_lossy(&divergent_mcp.stderr)
    );

    let unrelated = root.join("unrelated-lineage").join("fixture-repo");
    std::fs::create_dir_all(unrelated.join("src")).expect("create unrelated repository");
    std::fs::write(unrelated.join("src/lib.rs"), "pub fn unrelated() {}")
        .expect("write unrelated source");
    commit_fixture(&unrelated, "unrelated-lineage");
    let collision = common::cli()
        .arg("run")
        .arg(&unrelated)
        .args(["--mode", "lite", "--json"])
        .env("CODE_INTEL_ARTIFACT_ROOT", &artifacts)
        .output()
        .expect("refuse an unrelated repository with the same directory name");
    assert_eq!(collision.status.code(), Some(65));
    assert!(collision.stderr.is_empty());
    let collision: serde_json::Value =
        serde_json::from_slice(&collision.stdout).expect("collision error is JSON");
    assert!(
        collision["diagnostic"]
            .as_str()
            .is_some_and(|message| message.contains("repository key collision")),
        "collision={collision}"
    );

    let renamed_repo = root.join("renamed checkout");
    std::fs::rename(&repo, &renamed_repo).expect("rename the published checkout");
    let repo = renamed_repo;

    let project_query = common::cli()
        .arg("query")
        .arg(&repo)
        .args([
            "--kind",
            "evidence",
            "--type",
            "observed.evidence.payload",
            "--json",
        ])
        .env("CODE_INTEL_ARTIFACT_ROOT", &artifacts)
        .output()
        .expect("run project-context query");
    assert!(
        project_query.status.success(),
        "project query failed: {}",
        String::from_utf8_lossy(&project_query.stderr)
    );
    let project_query: serde_json::Value =
        serde_json::from_slice(&project_query.stdout).expect("project query is JSON");
    assert_eq!(project_query["schema"], "code-intel-evidence-query.v1");
    assert_eq!(project_query["repo"], "fixture-repo");
    assert_eq!(project_query["freshness"]["status"], "current");
    assert!(
        project_query["matches"]
            .as_array()
            .expect("project query matches")
            .len()
            >= 2,
        "query={project_query}"
    );

    let mcp = mcp_session(
        &repo,
        &artifacts,
        &[
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":"2025-06-18","capabilities":{},
                "clientInfo":{"name":"project-context-test","version":"1"}}}),
            serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"get_facts","arguments":{"type":"observed.evidence.payload"}}}),
        ],
    );
    assert_eq!(
        mcp.len(),
        2,
        "notification must not receive a response: {mcp:?}"
    );
    assert_eq!(
        mcp[0]["result"]["repositoryBinding"]["status"], "verified",
        "MCP startup did not bind the named-run publication: {}",
        mcp[0]
    );
    assert_eq!(mcp[1]["result"]["isError"], false, "MCP facts={}", mcp[1]);
    let mcp_facts: serde_json::Value = serde_json::from_str(
        mcp[1]["result"]["content"][0]["text"]
            .as_str()
            .expect("MCP facts text block"),
    )
    .expect("MCP facts payload is JSON");
    assert_eq!(mcp_facts, project_query, "CLI and MCP facts diverged");

    // Invalid UTF-8 in a source file fails the native-code node, and the run
    // has to stay visible for audit without becoming authoritative.
    std::fs::write(repo.join("broken.rs"), [0xff, 0xfe, 0xfd]).expect("invalid source");
    commit_fixture(&repo, "invalid-utf8-source");

    let (code, output) = run_wrapper(&repo, &artifacts, false);
    assert_ne!(
        code,
        Some(0),
        "wrapper hid an authoritative failure: {output}"
    );
    for marker in [
        "[FAIL]",
        "process_failed",
        "evidence.native-code",
        "Run evidence:",
    ] {
        assert!(
            output.contains(marker),
            "failure summary lacks {marker}: {output}"
        );
    }

    let failed_run = latest_core_run(&authority);
    assert_ne!(
        failed_run, completed_run,
        "the failed run was not retained for audit"
    );
    let failed = manifest_of(&failed_run);
    assert_eq!(failed["outcome"], "process_failed", "manifest={failed}");
    assert_eq!(
        failed["nodes"]["evidence.native-code"]["status"], "process_failed",
        "manifest={failed}"
    );

    // The last completed run keeps the authority, and the failed one is
    // classified outside the index rather than dropped.
    let index = assert_single_index_entry(&artifacts, &name);
    let failed_name = failed_run.file_name().and_then(|value| value.to_str());
    assert!(
        index["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|item| {
                item["repo"] == "fixture-repo"
                    && item["run"].as_str() == failed_name
                    && item["classification"] == "non_completed"
                    && item["reason"]
                        .as_str()
                        .is_some_and(|reason| reason.contains("process_failed"))
            }),
        "the failed run was not classified outside the authoritative index: {index}"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// `git init` is allowed to fail on the second call; everything else is not.
fn commit_fixture(repo: &std::path::Path, message: &str) {
    let _ = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["init", "--quiet"])
        .output();
    for args in [
        vec!["add", "."],
        vec![
            "-c",
            "user.name=CodeIntelTest",
            "-c",
            "user.email=code-intel-test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(&args)
            .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The default entry and its named `run` alias both take the repository from
/// the working directory; the test uses one for each authoritative iteration.
fn run_wrapper(
    repo: &std::path::Path,
    artifacts: &std::path::Path,
    named_alias: bool,
) -> (Option<i32>, String) {
    let mut command = common::cli();
    if named_alias {
        command.arg("run");
    }
    let output = command
        .env("CODE_INTEL_ARTIFACT_ROOT", artifacts)
        .current_dir(repo)
        .output()
        .expect("run the stable wrapper");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code(), text)
}

fn run_lite_json(repo: &std::path::Path, artifacts: &std::path::Path) -> serde_json::Value {
    let output = common::cli()
        .arg("run")
        .arg(repo)
        .args(["--mode", "lite", "--json"])
        .env("CODE_INTEL_ARTIFACT_ROOT", artifacts)
        .output()
        .expect("run content-identity repository");
    assert!(
        output.status.success(),
        "content-identity run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(output.stderr.is_empty(), "JSON run writes no stderr");
    serde_json::from_slice(&output.stdout).expect("content-identity run JSON")
}

fn mcp_session(
    repo: &std::path::Path,
    artifacts: &std::path::Path,
    requests: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let mut child = common::cli()
        .args(["serve", "--mcp", "--repo-path"])
        .arg(repo)
        .args(["--artifact-root"])
        .arg(artifacts)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start MCP server over the committed publication");
    {
        let stdin = child.stdin.as_mut().expect("MCP stdin");
        for request in requests {
            writeln!(stdin, "{request}").expect("write MCP request");
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for MCP server");
    assert_eq!(
        output.status.code(),
        Some(0),
        "MCP server failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "MCP server wrote stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("MCP stdout is UTF-8")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("MCP response is JSON"))
        .collect()
}

/// Authoritative runs are published as `<name>-core`; staging directories and
/// non-committed runs must never be picked up here.
fn latest_core_run(authority: &std::path::Path) -> PathBuf {
    let mut runs: Vec<PathBuf> = std::fs::read_dir(authority)
        .unwrap_or_else(|error| panic!("no authority at {}: {error}", authority.display()))
        .filter_map(|entry| {
            let path = entry.expect("read authority entry").path();
            let name = path.file_name()?.to_str()?.to_string();
            (path.is_dir() && name.ends_with("-core")).then_some(path)
        })
        .collect();
    runs.sort();
    runs.pop()
        .unwrap_or_else(|| panic!("no authoritative run under {}", authority.display()))
}

fn read_json(path: &std::path::Path) -> serde_json::Value {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|error| panic!("unreadable {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("unparsable {}: {error}", path.display()))
}

/// The manifest is reached through the marker rather than by name, so this
/// fails if the marker ever stops binding it.
fn manifest_of(run: &std::path::Path) -> serde_json::Value {
    let marker = read_json(&run.join("run-complete.json"));
    read_json(&run.join(marker["manifest"]["path"].as_str().expect("manifest path")))
}

fn assert_single_index_entry(artifacts: &std::path::Path, run: &str) -> serde_json::Value {
    let index = read_json(&artifacts.join("index.json"));
    let entries: Vec<&serde_json::Value> = index["entries"]
        .as_array()
        .expect("index entries")
        .iter()
        .filter(|entry| entry["repo"] == "fixture-repo")
        .collect();
    assert_eq!(entries.len(), 1, "index={index}");
    assert_eq!(entries[0]["run"].as_str(), Some(run), "index={index}");
    assert_eq!(entries[0]["outcome"], "completed", "index={index}");
    index
}
