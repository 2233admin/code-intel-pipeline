//! `code-intel friction <log|list|publish|sync>` — this crate's port of
//! wevm/frog's "automated friction logging for agents" concept: an agent
//! that hits friction records it here (`.agents/friction-log/`), the entry
//! can be turned into a GitHub issue, and reconciliation removes the entry
//! once that issue closes. Shared plumbing lives here; each subcommand owns
//! its own file for the god-file reason `edit_routes`/`repowise_routes`
//! split did.

use std::path::PathBuf;

pub(crate) mod entry;
pub(crate) mod list_cmd;
pub(crate) mod log_cmd;
pub(crate) mod publish_cmd;
pub(crate) mod sync_cmd;

/// This crate's compatibility routes report a bare exit code, not a typed
/// error object (`project_context::error::ProjectError` is for the
/// project-query surface only) — this mirrors that same sysexits mapping
/// (64 usage, 65 contract/data, 74 host I/O) for the four `friction`
/// handlers instead of inventing a fifth taxonomy.
#[derive(Debug)]
pub(crate) enum FrictionError {
    Usage(String),
    DataErr(String),
    HostIo(String),
}

impl FrictionError {
    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            FrictionError::Usage(_) => 64,
            FrictionError::DataErr(_) => 65,
            FrictionError::HostIo(_) => 74,
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            FrictionError::Usage(message)
            | FrictionError::DataErr(message)
            | FrictionError::HostIo(message) => message,
        }
    }
}

/// Prints an error to stderr and returns its exit code, or 0 on success —
/// the last line of every subcommand's `run_raw`.
pub(crate) fn report(result: Result<(), FrictionError>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("friction: {}", error.message());
            error.exit_code()
        }
    }
}

/// Pulls an optional `--repo <path>` out of `raw` (same shape as
/// `repowise_hooks::parse_cli`'s), returning the canonicalized repository
/// root -- defaulting to the current directory -- and the remaining
/// arguments in original order for the caller's own flag parsing.
pub(crate) fn take_repo(raw: &[String]) -> Result<(PathBuf, Vec<String>), FrictionError> {
    let mut repo = None;
    let mut rest = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == "--repo" {
            let value = raw
                .get(index + 1)
                .ok_or_else(|| FrictionError::Usage("--repo requires one value".into()))?;
            if repo.replace(PathBuf::from(value)).is_some() {
                return Err(FrictionError::Usage("duplicate --repo".into()));
            }
            index += 2;
        } else {
            rest.push(raw[index].clone());
            index += 1;
        }
    }
    let repo = match repo {
        Some(repo) => repo,
        None => {
            std::env::current_dir().map_err(|error| FrictionError::HostIo(error.to_string()))?
        }
    };
    if !repo.is_dir() {
        return Err(FrictionError::Usage(format!(
            "repository path is not a directory: {}",
            repo.display()
        )));
    }
    let repo = std::fs::canonicalize(&repo)
        .map_err(|error| FrictionError::HostIo(error.to_string()))?;
    Ok((repo, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_repo_defaults_to_the_current_directory() {
        let (repo, rest) = take_repo(&["--title".into(), "x".into()]).unwrap();
        assert_eq!(repo, std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap());
        assert_eq!(rest, vec!["--title".to_string(), "x".to_string()]);
    }

    #[test]
    fn take_repo_rejects_a_missing_value() {
        assert!(take_repo(&["--repo".into()]).is_err());
    }

    #[test]
    fn take_repo_rejects_duplicate_flags() {
        let dir = std::env::temp_dir();
        let arg = dir.to_string_lossy().into_owned();
        assert!(take_repo(&[
            "--repo".into(),
            arg.clone(),
            "--repo".into(),
            arg
        ])
        .is_err());
    }
}
