mod artifact_ref {
    pub(crate) struct VerifiedArtifact {
        artifact_schema: String,
        artifact_type: String,
        sha256: String,
        consumed_snapshot_identity: String,
    }

    impl VerifiedArtifact {
        pub(crate) fn artifact_schema(&self) -> &str {
            &self.artifact_schema
        }

        pub(crate) fn artifact_type(&self) -> &str {
            &self.artifact_type
        }

        pub(crate) fn sha256(&self) -> &str {
            &self.sha256
        }

        pub(crate) fn consumed_snapshot_identity(&self) -> &str {
            &self.consumed_snapshot_identity
        }
    }
}

#[path = "../src/dag_coordinator.rs"]
mod dag_coordinator;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use dag_coordinator::{
    Coordinator, CoordinatorErrorKind, DagSpec, DomainVerdict, EdgeSpec, ExecutionFailure,
    NodeExecutor, NodeOutcome, NodeSpec, NodeState, RunCheckpoint, RunOutcome, VerifiedArtifactRef,
};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn run_outcome_owns_the_stable_process_exit_mapping() {
    for (outcome, serialized, exit_code) in [
        (RunOutcome::Completed, "completed", 0),
        (RunOutcome::DomainFailed, "domain_failed", 10),
        (RunOutcome::DomainUnknown, "domain_unknown", 20),
        (RunOutcome::ProcessFailed, "process_failed", 70),
        (RunOutcome::Incomplete, "incomplete", 70),
    ] {
        assert_eq!(outcome.as_str(), serialized);
        assert_eq!(outcome.exit_code(), exit_code);
    }
}

fn node(id: &str) -> NodeSpec {
    NodeSpec::new(id, format!("fixture.{id}"), format!("request-v1:{id}"))
}

fn dag(nodes: &[&str], edges: &[(&str, &str)], max_concurrency: usize) -> DagSpec {
    DagSpec::new(
        SNAPSHOT,
        max_concurrency,
        nodes.iter().map(|id| node(id)).collect(),
        edges
            .iter()
            .map(|(from, to)| EdgeSpec::new(*from, *to))
            .collect(),
    )
}

#[test]
fn rejects_duplicate_unknown_and_cyclic_graphs_before_execution() {
    let cases = [
        (
            DagSpec::new(SNAPSHOT, 1, vec![node("a"), node("a")], vec![]),
            CoordinatorErrorKind::DuplicateNode,
        ),
        (
            dag(&["a"], &[("missing", "a")], 1),
            CoordinatorErrorKind::UnknownNode,
        ),
        (
            dag(&["a", "b"], &[("a", "b"), ("a", "b")], 1),
            CoordinatorErrorKind::DuplicateEdge,
        ),
        (
            dag(&["a", "b"], &[("a", "b"), ("b", "a")], 1),
            CoordinatorErrorKind::Cycle,
        ),
    ];

    for (spec, expected) in cases {
        let error = Coordinator::new(spec).unwrap_err();
        assert_eq!(error.kind(), expected);
    }
}

#[test]
fn ready_batches_are_sorted_bounded_and_carry_only_verified_dependency_refs() {
    let mut coordinator = Coordinator::new(dag(
        &["snapshot", "inventory", "independent"],
        &[("snapshot", "inventory")],
        2,
    ))
    .unwrap();

    let first = coordinator.next_batch().unwrap();
    assert_eq!(
        first
            .iter()
            .map(|item| item.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["independent", "snapshot"]
    );
    assert!(first.iter().all(|item| item.inputs.is_empty()));

    coordinator
        .record(
            "snapshot",
            NodeOutcome::success(
                DomainVerdict::Pass,
                vec![VerifiedArtifactRef::verified_for_test(
                    "code-intel-repository-snapshot.v1",
                    "repository.snapshot",
                    "snapshot.json",
                    "b".repeat(64),
                    SNAPSHOT,
                )
                .unwrap()],
            ),
        )
        .unwrap();
    coordinator
        .record(
            "independent",
            NodeOutcome::success(DomainVerdict::Pass, vec![]),
        )
        .unwrap();

    let second = coordinator.next_batch().unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].node_id, "inventory");
    assert_eq!(second[0].inputs.len(), 1);
    assert_eq!(second[0].inputs[0].path(), "snapshot.json");
}

