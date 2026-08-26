use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::committed_evidence::CommittedEvidence;
use crate::committed_evidence_controller::{CommittedAuthority, CommittedEvidenceController};
use crate::evidence_query::{EvidenceQueryRequest, MAX_LIMIT};
use crate::{artifacts, authoritative_run, execution_policy};

mod error;
mod identity;
mod status;
#[cfg(test)]
mod tests;

pub(crate) use error::ProjectError;
pub(crate) use identity::RepositoryBinding;
use identity::{bind_evidence, bind_repository, ensure_run_authority, resolve_repository_key};

const DEFAULT_QUERY_LIMIT: usize = 20;

fn validate_repository_key(repo: &str) -> Result<(), ProjectError> {
    if repo.trim().is_empty() || repo == "." || repo == ".." || repo.contains(['/', '\\', '\0']) {
        return Err(ProjectError::usage(
            "repository key must be one non-empty path component",
        ));
    }
    Ok(())
}

static RUN_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct RunAllocation {
    final_name: String,
    staging_root: PathBuf,
}

/// #352: `run()`'s identity used to be `millis + pid`, nothing else -- both
/// the informational `RunRequest` name *and* the staging directory derived
/// from it were built off that single colliding value, so two concurrent
/// `run()` calls whose clock reads landed in the same millisecond (a much
/// coarser window than nanoseconds, and therefore a much likelier
/// collision) would be handed the identical staging root *and* publish
/// under the identical name. `clock` is a parameter, not a direct
/// `SystemTime::now()` call, so a test can force that exact condition. The
/// clock reading is kept only so identities stay sortable by age;
/// `RUN_SEQUENCE` is what actually buys uniqueness. Both fields are
/// computed here, together, from one identity -- `run()` cannot build them
/// independently and let them drift apart. `validate_final_name`
/// (`run_commit.rs`) treats `final_name` as opaque -- no shape but
/// non-empty/no-separators/no-leading-dot/no-"staging"-substring -- so
/// widening the format here does not break that contract.
fn allocate_run_identity(
    pid: u32,
    clock: impl FnOnce() -> Result<u128, ProjectError>,
) -> Result<RunAllocation, ProjectError> {
    let nonce = clock()?;
    let sequence = RUN_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let final_name = format!("{nonce}-{pid}-{sequence}-core");
    let staging_root = std::env::temp_dir().join(format!("code-intel-a09-{final_name}"));
    Ok(RunAllocation {
        final_name,
        staging_root,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectSelector {
    repo_path: PathBuf,
    repo: Option<String>,
    artifact_root: Option<PathBuf>,
}

impl ProjectSelector {
    pub(crate) fn new(repo_path: PathBuf) -> Self {
        Self {
            repo_path,
            repo: None,
            artifact_root: None,
        }
    }

    pub(crate) fn with_repo(mut self, repo: Option<String>) -> Self {
        self.repo = repo;
        self
    }

    pub(crate) fn with_artifact_root(mut self, artifact_root: Option<PathBuf>) -> Self {
        self.artifact_root = artifact_root;
        self
    }
}

#[derive(Debug)]
pub(crate) struct ProjectContext {
    repo_path: PathBuf,
    repo: String,
    artifact_root: PathBuf,
}

impl ProjectContext {
    #[cfg(test)]
    pub(crate) fn for_test(repo_path: PathBuf, repo: String, artifact_root: PathBuf) -> Self {
        Self {
            repo_path,
            repo,
            artifact_root,
        }
    }

    pub(crate) fn resolve(selector: ProjectSelector) -> Result<Self, ProjectError> {
        if !selector.repo_path.is_dir() {
            return Err(ProjectError::usage(format!(
                "repository path is not a directory: {}",
                selector.repo_path.display()
            )));
        }
        let repo_path = fs::canonicalize(&selector.repo_path).map_err(|error| {
            ProjectError::host_io(format!(
                "resolve repository path {}: {error}",
                selector.repo_path.display()
            ))
        })?;
        let artifact_root = artifacts::resolve_artifact_root(selector.artifact_root.as_deref())
            .map_err(|error| ProjectError::host_io(format!("resolve artifact root: {error}")))?;
        let preferred = repo_path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ProjectError::usage("repository path has no usable directory name"))?;
        let repo = match selector.repo {
            Some(repo) => repo,
            None => resolve_repository_key(&artifact_root, preferred, &repo_path)?,
        };
        validate_repository_key(&repo)?;
        Ok(Self {
            repo_path,
            repo,
            artifact_root,
        })
    }

    pub(crate) fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    pub(crate) fn repo(&self) -> &str {
        &self.repo
    }

    pub(crate) fn artifact_root(&self) -> &Path {
        &self.artifact_root
    }

    pub(crate) fn binding(&self) -> Result<RepositoryBinding, ProjectError> {
        bind_repository(&self.artifact_root, &self.repo, &self.repo_path)
    }

    pub(crate) fn verify_evidence(
        &self,
        evidence: &CommittedEvidence,
    ) -> Result<RepositoryBinding, ProjectError> {
        bind_evidence(evidence, &self.repo, &self.repo_path)
    }

    pub(crate) fn run(&self, intent: RunIntent) -> Result<RunAnswer, ProjectError> {
        ensure_run_authority(&self.artifact_root, &self.repo, &self.repo_path)?;
        let artifact_root = artifacts::ensure_directory(&self.artifact_root).map_err(|error| {
            ProjectError::host_io(format!(
                "create artifact root {}: {error}",
                self.artifact_root.display()
            ))
        })?;
        let authority_root = artifact_root.join(&self.repo);
        let authority_root = artifacts::ensure_directory(&authority_root).map_err(|error| {
            ProjectError::host_io(format!(
                "create repository authority root {}: {error}",
                authority_root.display()
            ))
        })?;
        let allocation = allocate_run_identity(std::process::id(), || {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .map_err(|error| ProjectError::host_io(error.to_string()))
        })?;
        let request = authoritative_run::RunRequest::new(
            self.repo_path.clone(),
            allocation.staging_root,
            authority_root,
            allocation.final_name,
            execution_policy::ExecutionPolicy::for_profile(intent.mode.profile()),
        )
        .with_repository_key(self.repo.clone())
        .with_staging_cleanup();
        let result = authoritative_run::execute(request).map_err(ProjectError::from_run)?;
        let value = run_result(self, intent.mode, &result);
        Ok(RunAnswer {
            exit_code: result.exit_code(),
            value,
        })
    }

    pub(crate) fn query(&self, query: Query) -> Result<QueryAnswer, ProjectError> {
        match query {
            Query::Evidence(query) => {
                let request = EvidenceQueryRequest::new(
                    self.artifact_root.clone(),
                    self.repo.clone(),
                    Some(self.repo_path.clone()),
                    query.artifact_schema,
                    query.artifact_type,
                    query.contains,
                    query.limit,
                )
                .map_err(ProjectError::from)?;
                let (evidence, authority) =
                    CommittedEvidenceController::open(&self.artifact_root, &self.repo)
                        .map_err(ProjectError::from)?;
                self.verify_evidence(&evidence)?.require_verified()?;
                let result =
                    CommittedEvidenceController::query_opened(request, evidence, authority)
                        .map_err(ProjectError::from)?;
                Ok(QueryAnswer {
                    value: result.value().clone(),
                    authority: result.authority().clone(),
                })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunMode {
    Lite,
    Normal,
    Full,
}

impl RunMode {
    pub(crate) fn parse(value: &str) -> Result<Self, ProjectError> {
        match value {
            "lite" => Ok(Self::Lite),
            "normal" => Ok(Self::Normal),
            "full" => Ok(Self::Full),
            _ => Err(ProjectError::usage("--mode must be lite, normal, or full")),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Lite => "lite",
            Self::Normal => "normal",
            Self::Full => "full",
        }
    }

    fn profile(self) -> execution_policy::RunProfile {
        match self {
            Self::Lite => execution_policy::RunProfile::Offline,
            Self::Normal => execution_policy::RunProfile::Default,
            Self::Full => execution_policy::RunProfile::Strict,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RunIntent {
    mode: RunMode,
}

impl RunIntent {
    pub(crate) fn new(mode: RunMode) -> Self {
        Self { mode }
    }
}

pub(crate) struct RunAnswer {
    exit_code: i32,
    value: Value,
}

impl RunAnswer {
    pub(crate) fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub(crate) fn value(&self) -> &Value {
        &self.value
    }
}

#[derive(Debug)]
pub(crate) enum Query {
    Evidence(EvidenceQuery),
}

#[derive(Debug)]
pub(crate) struct EvidenceQuery {
    artifact_schema: Option<String>,
    artifact_type: Option<String>,
    contains: Option<String>,
    limit: usize,
}

impl EvidenceQuery {
    pub(crate) fn new(
        artifact_schema: Option<String>,
        artifact_type: Option<String>,
        contains: Option<String>,
        limit: Option<usize>,
    ) -> Result<Self, ProjectError> {
        let limit = limit.unwrap_or(DEFAULT_QUERY_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(ProjectError::usage("--limit must be an integer in 1..=100"));
        }
        Ok(Self {
            artifact_schema,
            artifact_type,
            contains,
            limit,
        })
    }
}

pub(crate) struct QueryAnswer {
    value: Value,
    authority: CommittedAuthority,
}

impl QueryAnswer {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    #[allow(dead_code)]
    pub(crate) fn authority(&self) -> &CommittedAuthority {
        &self.authority
    }
}

fn run_result(
    context: &ProjectContext,
    mode: RunMode,
    result: &authoritative_run::ProductionRunResult,
) -> Value {
    let (failure_node, diagnostic) = first_failure(result.manifest())
        .map(|(node, diagnostic)| (Value::String(node), Value::String(diagnostic)))
        .unwrap_or((Value::Null, Value::Null));
    json!({
        "schema": "code-intel-primary-result.v1",
        "repo": context.repo_path,
        "mode": mode.as_str(),
        "outcome": result.outcome(),
        "exitCode": result.exit_code(),
        "publication": {
            "path": result.publication_path(),
            "marker": result.publication_path().join("run-complete.json"),
        },
        "readableArtifacts": readable_artifact_paths(result.manifest(), result.publication_path()),
        "failureNode": failure_node,
        "diagnostic": diagnostic,
        "failures": result.failures(),
        "anchors": result.anchors(),
    })
}

fn readable_artifact_paths(manifest: &Value, publication_root: &Path) -> Value {
    let mut readable = serde_json::Map::new();
    let artifact_names = [
        ("diagnosis.hospital", "hospital"),
        ("diagnosis.hospital-view", "hospitalMarkdown"),
        ("code_evidence.agent_slice", "agentCodeSliceRanking"),
    ];
    for node in manifest["nodes"]
        .as_object()
        .into_iter()
        .flat_map(|nodes| nodes.values())
    {
        for artifact in node["artifacts"].as_array().into_iter().flatten() {
            let Some(artifact_type) = artifact["type"].as_str() else {
                continue;
            };
            let Some((_, name)) = artifact_names
                .iter()
                .find(|(candidate, _)| *candidate == artifact_type)
            else {
                continue;
            };
            let Some(path) = artifact["path"].as_str() else {
                continue;
            };
            readable.entry(*name).or_insert_with(|| {
                Value::String(publication_root.join(path).display().to_string())
            });
        }
    }
    Value::Object(readable)
}

fn first_failure(manifest: &Value) -> Option<(String, String)> {
    manifest["nodes"]
        .as_object()?
        .iter()
        .find_map(|(node, value)| {
            matches!(
                value["status"].as_str(),
                Some("process_failed" | "domain_failed" | "domain_unknown")
            )
            .then(|| {
                (
                    node.clone(),
                    value["diagnostic"]
                        .as_str()
                        .or_else(|| value["failure"].as_str())
                        .unwrap_or("")
                        .to_string(),
                )
            })
        })
}
