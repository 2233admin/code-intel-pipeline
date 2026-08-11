use std::fs;
use std::path::{Path, PathBuf};

use super::super::ProjectError;
use crate::snapshot;

#[derive(Debug)]
pub(super) struct GitIdentity {
    pub(super) root: PathBuf,
    pub(super) common_checkout: PathBuf,
    pub(super) repo_identity: String,
    pub(super) head: String,
}

pub(super) fn matches_expected(
    actual: &GitIdentity,
    expected_identity: &str,
    expected_head: &str,
) -> bool {
    actual.repo_identity == expected_identity && actual.head == expected_head
}

pub(super) fn git_identity(root: &Path) -> Result<Option<GitIdentity>, ProjectError> {
    let Some((repo_identity, head)) =
        snapshot::git_repository_identity(root).map_err(ProjectError::contract)?
    else {
        return Ok(None);
    };
    let top = git_text(root, &["rev-parse", "--show-toplevel"])?;
    let root = fs::canonicalize(top.trim()).map_err(|error| {
        ProjectError::host_io(format!("canonicalize Git worktree root: {error}"))
    })?;
    let common_dir = git_text(&root, &["rev-parse", "--git-common-dir"])?;
    let common_dir = PathBuf::from(common_dir.trim());
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        root.join(common_dir)
    };
    let common_dir = fs::canonicalize(&common_dir)
        .map_err(|error| ProjectError::host_io(format!("canonicalize Git common dir: {error}")))?;
    let common_checkout = common_dir
        .parent()
        .map(fs::canonicalize)
        .transpose()
        .map_err(|error| {
            ProjectError::host_io(format!("canonicalize Git common checkout: {error}"))
        })?
        .unwrap_or_else(|| root.clone());
    Ok(Some(GitIdentity {
        root,
        common_checkout,
        repo_identity,
        head,
    }))
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, ProjectError> {
    let output = crate::hardened_git::command(root)
        .args(args)
        .output()
        .map_err(|error| ProjectError::host_io(format!("launch git {:?}: {error}", args)))?;
    if !output.status.success() {
        return Err(ProjectError::contract(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout).map_err(|error| {
        ProjectError::contract(format!("git {:?} emitted non-UTF-8: {error}", args))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mismatch_is_not_a_degraded_success() {
        let actual = GitIdentity {
            root: PathBuf::from("requested"),
            common_checkout: PathBuf::from("main"),
            repo_identity: "git-lineage-v1:actual".into(),
            head: "actual-head".into(),
        };
        assert!(!matches_expected(
            &actual,
            "git-lineage-v1:indexed",
            "indexed-head"
        ));
    }
}
