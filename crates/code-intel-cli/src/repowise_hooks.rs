//! Detects whether the optional `repowise` codebase-intelligence tool has
//! its two Claude Code integration hooks installed for this repo, and, only
//! with `--write`, installs them by shelling out to `repowise hook install`
//! and `repowise hook rewrite install`.
//!
//! Both installs are pinned to `--no-workspace`: `repowise hook install`
//! defaults to workspace mode (every repo under a detected multi-repo root)
//! the moment it is invoked from inside one, and this command must only ever
//! touch the repo it was asked about.
//!
//! This is optional-dependency glue, not part of `doctor_bootstrap`'s
//! observation pipeline — that module is explicitly documented as
//! never-writes, and installing a hook is a write. `repowise` absent is a
//! valid, silent-success outcome: most contributors and CI machines will
//! never have it installed, and this repo is public, so nothing here ever
//! reads or writes a credential, API key, or provider endpoint.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "tool_path.rs"]
mod tool_path;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match parse_cli(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };

    let Some(repowise_binary) = tool_path::locate("repowise", None) else {
        return 0;
    };

    let status = detect(&cli.repo);
    print_status(&status);

    if !cli.write {
        return 0;
    }

    if status.post_commit_installed && status.rewrite_installed {
        println!("repowise-hooks: already installed, nothing to write");
        return 0;
    }

    let mut failures = 0;
    if !status.post_commit_installed {
        match run_install(
            &repowise_binary,
            &cli.repo,
            &["hook", "install", "--no-workspace"],
        ) {
            Ok(()) => println!("repowise-hooks: installed post-commit auto-sync hook"),
            Err(message) => {
                eprintln!("error: {message}");
                failures += 1;
            }
        }
    }
    if !status.rewrite_installed {
        match run_install(
            &repowise_binary,
            &cli.repo,
            &["hook", "rewrite", "install", "--no-workspace"],
        ) {
            Ok(()) => println!("repowise-hooks: installed distill rewrite hook"),
            Err(message) => {
                eprintln!("error: {message}");
                failures += 1;
            }
        }
    }

    i32::from(failures > 0) * 74
}

struct Cli {
    repo: PathBuf,
    write: bool,
}

fn parse_cli(raw: &[String]) -> Result<Cli, String> {
    let mut repo = None;
    let mut write = false;
    let mut index = 0;
    while index < raw.len() {
        match raw[index].as_str() {
            "--repo" => {
                let value = raw
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or("--repo requires one value")?;
                if repo.replace(PathBuf::from(value)).is_some() {
                    return Err("duplicate --repo".into());
                }
                index += 2;
            }
            "--write" => {
                write = true;
                index += 1;
            }
            other => return Err(format!("unknown repowise-hooks argument: {other}")),
        }
    }
    let repo = match repo {
        Some(repo) => repo,
        None => std::env::current_dir().map_err(|error| error.to_string())?,
    };
    if !repo.is_dir() {
        return Err(format!(
            "repository path is not a directory: {}",
            repo.display()
        ));
    }
    let repo = std::fs::canonicalize(&repo).map_err(|error| error.to_string())?;
    Ok(Cli { repo, write })
}

struct HookStatus {
    post_commit_installed: bool,
    rewrite_installed: bool,
}

const POST_COMMIT_MARKER: &str = "repowise-hook-start";
const REWRITE_MARKER: &str = "repowise-rewrite";

fn detect(repo: &Path) -> HookStatus {
    let post_commit_installed = git_dir(repo)
        .map(|dir| dir.join("hooks").join("post-commit"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|content| content.contains(POST_COMMIT_MARKER));

    let settings_path = crate::doctor_bootstrap::home_directory()
        .join(".claude")
        .join("settings.json");
    let rewrite_installed = std::fs::read_to_string(settings_path)
        .ok()
        .is_some_and(|content| content.contains(REWRITE_MARKER));

    HookStatus {
        post_commit_installed,
        rewrite_installed,
    }
}

/// `.git` is a directory in a normal checkout but a pointer file
/// (`gitdir: <path>`) inside a linked worktree, naming that worktree's own
/// *private* git directory (`<common>/worktrees/<name>`) -- not where hooks
/// live. `git worktree add` never duplicates hooks per worktree; they stay
/// under the one shared directory every worktree's private directory points
/// back to via its own `commondir` file. A normal checkout has no such
/// indirection, so its `.git` directory is already the shared one.
fn git_dir(repo: &Path) -> Option<PathBuf> {
    let dot_git = repo.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let contents = std::fs::read_to_string(&dot_git).ok()?;
    let pointer = contents.trim().strip_prefix("gitdir:")?.trim();
    let resolved = PathBuf::from(pointer);
    let private_dir = if resolved.is_absolute() {
        resolved
    } else {
        repo.join(resolved)
    };
    Some(common_git_dir(&private_dir))
}

/// Resolve a linked worktree's private git directory to the shared one hooks
/// actually live under, by following its `commondir` file. No `commondir`
/// (a plain checkout resolved through some other indirection, or a git
/// version that predates the file) means `private_dir` already is the
/// shared directory, so it is returned unchanged.
fn common_git_dir(private_dir: &Path) -> PathBuf {
    let Ok(contents) = std::fs::read_to_string(private_dir.join("commondir")) else {
        return private_dir.to_path_buf();
    };
    let pointer = PathBuf::from(contents.trim());
    if pointer.is_absolute() {
        pointer
    } else {
        private_dir.join(pointer)
    }
}

fn print_status(status: &HookStatus) {
    println!(
        "repowise-hooks: post-commit auto-sync hook {}",
        if status.post_commit_installed {
            "installed"
        } else {
            "not installed"
        }
    );
    println!(
        "repowise-hooks: distill rewrite hook {}",
        if status.rewrite_installed {
            "installed"
        } else {
            "not installed"
        }
    );
}

fn run_install(binary: &Path, repo: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new(binary)
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|error| format!("spawning repowise {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        return Err(format!(
            "repowise {} exited with {}: {}",
            args.join(" "),
            output.status,
            text.trim()
        ));
    }
    Ok(())
}