#[test]
fn domain_and_process_failures_block_only_descendants_and_preserve_taxonomy() {
    let mut coordinator = Coordinator::new(dag(
        &["domain", "domain_child", "process", "process_child", "free"],
        &[("domain", "domain_child"), ("process", "process_child")],
        3,
    ))
    .unwrap();
    let first = coordinator.next_batch().unwrap();
    assert_eq!(first.len(), 3);

    coordinator
        .record("domain", NodeOutcome::domain_fail("quality gate failed"))
        .unwrap();
    coordinator
        .record(
            "process",
            NodeOutcome::process_failure(ExecutionFailure::Unavailable, "tool missing"),
        )
        .unwrap();
    coordinator
        .record("free", NodeOutcome::success(DomainVerdict::Pass, vec![]))
        .unwrap();

    assert!(matches!(
        coordinator.state("domain"),
        Some(NodeState::DomainFailed { .. })
    ));
    assert!(matches!(
        coordinator.state("process"),
        Some(NodeState::ProcessFailed {
            failure: ExecutionFailure::Unavailable,
            ..
        })
    ));
    assert!(matches!(
        coordinator.state("domain_child"),
        Some(NodeState::DependencyBlocked { .. })
    ));
    assert!(matches!(
        coordinator.state("process_child"),
        Some(NodeState::DependencyBlocked { .. })
    ));
    assert!(matches!(
        coordinator.state("free"),
        Some(NodeState::Succeeded { .. })
    ));
    assert!(coordinator.next_batch().unwrap().is_empty());
    assert!(coordinator.is_terminal());
}

struct BranchOutcomeProbe;

impl NodeExecutor for BranchOutcomeProbe {
    fn execute(&self, dispatch: dag_coordinator::Dispatch) -> NodeOutcome {
        if dispatch.node_id == "failing" {
            NodeOutcome::domain_fail("independent quality branch failed")
        } else {
            NodeOutcome::success(DomainVerdict::Pass, vec![])
        }
    }
}

#[test]
fn scheduler_continues_an_independent_branch_after_domain_failure() {
    let manifest = Coordinator::new(dag(
        &["failing", "blocked_child", "independent"],
        &[("failing", "blocked_child")],
        2,
    ))
    .unwrap()
    .run_to_completion(&BranchOutcomeProbe)
    .unwrap();

    assert_eq!(manifest.outcome.as_str(), "domain_failed");
    assert!(matches!(
        manifest.nodes["failing"],
        NodeState::DomainFailed { .. }
    ));
    assert!(matches!(
        manifest.nodes["blocked_child"],
        NodeState::DependencyBlocked { .. }
    ));
    assert!(matches!(
        manifest.nodes["independent"],
        NodeState::Succeeded { .. }
    ));
}

#[test]
fn direct_fail_verdicts_and_empty_diagnostics_still_emit_schema_valid_failure_states() {
    let mut coordinator = Coordinator::new(dag(&["domain", "process"], &[], 2)).unwrap();
    assert_eq!(coordinator.next_batch().unwrap().len(), 2);
    coordinator
        .record(
            "domain",
            NodeOutcome::Success {
                verdict: DomainVerdict::Fail,
                artifacts: vec![],
            },
        )
        .unwrap();
    coordinator
        .record(
            "process",
            NodeOutcome::process_failure(ExecutionFailure::Internal, "  "),
        )
        .unwrap();

    let manifest = coordinator.manifest().to_json();
    assert_eq!(manifest["nodes"]["domain"]["status"], "domain_failed");
    assert_eq!(
        manifest["nodes"]["domain"]["diagnostic"],
        "executor returned a domain fail verdict"
    );
    assert_eq!(manifest["nodes"]["process"]["status"], "process_failed");
    assert_eq!(
        manifest["nodes"]["process"]["diagnostic"],
        "process failure without diagnostic"
    );
}

