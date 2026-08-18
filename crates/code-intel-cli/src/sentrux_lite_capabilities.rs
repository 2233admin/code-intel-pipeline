use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

pub(super) fn provider_discovery_json() -> Result<Value, String> {
    Ok(json!({
        "provider":"sentrux",
        "mode":"builtin_lite",
        "available":true,
        "operations":["scan","health","dsm","git_stats","evolution","test_gaps","check_rules","check","gate","rescan"],
        "aliases":["pro_status","plugin_list","plugin_validate"],
        "legacyFallback":"legacy/Invoke-SentruxAgentTool.ps1"
    }))
}

fn git_command(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|error| format!("start git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("git output is not UTF-8: {error}"))
}

pub(super) fn git_stats_json(repo: &Path) -> Result<Value, String> {
    let count_output = match git_command(repo, &["rev-list", "--count", "HEAD"]) {
        Ok(output) => output,
        Err(reason) => {
            return Ok(json!({
                "commitCount":0,
                "recentCommits":[],
                "status":"unavailable",
                "reason":reason
            }))
        }
    };
    let count = count_output
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("git commit count is invalid: {error}"))?;
    let recent_output = match git_command(repo, &["log", "-n", "20", "--format=%H%x09%aI"]) {
        Ok(output) => output,
        Err(reason) => {
            return Ok(json!({
                "commitCount":count,
                "recentCommits":[],
                "status":"unavailable",
                "reason":reason
            }))
        }
    };
    let recent = recent_output
        .lines()
        .filter_map(|line| {
            let (commit, authored_at) = line.split_once('\t')?;
            Some(json!({"commit":commit,"authoredAt":authored_at}))
        })
        .collect::<Vec<_>>();
    Ok(json!({"commitCount":count,"recentCommits":recent,"status":"ok"}))
}

pub(super) fn evolution_json(repo: &Path) -> Result<Value, String> {
    let recent_output = match git_command(repo, &["log", "-n", "20", "--format=%H%x09%aI%x09%an"]) {
        Ok(output) => output,
        Err(reason) => {
            return Ok(json!({
                "status":"unavailable",
                "windowCommits":0,
                "trend":"unknown",
                "recentCommits":[],
                "reason":reason
            }))
        }
    };
    let recent = recent_output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            Some(json!({
                "commit":fields.next()?,
                "authoredAt":fields.next()?,
                "author":fields.next()?
            }))
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "status":"ok",
        "windowCommits":recent.len(),
        "trend":"observed",
        "recentCommits":recent
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn history_capabilities_are_auditable_without_git_history() {
        let repo = std::path::Path::new("this-path-does-not-contain-a-git-checkout");
        let stats =
            super::git_stats_json(repo).expect("missing git history is a valid lite result");
        let evolution =
            super::evolution_json(repo).expect("missing git history is a valid lite result");

        assert_eq!(stats["status"], "unavailable");
        assert_eq!(stats["commitCount"], 0);
        assert_eq!(evolution["status"], "unavailable");
        assert_eq!(evolution["windowCommits"], 0);
        assert_eq!(evolution["trend"], "unknown");
    }
}

pub(super) fn test_gaps_json(repo: &Path) -> Result<Value, String> {
    let mut source_files = 0_u64;
    let mut test_files = 0_u64;
    let mut stack = vec![repo.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| {
            format!(
                "read test inventory directory {}: {error}",
                directory.display()
            )
        })? {
            let entry = entry.map_err(|error| format!("read test inventory entry: {error}"))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if entry
                .file_type()
                .map_err(|error| format!("inspect test inventory entry: {error}"))?
                .is_dir()
            {
                if !matches!(name.as_str(), ".git" | "target" | "node_modules") {
                    stack.push(path);
                }
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !matches!(
                extension,
                "rs" | "py" | "js" | "ts" | "tsx" | "go" | "java" | "cs"
            ) {
                continue;
            }
            let relative = path
                .strip_prefix(repo)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_ascii_lowercase();
            if relative.contains("test") || relative.contains("spec") {
                test_files += 1;
            } else {
                source_files += 1;
            }
        }
    }
    Ok(json!({
        "status":"heuristic",
        "sourceFiles":source_files,
        "testFiles":test_files,
        "gapStatus":if test_files == 0 { "unknown" } else { "inventory_only" },
        "limitations":["This lite fallback inventories test files; it does not prove symbol-level test coverage."]
    }))
}
