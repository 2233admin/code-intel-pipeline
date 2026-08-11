use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::ProjectError;
use crate::artifact_index::{self, IndexError};
use crate::committed_evidence::{self, EvidenceError};
use crate::snapshot;

mod git;

use git::{git_identity, matches_expected};

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
    pub(crate) fn degraded(reason: impl Into<String>) -> Self {
        Self::Degraded {
            reason: reason.into(),
        }
    }

    pub(crate) fn describe(&self, repo: &str, requested_root: &Path) -> Value {
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
                "next": "run code-intel against this checkout before trusting committed evidence",
            }),
        }
    }

    pub(crate) fn require_verified(&self) -> Result<(), ProjectError> {
        match self {
            Self::Verified { .. } => Ok(()),
            Self::Degraded { reason } => Err(ProjectError::contract(format!(
                "degraded repository binding: {reason}; no committed evidence is safe to serve"
            ))),
        }
    }
}

#[derive(Debug)]
struct RecordedIdentity {
    repo_identity: String,
    head: String,
    working_tree_policy: String,
    scopes: Vec<String>,
}

#[derive(Debug)]
struct ContentIdentity {
    root: PathBuf,
    repo_identity: String,
    head: String,
}

pub(super) fn resolve_repository_key(
    artifact_root: &Path,
    preferred: &str,
    requested_root: &Path,
) -> Result<String, ProjectError> {
    if !artifact_root.is_dir() {
        return Ok(preferred.to_string());
    }
    let requested_git = git_identity(requested_root)?;
    let index = artifact_index::rebuild(artifact_root).map_err(map_index_error)?;
    let repos = index["entries"]
        .as_array()
        .expect("A08 index entries")
        .iter()
        .filter_map(|entry| entry["repo"].as_str())
        .collect::<Vec<_>>();
    if repos.is_empty() {
        return Ok(preferred.to_string());
    }

    let mut preferred_exists = false;
    let mut matches = Vec::new();
    for repo in repos {
        preferred_exists |= repo == preferred;
        let evidence = committed_evidence::load(artifact_root, repo).map_err(map_evidence_error)?;
        let recorded = recorded_identity(&evidence)?;
        let matched = if is_git_identity(&recorded.repo_identity) {
            requested_git
                .as_ref()
                .is_some_and(|requested| requested.repo_identity == recorded.repo_identity)
        } else if is_content_identity(&recorded.repo_identity) && requested_git.is_none() {
            let requested = content_identity(requested_root, &recorded)?;
            matches_recorded_content(&requested, &recorded)
        } else {
            false
        };
        if matched {
            matches.push(repo.to_string());
        }
    }
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [repo] => Ok(repo.clone()),
        [] if preferred_exists => Err(ProjectError::contract(format!(
            "repository key collision: {preferred} is already bound to another repository identity"
        ))),
        [] => Ok(preferred.to_string()),
        repos => Err(ProjectError::contract(format!(
            "ambiguous repository key: the requested checkout matches committed identities in {}",
            repos.join(", "),
        ))),
    }
}

pub(super) fn ensure_run_authority(
    artifact_root: &Path,
    repo: &str,
    requested_root: &Path,
) -> Result<(), ProjectError> {
    if !artifact_root.is_dir() {
        return Ok(());
    }
    let evidence = match committed_evidence::load(artifact_root, repo) {
        Ok(evidence) => evidence,
        Err(EvidenceError::Contract(message))
            if message.contains("no committed authoritative run") =>
        {
            return Ok(())
        }
        Err(error) => return Err(map_evidence_error(error)),
    };
    let recorded = recorded_identity(&evidence)?;
    if is_git_identity(&recorded.repo_identity) {
        let requested = git_identity(requested_root)?.ok_or_else(|| {
            ProjectError::contract(format!(
                "repository identity unavailable for {}; authority key {repo} requires Git lineage {}",
                requested_root.display(),
                recorded.repo_identity,
            ))
        })?;
        if recorded.repo_identity != requested.repo_identity {
            return Err(ProjectError::contract(format!(
                "repository key collision: {repo} is bound to Git lineage {}, requested {}",
                recorded.repo_identity, requested.repo_identity
            )));
        }
        return Ok(());
    }
    if !is_content_identity(&recorded.repo_identity) {
        return Err(ProjectError::contract(format!(
            "repository key {repo} uses unsupported repository identity {}",
            recorded.repo_identity,
        )));
    }
    let requested = content_identity(requested_root, &recorded)?;
    if matches_recorded_content(&requested, &recorded) {
        return Ok(());
    }
    Err(ProjectError::contract(format!(
        "repository key collision: {repo} is bound to content identity {} at {}, requested {}; \
         content-based authority cannot prove continuity after inputs change, so initialize Git \
         or use a distinct artifact root",
        recorded.repo_identity, recorded.head, requested.repo_identity,
    )))
}

pub(super) fn bind_repository(
    artifact_root: &Path,
    repo: &str,
    requested_root: &Path,
) -> Result<RepositoryBinding, ProjectError> {
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
        Err(EvidenceError::Contract(message)) => return Err(ProjectError::contract(message)),
        Err(EvidenceError::HostIo(message)) => return Err(ProjectError::host_io(message)),
    };

    bind_evidence(&evidence, repo, requested_root)
}

