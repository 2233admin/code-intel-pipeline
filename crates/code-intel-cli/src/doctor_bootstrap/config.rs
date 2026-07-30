//! `pipeline.config.json` loading and repository resolution.
//!
//! Ports the resolution rules `check-code-intel-tools.ps1` implemented:
//! alias lookup through `repos`, the reverse path lookup an explicit
//! `-RepoPath` triggers, and the `sentruxPath` scope override.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::paths::{display, resolve_code_intel_path, trim_trailing_separator};
use super::Options;

/// Read and parse the config. A missing file is neither data nor an error; an
/// unparsable one yields the parse message so `missing` can quote it.
pub(super) fn load_config(path: &Path) -> (Option<Value>, Option<String>) {
    if !path.is_file() {
        return (None, None);
    }
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error.to_string())),
        },
        Err(error) => (None, Some(error.to_string())),
    }
}

pub(super) fn repo_config_by_alias<'a>(
    config: Option<&'a Value>,
    alias: &str,
) -> Option<&'a Value> {
    config?.get("repos")?.get(alias)
}

/// Reverse lookup: which configured repo entry points at `repo_path`. Mirrors
/// the PowerShell `Find-RepoConfigByPath`, including its trailing-separator
/// trim and case-insensitive comparison.
pub(super) fn find_repo_config_by_path<'a>(
    config: Option<&'a Value>,
    repo_path: &Path,
) -> Option<&'a Value> {
    let repos = config?.get("repos")?.as_object()?;
    let target = trim_trailing_separator(&display(repo_path));
    repos.values().find(|entry| {
        entry
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .is_some_and(|path| {
                let resolved = display(&resolve_code_intel_path(Path::new(path)));
                trim_trailing_separator(&resolved).eq_ignore_ascii_case(&target)
            })
    })
}

/// An explicit `--repo-path` wins over `--repo`; an alias resolves through the
/// config's `repos` map, falling back to treating the alias as a path.
pub(super) fn resolve_repo_path(options: &Options, config: Option<&Value>) -> Option<PathBuf> {
    if let Some(repo_path) = options
        .repo_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return Some(existing_or_literal(PathBuf::from(repo_path)));
    }
    let alias = options
        .repo
        .as_deref()
        .filter(|value| !value.trim().is_empty())?;
    let configured = repo_config_by_alias(config, alias)
        .and_then(|entry| entry.get("path"))
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty());
    Some(existing_or_literal(PathBuf::from(
        configured.unwrap_or(alias),
    )))
}

/// The configured `sentruxPath`, relative to the repo unless already rooted.
/// Absent config leaves the scope at the repository root.
pub(super) fn resolve_sentrux_scope(repo_path: &Path, repo_config: Option<&&Value>) -> PathBuf {
    let configured = repo_config
        .and_then(|entry| entry.get("sentruxPath"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let Some(configured) = configured else {
        return repo_path.to_path_buf();
    };
    let scope = if Path::new(configured).is_absolute() {
        PathBuf::from(configured)
    } else {
        repo_path.join(configured)
    };
    existing_or_literal(scope)
}

/// Resolve a directory that exists; otherwise keep the literal path so the
/// observation can report a missing repo instead of inventing one.
fn existing_or_literal(path: PathBuf) -> PathBuf {
    if path.is_dir() {
        resolve_code_intel_path(&path)
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn options(repo: Option<&str>, repo_path: Option<&str>) -> Options {
        let mut options = Options::new(PathBuf::from("."));
        options.repo = repo.map(str::to_string);
        options.repo_path = repo_path.map(str::to_string);
        options
    }

    #[test]
    fn an_absent_config_file_is_neither_data_nor_an_error() {
        let (data, error) = load_config(Path::new("does-not-exist.json"));
        assert!(data.is_none());
        assert!(error.is_none());
    }

    #[test]
    fn an_alias_without_a_configured_path_falls_back_to_the_alias_itself() {
        let config = json!({"repos": {"fixture": {}}});
        let resolved = resolve_repo_path(&options(Some("fixture"), None), Some(&config)).unwrap();
        assert_eq!(resolved, PathBuf::from("fixture"));
    }

    #[test]
    fn an_alias_resolves_through_the_configured_path() {
        let config = json!({"repos": {"fixture": {"path": "some/configured/repo"}}});
        let resolved = resolve_repo_path(&options(Some("fixture"), None), Some(&config)).unwrap();
        assert_eq!(resolved, PathBuf::from("some/configured/repo"));
    }

    #[test]
    fn an_explicit_repo_path_wins_over_the_alias() {
        let config = json!({"repos": {"fixture": {"path": "some/configured/repo"}}});
        let resolved = resolve_repo_path(
            &options(Some("fixture"), Some("explicit/path")),
            Some(&config),
        )
        .unwrap();
        assert_eq!(resolved, PathBuf::from("explicit/path"));
    }

    #[test]
    fn no_repo_selector_resolves_to_nothing() {
        assert!(resolve_repo_path(&options(None, None), None).is_none());
    }

    #[test]
    fn an_absent_sentrux_path_leaves_the_scope_at_the_repository_root() {
        let repo = Path::new("some/repo");
        assert_eq!(resolve_sentrux_scope(repo, None), repo.to_path_buf());
        let entry = json!({});
        assert_eq!(
            resolve_sentrux_scope(repo, Some(&&entry)),
            repo.to_path_buf()
        );
    }

    #[test]
    fn a_relative_sentrux_path_is_joined_onto_the_repository() {
        let entry = json!({"sentruxPath": "backend"});
        assert_eq!(
            resolve_sentrux_scope(Path::new("some/repo"), Some(&&entry)),
            PathBuf::from("some/repo").join("backend")
        );
    }

    #[test]
    fn a_rooted_sentrux_path_is_used_verbatim() {
        let rooted = if cfg!(windows) {
            r"C:\scoped"
        } else {
            "/scoped"
        };
        let entry = json!({ "sentruxPath": rooted });
        assert_eq!(
            resolve_sentrux_scope(Path::new("some/repo"), Some(&&entry)),
            PathBuf::from(rooted)
        );
    }

    #[test]
    fn reverse_lookup_ignores_case_and_a_trailing_separator() {
        let here = resolve_code_intel_path(Path::new("."));
        let config = json!({"repos": {"fixture": {
            "path": format!("{}{}", display(&here), std::path::MAIN_SEPARATOR),
            "sentruxPath": "backend"
        }}});
        let entry = find_repo_config_by_path(Some(&config), &here).expect("reverse lookup");
        assert_eq!(entry["sentruxPath"], json!("backend"));
    }

    #[test]
    fn reverse_lookup_skips_entries_without_a_path() {
        let config = json!({"repos": {"fixture": {"sentruxPath": "backend"}}});
        assert!(find_repo_config_by_path(Some(&config), Path::new(".")).is_none());
    }
}
