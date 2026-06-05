//! Thin wrapper around the `repowise` CLI. We shell out rather than link
//! repowise as a Rust dependency because repowise ships a Python binary, not
//! a Rust library.

use std::process::Stdio;

use anyhow::{Context, Result, anyhow};
use tokio::process::Command;

/// Public output of `repowise init` / `augment` — captured by tail-parsing stdout.
#[derive(Debug)]
pub struct CmdOutput {
    pub files: usize,
    pub languages: Vec<String>,
    pub commits: u32,
    pub stdout_tail: String,
}

pub fn check_installed() -> Result<()> {
    let status = std::process::Command::new("repowise")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to spawn `repowise --version`")?;
    if !status.success() {
        return Err(anyhow!(
            "repowise --version exited with status {status}; install via `pip install repowise`"
        ));
    }
    Ok(())
}

pub fn binary_path() -> String {
    which_first("repowise").unwrap_or_else(|| "(not found)".to_string())
}

pub fn version() -> Result<String> {
    let out = std::process::Command::new("repowise")
        .arg("--version")
        .output()
        .context("failed to run repowise --version")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.trim().to_string())
}

pub async fn run_init(repo: &str) -> Result<CmdOutput> {
    let out = Command::new("repowise")
        .arg("init")
        .arg(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("failed to spawn repowise init {repo}"))?;

    if !out.status.success() {
        return Err(anyhow!(
            "repowise init failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    parse_output(&String::from_utf8_lossy(&out.stdout))
}

pub async fn run_augment(repo: &str) -> Result<CmdOutput> {
    let out = Command::new("repowise")
        .arg("augment")
        .arg(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("failed to spawn repowise augment {repo}"))?;

    if !out.status.success() {
        return Err(anyhow!(
            "repowise augment failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    parse_output(&String::from_utf8_lossy(&out.stdout))
}

/// `codenexus::lite` reads `.repowise/wiki.db` to get a compact Agent context.
/// We read up to `max_files` rows from the files table and up to `max_references_per_file`
/// from the references table.
pub async fn read_wiki_summary(
    db_path: &std::path::Path,
    max_files: usize,
    max_refs: usize,
) -> Result<serde_json::Value> {
    use serde_json::json;

    // We try the `sqlite3` CLI first; if missing, return a graceful hint instead of failing.
    let sqlite = which_first("sqlite3").ok_or_else(|| {
        anyhow!("sqlite3 CLI not on PATH; install or rely on the Python sqlite3 stdlib")
    })?;

    let files_sql = format!(
        "SELECT path, language, loc FROM files ORDER BY loc DESC LIMIT {max_files};"
    );
    let refs_sql = format!("SELECT from_path, to_path, kind FROM references LIMIT {max_refs};");

    let files_out = Command::new(&sqlite)
        .arg(db_path)
        .arg(&files_sql)
        .output()
        .await
        .context("sqlite3 files query failed")?;

    let refs_out = Command::new(&sqlite)
        .arg(db_path)
        .arg(&refs_sql)
        .output()
        .await
        .context("sqlite3 references query failed")?;

    let files_text = String::from_utf8_lossy(&files_out.stdout);
    let refs_text = String::from_utf8_lossy(&refs_out.stdout);

    let files: Vec<serde_json::Value> = files_text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let parts: Vec<&str> = l.split('|').collect();
            json!({
                "path": parts.first().copied().unwrap_or(""),
                "language": parts.get(1).copied().unwrap_or(""),
                "loc": parts.get(2).and_then(|p| p.parse::<u64>().ok()).unwrap_or(0),
            })
        })
        .collect();

    let references: Vec<serde_json::Value> = refs_text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let parts: Vec<&str> = l.split('|').collect();
            json!({
                "from": parts.first().copied().unwrap_or(""),
                "to": parts.get(1).copied().unwrap_or(""),
                "kind": parts.get(2).copied().unwrap_or(""),
            })
        })
        .collect();

    Ok(json!({
        "files": files,
        "references": references,
        "limits": { "max_files": max_files, "max_references_per_file": max_refs },
    }))
}

fn parse_output(stdout: &str) -> Result<CmdOutput> {
    // Repowise's `init` output contains a one-liner like:
    //   "39 files · 3 languages · 32 commits"
    // We extract numbers conservatively.
    let mut files = 0usize;
    let mut commits = 0u32;
    let mut languages: Vec<String> = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_suffix(" commits") {
            if let Some(n) = rest.rsplit(' ').next() {
                commits = n.parse().unwrap_or(0);
            }
        }
        if let Some(rest) = trimmed.strip_suffix(" files") {
            // "39 files · 3 languages · ..."
            let parts: Vec<&str> = rest.split('·').collect();
            if let Some(first) = parts.first() {
                if let Some(n) = first.trim().split(' ').next() {
                    files = n.parse().unwrap_or(0);
                }
            }
            for chunk in &parts[1..] {
                let t = chunk.trim();
                if t.ends_with(" languages") {
                    languages = t
                        .split(' ')
                        .filter(|s| !s.is_empty() && *s != "languages")
                        .map(|s| s.trim_end_matches(','))
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect();
                }
            }
        }
    }

    let stdout_tail: String = stdout
        .lines()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");

    Ok(CmdOutput {
        files,
        languages,
        commits,
        stdout_tail,
    })
}

/// Lightweight `which` replacement — tries to find the binary by running it with `--version`.
/// We don't need a full PATH search; repowise is typically a `pip install` Script.
fn which_first(name: &str) -> Option<String> {
    let out = std::process::Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => Some(name.to_string()),
        _ => None,
    }
}