pub(super) fn bind_evidence(
    evidence: &committed_evidence::CommittedEvidence,
    repo: &str,
    requested_root: &Path,
) -> Result<RepositoryBinding, ProjectError> {
    let expected = recorded_identity(evidence)?;

    if is_content_identity(&expected.repo_identity) {
        let requested = content_identity(requested_root, &expected)?;
        if matches_recorded_content(&requested, &expected) {
            return Ok(RepositoryBinding::Verified {
                source: "requested_content_snapshot",
                requested_root: requested.root.clone(),
                bound_root: requested.root,
                repo_identity: requested.repo_identity,
                head: requested.head,
            });
        }
        return Err(ProjectError::contract(format!(
            "repository identity mismatch; refusing unrelated evidence: repo={repo}, \
             indexed repoIdentity={}, indexed HEAD={}, requested root={}, \
             requested repoIdentity={}, requested HEAD={} ",
            expected.repo_identity,
            expected.head,
            requested.root.display(),
            requested.repo_identity,
            requested.head,
        )));
    }
    if !is_git_identity(&expected.repo_identity) {
        return Err(ProjectError::contract(format!(
            "repository binding does not support repository identity {}",
            expected.repo_identity,
        )));
    }

    let requested = git_identity(requested_root)?;
    let Some(requested) = requested else {
        return Err(ProjectError::contract(format!(
            "repository identity unavailable for {}; refusing indexed evidence from {repo}",
            requested_root.display()
        )));
    };
    if matches_expected(&requested, &expected.repo_identity, &expected.head) {
        return Ok(RepositoryBinding::Verified {
            source: "requested_worktree",
            requested_root: requested.root.clone(),
            bound_root: requested.root,
            repo_identity: requested.repo_identity,
            head: requested.head,
        });
    }

    Err(ProjectError::contract(format!(
        "repository identity mismatch; refusing unrelated evidence: repo={repo}, \
         indexed repoIdentity={}, indexed HEAD={}, \
         requested root={}, requested repoIdentity={}, requested HEAD={}, common checkout={}",
        expected.repo_identity,
        expected.head,
        requested.root.display(),
        requested.repo_identity,
        requested.head,
        requested.common_checkout.display(),
    )))
}

fn recorded_identity(
    evidence: &committed_evidence::CommittedEvidence,
) -> Result<RecordedIdentity, ProjectError> {
    let expected = evidence
        .artifact("repository.snapshot")
        .ok_or_else(|| {
            ProjectError::contract("repository binding requires repository.snapshot evidence")
        })?
        .1;
    let expected: Value = serde_json::from_slice(expected.bytes()).map_err(|error| {
        ProjectError::contract(format!("repository snapshot is invalid JSON: {error}"))
    })?;
    let repo_identity = expected["snapshot"]["repoIdentity"]
        .as_str()
        .ok_or_else(|| ProjectError::contract("repository snapshot omits snapshot.repoIdentity"))?
        .to_string();
    let head = expected["snapshot"]["head"]
        .as_str()
        .ok_or_else(|| ProjectError::contract("repository snapshot omits snapshot.head"))?
        .to_string();
    let working_tree_policy = expected["snapshot"]["workingTreePolicy"]
        .as_str()
        .ok_or_else(|| {
            ProjectError::contract("repository snapshot omits snapshot.workingTreePolicy")
        })?
        .to_string();
    let scopes = expected["snapshot"]["scope"]
        .as_array()
        .ok_or_else(|| ProjectError::contract("repository snapshot omits snapshot.scope"))?
        .iter()
        .map(|scope| {
            scope.as_str().map(str::to_string).ok_or_else(|| {
                ProjectError::contract("repository snapshot scope contains a non-string")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RecordedIdentity {
        repo_identity,
        head,
        working_tree_policy,
        scopes,
    })
}

fn is_git_identity(identity: &str) -> bool {
    identity.starts_with("git-lineage-v1:")
}

fn is_content_identity(identity: &str) -> bool {
    identity.starts_with("content-v1:")
}

fn matches_recorded_content(actual: &ContentIdentity, expected: &RecordedIdentity) -> bool {
    actual.repo_identity == expected.repo_identity && actual.head == expected.head
}

fn content_identity(
    root: &Path,
    expected: &RecordedIdentity,
) -> Result<ContentIdentity, ProjectError> {
    let current = snapshot::build_for_dag(root, &expected.working_tree_policy, &expected.scopes)
        .map_err(|error| {
            ProjectError::contract(format!("evaluate repository identity: {error}"))
        })?;
    let repo_identity = current["snapshot"]["repoIdentity"]
        .as_str()
        .ok_or_else(|| ProjectError::contract("current snapshot omits snapshot.repoIdentity"))?
        .to_string();
    let head = current["snapshot"]["head"]
        .as_str()
        .ok_or_else(|| ProjectError::contract("current snapshot omits snapshot.head"))?
        .to_string();
    let root = fs::canonicalize(root).map_err(|error| {
        ProjectError::host_io(format!("canonicalize content repository root: {error}"))
    })?;
    Ok(ContentIdentity {
        root,
        repo_identity,
        head,
    })
}

fn map_evidence_error(error: EvidenceError) -> ProjectError {
    match error {
        EvidenceError::Contract(message) => ProjectError::contract(message),
        EvidenceError::HostIo(message) => ProjectError::host_io(message),
    }
}

fn map_index_error(error: IndexError) -> ProjectError {
    match error {
        IndexError::Contract(message) => ProjectError::contract(message),
        IndexError::HostIo(message) => ProjectError::host_io(message),
    }
}
