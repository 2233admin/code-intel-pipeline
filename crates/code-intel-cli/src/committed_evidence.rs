use std::path::Path;

use serde_json::{json, Value};

use crate::artifact_index;
use crate::artifact_ref::{self, VerifiedArtifact};
use crate::snapshot;

pub(crate) struct CommittedEvidence {
    pub(crate) entry: Value,
    pub(crate) refs: Vec<Value>,
    pub(crate) verified: Vec<VerifiedArtifact>,
}

pub(crate) fn load(artifact_root: &Path, repo: &str) -> Result<CommittedEvidence, EvidenceError> {
    let index = artifact_index::rebuild(artifact_root).map_err(|error| match error {
        artifact_index::IndexError::Contract(message) => EvidenceError::Contract(message),
        artifact_index::IndexError::HostIo(message) => EvidenceError::HostIo(message),
    })?;
    let entry = index["entries"]
        .as_array()
        .and_then(|entries| entries.iter().find(|entry| entry["repo"] == repo))
        .cloned()
        .ok_or_else(|| {
            EvidenceError::Contract(format!(
                "no committed authoritative run is indexed for repository: {repo}"
            ))
        })?;
    let run = entry["run"].as_str().expect("A08 entry run");
    let run_root = artifact_root.join(repo).join(run);
    let snapshot_identity = entry["snapshotIdentity"]
        .as_str()
        .expect("A08 entry snapshot identity");
    let refs = entry["artifactRefs"]
        .as_array()
        .expect("A08 entry Artifact Refs")
        .clone();
    let verified = artifact_ref::verify_inputs(
        &Value::Array(refs.clone()),
        Some(&run_root),
        snapshot_identity,
    )
    .map_err(|error| match error {
        artifact_ref::ArtifactError::Contract(message) => EvidenceError::Contract(message),
        artifact_ref::ArtifactError::Io(message) => EvidenceError::HostIo(message),
    })?;
    Ok(CommittedEvidence {
        entry,
        refs,
        verified,
    })
}

impl CommittedEvidence {
    pub(crate) fn snapshot_identity(&self) -> &str {
        self.entry["snapshotIdentity"]
            .as_str()
            .expect("A08 entry snapshot identity")
    }

    pub(crate) fn artifact(&self, artifact_type: &str) -> Option<(&Value, &VerifiedArtifact)> {
        self.refs
            .iter()
            .zip(self.verified.iter())
            .find(|(artifact, _)| artifact["type"] == artifact_type)
    }

    pub(crate) fn freshness(&self, repo_path: Option<&Path>) -> Result<Value, EvidenceError> {
        let recorded_identity = self.snapshot_identity();
        let Some(repo_path) = repo_path else {
            return Ok(json!({
                "status":"unknown",
                "recordedIdentity":recorded_identity,
                "currentIdentity":Value::Null,
                "workingTreePolicy":Value::Null,
                "scope":[],
            }));
        };
        let snapshot_bytes = self
            .artifact("repository.snapshot")
            .map(|(_, verified)| verified.bytes())
            .ok_or_else(|| {
                EvidenceError::Contract(
                    "freshness evaluation requires a committed repository snapshot artifact".into(),
                )
            })?;
        let recorded: Value = serde_json::from_slice(snapshot_bytes).map_err(|_| {
            EvidenceError::Contract("repository snapshot artifact is invalid JSON".into())
        })?;
        let policy = recorded["snapshot"]["workingTreePolicy"]
            .as_str()
            .expect("validated A02 working tree policy");
        let scopes = recorded["snapshot"]["scope"]
            .as_array()
            .expect("validated A02 scope")
            .iter()
            .map(|scope| scope.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        let current = snapshot::build_for_dag(repo_path, policy, &scopes).map_err(|error| {
            EvidenceError::Contract(format!("evaluate current snapshot: {error}"))
        })?;
        let current_identity = current["snapshot"]["identity"]
            .as_str()
            .expect("A02 current snapshot identity");
        Ok(json!({
            "status":if current_identity == recorded_identity { "current" } else { "stale" },
            "recordedIdentity":recorded_identity,
            "currentIdentity":current_identity,
            "workingTreePolicy":policy,
            "scope":scopes,
        }))
    }
}

pub(crate) enum EvidenceError {
    Contract(String),
    HostIo(String),
}
