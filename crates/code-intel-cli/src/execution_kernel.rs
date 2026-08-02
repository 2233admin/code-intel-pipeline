use std::fs;
use std::path::{Path, PathBuf};

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
    pub(crate) repo: String,
    pub(crate) path: PathBuf,
}

impl Publication {
    fn to_json(&self) -> Value {
        json!({
            "status":"committed",
            "name":self.name,
            "repo":self.repo,
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
                // An unknown domain verdict is a reportable failure (run
                // outcome domain_unknown, exit 20) — never an empty report.
                Some("domain_unknown") => domain.push(json!({
                    "node": node,
                    "verdict": "unknown",
                })),
                Some("succeeded") if state["verdict"] == "unknown" => domain.push(json!({
                    "node": node,
                    "verdict": "unknown",
                })),
                _ => {}
            }
        }
    }
    json!({"process": process, "domain": domain})
}

pub(crate) fn execute(request: RunRequest) -> Result<ExecutionResult, RunError> {
    // Resolved before `request.repo` moves into the DAG request below. Issue
    // #111 (A08 F2): `artifact_index::scan` and `committed_evidence::load`
    // both key a committed run on a two-level `<authority-root>/<repo-name>/
    // <run>` layout, but `run execute` used to publish straight into
    // `--authority-root` with no repo segment at all. A run could report
    // `"publication":{"status":"committed"}` at exit 0 and still be a dead
    // end no `artifact query` invocation could ever find. Deriving and
    // nesting the repo name here — the same convention already used by
    // `artifacts::compose_dag_staging_dir` and the primary entry in
    // `main.rs` — keeps the write side and the query side agreeing on the
    // path without changing either reader.
    let repo_name = resolve_repo_name(&request.repo)?;
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
    let publication_root = nest_authority_root(&request.authority_root, &repo_name)?;
    let publication = run_commit::publish_existing(
        &dag.run_root,
        &publication_root,
        &dag.run_root.join("run-manifest-ref.json"),
        &request.final_name,
    )
    .map_err(map_commit_error)?;
    Ok(ExecutionResult {
        outcome: dag.outcome,
        manifest: dag.manifest,
        publication: Publication {
            name: request.final_name,
            repo: repo_name,
            path: publication.final_path,
        },
    })
}

/// The repository key `artifact query`/`artifact index` read a committed run
/// back under: the final path component of the resolved `--repo`. Requires
/// `--repo` to exist (already validated at CLI parse time) so a symlink or a
/// `.`/`..`-laden path still resolves to the real directory name instead of
/// an empty or misleading component.
fn resolve_repo_name(repo: &Path) -> Result<String, RunError> {
    let canonical = repo.canonicalize().map_err(|error| {
        RunError::contract(format!("cannot resolve --repo {}: {error}", repo.display()))
    })?;
    canonical
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            RunError::contract(format!(
                "--repo {} has no usable directory name to publish under",
                repo.display()
            ))
        })
}

/// Folds `repo_name` under `authority_root`, unless the caller already did
/// it themselves — issue #111's confirmed workaround was exactly that,
/// passing `--authority-root <root>/<repo-name>`. Appending it again in that
/// case would double-nest (`<root>/<repo-name>/<repo-name>/<run>`) instead
/// of publishing where the caller already pointed.
fn nest_authority_root(authority_root: &Path, repo_name: &str) -> Result<PathBuf, RunError> {
    let already_nested =
        authority_root.file_name().and_then(|name| name.to_str()) == Some(repo_name);
    let nested = if already_nested {
        authority_root.to_path_buf()
    } else {
        authority_root.join(repo_name)
    };
    fs::create_dir_all(&nested)
        .map_err(|error| RunError::io(format!("create repository authority root: {error}")))?;
    Ok(nested)
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("code-intel-execution-kernel-{name}-{stamp}"))
    }

    #[test]
    fn typed_result_owns_outcome_exit_and_publication_serialization() {
        let result = ExecutionResult {
            outcome: RunOutcome::DomainUnknown,
            manifest: json!({"outcome":"domain_unknown"}),
            publication: Publication {
                name: "run-001".into(),
                repo: "widget-repo".into(),
                path: PathBuf::from("authority/widget-repo/run-001"),
            },
        };

        assert_eq!(result.exit_code(), 20);
        let json = result.to_json();
        assert_eq!(json["outcome"], "domain_unknown");
        assert_eq!(json["exitCode"], 20);
        assert_eq!(json["manifest"]["outcome"], json["outcome"]);
        assert_eq!(json["publication"]["status"], "committed");
        assert_eq!(json["publication"]["marker"], "run-complete.json");
        // A08 F2: the JSON output must be machine-first about the exact
        // repo key `artifact query --repo` needs, not just the path.
        assert_eq!(json["publication"]["repo"], "widget-repo");
        assert_eq!(json["publication"]["path"], "authority/widget-repo/run-001");
    }

    #[test]
    fn resolve_repo_name_takes_the_final_path_component_of_the_resolved_repo() {
        let root = unique_temp_dir("resolve-name");
        let repo = root.join("widget-repo");
        fs::create_dir_all(&repo).expect("fixture repo dir");

        let name = resolve_repo_name(&repo).expect("resolvable repo path");
        assert_eq!(name, "widget-repo");

        // A relative path through a `.` component must resolve to the same
        // real directory name a caller who already typed the absolute path
        // would get — this is the exact shape CI's self-scan invokes
        // (`--repo .`).
        let dotted = repo.join(".");
        let name_via_dot = resolve_repo_name(&dotted).expect("resolvable dotted repo path");
        assert_eq!(name_via_dot, "widget-repo");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_repo_name_rejects_a_repo_path_that_does_not_exist() {
        let missing = unique_temp_dir("resolve-name-missing");
        assert!(resolve_repo_name(&missing).is_err());
    }

    #[test]
    fn nest_authority_root_appends_the_repo_name_exactly_once() {
        let root = unique_temp_dir("nest-once");
        fs::create_dir_all(&root).expect("fixture authority root");

        let nested = nest_authority_root(&root, "widget-repo").expect("nest under authority root");
        assert_eq!(nested, root.join("widget-repo"));
        assert!(nested.is_dir());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nest_authority_root_does_not_double_nest_when_the_caller_already_folded_it_in() {
        // Issue #111's confirmed workaround: the caller pre-nests the repo
        // name into --authority-root themselves. The fix must recognize
        // that and publish there directly, not append a second
        // widget-repo/widget-repo layer.
        let root = unique_temp_dir("nest-workaround").join("widget-repo");
        fs::create_dir_all(&root).expect("fixture pre-nested authority root");

        let nested = nest_authority_root(&root, "widget-repo").expect("reuse pre-nested root");
        assert_eq!(
            nested, root,
            "must not append the repo name a second time: {nested:?}"
        );

        fs::remove_dir_all(root.parent().unwrap()).ok();
    }
}
