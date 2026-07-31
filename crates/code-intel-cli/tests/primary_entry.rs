use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_code-intel"))
}

#[test]
fn root_help_leads_with_the_compiled_primary_entry() {
    let output = Command::new(binary())
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
    let output = Command::new(binary())
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
    let output = Command::new(binary())
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
fn named_commands_are_not_misclassified_as_repository_paths() {
    let output = Command::new(binary())
        .args(["orchestrate", "--action", "List", "--json"])
        .output()
        .expect("run an existing named command");

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("error is UTF-8");
    assert!(!stderr.contains("unknown primary entry argument"));
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
        let output = Command::new(binary())
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

    let (code, output) = run_wrapper(&repo, &artifacts);
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

    let query = Command::new(binary())
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

    // Invalid UTF-8 in a source file fails the native-code node, and the run
    // has to stay visible for audit without becoming authoritative.
    std::fs::write(repo.join("broken.rs"), [0xff, 0xfe, 0xfd]).expect("invalid source");
    commit_fixture(&repo, "invalid-utf8-source");

    let (code, output) = run_wrapper(&repo, &artifacts);
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
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The wrapper takes the repository from the working directory, which is the
/// route a user actually types.
fn run_wrapper(repo: &std::path::Path, artifacts: &std::path::Path) -> (Option<i32>, String) {
    let output = Command::new(binary())
        .arg("--artifact-root")
        .arg(artifacts)
        .current_dir(repo)
        .output()
        .expect("run the stable wrapper");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code(), text)
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
