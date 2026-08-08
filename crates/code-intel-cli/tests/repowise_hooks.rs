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

fn init_repo(repo: &Path) {
    let output = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(output.status.success());
}

/// A fake `$HOME`/`%USERPROFILE%` so tests never read or write this
/// machine's real `~/.claude/settings.json`.
fn fake_home() -> TempTree {
    TempTree::new("repowise-hooks-home")
}

/// A directory holding a stub `repowise` executable that never touches the
/// real tool: it only records its own argv and working directory to
/// `REPOWISE_STUB_LOG`, then exits 0. Whether the *real* `repowise` is on
/// this machine's PATH must not change what these tests exercise, so every
/// `run()` call prepends this directory ahead of the real PATH.
struct FakeRepowise {
    dir: TempTree,
    log: PathBuf,
}

impl FakeRepowise {
    fn new() -> Self {
        let dir = TempTree::new("repowise-hooks-stub");
        let log = dir.0.join("invocations.log");
        write_stub(&dir.0);
        Self { dir, log }
    }

    /// One `ARGS=...` line per invocation, in call order. Empty means the
    /// stub was never invoked.
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

/// `tool_path::locate` only tries `{name}.exe`/`.cmd`/`.bat`/bare on
/// Windows and only the bare name elsewhere, so the stub's own extension
/// and script dialect must match whichever platform is running the test.
#[cfg(windows)]
fn write_stub(dir: &Path) {
    let script = "@echo off\r\necho CWD=%CD%>>\"%REPOWISE_STUB_LOG%\"\r\necho ARGS=%*>>\"%REPOWISE_STUB_LOG%\"\r\nexit /b 0\r\n";
    fs::write(dir.join("repowise.cmd"), script).unwrap();
}

#[cfg(unix)]
fn write_stub(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let script = "#!/bin/sh\necho \"CWD=$(pwd)\" >> \"$REPOWISE_STUB_LOG\"\necho \"ARGS=$*\" >> \"$REPOWISE_STUB_LOG\"\nexit 0\n";
    let path = dir.join("repowise");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn run(repo: &Path, home: &Path, stub: &FakeRepowise, extra_args: &[&str]) -> Output {
    // `;`-joining unconditionally would be wrong on Unix, where PATH is
    // `:`-delimited -- `env::join_paths` picks whichever the running
    // platform actually uses.
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let entries = std::iter::once(stub.dir.0.clone()).chain(std::env::split_paths(&existing));
    let path_with_stub = std::env::join_paths(entries).unwrap();
    let mut command = common::cli();
    command
        .arg("repowise-hooks")
        .arg("--repo")
        .arg(repo)
        .env("USERPROFILE", home)
        .env("HOME", home)
        .env("PATH", path_with_stub)
        .env("REPOWISE_STUB_LOG", &stub.log);
    for arg in extra_args {
        command.arg(arg);
    }
    command.output().unwrap()
}

#[test]
fn detects_missing_hooks_without_write() {
    let repo = TempTree::new("repowise-hooks-missing");
    init_repo(&repo.0);
    let home = fake_home();
    let stub = FakeRepowise::new();

    let output = run(&repo.0, &home.0, &stub, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("post-commit auto-sync hook not installed"));
    assert!(stdout.contains("distill rewrite hook not installed"));
    assert!(
        !repo
            .0
            .join(".git")
            .join("hooks")
            .join("post-commit")
            .exists(),
        "detection alone must never write a hook"
    );
    assert!(
        stub.invocations().is_empty(),
        "a read-only run must never invoke repowise: {:?}",
        stub.invocations()
    );
}

#[test]
fn detects_installed_post_commit_hook() {
    let repo = TempTree::new("repowise-hooks-postcommit");
    init_repo(&repo.0);
    let home = fake_home();
    let stub = FakeRepowise::new();

    let hooks_dir = repo.0.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(
        hooks_dir.join("post-commit"),
        "#!/bin/sh\n# repowise-hook-start\necho installed\n# repowise-hook-end\n",
    )
    .unwrap();

    let output = run(&repo.0, &home.0, &stub, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("post-commit auto-sync hook installed"));
}

#[test]
fn detects_installed_rewrite_hook() {
    let repo = TempTree::new("repowise-hooks-rewrite");
    init_repo(&repo.0);
    let home = fake_home();
    let stub = FakeRepowise::new();

    let claude_dir = home.0.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.json"),
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"command":"repowise-rewrite"}]}]}}"#,
    )
    .unwrap();

    let output = run(&repo.0, &home.0, &stub, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("distill rewrite hook installed"));
}

