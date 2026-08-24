mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_TREE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEMP_TREE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "code-intel-{label}-{nonce}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A directory holding a stub `gh` executable that never touches the real
/// GitHub CLI: it records its own argv to `GH_STUB_LOG`, then answers
/// `issue create` with a canned URL and `issue view ... --json state` with
/// `GH_STUB_STATE` (default `OPEN`), matching the two calls
/// `friction publish`/`friction sync` make.
struct FakeGh {
    dir: TempTree,
    log: PathBuf,
}

impl FakeGh {
    fn new() -> Self {
        let dir = TempTree::new("friction-gh-stub");
        let log = dir.0.join("invocations.log");
        write_stub(&dir.0);
        Self { dir, log }
    }

    fn invocations(&self) -> Vec<String> {
        let Ok(content) = fs::read_to_string(&self.log) else {
            return Vec::new();
        };
        content
            .lines()
            .filter_map(|line| line.strip_prefix("ARGS="))
            .map(str::to_string)
            .collect()
    }
}

#[cfg(windows)]
fn write_stub(dir: &Path) {
    let script = r#"@echo off
echo ARGS=%*>>"%GH_STUB_LOG%"
echo %*| findstr /C:"issue create" >nul
if %errorlevel%==0 (
  echo https://github.com/example/repo/issues/1
  exit /b 0
)
echo %*| findstr /C:"issue view" >nul
if %errorlevel%==0 (
  echo {"state":"%GH_STUB_STATE%"}
  exit /b 0
)
exit /b 1
"#;
    fs::write(dir.join("gh.cmd"), script).unwrap();
}

#[cfg(unix)]
fn write_stub(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let script = r#"#!/bin/sh
echo "ARGS=$*" >> "$GH_STUB_LOG"
case "$*" in
  *"issue create"*) echo "https://github.com/example/repo/issues/1"; exit 0 ;;
  *"issue view"*) echo "{\"state\":\"$GH_STUB_STATE\"}"; exit 0 ;;
  *) exit 1 ;;
esac
"#;
    let path = dir.join("gh");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn run(repo: &Path, args: &[&str], gh: Option<(&FakeGh, &str)>) -> Output {
    let mut command = common::cli();
    command.arg("friction");
    for arg in args {
        command.arg(arg);
    }
    command.arg("--repo").arg(repo);
    if let Some((stub, state)) = gh {
        let existing = std::env::var_os("PATH").unwrap_or_default();
        let entries = std::iter::once(stub.dir.0.clone()).chain(std::env::split_paths(&existing));
        let path_with_stub = std::env::join_paths(entries).unwrap();
        command
            .env("PATH", path_with_stub)
            .env("GH_STUB_LOG", &stub.log)
            .env("GH_STUB_STATE", state);
    }
    command.output().unwrap()
}

