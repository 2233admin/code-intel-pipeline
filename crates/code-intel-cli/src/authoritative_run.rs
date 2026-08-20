mod completion;
mod execution_kernel;

pub(crate) struct RunRequest {
    repo: std::path::PathBuf,
    repository_key: Option<String>,
    staging_root: std::path::PathBuf,
    authority_root: std::path::PathBuf,
    final_name: String,
    manifest: Option<std::path::PathBuf>,
    max_concurrency: usize,
    budget: crate::budget::Budget,
    node_timeout: std::time::Duration,
    policy: crate::execution_policy::ExecutionPolicy,
    session_evidence: Option<std::path::PathBuf>,
    cleanup_staging: bool,
}

impl RunRequest {
    pub(crate) fn new(
        repo: std::path::PathBuf,
        staging_root: std::path::PathBuf,
        authority_root: std::path::PathBuf,
        final_name: String,
        policy: crate::execution_policy::ExecutionPolicy,
    ) -> Self {
        Self {
            repo,
            repository_key: None,
            staging_root,
            authority_root,
            final_name,
            manifest: None,
            max_concurrency: 2,
            node_timeout: std::time::Duration::from_secs(
                crate::node_timeout::DEFAULT_NODE_TIMEOUT_SECONDS,
            ),
            budget: crate::budget::Budget::with_defaults(),
            policy,
            session_evidence: None,
            cleanup_staging: false,
        }
    }

    pub(crate) fn with_repository_key(mut self, repository_key: String) -> Self {
        self.repository_key = Some(repository_key);
        self
    }

    pub(crate) fn with_manifest(mut self, manifest: Option<std::path::PathBuf>) -> Self {
        self.manifest = manifest;
        self
    }

    pub(crate) fn with_max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.max_concurrency = max_concurrency;
        self
    }

    pub(crate) fn with_budget(mut self, budget: crate::budget::Budget) -> Self {
        self.budget = budget;
        self
    }
    pub(crate) fn with_node_timeout(mut self, node_timeout: std::time::Duration) -> Self {
        self.node_timeout = node_timeout;
        self
    }

    pub(crate) fn with_session_evidence(
        mut self,
        session_evidence: Option<std::path::PathBuf>,
    ) -> Self {
        self.session_evidence = session_evidence;
        self
    }

    pub(crate) fn with_staging_cleanup(mut self) -> Self {
        self.cleanup_staging = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Publication {
    name: String,
    repo: String,
    path: std::path::PathBuf,
}

pub(crate) struct ProductionRunResult {
    outcome: crate::dag_coordinator::RunOutcome,
    manifest: serde_json::Value,
    publication: Publication,
    anchors: crate::anchor_verification::AnchorCounts,
}

impl ProductionRunResult {
    pub(crate) fn exit_code(&self) -> i32 {
        self.outcome.exit_code()
    }

    pub(crate) fn outcome(&self) -> &'static str {
        self.outcome.as_str()
    }

    pub(crate) fn manifest(&self) -> &serde_json::Value {
        &self.manifest
    }

    pub(crate) fn publication_path(&self) -> &std::path::Path {
        &self.publication.path
    }

    pub(crate) fn failures(&self) -> serde_json::Value {
        execution_kernel::failures(&self.manifest)
    }

    /// Issue #151: the aggregate `{verified, approximate, dropped}` anchor
    /// counts for this run, so callers other than `to_execution_json` (e.g.
    /// the `primary` entry point) can surface `dropped > 0` without parsing
    /// it back out of the manifest.
    pub(crate) fn anchors(&self) -> serde_json::Value {
        self.anchors.to_json()
    }

    pub(crate) fn to_execution_json(&self) -> serde_json::Value {
        serde_json::json!({
            "schema":"code-intel-execution-result.v1",
            "outcome":self.outcome(),
            "exitCode":self.exit_code(),
            "failures":self.failures(),
            "manifest":self.manifest,
            "publication":{
                "status":"committed",
                "name":self.publication.name,
                "repo":self.publication.repo,
                "path":self.publication.path,
                "marker":"run-complete.json",
            },
            "anchors":self.anchors(),
        })
    }
}

/// Performs exactly one authoritative repository-intelligence iteration.
///
/// The controller is the only production seam allowed to construct the
/// kernel request. The kernel owns snapshot -> DAG -> manifest -> marker-last
/// publication; this controller revalidates that committed handoff before it
/// refreshes the completed-only read index.
pub(crate) fn execute(
    request: RunRequest,
) -> Result<ProductionRunResult, crate::run_error::RunError> {
    let staging_root = request.staging_root.clone();
    let cleanup_staging = request.cleanup_staging;
    let result = execution_kernel::execute(execution_kernel::RunRequest {
        repo: request.repo,
        repository_key: request.repository_key,
        staging_root: request.staging_root,
        authority_root: request.authority_root,
        final_name: request.final_name,
        manifest: request.manifest,
        max_concurrency: request.max_concurrency,
        budget: request.budget,
        node_timeout: request.node_timeout,
        policy: request.policy,
        session_evidence: request.session_evidence,
    })?;

    completion::admit(result.outcome, &result.publication)?;

    if cleanup_staging && staging_root.is_dir() {
        std::fs::remove_dir_all(&staging_root).map_err(|error| {
            crate::run_error::RunError::io(format!("remove committed staging directory: {error}"))
        })?;
    }
    Ok(ProductionRunResult {
        outcome: result.outcome,
        manifest: result.manifest,
        publication: Publication {
            name: result.publication.name,
            repo: result.publication.repo,
            path: result.publication.path,
        },
        anchors: result.anchors,
    })
}

#[cfg(test)]
mod tests {
    const CONTROLLER: &str = include_str!("authoritative_run.rs");
    const COMPLETION: &str = include_str!("authoritative_run/completion.rs");
    const PRIMARY: &str = include_str!("cli/primary.rs");
    const RUN_COMPATIBILITY: &str = include_str!("run_cli.rs");

    #[test]
    fn production_spellings_have_one_kernel_builder_and_no_index_side_path() {
        let kernel_call = ["execution_kernel", "::execute"].concat();
        let index_rebuild = ["artifact_index", "::rebuild"].concat();
        let index_write = ["artifact_index", "::write_index"].concat();

        assert_eq!(CONTROLLER.matches(&kernel_call).count(), 1);
        assert_eq!(COMPLETION.matches(&index_rebuild).count(), 1);
        assert_eq!(COMPLETION.matches(&index_write).count(), 1);
        assert_eq!(PRIMARY.matches(&kernel_call).count(), 0);
        assert_eq!(RUN_COMPATIBILITY.matches(&kernel_call).count(), 0);
        assert_eq!(PRIMARY.matches(&index_rebuild).count(), 0);
        assert_eq!(PRIMARY.matches(&index_write).count(), 0);
    }

    #[test]
    fn completed_publication_must_be_the_run_selected_by_the_index() {
        let current = serde_json::json!({"entries":[{"repo":"repo-a","run":"run-002"}]});
        assert!(super::completion::require_completed_latest(&current, "repo-a", "run-002").is_ok());

        let error =
            super::completion::require_completed_latest(&current, "repo-a", "run-001").unwrap_err();
        assert_eq!(error.exit_code, 65);
        assert!(error.message.contains("selected run-002"));
        assert!(super::completion::require_completed_latest(
            &serde_json::json!({"entries":[]}),
            "repo-a",
            "run-001"
        )
        .is_err());
    }
}