#[test]
fn resolves_hooks_dir_through_linked_worktree_commondir() {
    // Mirrors a real `git worktree add`: the worktree's `.git` is a pointer
    // FILE naming its own *private* git directory
    // (`<common>/worktrees/<name>`), which is NOT where hooks live. That
    // private directory's own `commondir` file names the shared directory
    // hooks actually live under -- the exact indirection `git worktree add`
    // itself creates, so a resolver that skips it looks for hooks in a
    // directory git never puts them in.
    let common_git = TempTree::new("repowise-hooks-commongit");
    let hooks_dir = common_git.0.join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(
        hooks_dir.join("post-commit"),
        "#!/bin/sh\n# repowise-hook-start\necho installed\n# repowise-hook-end\n",
    )
    .unwrap();

    let private_git = common_git.0.join("worktrees").join("wt");
    fs::create_dir_all(&private_git).unwrap();
    fs::write(
        private_git.join("commondir"),
        format!("{}\n", common_git.0.display()),
    )
    .unwrap();

    let worktree = TempTree::new("repowise-hooks-worktree");
    fs::write(
        worktree.0.join(".git"),
        format!("gitdir: {}\n", private_git.display()),
    )
    .unwrap();
    let home = fake_home();
    let stub = FakeRepowise::new();

    let output = run(&worktree.0, &home.0, &stub, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("post-commit auto-sync hook installed"));
}

#[test]
fn rejects_non_directory_repo() {
    let home = fake_home();
    let stub = FakeRepowise::new();
    let missing = std::env::temp_dir().join("code-intel-repowise-hooks-does-not-exist");
    let _ = fs::remove_dir_all(&missing);

    let output = run(&missing, &home.0, &stub, &[]);

    assert_eq!(output.status.code(), Some(64));
}

#[test]
fn write_installs_both_missing_hooks_scoped_to_this_repo_only() {
    let repo = TempTree::new("repowise-hooks-write-missing");
    init_repo(&repo.0);
    let home = fake_home();
    let stub = FakeRepowise::new();

    let output = run(&repo.0, &home.0, &stub, &["--write"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let invocations = stub.invocations();
    assert_eq!(
        invocations.len(),
        2,
        "expected exactly one call per missing hook: {invocations:?}"
    );
    assert!(invocations.iter().any(|args| {
        args.contains("hook")
            && args.contains("install")
            && args.contains("--no-workspace")
            && !args.contains("rewrite")
    }));
    assert!(invocations.iter().any(|args| args.contains("hook")
        && args.contains("rewrite")
        && args.contains("install")
        && args.contains("--no-workspace")));
}

#[test]
fn write_skips_already_installed_hooks_without_invoking_repowise() {
    let repo = TempTree::new("repowise-hooks-write-already-installed");
    init_repo(&repo.0);
    let home = fake_home();
    let stub = FakeRepowise::new();

    let hooks_dir = repo.0.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(
        hooks_dir.join("post-commit"),
        "#!/bin/sh\n# repowise-hook-start\necho installed\n# repowise-hook-end\n",
    )
    .unwrap();
    let claude_dir = home.0.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.json"),
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"command":"repowise-rewrite"}]}]}}"#,
    )
    .unwrap();

    let output = run(&repo.0, &home.0, &stub, &["--write"]);

    assert!(output.status.success());
    assert!(
        stub.invocations().is_empty(),
        "both hooks were already installed, repowise must not be invoked: {:?}",
        stub.invocations()
    );
}
