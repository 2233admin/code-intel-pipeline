use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

mod predicates;
mod scoring;

/// A fresh, empty temp directory namespaced the same way `init_repo` names
/// its repos, but without running `git init` — the starting point for both
/// a to-be-initialized repo and (for the invalid-`--repo` test) a directory
/// that must never look like one.
fn temp_dir_for(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "code-intel-change-risk-{name}-{nonce}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn init_repo(name: &str) -> PathBuf {
    let repo = temp_dir_for(name);
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "user.name", "Change Risk Test"],
        vec!["config", "user.email", "change-risk@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    repo
}

fn write_file(repo: &Path, relative: &str, contents: &str) {
    let path = repo.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn commit(repo: &Path, message: &str) {
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["commit", "--quiet", "-m", message])
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
}

fn checkout_new_branch(repo: &Path, name: &str, start_point: Option<&str>) {
    let mut args = vec!["checkout", "--quiet", "-b", name];
    if let Some(start_point) = start_point {
        args.push(start_point);
    }
    assert!(Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
}

fn rev_parse(repo: &Path, rev: &str) -> String {
    let output = Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
