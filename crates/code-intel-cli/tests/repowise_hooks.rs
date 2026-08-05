mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("code-intel-{label}-{nonce}"));
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

fn run(repo: &Path, home: &Path, extra_args: &[&str]) -> Output {
    let mut command = common::cli();
    command
        .arg("repowise-hooks")
        .arg("--repo")
        .arg(repo)
        .env("USERPROFILE", home)
        .env("HOME", home);
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

    let output = run(&repo.0, &home.0, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("post-commit auto-sync hook not installed"));
    assert!(stdout.contains("distill rewrite hook not installed"));
    assert!(
        !repo.0.join(".git").join("hooks").join("post-commit").exists(),
        "detection alone must never write a hook"
    );
}

#[test]
fn detects_installed_post_commit_hook() {
    let repo = TempTree::new("repowise-hooks-postcommit");
    init_repo(&repo.0);
    let home = fake_home();

    let hooks_dir = repo.0.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(
        hooks_dir.join("post-commit"),
        "#!/bin/sh\n# repowise-hook-start\necho installed\n# repowise-hook-end\n",
    )
    .unwrap();

    let output = run(&repo.0, &home.0, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("post-commit auto-sync hook installed"));
}

#[test]
fn detects_installed_rewrite_hook() {
    let repo = TempTree::new("repowise-hooks-rewrite");
    init_repo(&repo.0);
    let home = fake_home();

    let claude_dir = home.0.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.json"),
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"command":"repowise-rewrite"}]}]}}"#,
    )
    .unwrap();

    let output = run(&repo.0, &home.0, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("distill rewrite hook installed"));
}

#[test]
fn resolves_hooks_dir_through_linked_worktree_gitdir_pointer() {
    // Mirrors a linked `git worktree`: `.git` is a pointer FILE naming the
    // real git directory elsewhere, not a directory of its own. Hooks live
    // under that real directory, never under the worktree's own `.git`.
    let real_git = TempTree::new("repowise-hooks-realgit");
    let hooks_dir = real_git.0.join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(
        hooks_dir.join("post-commit"),
        "#!/bin/sh\n# repowise-hook-start\necho installed\n# repowise-hook-end\n",
    )
    .unwrap();

    let worktree = TempTree::new("repowise-hooks-worktree");
    fs::write(
        worktree.0.join(".git"),
        format!("gitdir: {}\n", real_git.0.display()),
    )
    .unwrap();
    let home = fake_home();

    let output = run(&worktree.0, &home.0, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("post-commit auto-sync hook installed"));
}

#[test]
fn rejects_non_directory_repo() {
    let home = fake_home();
    let missing = std::env::temp_dir().join("code-intel-repowise-hooks-does-not-exist");
    let _ = fs::remove_dir_all(&missing);

    let output = run(&missing, &home.0, &[]);

    assert_eq!(output.status.code(), Some(64));
}
