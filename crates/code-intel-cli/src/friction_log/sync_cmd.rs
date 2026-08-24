//! `friction sync` — reconciliation: for every published entry, checks
//! whether its GitHub issue closed, and (only with `--yes`) removes the
//! entry directory once it has. Checking state is a read and runs either
//! way; only the deletion is gated, for the same reason `publish` gates its
//! `gh issue create` call behind `--yes` (see that module's docs).

use super::entry::{self, Entry, Status};
use super::{take_repo, FrictionError};
use crate::hardened_gh;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match run(raw) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("friction: {}", error.message());
            error.exit_code()
        }
    }
}

fn run(raw: &[String]) -> Result<i32, FrictionError> {
    let (repo, rest) = take_repo(raw)?;
    let yes = parse_cli(&rest)?;

    let dirs = entry::list_dirs(&repo).map_err(|error| FrictionError::HostIo(error.to_string()))?;
    let mut any_error = false;
    for dir in &dirs {
        let entry = match Entry::parse(dir) {
            Ok(entry) => entry,
            Err(message) => {
                eprintln!("friction: {message}");
                any_error = true;
                continue;
            }
        };
        if entry.status != Status::Published {
            continue;
        }
        let Some(issue_url) = entry.issue.clone() else {
            eprintln!("friction: {}: published with no recorded issue", entry.id);
            any_error = true;
            continue;
        };
        match issue_state(&repo, &issue_url) {
            Ok(state) if state == "CLOSED" => {
                if yes {
                    std::fs::remove_dir_all(dir)
                        .map_err(|error| FrictionError::HostIo(error.to_string()))?;
                    println!("friction: removed {} (issue closed)", entry.id);
                } else {
                    println!(
                        "friction: would remove {} (issue closed, dry run)",
                        entry.id
                    );
                }
            }
            Ok(state) => println!("friction: kept {} (issue {state})", entry.id),
            Err(message) => {
                eprintln!("friction: {}: {message}", entry.id);
                any_error = true;
            }
        }
    }

    Ok(if any_error { 65 } else { 0 })
}

fn issue_state(repo: &std::path::Path, issue_url: &str) -> Result<String, String> {
    let output = hardened_gh::command(repo)
        .args(["issue", "view", issue_url, "--json", "state"])
        .output()
        .map_err(|error| format!("spawning gh: {error}"))?;
    if !output.status.success() {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        return Err(format!(
            "gh issue view exited with {}: {}",
            output.status,
            hardened_gh::redact(text.trim())
        ));
    }
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("gh issue view produced invalid JSON: {error}"))?;
    parsed["state"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "gh issue view JSON omitted state".to_string())
}

fn parse_cli(raw: &[String]) -> Result<bool, FrictionError> {
    let mut yes = false;
    for argument in raw {
        match argument.as_str() {
            "--yes" => yes = true,
            other => {
                return Err(FrictionError::Usage(format!(
                    "unknown friction sync argument: {other}"
                )))
            }
        }
    }
    Ok(yes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_recognizes_yes() {
        assert!(parse_cli(&["--yes".into()]).unwrap());
        assert!(!parse_cli(&[]).unwrap());
    }

    #[test]
    fn parse_cli_rejects_unknown_flags() {
        assert!(parse_cli(&["--bogus".into()]).is_err());
    }
}