fn entry_dirs(repo: &Path) -> Vec<PathBuf> {
    let root = repo.join(".agents/friction-log");
    if !root.is_dir() {
        return Vec::new();
    }
    let mut dirs: Vec<_> = fs::read_dir(root)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn log_writes_frontmatter_body_and_artifacts() {
    let repo = TempTree::new("friction-log-basic");
    let artifact = repo.0.join("repro.txt");
    fs::write(&artifact, "reproduction steps").unwrap();

    let output = run(
        &repo.0,
        &[
            "log",
            "--title",
            "Config loader turns missing env into a string",
            "--summary",
            "Setting FOO unset yields the literal string \"undefined\".",
            "--artifact",
            artifact.to_str().unwrap(),
        ],
        None,
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let dirs = entry_dirs(&repo.0);
    assert_eq!(dirs.len(), 1, "expected exactly one entry: {dirs:?}");
    let content = fs::read_to_string(dirs[0].join("friction.md")).unwrap();
    assert!(content.starts_with("title: Config loader turns missing env into a string\n"));
    assert!(content.contains("status: pending\n"));
    assert!(content.contains("issue: \n"));
    assert!(content.contains("Setting FOO unset yields the literal string \"undefined\"."));
    assert!(dirs[0].join("artifacts").join("repro.txt").is_file());
}

#[test]
fn list_reports_a_logged_entry_and_exits_zero() {
    let repo = TempTree::new("friction-list-happy");
    run(&repo.0, &["log", "--title", "t", "--summary", "s"], None);

    let output = run(&repo.0, &["list"], None);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[pending] t"));
}

#[test]
fn list_exits_65_when_an_entry_fails_to_parse() {
    let repo = TempTree::new("friction-list-malformed");
    let entry_dir = repo.0.join(".agents/friction-log/20260101T000000Z-bad");
    fs::create_dir_all(&entry_dir).unwrap();
    fs::write(entry_dir.join("friction.md"), "not frontmatter at all").unwrap();

    let output = run(&repo.0, &["list"], None);
    assert_eq!(output.status.code(), Some(65));
}

#[test]
fn publish_defaults_to_dry_run_and_calls_gh_only_with_yes() {
    let repo = TempTree::new("friction-publish-dry-run");
    run(&repo.0, &["log", "--title", "t", "--summary", "s"], None);
    let slug = entry_dirs(&repo.0)[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let stub = FakeGh::new();

    let dry_run = run(
        &repo.0,
        &["publish", "--slug", &slug],
        Some((&stub, "OPEN")),
    );
    assert!(dry_run.status.success());
    assert!(
        stub.invocations().is_empty(),
        "dry run must never invoke gh: {:?}",
        stub.invocations()
    );
    let content_before = fs::read_to_string(entry_dirs(&repo.0)[0].join("friction.md")).unwrap();
    assert!(content_before.contains("status: pending"));

    let published = run(
        &repo.0,
        &["publish", "--slug", &slug, "--yes"],
        Some((&stub, "OPEN")),
    );
    assert!(
        published.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&published.stderr)
    );
    let invocations = stub.invocations();
    assert_eq!(invocations.len(), 1, "{invocations:?}");
    assert!(
        invocations[0].contains("--body-file"),
        "issue body must go through --body-file, not inline argv: {}",
        invocations[0]
    );
    assert!(
        !invocations[0].contains("s\n") && !invocations[0].to_lowercase().contains("--body s"),
        "the entry summary text must never appear inline on argv: {}",
        invocations[0]
    );
    let content_after = fs::read_to_string(entry_dirs(&repo.0)[0].join("friction.md")).unwrap();
    assert!(content_after.contains("status: published"));
    assert!(content_after.contains("issue: https://github.com/example/repo/issues/1"));
}

#[test]
fn sync_dry_run_lists_without_deleting_then_yes_removes_closed_entries() {
    let repo = TempTree::new("friction-sync");
    run(&repo.0, &["log", "--title", "t", "--summary", "s"], None);
    let slug = entry_dirs(&repo.0)[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let stub = FakeGh::new();
    run(
        &repo.0,
        &["publish", "--slug", &slug, "--yes"],
        Some((&stub, "OPEN")),
    );

    let dry_run = run(&repo.0, &["sync"], Some((&stub, "CLOSED")));
    assert!(dry_run.status.success());
    assert!(String::from_utf8_lossy(&dry_run.stdout).contains("would remove"));
    assert_eq!(entry_dirs(&repo.0).len(), 1, "dry run must not delete");

    let applied = run(&repo.0, &["sync", "--yes"], Some((&stub, "CLOSED")));
    assert!(applied.status.success());
    assert!(
        entry_dirs(&repo.0).is_empty(),
        "closed entry must be removed"
    );
}

#[test]
fn sync_keeps_entries_whose_issue_is_still_open() {
    let repo = TempTree::new("friction-sync-open");
    run(&repo.0, &["log", "--title", "t", "--summary", "s"], None);
    let slug = entry_dirs(&repo.0)[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let stub = FakeGh::new();
    run(
        &repo.0,
        &["publish", "--slug", &slug, "--yes"],
        Some((&stub, "OPEN")),
    );

    let output = run(&repo.0, &["sync", "--yes"], Some((&stub, "OPEN")));
    assert!(output.status.success());
    assert_eq!(entry_dirs(&repo.0).len(), 1, "open entry must be kept");
}
