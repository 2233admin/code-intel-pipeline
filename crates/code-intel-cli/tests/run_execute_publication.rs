//! Issue #111 / docs/runbooks/frictions.md F2: `run execute --authority-root
//! X --final-name Y` used to publish flat into `X/Y/` and still report exit 0
//! with `"publication":{"status":"committed"}`, but `artifact
//! index`/`artifact query` only ever scan a two-level `X/<repo-name>/<run>`
//! layout (`artifact_index.rs::scan`, `committed_evidence.rs::load`). That
//! made a clean, "committed" run a silent dead end no query could ever find.
//!
//! These tests drive the exact failing sequence end to end through the
//! compiled binary, in the `dag_run.rs` / `primary_entry.rs` style: spawn
//! `run execute`, then spawn `artifact query` against what it reported, and
//! assert the run is found.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_code-intel"))
}

fn temp_dir(label: &str) -> PathBuf {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "code-intel-a08-f2-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

/// A minimal source tree `run execute` can scan. No `.git` needed: the
/// default working-tree policy (`explicit_overlay`) reads the working tree
/// directly.
fn fixture_repo(root: &Path) -> PathBuf {
    let repo = root.join("fixture-repo");
    fs::create_dir_all(repo.join("src")).expect("fixture src dir");
    fs::write(repo.join("README.md"), "fixture\n").expect("fixture readme");
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").expect("fixture source");
    repo
}

/// Stub tool binaries so the DAG's always-required `doctor` node passes its
/// bootstrap/conformance checks without depending on whatever happens to be
/// on the host's real PATH. Mirrors `dag_run.rs`'s `doctor_tool_fixture`
/// (including the Windows `.cmd` branch — a plain shebang script is not an
/// executable Windows can launch, and a non-conforming `sentrux` fails the
/// doctor's provider-conformance check even though `--profile offline`
/// disables the sentrux *capability* itself).
fn doctor_tool_fixture(root: &Path) -> PathBuf {
    let bin = root.join("doctor-tools");
    fs::create_dir_all(&bin).expect("doctor tool fixture dir");
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

/// Runs `run execute` against `repo`, publishing under `authority` (which
/// must already exist — same contract the CLI enforces). `--profile offline`
/// keeps the DAG to its always-required spine (`repo.snapshot`, `doctor`,
/// `inventory.rg`, `evidence.native-code`) with no external adapters, so the
/// only host dependency left is what `doctor_tool_fixture` already covers.
fn run_execute(root: &Path, repo: &Path, authority: &Path, final_name: &str) -> (Output, Value) {
    let doctor_tools = doctor_tool_fixture(root);
    let staging = root.join(format!("stage-{final_name}"));
    let output = Command::new(binary())
        .args(["run", "execute", "--repo"])
        .arg(repo)
        .arg("--out")
        .arg(&staging)
        .arg("--authority-root")
        .arg(authority)
        .args(["--final-name", final_name, "--profile", "offline"])
        .arg("--doctor-tool-path-prefix")
        .arg(&doctor_tools)
        .output()
        .expect("spawn code-intel run execute");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "run execute did not print JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output, result)
}

fn query(artifact_root: &Path, repo_name: &str, repo_path: &Path) -> (Output, Value) {
    let output = Command::new(binary())
        .args(["artifact", "query", "--artifact-root"])
        .arg(artifact_root)
        .args(["--repo", repo_name, "--repo-path"])
        .arg(repo_path)
        .args(["--type", "inventory.files"])
        .output()
        .expect("spawn code-intel artifact query");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "artifact query did not print JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output, result)
}

/// The exact failing sequence from F2: `run execute --authority-root
/// <artifact-root> --final-name <name>` followed by `artifact query
/// --artifact-root <artifact-root> --repo <repo-name>` must find the run.
#[test]
fn run_execute_publishes_where_artifact_query_can_find_it() {
    let root = temp_dir("fresh-nest");
    let repo = fixture_repo(&root);
    let authority = root.join("authority");
    fs::create_dir_all(&authority).expect("fixture authority root");

    let (output, result) = run_execute(&root, &repo, &authority, "run-001");
    assert_eq!(
        output.status.code(),
        Some(0),
        "run execute did not exit 0: {result}\nstderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(result["outcome"], "completed", "manifest={result}");

    // Machine-first check: the write side must report both the resolved,
    // queryable path and the exact repo key the query side wants.
    assert_eq!(result["publication"]["status"], "committed");
    assert_eq!(result["publication"]["repo"], "fixture-repo");
    let published_path = PathBuf::from(
        result["publication"]["path"]
            .as_str()
            .expect("publication path is a string"),
    );
    assert_eq!(
        published_path,
        authority.join("fixture-repo").join("run-001"),
        "publication must nest under the repo name instead of publishing flat (issue #111)"
    );
    assert!(
        published_path.join("run-complete.json").is_file(),
        "completion marker missing at the reported publication path"
    );

    let (query_output, query_result) = query(&authority, "fixture-repo", &repo);
    assert!(
        query_output.status.success(),
        "artifact query could not find the run F2 describes as a dead end: {query_result}\nstderr={}",
        String::from_utf8_lossy(&query_output.stderr)
    );
    assert_eq!(
        query_result["freshness"]["status"], "current",
        "query={query_result}"
    );
    assert!(
        !query_result["matches"]
            .as_array()
            .expect("matches array")
            .is_empty(),
        "query matched nothing: {query_result}"
    );

    fs::remove_dir_all(&root).ok();
}

/// Issue #111's confirmed workaround was folding the repo name into
/// `--authority-root` by hand. The fix must recognize an already-nested
/// authority root and publish there directly rather than double-nesting
/// `fixture-repo/fixture-repo/run-002` — and query must still find it.
#[test]
fn run_execute_does_not_double_nest_when_authority_root_already_ends_with_repo_name() {
    let root = temp_dir("pre-nested");
    let repo = fixture_repo(&root);
    let artifact_root = root.join("authority");
    let pre_nested_authority = artifact_root.join("fixture-repo");
    fs::create_dir_all(&pre_nested_authority).expect("fixture pre-nested authority root");

    let (output, result) = run_execute(&root, &repo, &pre_nested_authority, "run-002");
    assert_eq!(
        output.status.code(),
        Some(0),
        "run execute did not exit 0: {result}\nstderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(result["publication"]["repo"], "fixture-repo");
    let published_path = PathBuf::from(
        result["publication"]["path"]
            .as_str()
            .expect("publication path is a string"),
    );
    assert_eq!(
        published_path,
        pre_nested_authority.join("run-002"),
        "must not double-nest when --authority-root already ends with the repo name: {}",
        published_path.display()
    );

    // Query still points at the un-nested artifact root, exactly as issue
    // #111's confirmed workaround did.
    let (query_output, query_result) = query(&artifact_root, "fixture-repo", &repo);
    assert!(
        query_output.status.success(),
        "artifact query failed against a pre-nested --authority-root: {query_result}\nstderr={}",
        String::from_utf8_lossy(&query_output.stderr)
    );

    fs::remove_dir_all(&root).ok();
}