#[test]
fn terminal_unknown_verdict_is_not_reported_as_completed() {
    let mut coordinator = Coordinator::new(dag(&["evidence", "independent"], &[], 2)).unwrap();
    assert_eq!(coordinator.next_batch().unwrap().len(), 2);
    coordinator
        .record(
            "evidence",
            NodeOutcome::success(DomainVerdict::Unknown, vec![]),
        )
        .unwrap();
    coordinator
        .record(
            "independent",
            NodeOutcome::success(DomainVerdict::Pass, vec![]),
        )
        .unwrap();

    let manifest = coordinator.manifest().to_json();
    assert_eq!(manifest["outcome"], "domain_unknown");
    assert_eq!(manifest["nodes"]["evidence"]["status"], "succeeded");
    assert_eq!(manifest["nodes"]["evidence"]["verdict"], "unknown");
}

#[test]
fn resume_requires_matching_identity_and_replays_only_unfinished_nodes() {
    let spec = dag(&["a", "b", "c"], &[("a", "b")], 2);
    let mut first = Coordinator::new(spec.clone()).unwrap();
    let identity = first.run_identity().to_string();
    let batch = first.next_batch().unwrap();
    assert_eq!(
        batch
            .iter()
            .map(|item| item.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "c"]
    );
    first
        .record("a", NodeOutcome::success(DomainVerdict::Pass, vec![]))
        .unwrap();
    let checkpoint = first.checkpoint();

    let mut resumed = Coordinator::resume(spec.clone(), checkpoint.clone()).unwrap();
    assert_eq!(resumed.run_identity(), identity);
    assert!(matches!(
        resumed.state("a"),
        Some(NodeState::Succeeded { .. })
    ));
    assert_eq!(
        resumed
            .next_batch()
            .unwrap()
            .iter()
            .map(|item| item.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "c"]
    );

    let changed = dag(&["a", "b", "c", "d"], &[("a", "b")], 2);
    let error = Coordinator::resume(changed, checkpoint).unwrap_err();
    assert_eq!(error.kind(), CoordinatorErrorKind::ResumeIdentityMismatch);
}

#[test]
fn identity_ignores_declaration_order_and_resume_rejects_impossible_history() {
    let left = dag(&["a", "b", "c"], &[("a", "c"), ("b", "c")], 2);
    let right = dag(&["c", "a", "b"], &[("b", "c"), ("a", "c")], 2);
    let left_coordinator = Coordinator::new(left.clone()).unwrap();
    assert_eq!(left_coordinator.run_identity().len(), 71);
    assert!(left_coordinator.run_identity().starts_with("dag-v1:"));
    assert_eq!(
        left_coordinator.run_identity(),
        Coordinator::new(right).unwrap().run_identity()
    );

    let mut nodes = left_coordinator.checkpoint().nodes;
    nodes.insert(
        "c".to_string(),
        NodeState::Succeeded {
            verdict: DomainVerdict::Pass,
            artifacts: vec![],
        },
    );
    let forged = RunCheckpoint {
        schema: dag_coordinator::RUN_STATE_SCHEMA,
        run_identity: left_coordinator.run_identity().to_string(),
        nodes,
    };
    let error = Coordinator::resume(left, forged).unwrap_err();
    assert_eq!(error.kind(), CoordinatorErrorKind::ResumeIdentityMismatch);
}

#[test]
fn resume_rejects_every_forged_terminal_child_while_parent_is_pending() {
    let spec = dag(&["parent", "child"], &[("parent", "child")], 1);
    let coordinator = Coordinator::new(spec.clone()).unwrap();
    let forged_states = [
        NodeState::DomainFailed {
            diagnostic: "forged domain result".to_string(),
            artifacts: vec![],
        },
        NodeState::ProcessFailed {
            failure: ExecutionFailure::Internal,
            diagnostic: "forged process result".to_string(),
        },
        NodeState::DependencyBlocked {
            blocked_by: vec!["parent".to_string()],
        },
        NodeState::Succeeded {
            verdict: DomainVerdict::Pass,
            artifacts: vec![],
        },
    ];
    for forged_state in forged_states {
        let mut nodes = coordinator.checkpoint().nodes;
        nodes.insert("child".to_string(), forged_state);
        let checkpoint = RunCheckpoint {
            schema: dag_coordinator::RUN_STATE_SCHEMA,
            run_identity: coordinator.run_identity().to_string(),
            nodes,
        };
        let error = Coordinator::resume(spec.clone(), checkpoint).unwrap_err();
        assert_eq!(error.kind(), CoordinatorErrorKind::ResumeIdentityMismatch);
    }
}

struct ProbeExecutor {
    active: AtomicUsize,
    peak: AtomicUsize,
    seen: Mutex<BTreeSet<String>>,
}

