use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde_json::{json, Value};

pub const DAG_SCHEMA: &str = "code-intel-run-dag.v1";
pub const RUN_STATE_SCHEMA: &str = "code-intel-run-state.v1";
pub const RUN_MANIFEST_SCHEMA: &str = "code-intel-run-manifest.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeSpec {
    pub id: String,
    pub capability: String,
    pub request_identity: String,
}

impl NodeSpec {
    pub fn new(
        id: impl Into<String>,
        capability: impl Into<String>,
        request_identity: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            capability: capability.into(),
            request_identity: request_identity.into(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "capability": self.capability,
            "requestIdentity": self.request_identity,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EdgeSpec {
    pub from: String,
    pub to: String,
}

impl EdgeSpec {
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
        }
    }

    fn to_json(&self) -> Value {
        json!({"from": self.from, "to": self.to})
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DagSpec {
    pub snapshot_identity: String,
    pub max_concurrency: usize,
    pub nodes: Vec<NodeSpec>,
    pub edges: Vec<EdgeSpec>,
}

impl DagSpec {
    pub fn new(
        snapshot_identity: impl Into<String>,
        max_concurrency: usize,
        nodes: Vec<NodeSpec>,
        edges: Vec<EdgeSpec>,
    ) -> Self {
        Self {
            snapshot_identity: snapshot_identity.into(),
            max_concurrency,
            nodes,
            edges,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut nodes = self.nodes.clone();
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        let mut edges = self.edges.clone();
        edges.sort();
        json!({
            "schema": DAG_SCHEMA,
            "snapshotIdentity": self.snapshot_identity,
            "maxConcurrency": self.max_concurrency,
            "nodes": nodes.iter().map(NodeSpec::to_json).collect::<Vec<_>>(),
            "edges": edges.iter().map(EdgeSpec::to_json).collect::<Vec<_>>(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordinatorErrorKind {
    InvalidSpec,
    DuplicateNode,
    UnknownNode,
    DuplicateEdge,
    Cycle,
    InvalidTransition,
    ResumeIdentityMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoordinatorError {
    kind: CoordinatorErrorKind,
    message: String,
}

impl CoordinatorError {
    fn new(kind: CoordinatorErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CoordinatorErrorKind {
        self.kind
    }
}

impl fmt::Display for CoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for CoordinatorError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedArtifactRef {
    artifact_schema: String,
    artifact_type: String,
    path: String,
    sha256: String,
    consumed_snapshot_identity: String,
}

impl VerifiedArtifactRef {
    #[cfg(test)]
    pub(super) fn verified_for_test(
        artifact_schema: impl Into<String>,
        artifact_type: impl Into<String>,
        path: impl Into<String>,
        sha256: impl Into<String>,
        consumed_snapshot_identity: impl Into<String>,
    ) -> Result<Self, CoordinatorError> {
        let value = Self {
            artifact_schema: artifact_schema.into(),
            artifact_type: artifact_type.into(),
            path: path.into(),
            sha256: sha256.into(),
            consumed_snapshot_identity: consumed_snapshot_identity.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn from_a03(
        path: impl Into<String>,
        verified: &crate::artifact_ref::VerifiedArtifact,
    ) -> Result<Self, CoordinatorError> {
        Self::from_a03_fields(
            verified.artifact_schema().to_string(),
            verified.artifact_type().to_string(),
            path,
            verified.sha256().to_string(),
            verified.consumed_snapshot_identity().to_string(),
        )
    }

    fn from_a03_fields(
        artifact_schema: impl Into<String>,
        artifact_type: impl Into<String>,
        path: impl Into<String>,
        sha256: impl Into<String>,
        consumed_snapshot_identity: impl Into<String>,
    ) -> Result<Self, CoordinatorError> {
        let value = Self {
            artifact_schema: artifact_schema.into(),
            artifact_type: artifact_type.into(),
            path: path.into(),
            sha256: sha256.into(),
            consumed_snapshot_identity: consumed_snapshot_identity.into(),
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), CoordinatorError> {
        if self.artifact_schema.is_empty()
            || self.artifact_type.is_empty()
            || self.path.is_empty()
            || !valid_digest(&self.sha256)
            || !valid_digest(&self.consumed_snapshot_identity)
        {
            return Err(CoordinatorError::new(
                CoordinatorErrorKind::InvalidSpec,
                "verified Artifact Ref fields are invalid",
            ));
        }
        Ok(())
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn artifact_type(&self) -> &str {
        &self.artifact_type
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "schema": "code-intel-artifact-ref.v1",
            "artifactSchema": self.artifact_schema,
            "type": self.artifact_type,
            "path": self.path,
            "sha256": self.sha256,
            "consumedSnapshotIdentity": self.consumed_snapshot_identity,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainVerdict {
    Pass,
    Fail,
    Unknown,
    NotApplicable,
}

impl DomainVerdict {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Unknown => "unknown",
            Self::NotApplicable => "not_applicable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionFailure {
    Contract,
    Unavailable,
    Internal,
    Io,
}

impl ExecutionFailure {
    fn as_str(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
            Self::Io => "io",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeOutcome {
    Success {
        verdict: DomainVerdict,
        artifacts: Vec<VerifiedArtifactRef>,
    },
    DomainFail {
        diagnostic: String,
        artifacts: Vec<VerifiedArtifactRef>,
    },
    ProcessFailure {
        failure: ExecutionFailure,
        diagnostic: String,
    },
}

impl NodeOutcome {
    pub fn success(verdict: DomainVerdict, artifacts: Vec<VerifiedArtifactRef>) -> Self {
        if verdict == DomainVerdict::Fail {
            Self::DomainFail {
                diagnostic: "executor returned a domain fail verdict".to_string(),
                artifacts,
            }
        } else {
            Self::Success { verdict, artifacts }
        }
    }

    pub fn domain_fail(diagnostic: impl Into<String>) -> Self {
        Self::DomainFail {
            diagnostic: diagnostic.into(),
            artifacts: Vec::new(),
        }
    }

    pub fn domain_fail_with_artifacts(
        diagnostic: impl Into<String>,
        artifacts: Vec<VerifiedArtifactRef>,
    ) -> Self {
        Self::DomainFail {
            diagnostic: diagnostic.into(),
            artifacts,
        }
    }

    pub fn process_failure(failure: ExecutionFailure, diagnostic: impl Into<String>) -> Self {
        Self::ProcessFailure {
            failure,
            diagnostic: diagnostic.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeState {
    Pending,
    Running,
    Succeeded {
        verdict: DomainVerdict,
        artifacts: Vec<VerifiedArtifactRef>,
    },
    DomainFailed {
        diagnostic: String,
        artifacts: Vec<VerifiedArtifactRef>,
    },
    ProcessFailed {
        failure: ExecutionFailure,
        diagnostic: String,
    },
    DependencyBlocked {
        blocked_by: Vec<String>,
    },
}

impl NodeState {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded { .. }
                | Self::DomainFailed { .. }
                | Self::ProcessFailed { .. }
                | Self::DependencyBlocked { .. }
        )
    }

    fn blocks_dependents(&self) -> bool {
        matches!(
            self,
            Self::DomainFailed { .. } | Self::ProcessFailed { .. } | Self::DependencyBlocked { .. }
        )
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Pending => json!({"status":"pending"}),
            Self::Running => json!({"status":"running"}),
            Self::Succeeded { verdict, artifacts } => json!({
                "status":"succeeded",
                "verdict":verdict.as_str(),
                "artifacts":artifacts.iter().map(VerifiedArtifactRef::to_json).collect::<Vec<_>>(),
            }),
            Self::DomainFailed {
                diagnostic,
                artifacts,
            } => json!({
                "status":"domain_failed",
                "verdict":"fail",
                "diagnostic":diagnostic,
                "artifacts":artifacts.iter().map(VerifiedArtifactRef::to_json).collect::<Vec<_>>(),
            }),
            Self::ProcessFailed {
                failure,
                diagnostic,
            } => json!({
                "status":"process_failed",
                "failure":failure.as_str(),
                "diagnostic":diagnostic,
            }),
            Self::DependencyBlocked { blocked_by } => json!({
                "status":"dependency_blocked",
                "blockedBy":blocked_by,
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    pub node_id: String,
    pub capability: String,
    pub request_identity: String,
    pub inputs: Vec<VerifiedArtifactRef>,
}

pub trait NodeExecutor: Sync {
    fn execute(&self, dispatch: Dispatch) -> NodeOutcome;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunCheckpoint {
    pub schema: &'static str,
    pub run_identity: String,
    pub nodes: BTreeMap<String, NodeState>,
}

impl RunCheckpoint {
    pub fn to_json(&self) -> Value {
        json!({
            "schema":self.schema,
            "runIdentity":self.run_identity,
            "nodes":self.nodes.iter().map(|(id,state)|(id.clone(),state.to_json())).collect::<BTreeMap<_,_>>(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutcome(&'static str);

impl RunOutcome {
    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunManifest {
    pub schema: &'static str,
    pub run_identity: String,
    pub snapshot_identity: String,
    pub outcome: RunOutcome,
    pub nodes: BTreeMap<String, NodeState>,
}

impl RunManifest {
    pub fn to_json(&self) -> Value {
        json!({
            "schema":self.schema,
            "runIdentity":self.run_identity,
            "snapshotIdentity":self.snapshot_identity,
            "outcome":self.outcome.as_str(),
            "nodes":self.nodes.iter().map(|(id,state)|(id.clone(),state.to_json())).collect::<BTreeMap<_,_>>(),
        })
    }
}

#[derive(Debug)]
pub struct Coordinator {
    spec: DagSpec,
    run_identity: String,
    nodes: BTreeMap<String, NodeSpec>,
    dependencies: BTreeMap<String, Vec<String>>,
    states: BTreeMap<String, NodeState>,
}

impl Coordinator {
    pub fn new(spec: DagSpec) -> Result<Self, CoordinatorError> {
        let (nodes, dependencies) = validate_spec(&spec)?;
        let states = nodes
            .keys()
            .map(|id| (id.clone(), NodeState::Pending))
            .collect();
        Ok(Self {
            run_identity: identity_for(&spec),
            spec,
            nodes,
            dependencies,
            states,
        })
    }

    pub fn resume(spec: DagSpec, checkpoint: RunCheckpoint) -> Result<Self, CoordinatorError> {
        let mut coordinator = Self::new(spec)?;
        if checkpoint.schema != RUN_STATE_SCHEMA
            || checkpoint.run_identity != coordinator.run_identity
        {
            return Err(CoordinatorError::new(
                CoordinatorErrorKind::ResumeIdentityMismatch,
                "checkpoint does not match the deterministic DAG identity",
            ));
        }
        for id in checkpoint.nodes.keys() {
            if !coordinator.nodes.contains_key(id) {
                return Err(CoordinatorError::new(
                    CoordinatorErrorKind::ResumeIdentityMismatch,
                    format!("checkpoint contains unknown node: {id}"),
                ));
            }
        }
        coordinator.validate_checkpoint_history(&checkpoint.nodes)?;
        for (id, state) in checkpoint.nodes {
            let restored = match state {
                NodeState::Running | NodeState::Pending => NodeState::Pending,
                NodeState::DependencyBlocked { .. } => NodeState::Pending,
                NodeState::Succeeded { verdict, artifacts } => {
                    if verdict == DomainVerdict::Fail
                        || artifacts.iter().any(|artifact| {
                            artifact.consumed_snapshot_identity
                                != coordinator.spec.snapshot_identity
                        })
                    {
                        return Err(CoordinatorError::new(
                            CoordinatorErrorKind::ResumeIdentityMismatch,
                            format!("checkpoint has invalid completed node: {id}"),
                        ));
                    }
                    NodeState::Succeeded { verdict, artifacts }
                }
                NodeState::DomainFailed {
                    diagnostic,
                    artifacts,
                } => {
                    if diagnostic.trim().is_empty()
                        || artifacts.iter().any(|artifact| {
                            artifact.consumed_snapshot_identity
                                != coordinator.spec.snapshot_identity
                        })
                    {
                        return Err(CoordinatorError::new(
                            CoordinatorErrorKind::ResumeIdentityMismatch,
                            format!("checkpoint has invalid domain-failed node: {id}"),
                        ));
                    }
                    NodeState::DomainFailed {
                        diagnostic,
                        artifacts,
                    }
                }
                NodeState::ProcessFailed {
                    failure,
                    diagnostic,
                } => {
                    if diagnostic.trim().is_empty() {
                        return Err(CoordinatorError::new(
                            CoordinatorErrorKind::ResumeIdentityMismatch,
                            format!("checkpoint has empty process diagnostic: {id}"),
                        ));
                    }
                    NodeState::ProcessFailed {
                        failure,
                        diagnostic,
                    }
                }
            };
            coordinator.states.insert(id, restored);
        }
        coordinator.propagate_blocked();
        Ok(coordinator)
    }

    pub fn run_identity(&self) -> &str {
        &self.run_identity
    }

    pub fn state(&self, node_id: &str) -> Option<&NodeState> {
        self.states.get(node_id)
    }

    pub fn next_batch(&mut self) -> Result<Vec<Dispatch>, CoordinatorError> {
        self.propagate_blocked();
        let running = self
            .states
            .values()
            .filter(|state| matches!(state, NodeState::Running))
            .count();
        let available = self.spec.max_concurrency.saturating_sub(running);
        if available == 0 {
            return Ok(Vec::new());
        }
        let ready = self
            .states
            .iter()
            .filter_map(|(id, state)| {
                matches!(state, NodeState::Pending)
                    .then_some(id)
                    .filter(|id| self.dependencies_succeeded(id))
            })
            .take(available)
            .cloned()
            .collect::<Vec<_>>();

        let mut dispatches = Vec::with_capacity(ready.len());
        for id in ready {
            let node = self.nodes.get(&id).expect("validated node");
            let mut inputs = Vec::new();
            for dependency in self.dependencies.get(&id).expect("validated dependencies") {
                if let Some(NodeState::Succeeded { artifacts, .. }) = self.states.get(dependency) {
                    inputs.extend(artifacts.iter().cloned());
                }
            }
            self.states.insert(id.clone(), NodeState::Running);
            dispatches.push(Dispatch {
                node_id: id,
                capability: node.capability.clone(),
                request_identity: node.request_identity.clone(),
                inputs,
            });
        }
        Ok(dispatches)
    }

    pub fn record(&mut self, node_id: &str, outcome: NodeOutcome) -> Result<(), CoordinatorError> {
        if !matches!(self.states.get(node_id), Some(NodeState::Running)) {
            return Err(CoordinatorError::new(
                CoordinatorErrorKind::InvalidTransition,
                format!("node is not running: {node_id}"),
            ));
        }
        let state = match outcome {
            NodeOutcome::Success { verdict, artifacts } => {
                if artifacts.iter().any(|artifact| {
                    artifact.consumed_snapshot_identity != self.spec.snapshot_identity
                }) {
                    return Err(CoordinatorError::new(
                        CoordinatorErrorKind::InvalidTransition,
                        format!("node returned Artifact Ref for another snapshot: {node_id}"),
                    ));
                }
                if verdict == DomainVerdict::Fail {
                    NodeState::DomainFailed {
                        diagnostic: "executor returned a domain fail verdict".to_string(),
                        artifacts,
                    }
                } else {
                    NodeState::Succeeded { verdict, artifacts }
                }
            }
            NodeOutcome::DomainFail {
                diagnostic,
                artifacts,
            } => {
                if artifacts.iter().any(|artifact| {
                    artifact.consumed_snapshot_identity != self.spec.snapshot_identity
                }) {
                    return Err(CoordinatorError::new(
                        CoordinatorErrorKind::InvalidTransition,
                        format!("node returned Artifact Ref for another snapshot: {node_id}"),
                    ));
                }
                NodeState::DomainFailed {
                    diagnostic: nonempty_diagnostic(
                        diagnostic,
                        "domain failure without diagnostic",
                    ),
                    artifacts,
                }
            }
            NodeOutcome::ProcessFailure {
                failure,
                diagnostic,
            } => NodeState::ProcessFailed {
                failure,
                diagnostic: nonempty_diagnostic(diagnostic, "process failure without diagnostic"),
            },
        };
        self.states.insert(node_id.to_string(), state);
        self.propagate_blocked();
        Ok(())
    }

    pub fn checkpoint(&self) -> RunCheckpoint {
        let nodes = self
            .states
            .iter()
            .map(|(id, state)| {
                let persisted = if matches!(state, NodeState::Running) {
                    NodeState::Pending
                } else {
                    state.clone()
                };
                (id.clone(), persisted)
            })
            .collect();
        RunCheckpoint {
            schema: RUN_STATE_SCHEMA,
            run_identity: self.run_identity.clone(),
            nodes,
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.states.values().all(NodeState::is_terminal)
    }

    pub fn run_to_completion<E: NodeExecutor>(
        mut self,
        executor: &E,
    ) -> Result<RunManifest, CoordinatorError> {
        while !self.is_terminal() {
            let batch = self.next_batch()?;
            if batch.is_empty() {
                return Err(CoordinatorError::new(
                    CoordinatorErrorKind::InvalidTransition,
                    "DAG has unfinished nodes but no schedulable work",
                ));
            }
            let mut results = std::thread::scope(|scope| {
                batch
                    .into_iter()
                    .map(|dispatch| {
                        let id = dispatch.node_id.clone();
                        scope.spawn(move || (id, executor.execute(dispatch)))
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|handle| {
                        handle.join().map_err(|_| {
                            CoordinatorError::new(
                                CoordinatorErrorKind::InvalidTransition,
                                "node executor panicked",
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })?;
            results.sort_by(|left, right| left.0.cmp(&right.0));
            for (id, outcome) in results {
                self.record(&id, outcome)?;
            }
        }
        Ok(self.manifest())
    }

    pub fn manifest(&self) -> RunManifest {
        let outcome = if self
            .states
            .values()
            .any(|state| matches!(state, NodeState::ProcessFailed { .. }))
        {
            RunOutcome("process_failed")
        } else if self
            .states
            .values()
            .any(|state| matches!(state, NodeState::DomainFailed { .. }))
        {
            RunOutcome("domain_failed")
        } else if self.states.values().any(|state| {
            matches!(
                state,
                NodeState::Succeeded {
                    verdict: DomainVerdict::Unknown,
                    ..
                }
            )
        }) {
            RunOutcome("domain_unknown")
        } else if self.is_terminal() {
            RunOutcome("completed")
        } else {
            RunOutcome("incomplete")
        };
        RunManifest {
            schema: RUN_MANIFEST_SCHEMA,
            run_identity: self.run_identity.clone(),
            snapshot_identity: self.spec.snapshot_identity.clone(),
            outcome,
            nodes: self.states.clone(),
        }
    }

    fn dependencies_succeeded(&self, node_id: &str) -> bool {
        self.dependencies
            .get(node_id)
            .expect("validated dependencies")
            .iter()
            .all(|dependency| {
                matches!(
                    self.states.get(dependency),
                    Some(NodeState::Succeeded { .. })
                )
            })
    }

    fn validate_checkpoint_history(
        &self,
        checkpoint: &BTreeMap<String, NodeState>,
    ) -> Result<(), CoordinatorError> {
        for (id, state) in checkpoint {
            let dependencies = self.dependencies.get(id).expect("validated dependencies");
            match state {
                NodeState::Succeeded { .. }
                | NodeState::DomainFailed { .. }
                | NodeState::ProcessFailed { .. }
                    if !dependencies.iter().all(|dependency| {
                        matches!(
                            checkpoint.get(dependency),
                            Some(NodeState::Succeeded { .. })
                        )
                    }) =>
                {
                    return Err(CoordinatorError::new(
                        CoordinatorErrorKind::ResumeIdentityMismatch,
                        format!("checkpoint finalized node before its dependencies: {id}"),
                    ));
                }
                NodeState::DependencyBlocked { blocked_by } => {
                    let expected = dependencies
                        .iter()
                        .filter(|dependency| {
                            checkpoint
                                .get(*dependency)
                                .is_some_and(NodeState::blocks_dependents)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    if expected.is_empty() || *blocked_by != expected {
                        return Err(CoordinatorError::new(
                            CoordinatorErrorKind::ResumeIdentityMismatch,
                            format!("checkpoint has inconsistent dependency block history: {id}"),
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn propagate_blocked(&mut self) {
        loop {
            let blocked = self
                .states
                .iter()
                .filter(|(_, state)| matches!(state, NodeState::Pending))
                .filter_map(|(id, _)| {
                    let blocked_by = self
                        .dependencies
                        .get(id)
                        .expect("validated dependencies")
                        .iter()
                        .filter(|dependency| {
                            self.states
                                .get(*dependency)
                                .is_some_and(NodeState::blocks_dependents)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    (!blocked_by.is_empty()).then_some((id.clone(), blocked_by))
                })
                .collect::<Vec<_>>();
            if blocked.is_empty() {
                break;
            }
            for (id, blocked_by) in blocked {
                self.states
                    .insert(id, NodeState::DependencyBlocked { blocked_by });
            }
        }
    }
}

fn validate_spec(
    spec: &DagSpec,
) -> Result<(BTreeMap<String, NodeSpec>, BTreeMap<String, Vec<String>>), CoordinatorError> {
    if !valid_digest(&spec.snapshot_identity)
        || spec.nodes.is_empty()
        || spec.max_concurrency == 0
        || spec.max_concurrency > 256
    {
        return Err(CoordinatorError::new(
            CoordinatorErrorKind::InvalidSpec,
            "DAG snapshot, nodes, or concurrency bound is invalid",
        ));
    }
    let mut nodes = BTreeMap::new();
    for node in &spec.nodes {
        if !valid_id(&node.id)
            || !valid_id(&node.capability)
            || node.request_identity.trim().is_empty()
        {
            return Err(CoordinatorError::new(
                CoordinatorErrorKind::InvalidSpec,
                format!("invalid DAG node: {}", node.id),
            ));
        }
        if nodes.insert(node.id.clone(), node.clone()).is_some() {
            return Err(CoordinatorError::new(
                CoordinatorErrorKind::DuplicateNode,
                format!("duplicate DAG node: {}", node.id),
            ));
        }
    }
    let mut dependencies = nodes
        .keys()
        .map(|id| (id.clone(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let mut edges = BTreeSet::new();
    for edge in &spec.edges {
        if !nodes.contains_key(&edge.from) || !nodes.contains_key(&edge.to) {
            return Err(CoordinatorError::new(
                CoordinatorErrorKind::UnknownNode,
                format!(
                    "DAG edge references unknown node: {} -> {}",
                    edge.from, edge.to
                ),
            ));
        }
        if !edges.insert(edge.clone()) {
            return Err(CoordinatorError::new(
                CoordinatorErrorKind::DuplicateEdge,
                format!("duplicate DAG edge: {} -> {}", edge.from, edge.to),
            ));
        }
        dependencies
            .get_mut(&edge.to)
            .expect("known edge target")
            .push(edge.from.clone());
    }
    for values in dependencies.values_mut() {
        values.sort();
    }
    validate_acyclic(&nodes, &dependencies)?;
    Ok((nodes, dependencies))
}

fn validate_acyclic(
    nodes: &BTreeMap<String, NodeSpec>,
    dependencies: &BTreeMap<String, Vec<String>>,
) -> Result<(), CoordinatorError> {
    let mut remaining = dependencies
        .iter()
        .map(|(id, values)| (id.clone(), values.len()))
        .collect::<BTreeMap<_, _>>();
    let mut ready = remaining
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(id.clone()))
        .collect::<BTreeSet<_>>();
    let mut visited = 0;
    while let Some(id) = ready.pop_first() {
        visited += 1;
        for (target, values) in dependencies {
            if values.binary_search(&id).is_ok() {
                let count = remaining.get_mut(target).expect("known target");
                *count -= 1;
                if *count == 0 {
                    ready.insert(target.clone());
                }
            }
        }
    }
    if visited != nodes.len() {
        return Err(CoordinatorError::new(
            CoordinatorErrorKind::Cycle,
            "DAG contains a dependency cycle",
        ));
    }
    Ok(())
}

fn identity_for(spec: &DagSpec) -> String {
    let canonical = serde_json::to_vec(&spec.to_json()).expect("DAG spec is JSON serializable");
    format!("dag-v1:{}", sha256_hex(&canonical))
}

fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut data = bytes.to_vec();
    let bits = (data.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0)
    }
    data.extend_from_slice(&bits.to_be_bytes());
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in data.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes(word.try_into().expect("four-byte SHA-256 word"))
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1)
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2)
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *state = state.wrapping_add(value)
        }
    }
    h.iter().map(|value| format!("{value:08x}")).collect()
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn nonempty_diagnostic(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        })
}
