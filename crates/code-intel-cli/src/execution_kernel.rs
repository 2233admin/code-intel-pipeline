use std::path::PathBuf;

use serde_json::{json, Value};

use crate::dag_coordinator::RunOutcome;
use crate::dag_run::{self, DagExecutionRequest};
use crate::execution_policy::ExecutionPolicy;
use crate::run_commit;
use crate::run_error::RunError;

pub(crate) struct RunRequest {
    pub(crate) repo: PathBuf,
    pub(crate) staging_root: PathBuf,
    pub(crate) authority_root: PathBuf,
    pub(crate) final_name: String,
    pub(crate) manifest: Option<PathBuf>,
    pub(crate) max_concurrency: usize,
    pub(crate) policy: ExecutionPolicy,
    pub(crate) session_evidence: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Publication {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
}

impl Publication {
    fn to_json(&self) -> Value {
        json!({
            "status":"committed",
            "name":self.name,
            "path":self.path,
            "marker":"run-complete.json",
        })
    }
}

pub(crate) struct ExecutionResult {
    pub(crate) outcome: RunOutcome,
    pub(crate) manifest: Value,
    pub(crate) publication: Publication,
}

impl ExecutionResult {
    pub(crate) fn exit_code(&self) -> i32 {
        self.outcome.exit_code()
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "schema":"code-intel-execution-result.v1",
            "outcome":self.outcome.as_str(),
            "exitCode":self.exit_code(),
            "failures":failures(&self.manifest),
            "manifest":self.manifest,
            "publication":self.publication.to_json(),
        })
    }
}

/// Split node failures by class so a process hiccup (tooling, environment)
/// can never visually swallow a domain verdict such as a failed architecture
/// gate — the run outcome keeps ProcessFailed precedence, but both lists are
/// always reported side by side.
pub(crate) fn failures(manifest: &Value) -> Value {
    let mut process = Vec::new();
    let mut domain = Vec::new();
    if let Some(nodes) = manifest["nodes"].as_object() {
        for (node, state) in nodes {
            match state["status"].as_str() {
                Some("process_failed") => process.push(json!({
                    "node": node,
                    "diagnostic": state["diagnostic"].as_str().unwrap_or(""),
                })),
                Some("domain_failed") => domain.push(json!({
                    "node": node,
                    "verdict": state["verdict"].as_str().unwrap_or("fail"),
                })),
                _ => {}
            }
        }
    }
    json!({"process": process, "domain": domain})
}

pub(crate) fn execute(request: RunRequest) -> Result<ExecutionResult, RunError> {
    let dag = dag_run::execute_dag(DagExecutionRequest {
        repo: request.repo,
        out: request.staging_root,
        manifest: request.manifest,
        max_concurrency: request.max_concurrency,
        policy: request.policy,
        diagnosis_inputs: None,
        seed_artifact_root: None,
        session_evidence: request.session_evidence,
    })?;
    let publication = run_commit::publish_existing(
        &dag.run_root,
        &request.authority_root,
        &dag.run_root.join("run-manifest-ref.json"),
        &request.final_name,
    )
    .map_err(map_commit_error)?;
    Ok(ExecutionResult {
        outcome: dag.outcome,
        manifest: dag.manifest,
        publication: Publication {
            name: request.final_name,
            path: publication.final_path,
        },
    })
}

fn map_commit_error(error: run_commit::CommitError) -> RunError {
    match error {
        run_commit::CommitError::Contract(message)
        | run_commit::CommitError::Collision(message) => RunError::contract(message),
        run_commit::CommitError::HostIo(message) => RunError::io(message),
        run_commit::CommitError::Interrupted(phase) => RunError {
            exit_code: 75,
            message: format!("publication interrupted before {phase:?}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_result_owns_outcome_exit_and_publication_serialization() {
        let result = ExecutionResult {
            outcome: RunOutcome::DomainUnknown,
            manifest: json!({"outcome":"domain_unknown"}),
            publication: Publication {
                name: "run-001".into(),
                path: PathBuf::from("authority/run-001"),
            },
        };

        assert_eq!(result.exit_code(), 20);
        let json = result.to_json();
        assert_eq!(json["outcome"], "domain_unknown");
        assert_eq!(json["exitCode"], 20);
        assert_eq!(json["manifest"]["outcome"], json["outcome"]);
        assert_eq!(json["publication"]["status"], "committed");
        assert_eq!(json["publication"]["marker"], "run-complete.json");
    }
}
