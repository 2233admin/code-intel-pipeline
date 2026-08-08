use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::committed_evidence::{self, EvidenceError};
use crate::snapshot;

#[derive(Debug, Clone)]
pub(crate) enum RepositoryBinding {
    Verified {
        source: &'static str,
        requested_root: PathBuf,
        bound_root: PathBuf,
        repo_identity: String,
        head: String,
    },
    Degraded {
        reason: String,
    },
}

impl RepositoryBinding {
    pub(super) fn degraded(reason: impl Into<String>) -> Self {
        Self::Degraded {
            reason: reason.into(),
        }
    }

    pub(super) fn initialize_value(&self, repo: &str, requested_root: &Path) -> Value {
        match self {
            Self::Verified {
                source,
                requested_root: bound_request,
                bound_root,
                repo_identity,
                head,
            } => json!({
                "status": "verified",
                "source": source,
                "repo": repo,
                "requestedPath": bound_request,
                "boundPath": bound_root,
                "repoIdentity": repo_identity,
                "head": head,
            }),
            Self::Degraded { reason } => json!({
                "status": "degraded",
                "repo": repo,
                "requestedPath": requested_root,
                "reason": reason,
                "next": "run code-intel against this checkout before trusting MCP evidence",
            }),
        }
    }

    pub(super) fn require_verified(&self) -> Result<(), String> {
        match self {
            Self::Verified { .. } => Ok(()),
            Self::Degraded { reason } => Err(format!(
                "degraded MCP repository binding: {reason}; no committed evidence is safe to serve"
            )),
        }
    }
}

#[derive(Debug)]
struct GitIdentity {
    root: PathBuf,
    common_checkout: PathBuf,
    repo_identity: String,
    head: String,
}

pub(super) fn bind(
    artifact_root: &Path,
    repo: &str,
    requested_root: &Path,
) -> Result<RepositoryBinding, String> {
    if !artifact_root.is_dir() {
        return Ok(RepositoryBinding::degraded(format!(
            "artifact root is not present: {}",
            artifact_root.display()
        )));
    }
    let evidence = match committed_evidence::load(artifact_root, repo) {
        Ok(evidence) => evidence,
        Err(EvidenceError::Contract(message))
            if message.contains("no committed authoritative run") =>
        {
            return Ok(RepositoryBinding::degraded(message));
        }
        Err(EvidenceError::Contract(message) | EvidenceError::HostIo(message)) => {
            return Err(format!(
                "MCP repository binding cannot load the qualified run for {repo}: {message}"
            ));
        }
    };

    let expected = evidence
        .artifact("repository.snapshot")
        .ok_or_else(|| "MCP repository binding requires repository.snapshot evidence".to_string())?
        .1;
    let expected: Value = serde_json::from_slice(expected.bytes())
        .map_err(|error| format!("MCP repository snapshot is invalid JSON: {error}"))?;
    let expected_identity = expected["snapshot"]["repoIdentity"]
        .as_str()
        .ok_or_else(|| "MCP repository snapshot omits snapshot.repoIdentity".to_string())?;
    let expected_head = expected["snapshot"]["head"]
        .as_str()
        .ok_or_else(|| "MCP repository snapshot omits snapshot.head".to_string())?;

    let requested = git_identity(requested_root).map_err(|error| {
        format!(
            "MCP repository identity cannot be inspected for {}: {error}",
            requested_root.display()
        )
    })?;
    let Some(requested) = requested else {
        return Err(format!(
            "MCP repository identity unavailable for {}; refusing to serve indexed evidence from {repo}",
            requested_root.display()
        ));
    };
    if matches_expected(&requested, expected_identity, expected_head) {
        return Ok(RepositoryBinding::Verified {
            source: "requested_worktree",
            requested_root: requested.root.clone(),
            bound_root: requested.root,
            repo_identity: requested.repo_identity,
            head: requested.head,
        });
    }

    if requested.common_checkout != requested.root {
        if let Some(common) = git_identity(&requested.common_checkout).map_err(|error| {
            format!(
                "MCP common checkout identity cannot be inspected for {}: {error}",
                requested.common_checkout.display()
            )
        })? {
            if matches_expected(&common, expected_identity, expected_head) {
                return Ok(RepositoryBinding::Verified {
                    source: "common_checkout_fallback",
                    requested_root: requested.root,
                    bound_root: common.root,
                    repo_identity: common.repo_identity,
                    head: common.head,
                });
            }
        }
    }

    Err(format!(
        "MCP repository identity mismatch; refusing to serve unrelated evidence: repo={repo}, \
         indexed repoIdentity={expected_identity}, indexed HEAD={expected_head}, \
         requested root={}, requested repoIdentity={}, requested HEAD={}, common checkout={}",
        requested.root.display(),
        requested.repo_identity,
        requested.head,
        requested.common_checkout.display(),
    ))
}

fn matches_expected(actual: &GitIdentity, expected_identity: &str, expected_head: &str) -> bool {
    actual.repo_identity == expected_identity && actual.head == expected_head
}

fn git_identity(root: &Path) -> Result<Option<GitIdentity>, String> {
    let Some((repo_identity, head)) = snapshot::git_repository_identity(root)? else {
        return Ok(None);
    };
    let top = git_text(root, &["rev-parse", "--show-toplevel"])?;
    let root = fs::canonicalize(top.trim())
        .map_err(|error| format!("canonicalize Git worktree root: {error}"))?;
    let common_dir = git_text(&root, &["rev-parse", "--git-common-dir"])?;
    let common_dir = PathBuf::from(common_dir.trim());
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        root.join(common_dir)
    };
    let common_dir = fs::canonicalize(&common_dir)
        .map_err(|error| format!("canonicalize Git common dir: {error}"))?;
    let common_checkout = common_dir
        .parent()
        .map(fs::canonicalize)
        .transpose()
        .map_err(|error| format!("canonicalize Git common checkout: {error}"))?
        .unwrap_or_else(|| root.clone());
    Ok(Some(GitIdentity {
        root,
        common_checkout,
        repo_identity,
        head,
    }))
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = crate::hardened_git::command(root)
        .args(args)
        .output()
        .map_err(|error| format!("launch git {:?}: {error}", args))?;
    if !output.status.success() {
        return Err(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("git {:?} emitted non-UTF-8: {error}", args))
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
