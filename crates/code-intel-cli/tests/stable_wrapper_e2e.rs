//! End-to-end cover for the stable wrapper: the default route with no
//! subcommand, the human-facing summary it prints, and what the authoritative
//! index does with a run that failed.
//!
//! This was `legacy/scripts/tests/test-stable-wrapper-e2e.ps1`. It never tested
//! PowerShell — every assertion was already about the compiled binary — but it
//! was the only cover for the summary lines `main.rs` prints, so it could not
//! simply be dropped.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn temp_dir() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "code-intel-wrapper-e2e-{}-{nonce}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("git {args:?} could not start: {error}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn code_intel(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_code-intel"));
    command.args(args);
    command
}

/// The wrapper takes the repository from the working directory, which is the
/// route a user actually types.
fn run_wrapper(repo: &Path, artifact_root: &Path) -> (Option<i32>, String) {
    let output = code_intel(&["--artifact-root"])
        .arg(artifact_root)
        .current_dir(repo)
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code(), text)
}

/// Authoritative runs are published as `<name>-core`; staging directories and
/// non-committed runs must never be picked up here.
fn latest_core_run(authority: &Path) -> PathBuf {
    let mut runs: Vec<PathBuf> = fs::read_dir(authority)
        .unwrap_or_else(|error| panic!("no authority at {}: {error}", authority.display()))
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name()?.to_str()?.to_string();
            (path.is_dir() && name.ends_with("-core")).then_some(path)
        })
        .collect();
    runs.sort();
    runs.pop()
        .unwrap_or_else(|| panic!("no authoritative run under {}", authority.display()))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(path).unwrap_or_else(|error| panic!("unreadable {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("unparsable {}: {error}", path.display()))
}

/// The manifest is reached through the marker rather than by name, so the test
/// fails if the marker stops binding it.
fn manifest_of(run: &Path) -> Value {
    let marker = read_json(&run.join("run-complete.json"));
    read_json(&run.join(marker["manifest"]["path"].as_str().unwrap()))
}

#[test]
fn stable_wrapper_publishes_a_completed_run_then_keeps_a_failed_one_out_of_the_index() {
    let root = temp_dir();
    let repo = root.join("fixture-repo");
    let artifact_root = root.join("artifacts");
    fs::create_dir_all(repo.join("assets")).unwrap();
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(repo.join(".sentrux")).unwrap();
    fs::write(repo.join("README.md"), "stable wrapper fixture").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}").unwrap();
    fs::write(
        repo.join(".sentrux/rules.toml"),
        "[constraints]\nmax_cycles = 0\nmax_coupling = \"F\"\nmax_cc = 100\nno_god_files = false\n",
    )
    .unwrap();
    // An unsupported binary file must not be grounds for rejecting the run.
    fs::write(repo.join("assets/logo.png"), [0x89, 0x50, 0x4e, 0x47, 0xff]).unwrap();

    // Provision the baseline with the built-in engine the authoritative run
    // gates with; a PATH-resolved external Sentrux writes a foreign baseline
    // identity and trips the engine-mismatch check.
    for operation in ["save_baseline", "check"] {
        let output = code_intel(&["sentrux", "--operation", operation, "--repo"])
            .arg(&repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "sentrux {operation} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    git(&repo, &["init", "--quiet"]);
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=CodeIntelTest",
            "-c",
            "user.email=code-intel-test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "baseline",
        ],
    );

    let (code, output) = run_wrapper(&repo, &artifact_root);
    assert_eq!(
        code,
        Some(0),
        "wrapper rejected a clean repository: {output}"
    );
    assert!(
        !output.contains("legacy compatibility pipeline"),
        "the default route still executed the legacy pipeline: {output}"
    );
    for marker in ["[PASS]", "Run evidence:"] {
        assert!(output.contains(marker), "summary lacks {marker}: {output}");
    }
    assert!(
        output.contains("Outcome:") && output.contains("completed"),
        "summary lacks the completed outcome: {output}"
    );

    let authority = artifact_root.join("fixture-repo");
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

    let doctor_artifacts: Vec<&Value> = completed["nodes"]["doctor"]["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|artifact| artifact["type"] == "doctor.observation")
        .collect();
    assert_eq!(
        doctor_artifacts.len(),
        1,
        "authoritative doctor observation was not published: {completed}"
    );
    let observation = read_json(&completed_run.join(doctor_artifacts[0]["path"].as_str().unwrap()));
    assert_eq!(
        observation["environmentPolicy"]["policy"]["requireRepowise"], false,
        "the default route's repowise skip did not reach the authoritative doctor policy"
    );

    let index = read_json(&artifact_root.join("index.json"));
    let entries: Vec<&Value> = index["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["repo"] == "fixture-repo")
        .collect();
    assert_eq!(entries.len(), 1, "index={index}");
    assert_eq!(
        entries[0]["run"].as_str().unwrap(),
        completed_run.file_name().unwrap().to_str().unwrap()
    );
    assert_eq!(entries[0]["outcome"], "completed");

    let query = code_intel(&["artifact", "query", "--artifact-root"])
        .arg(&artifact_root)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--type", "observed.evidence.payload"])
        .output()
        .unwrap();
    assert!(
        query.status.success(),
        "provider payload query failed: {}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query: Value = serde_json::from_slice(&query.stdout).unwrap();
    assert_eq!(query["freshness"]["status"], "current", "query={query}");
    assert!(
        query["matches"].as_array().unwrap().len() >= 2,
        "query={query}"
    );

    // Invalid UTF-8 in a source file fails the native-code node, and the run
    // has to stay visible for audit without becoming authoritative.
    fs::write(repo.join("broken.rs"), [0xff, 0xfe, 0xfd]).unwrap();
    git(&repo, &["add", "broken.rs"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=CodeIntelTest",
            "-c",
            "user.email=code-intel-test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "invalid-utf8-source",
        ],
    );

    let (code, output) = run_wrapper(&repo, &artifact_root);
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

    let index = read_json(&artifact_root.join("index.json"));
    let entries: Vec<&Value> = index["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["repo"] == "fixture-repo")
        .collect();
    assert_eq!(entries.len(), 1, "index={index}");
    assert_eq!(
        entries[0]["run"].as_str().unwrap(),
        completed_run.file_name().unwrap().to_str().unwrap(),
        "a non-completed run replaced the last completed authority: {index}"
    );
    assert!(
        index["diagnostics"].as_array().unwrap().iter().any(|item| {
            item["repo"] == "fixture-repo"
                && item["run"].as_str() == failed_run.file_name().unwrap().to_str()
                && item["classification"] == "non_completed"
                && item["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("process_failed"))
        }),
        "the failed run was not classified outside the authoritative index: {index}"
    );

    let _ = fs::remove_dir_all(root);
}

/// A bad path is a user error, so it gets one line and no source location.
#[test]
fn primary_entry_reports_an_invalid_repository_path_without_a_source_location() {
    let root = temp_dir();
    fs::create_dir_all(&root).unwrap();

    let output = code_intel(&[])
        .arg(root.join("missing-repo"))
        .arg("--artifact-root")
        .arg(root.join("artifacts"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stderr).into_owned()
        + &String::from_utf8_lossy(&output.stdout);
    assert_ne!(output.status.code(), Some(0), "{text}");
    assert!(
        text.contains("repository path is not a directory:"),
        "{text}"
    );
    assert!(
        !text.contains("main.rs:"),
        "a usage error leaked a source location: {text}"
    );

    let _ = fs::remove_dir_all(root);
}
