use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::change_impact::{self, ChangeImpactRequest, ChangeImpactResult, ImpactError};
use crate::committed_evidence::{self, CommittedEvidence, EvidenceError};
use crate::evidence_query::{self, EvidenceQueryRequest, EvidenceQueryResult, QueryError};

/// Identity receipt for an admitted A08 repository iteration. There is no
/// boolean authority switch: callers receive this only after the completed
/// index entry, every Artifact Ref, and repository.iteration provenance have
/// been reverified from the publication root.
#[derive(Clone, Debug)]
pub(crate) struct CommittedReceipt {
    repo: String,
    run: String,
    run_identity: String,
    snapshot_identity: String,
    provenance_ref: Value,
}

impl CommittedReceipt {
    pub(crate) fn repo(&self) -> &str {
        &self.repo
    }

    pub(crate) fn run(&self) -> &str {
        &self.run
    }

    pub(crate) fn run_identity(&self) -> &str {
        &self.run_identity
    }

    pub(crate) fn snapshot_identity(&self) -> &str {
        &self.snapshot_identity
    }

    pub(crate) fn provenance_ref(&self) -> &Value {
        &self.provenance_ref
    }

    /// Test-only constructor: builds a receipt without a completed index.
    /// Production code must obtain receipts through the controller open path.
    #[cfg(test)]
    pub(crate) fn for_test(
        repo: impl Into<String>,
        run: impl Into<String>,
        run_identity: impl Into<String>,
        snapshot_identity: impl Into<String>,
    ) -> Self {
        Self {
            repo: repo.into(),
            run: run.into(),
            run_identity: run_identity.into(),
            snapshot_identity: snapshot_identity.into(),
            provenance_ref: Value::Null,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum CommittedAuthority {
    Receipt(CommittedReceipt),
}

impl CommittedAuthority {
    pub(crate) fn receipt(&self) -> &CommittedReceipt {
        match self {
            Self::Receipt(receipt) => receipt,
        }
    }
}

pub(crate) struct CommittedQueryResult {
    result: EvidenceQueryResult,
    authority: CommittedAuthority,
}

impl CommittedQueryResult {
    pub(crate) fn value(&self) -> &Value {
        self.result.value()
    }

    pub(crate) fn authority(&self) -> &CommittedAuthority {
        &self.authority
    }
}

pub(crate) struct CommittedImpactResult {
    result: ChangeImpactResult,
    authority: CommittedAuthority,
}

impl CommittedImpactResult {
    pub(crate) fn into_value(self) -> Value {
        self.result.into_value()
    }

    pub(crate) fn authority(&self) -> &CommittedAuthority {
        &self.authority
    }
}

pub(crate) struct FreshnessRequest {
    pub(crate) artifact_root: PathBuf,
    pub(crate) repo: String,
    pub(crate) repo_path: Option<PathBuf>,
}

pub(crate) struct FreshnessResult {
    pub(crate) value: Value,
    pub(crate) authority: CommittedAuthority,
}

pub(crate) struct ResumeInspectionRequest {
    pub(crate) artifact_root: PathBuf,
    pub(crate) repo: String,
}

pub(crate) struct ResumeInspection {
    receipt: CommittedReceipt,
    run_root: PathBuf,
    report_path: PathBuf,
    report: Value,
    hospital_path: Option<PathBuf>,
    hospital: Value,
    verified_paths: Vec<PathBuf>,
}

impl ResumeInspection {
    pub(crate) fn receipt(&self) -> &CommittedReceipt {
        &self.receipt
    }

    pub(crate) fn run_root(&self) -> &Path {
        &self.run_root
    }

    pub(crate) fn report_path(&self) -> &Path {
        &self.report_path
    }

    pub(crate) fn report(&self) -> &Value {
        &self.report
    }

    pub(crate) fn hospital_path(&self) -> Option<&Path> {
        self.hospital_path.as_deref()
    }

    pub(crate) fn hospital(&self) -> &Value {
        &self.hospital
    }

    pub(crate) fn verified_path(&self, file_name: &str) -> Option<PathBuf> {
        self.verified_paths
            .iter()
            .find(|path| path.file_name().and_then(|name| name.to_str()) == Some(file_name))
            .cloned()
    }
}

pub(crate) struct CommittedEvidenceController;

impl CommittedEvidenceController {
    pub(crate) fn query(request: EvidenceQueryRequest) -> Result<CommittedQueryResult, QueryError> {
        let (evidence, authority) = Self::open(&request.artifact_root, &request.repo)
            .map_err(evidence_query::map_evidence_error)?;
        Self::query_opened(request, evidence, authority)
    }

    pub(crate) fn query_opened(
        request: EvidenceQueryRequest,
        evidence: CommittedEvidence,
        authority: CommittedAuthority,
    ) -> Result<CommittedQueryResult, QueryError> {
        let result = evidence_query::execute(request, &evidence)?;
        Ok(CommittedQueryResult { result, authority })
    }

    pub(crate) fn change_impact(
        request: ChangeImpactRequest,
    ) -> Result<CommittedImpactResult, ImpactError> {
        let (evidence, authority) = Self::open(&request.artifact_root, &request.repo)
            .map_err(change_impact::map_evidence)?;
        let result = change_impact::execute_committed(request, &evidence)?;
        Ok(CommittedImpactResult { result, authority })
    }

    pub(crate) fn freshness(request: FreshnessRequest) -> Result<FreshnessResult, EvidenceError> {
        let (evidence, authority) = Self::open(&request.artifact_root, &request.repo)?;
        let value = evidence.freshness(request.repo_path.as_deref())?;
        Ok(FreshnessResult { value, authority })
    }

    pub(crate) fn resume_inspection(
        request: ResumeInspectionRequest,
    ) -> Result<ResumeInspection, EvidenceError> {
        let (evidence, authority) = Self::open(&request.artifact_root, &request.repo)?;
        let receipt = authority.receipt().clone();
        let mut verified_paths = Vec::new();
        let mut report = None;
        let mut hospital = None;
        for (reference, verified) in evidence.refs.iter().zip(evidence.verified.iter()) {
            let Some(relative) = reference["path"].as_str() else {
                return Err(EvidenceError::Contract(
                    "verified Artifact Ref has no portable path".into(),
                ));
            };
            let path = evidence.run_root.join(relative);
            verified_paths.push(path.clone());
            match Path::new(relative)
                .file_name()
                .and_then(|name| name.to_str())
            {
                Some("report.json") => {
                    report = Some((path, parse_verified_json(verified.bytes(), "report.json")?));
                }
                Some("hospital-report.json") => {
                    hospital = Some((
                        path,
                        parse_verified_json(verified.bytes(), "hospital-report.json")?,
                    ));
                }
                _ => {}
            }
        }
        let (report_path, report) = report.ok_or_else(|| {
            EvidenceError::Contract(
                "completed run has no verified report.json Artifact Ref for resume".into(),
            )
        })?;
        let (hospital_path, hospital) = hospital
            .map(|(path, value)| (Some(path), value))
            .unwrap_or((None, Value::Null));
        Ok(ResumeInspection {
            receipt,
            run_root: evidence.run_root,
            report_path,
            report,
            hospital_path,
            hospital,
            verified_paths,
        })
    }

    pub(crate) fn open(
        artifact_root: &Path,
        repo: &str,
    ) -> Result<(CommittedEvidence, CommittedAuthority), EvidenceError> {
        let evidence = committed_evidence::load(artifact_root, repo)?;
        if evidence.entry["outcome"] != "completed" {
            return Err(EvidenceError::Contract(
                "committed evidence controller accepts only completed runs".into(),
            ));
        }
        let provenance_ref = evidence
            .refs
            .iter()
            .find(|reference| reference["type"] == "repository.iteration")
            .cloned()
            .ok_or_else(|| {
                EvidenceError::Contract(
                    "completed run lacks reverified repository.iteration provenance".into(),
                )
            })?;
        let receipt = CommittedReceipt {
            repo: evidence.entry["repo"]
                .as_str()
                .expect("A08 entry repo")
                .to_string(),
            run: evidence.entry["run"]
                .as_str()
                .expect("A08 entry run")
                .to_string(),
            run_identity: evidence.entry["runIdentity"]
                .as_str()
                .expect("A08 entry run identity")
                .to_string(),
            snapshot_identity: evidence.snapshot_identity().to_string(),
            provenance_ref,
        };
        Ok((evidence, CommittedAuthority::Receipt(receipt)))
    }
}

fn parse_verified_json(bytes: &[u8], name: &str) -> Result<Value, EvidenceError> {
    serde_json::from_slice(bytes)
        .map_err(|_| EvidenceError::Contract(format!("verified {name} is invalid JSON")))
}
