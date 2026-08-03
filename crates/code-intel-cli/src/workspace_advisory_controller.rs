use std::path::PathBuf;

use serde_json::Value;

use crate::change_agenda::{self, ChangeAgendaRequest};
use crate::change_impact::{self, ChangeImpactRequest, ImpactError};
use crate::change_risk::{self, ChangeRiskRequest, RiskError};
use crate::committed_evidence;
use crate::edit_impact::{self, EditImpactError, EditImpactRequest};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceSource {
    WorkingTree,
    GitHistory,
    StaleCommittedSnapshot,
}

/// Concrete basis for a gate-ineligible workspace answer. A workspace result
/// cannot be converted to a committed receipt: the two controllers expose
/// disjoint result types and this basis records what was actually observed.
#[derive(Clone, Debug)]
pub(crate) enum AdvisoryBasis {
    WorkingTree {
        repo_root: PathBuf,
    },
    GitHistory {
        repo_root: PathBuf,
        revspec: String,
    },
    StaleCommittedSnapshot {
        repo: String,
        run: String,
        recorded_snapshot_identity: String,
        current_snapshot_identity: Option<String>,
    },
}

impl AdvisoryBasis {
    pub(crate) fn source(&self) -> WorkspaceSource {
        match self {
            Self::WorkingTree { .. } => WorkspaceSource::WorkingTree,
            Self::GitHistory { .. } => WorkspaceSource::GitHistory,
            Self::StaleCommittedSnapshot { .. } => WorkspaceSource::StaleCommittedSnapshot,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum WorkspaceAuthority {
    Advisory(AdvisoryBasis),
}

impl WorkspaceAuthority {
    pub(crate) fn basis(&self) -> &AdvisoryBasis {
        match self {
            Self::Advisory(basis) => basis,
        }
    }
}

pub(crate) struct WorkspaceAdvisoryResult {
    value: Value,
    authority: WorkspaceAuthority,
}

impl WorkspaceAdvisoryResult {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    pub(crate) fn into_value(self) -> Value {
        self.value
    }

    pub(crate) fn authority(&self) -> &WorkspaceAuthority {
        &self.authority
    }
}

pub(crate) struct WorkspaceAdvisoryController;

impl WorkspaceAdvisoryController {
    pub(crate) fn edit_impact(
        request: EditImpactRequest,
    ) -> Result<WorkspaceAdvisoryResult, EditImpactError> {
        let result = edit_impact::execute(request)?;
        let authority = WorkspaceAuthority::Advisory(AdvisoryBasis::WorkingTree {
            repo_root: result.repo_root().to_path_buf(),
        });
        Ok(WorkspaceAdvisoryResult {
            value: result.value().clone(),
            authority,
        })
    }

    pub(crate) fn change_risk(
        request: ChangeRiskRequest,
    ) -> Result<WorkspaceAdvisoryResult, RiskError> {
        let result = change_risk::execute_request(request)?;
        let authority = WorkspaceAuthority::Advisory(AdvisoryBasis::GitHistory {
            repo_root: result.repo_root().to_path_buf(),
            revspec: result.revspec().to_string(),
        });
        Ok(WorkspaceAdvisoryResult {
            value: result.value().clone(),
            authority,
        })
    }

    /// Same git-history basis as [`Self::change_risk`]: the agenda is
    /// derived from commit history alone, so it carries advisory authority
    /// and can never be promoted into a committed receipt (issue #150).
    pub(crate) fn change_agenda(
        request: ChangeAgendaRequest,
    ) -> Result<WorkspaceAdvisoryResult, RiskError> {
        let result = change_agenda::execute_request(request)?;
        let authority = WorkspaceAuthority::Advisory(AdvisoryBasis::GitHistory {
            repo_root: result.repo_root().to_path_buf(),
            revspec: result.revspec().to_string(),
        });
        Ok(WorkspaceAdvisoryResult {
            value: result.value().clone(),
            authority,
        })
    }

    pub(crate) fn stale_committed_impact(
        request: ChangeImpactRequest,
    ) -> Result<WorkspaceAdvisoryResult, ImpactError> {
        let evidence = committed_evidence::load(&request.artifact_root, &request.repo)
            .map_err(change_impact::map_evidence)?;
        let result = change_impact::execute_stale_advisory(request, &evidence)?;
        let freshness = &result.value()["freshness"];
        let authority = WorkspaceAuthority::Advisory(AdvisoryBasis::StaleCommittedSnapshot {
            repo: result.value()["repo"].as_str().unwrap_or("").to_string(),
            run: result.value()["run"].as_str().unwrap_or("").to_string(),
            recorded_snapshot_identity: freshness["recordedIdentity"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            current_snapshot_identity: freshness["currentIdentity"].as_str().map(str::to_string),
        });
        Ok(WorkspaceAdvisoryResult {
            value: result.into_value(),
            authority,
        })
    }
}