impl NodeExecutor for ProbeExecutor {
    fn execute(&self, dispatch: dag_coordinator::Dispatch) -> NodeOutcome {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(30));
        self.seen.lock().unwrap().insert(dispatch.node_id);
        self.active.fetch_sub(1, Ordering::SeqCst);
        NodeOutcome::success(DomainVerdict::Pass, vec![])
    }
}

#[test]
fn run_uses_bounded_parallelism_and_manifest_is_deterministic_and_complete() {
    let spec = dag(&["d", "b", "a", "c"], &[], 2);
    let identity = Coordinator::new(spec.clone())
        .unwrap()
        .run_identity()
        .to_string();
    let executor = ProbeExecutor {
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        seen: Mutex::new(BTreeSet::new()),
    };
    let manifest = Coordinator::new(spec)
        .unwrap()
        .run_to_completion(&executor)
        .unwrap();

    assert_eq!(executor.peak.load(Ordering::SeqCst), 2);
    assert_eq!(executor.seen.lock().unwrap().len(), 4);
    assert_eq!(manifest.run_identity, identity);
    assert_eq!(manifest.snapshot_identity, SNAPSHOT);
    assert_eq!(manifest.outcome.as_str(), "completed");
    assert_eq!(manifest.nodes.len(), 4);
    assert_eq!(
        manifest
            .nodes
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["a", "b", "c", "d"]
    );
}

#[test]
fn node_contract_has_no_implicit_provider_command_or_tool_authority() {
    let value = node("safe").to_json();
    assert_eq!(value["id"], "safe");
    assert_eq!(value["capability"], "fixture.safe");
    for forbidden in ["provider", "command", "executable", "tool", "effects"] {
        assert!(
            value.get(forbidden).is_none(),
            "forbidden authority field: {forbidden}"
        );
    }
}

#[test]
fn production_source_exposes_no_field_based_verified_artifact_constructor() {
    let source = include_str!("../src/dag_coordinator.rs");
    let artifact_source = include_str!("../src/artifact_ref.rs");
    assert!(!source.contains("pub(crate) fn from_verified_fields"));
    assert!(!source.contains("pub fn from_verified_fields"));
    let verified_struct = artifact_source
        .split("pub(crate) struct VerifiedArtifact")
        .nth(1)
        .and_then(|tail| tail.split('}').next())
        .expect("VerifiedArtifact declaration");
    assert!(
        !verified_struct
            .lines()
            .any(|line| line.trim_start().starts_with("pub")),
        "every A03 token field must be private so siblings cannot forge the token"
    );
    for field in [
        "bytes",
        "artifact_schema",
        "artifact_type",
        "sha256",
        "consumed_snapshot_identity",
        "stable_file_id",
    ] {
        assert!(
            verified_struct.contains(&format!("{field}:")),
            "static forge-chain test must cover token field {field}"
        );
    }
    assert!(source.contains("verified: &crate::artifact_ref::VerifiedArtifact"));
    assert_eq!(
        artifact_source.matches("Ok(VerifiedArtifact {").count(),
        1,
        "only the A03 verifier may construct a successful VerifiedArtifact token"
    );
}

#[test]
fn checked_in_dag_schema_is_closed_and_contains_no_execution_authority_fields() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../orchestration/schemas/code-intel-run-dag.v1.schema.json"
    ))
    .unwrap();
    assert_eq!(
        schema["$id"],
        "https://code-intel.local/schemas/code-intel-run-dag.v1.schema.json"
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["nodes"]["items"]["additionalProperties"],
        false
    );
    let node_properties = schema["properties"]["nodes"]["items"]["properties"]
        .as_object()
        .unwrap();
    assert_eq!(
        node_properties
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["capability", "id", "requestIdentity"])
    );

    for checked_in in [
        include_str!("../../../orchestration/schemas/code-intel-run-state.v1.schema.json"),
        include_str!("../../../orchestration/schemas/code-intel-run-manifest.v1.schema.json"),
    ] {
        let schema: serde_json::Value = serde_json::from_str(checked_in).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["definitions"]["nodeState"].is_object());
        assert_ne!(
            schema["properties"]["nodes"]["additionalProperties"],
            serde_json::Value::Bool(true)
        );
    }
}
