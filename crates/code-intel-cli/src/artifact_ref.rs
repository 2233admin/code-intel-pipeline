use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use serde_json::{json, Value};

#[path = "content_contract.rs"]
mod content_contract;
#[path = "design_proposal_contract.rs"]
pub(crate) mod design_proposal_contract;

use design_proposal_contract::{
    validate_candidate_payload, validate_context_payload, validate_proposal_payload,
};
use crate::stable_artifact::{self, FileId, StableReadError};
use content_contract::{
    is_digest as valid_digest, is_run_identity as valid_run_identity, reject_duplicate_json_keys,
    require_exact_keys, sha256_hex, validate_artifact_ref_shape,
};

const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) const REPOSITORY_ITERATION_SCHEMA: &str =
    "code-intel-repository-iteration-provenance.v1";
pub(crate) const REPOSITORY_ITERATION_TYPE: &str = "repository.iteration";
pub(crate) const REPOSITORY_ITERATION_PURPOSE: &str = "repository_intelligence_iteration";
pub(crate) const REPOSITORY_ITERATION_PRODUCER_COMPONENT: &str = "code-intel.authoritative-run";
pub(crate) const REPOSITORY_ITERATION_PRODUCER_CONTRACT: &str = "repository-iteration-producer";
pub(crate) const REPOSITORY_ITERATION_PRODUCER_VERSION: &str = "1";

#[derive(Clone, Copy)]
pub(crate) struct ArtifactContract {
    pub(crate) artifact_schema: &'static str,
    pub(crate) artifact_type: &'static str,
    pub(crate) max_bytes: u64,
    pub(crate) validate_payload: fn(&[u8]) -> Result<(), String>,
}

pub(crate) struct VerifiedArtifact {
    bytes: Vec<u8>,
    artifact_schema: String,
    artifact_type: String,
    sha256: String,
    consumed_snapshot_identity: String,
    stable_file_id: FileId,
}

impl VerifiedArtifact {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

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

#[derive(Debug)]
pub(crate) enum ArtifactError {
    Contract(String),
    Io(String),
}

impl ArtifactError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Contract(message) | Self::Io(message) => message,
        }
    }
}

pub(crate) fn verify_inputs(
    inputs: &Value,
    artifact_root: Option<&Path>,
    expected_snapshot_identity: &str,
) -> Result<Vec<VerifiedArtifact>, ArtifactError> {
    let inputs = inputs
        .as_array()
        .ok_or_else(|| ArtifactError::Contract("request inputs must be an array".to_string()))?;
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let root = artifact_root.ok_or_else(|| {
        ArtifactError::Contract(
            "request with Artifact Ref inputs requires an explicit --artifact-root".to_string(),
        )
    })?;
    let mut paths = BTreeSet::new();
    let mut identities = BTreeSet::new();
    let mut preflight = Vec::with_capacity(inputs.len());
    for artifact in inputs {
        validate_artifact_ref_shape(artifact).map_err(ArtifactError::Contract)?;
        let path = artifact.get("path").and_then(Value::as_str).unwrap_or("");
        let canonical_path = portable_relative_path(path)?;
        if !paths.insert(canonical_path.to_lowercase()) {
            return Err(ArtifactError::Contract(
                "Artifact Ref inputs contain duplicate or case-colliding paths".to_string(),
            ));
        }
        let digest = artifact.get("sha256").and_then(Value::as_str).unwrap_or("");
        if !identities.insert((digest.to_string(), canonical_path.clone())) {
            return Err(ArtifactError::Contract(
                "Artifact Ref inputs contain duplicate identities".to_string(),
            ));
        }
        let contract = registered_contract(artifact)?;
        validate_preflight_contract(artifact, expected_snapshot_identity, contract)?;
        preflight.push((artifact, contract));
    }

    let mut stable_files = BTreeSet::new();
    let mut verified = Vec::with_capacity(preflight.len());
    for (artifact, contract) in preflight {
        let item = verify_artifact_ref(root, expected_snapshot_identity, contract, artifact)?;
        if !stable_files.insert(item.stable_file_id) {
            return Err(ArtifactError::Contract(
                "Artifact Ref inputs alias the same stable file identity".to_string(),
            ));
        }
        verified.push(item);
    }
    Ok(verified)
}

fn validate_preflight_contract(
    artifact: &Value,
    expected_snapshot_identity: &str,
    expected_contract: ArtifactContract,
) -> Result<(), ArtifactError> {
    if artifact["artifactSchema"] != expected_contract.artifact_schema
        || artifact["type"] != expected_contract.artifact_type
    {
        return Err(ArtifactError::Contract(
            "Artifact Ref schema/type differs from the expected input contract".to_string(),
        ));
    }
    let consumed = artifact["consumedSnapshotIdentity"]
        .as_str()
        .ok_or_else(|| {
            ArtifactError::Contract(
                "capability input Artifact Ref requires consumedSnapshotIdentity".to_string(),
            )
        })?;
    if consumed != expected_snapshot_identity {
        return Err(ArtifactError::Contract(
            "Artifact Ref consumed snapshot identity mismatch".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn verify_artifact_ref(
    root_authority: &Path,
    expected_snapshot_identity: &str,
    expected_contract: ArtifactContract,
    artifact: &Value,
) -> Result<VerifiedArtifact, ArtifactError> {
    validate_artifact_ref_shape(artifact).map_err(ArtifactError::Contract)?;
    validate_preflight_contract(artifact, expected_snapshot_identity, expected_contract)?;
    let consumed = artifact["consumedSnapshotIdentity"]
        .as_str()
        .expect("preflight validated snapshot identity");
    let relative = portable_relative_path(artifact["path"].as_str().expect("validated path"))?;
    let components = relative.split('/').collect::<Vec<_>>();
    let stable =
        stable_artifact::read_beneath(root_authority, &components, expected_contract.max_bytes)
            .map_err(|error| match error {
                StableReadError::HostIo(message) => ArtifactError::Io(message),
                StableReadError::TooLarge(message)
                | StableReadError::Boundary(message)
                | StableReadError::Identity(message) => ArtifactError::Contract(message),
            })?;
    let bytes = stable.bytes;
    let actual = sha256_hex(&bytes);
    let expected = artifact["sha256"].as_str().expect("validated digest");
    if actual != expected {
        return Err(ArtifactError::Contract(
            "Artifact Ref payload SHA-256 mismatch".to_string(),
        ));
    }
    (expected_contract.validate_payload)(&bytes).map_err(ArtifactError::Contract)?;
    Ok(VerifiedArtifact {
        bytes,
        artifact_schema: expected_contract.artifact_schema.to_string(),
        artifact_type: expected_contract.artifact_type.to_string(),
        sha256: actual,
        consumed_snapshot_identity: consumed.to_string(),
        stable_file_id: stable.id,
    })
}

pub(crate) fn registered_contract(artifact: &Value) -> Result<ArtifactContract, ArtifactError> {
    let (Some(schema), Some(artifact_type)) = (
        artifact.get("artifactSchema").and_then(Value::as_str),
        artifact.get("type").and_then(Value::as_str),
    ) else {
        return Err(unregistered_contract_error());
    };
    repository_family_contract(schema, artifact_type)
        .or_else(|| diagnosis_family_contract(schema, artifact_type))
        .or_else(|| orientation_family_contract(schema, artifact_type))
        .or_else(|| advisory_family_contract(schema, artifact_type))
        .or_else(|| retirement_family_contract(schema, artifact_type))
        .or_else(|| run_delivery_family_contract(schema, artifact_type))
        .or_else(|| method_decision_family_contract(schema, artifact_type))
        .or_else(|| {
            native_code_contract(schema, artifact_type).map(
                |(artifact_schema, artifact_type, validate_payload)| ArtifactContract {
                    artifact_schema,
                    artifact_type,
                    max_bytes: MAX_ARTIFACT_BYTES,
                    validate_payload,
                },
            )
        })
        .ok_or_else(unregistered_contract_error)
}

fn unregistered_contract_error() -> ArtifactError {
    ArtifactError::Contract(
        "Artifact Ref schema/type is not registered for capability input consumption".to_string(),
    )
}

fn repository_family_contract(schema: &str, artifact_type: &str) -> Option<ArtifactContract> {
    match (schema, artifact_type) {
        (REPOSITORY_ITERATION_SCHEMA, REPOSITORY_ITERATION_TYPE) => Some(ArtifactContract {
            artifact_schema: REPOSITORY_ITERATION_SCHEMA,
            artifact_type: REPOSITORY_ITERATION_TYPE,
            max_bytes: 64 * 1024,
            validate_payload: validate_repository_iteration_provenance,
        }),
        ("code-intel-file-inventory.v1", "inventory.files") => Some(ArtifactContract {
            artifact_schema: "code-intel-file-inventory.v1",
            artifact_type: "inventory.files",
            max_bytes: MAX_ARTIFACT_BYTES,
            validate_payload: validate_inventory,
        }),
        ("code-intel-repository-snapshot.v1", "repository.snapshot") => Some(ArtifactContract {
            artifact_schema: "code-intel-repository-snapshot.v1",
            artifact_type: "repository.snapshot",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_repository_snapshot,
        }),
        _ => None,
    }
}

fn diagnosis_family_contract(schema: &str, artifact_type: &str) -> Option<ArtifactContract> {
    match (schema, artifact_type) {
        ("code-intel-doctor-observation.v1", "doctor.observation") => Some(ArtifactContract {
            artifact_schema: "code-intel-doctor-observation.v1",
            artifact_type: "doctor.observation",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_doctor_observation,
        }),
        ("code-intel-repository-survival-scan-result.v1", "repository.survival-scan") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-repository-survival-scan-result.v1",
                artifact_type: "repository.survival-scan",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_survival_scan,
            })
        }
        ("code-intel-evidence-admissibility-result.v1", "evidence.admission") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-evidence-admissibility-result.v1",
                artifact_type: "evidence.admission",
                max_bytes: 16 * 1024 * 1024,
                validate_payload: validate_evidence_admission,
            })
        }
        ("code-intel-evidence-payload.v1", "observed.evidence.payload") => Some(ArtifactContract {
            artifact_schema: "code-intel-evidence-payload.v1",
            artifact_type: "observed.evidence.payload",
            max_bytes: 64 * 1024 * 1024,
            validate_payload: validate_evidence_payload,
        }),
        ("code-intel-sentrux-command-observation.v1", "provider.sentrux.command-observation") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-sentrux-command-observation.v1",
                artifact_type: "provider.sentrux.command-observation",
                max_bytes: 2 * 1024 * 1024,
                validate_payload: validate_sentrux_command_observation,
            })
        }
        ("code-intel-sentrux-capability-artifact.v1", "provider.sentrux.capability-artifact") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-sentrux-capability-artifact.v1",
                artifact_type: "provider.sentrux.capability-artifact",
                // Was 8 MiB until issue #383/#386: `outputs.structuredData`
                // now legitimately carries a capability's full output (see
                // `content_contract::MAX_JSON_BYTES`'s comment for the
                // measured ~8.65 MiB `sentrux.dsm` artifact this repository
                // already produces). Kept under that 24 MiB scanner ceiling
                // so this schema's own message is the one a caller sees.
                max_bytes: 20 * 1024 * 1024,
                validate_payload: validate_sentrux_capability_artifact,
            })
        }
        ("code-intel-hospital.v1", "diagnosis.hospital") => Some(ArtifactContract {
            artifact_schema: "code-intel-hospital.v1",
            artifact_type: "diagnosis.hospital",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_hospital_report,
        }),
        ("code-intel-audit-report.v1", "diagnosis.audit") => Some(ArtifactContract {
            artifact_schema: "code-intel-audit-report.v1",
            artifact_type: "diagnosis.audit",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_audit_report,
        }),
        ("code-intel-hospital-markdown.v1", "diagnosis.hospital-view") => Some(ArtifactContract {
            artifact_schema: "code-intel-hospital-markdown.v1",
            artifact_type: "diagnosis.hospital-view",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_hospital_markdown,
        }),
        ("code-intel-surgery-plan.v1", "diagnosis.surgery-plan") => Some(ArtifactContract {
            artifact_schema: "code-intel-surgery-plan.v1",
            artifact_type: "diagnosis.surgery-plan",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_surgery_plan,
        }),
        ("code-intel-surgery-plan-markdown.v1", "diagnosis.surgery-plan-view") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-surgery-plan-markdown.v1",
                artifact_type: "diagnosis.surgery-plan-view",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_surgery_markdown,
            })
        }
        _ => None,
    }
}

fn advisory_family_contract(schema: &str, artifact_type: &str) -> Option<ArtifactContract> {
    match (schema, artifact_type) {
        ("code-intel-advisory-workflow-recommendation.v1", "advisory.workflow-recommendation") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-advisory-workflow-recommendation.v1",
                artifact_type: "advisory.workflow-recommendation",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_workflow_recommendation,
            })
        }
        ("code-intel-design-context.v1", "design.context") => Some(ArtifactContract {
            artifact_schema: "code-intel-design-context.v1",
            artifact_type: "design.context",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_context_payload,
        }),
        ("code-intel-design-proposal-candidate.v1", "design.proposal-candidate") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-design-proposal-candidate.v1",
                artifact_type: "design.proposal-candidate",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_candidate_payload,
            })
        }
        ("code-intel-design-proposal.v1", "design.proposal") => Some(ArtifactContract {
            artifact_schema: "code-intel-design-proposal.v1",
            artifact_type: "design.proposal",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_proposal_payload,
        }),
        _ => None,
    }
}

fn orientation_family_contract(schema: &str, artifact_type: &str) -> Option<ArtifactContract> {
    match (schema, artifact_type) {
        ("code-intel-project-orientation.v1", "project.orientation") => Some(ArtifactContract {
            artifact_schema: "code-intel-project-orientation.v1",
            artifact_type: "project.orientation",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_project_orientation,
        }),
        ("code-intel-understanding-quadrant.v1", "understanding.quadrant") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-understanding-quadrant.v1",
                artifact_type: "understanding.quadrant",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_understanding_quadrant,
            })
        }
        (
            "code-intel-project-orientation-benchmark-observations.v1",
            "benchmark.orientation-observations",
        ) => Some(ArtifactContract {
            artifact_schema: "code-intel-project-orientation-benchmark-observations.v1",
            artifact_type: "benchmark.orientation-observations",
            max_bytes: 64 * 1024 * 1024,
            validate_payload: validate_orientation_benchmark_observations,
        }),
        ("code-intel-project-orientation-benchmark.v1", "benchmark.orientation-report") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-project-orientation-benchmark.v1",
                artifact_type: "benchmark.orientation-report",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_orientation_benchmark_report,
            })
        }
        (
            "code-intel-project-orientation-benchmark-markdown.v1",
            "benchmark.orientation-report-view",
        ) => Some(ArtifactContract {
            artifact_schema: "code-intel-project-orientation-benchmark-markdown.v1",
            artifact_type: "benchmark.orientation-report-view",
            max_bytes: 1024 * 1024,
            validate_payload: validate_orientation_benchmark_markdown,
        }),
        _ => None,
    }
}

fn retirement_family_contract(schema: &str, artifact_type: &str) -> Option<ArtifactContract> {
    match (schema, artifact_type) {
        (
            "code-intel-compatibility-retirement-manifest.v1",
            "compatibility.retirement-manifest",
        ) => Some(ArtifactContract {
            artifact_schema: "code-intel-compatibility-retirement-manifest.v1",
            artifact_type: "compatibility.retirement-manifest",
            max_bytes: 4 * 1024 * 1024,
            validate_payload: validate_retirement_manifest,
        }),
        (
            "code-intel-compatibility-retirement-evidence.v1",
            "compatibility.retirement-evidence",
        ) => Some(ArtifactContract {
            artifact_schema: "code-intel-compatibility-retirement-evidence.v1",
            artifact_type: "compatibility.retirement-evidence",
            max_bytes: 4 * 1024 * 1024,
            validate_payload: validate_retirement_evidence,
        }),
        (
            "code-intel-compatibility-retirement-decision.v1",
            "compatibility.retirement-decision",
        ) => Some(ArtifactContract {
            artifact_schema: "code-intel-compatibility-retirement-decision.v1",
            artifact_type: "compatibility.retirement-decision",
            max_bytes: 4 * 1024 * 1024,
            validate_payload: validate_retirement_decision,
        }),
        (
            "code-intel-compatibility-retirement-ticket-template.v1",
            "compatibility.retirement-ticket-template",
        ) => Some(ArtifactContract {
            artifact_schema: "code-intel-compatibility-retirement-ticket-template.v1",
            artifact_type: "compatibility.retirement-ticket-template",
            max_bytes: 4 * 1024 * 1024,
            validate_payload: validate_retirement_ticket_template,
        }),
        (
            "code-intel-compatibility-retirement-deletion-diff.v1",
            "compatibility.retirement-deletion-diff",
        ) => Some(ArtifactContract {
            artifact_schema: "code-intel-compatibility-retirement-deletion-diff.v1",
            artifact_type: "compatibility.retirement-deletion-diff",
            max_bytes: 4 * 1024 * 1024,
            validate_payload: validate_retirement_deletion_diff,
        }),
        _ => None,
    }
}

fn run_delivery_family_contract(schema: &str, artifact_type: &str) -> Option<ArtifactContract> {
    match (schema, artifact_type) {
        ("code-intel-run-timing-events.v1", "delivery.run-timing-events") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-run-timing-events.v1",
                artifact_type: "delivery.run-timing-events",
                max_bytes: 64 * 1024 * 1024,
                validate_payload: validate_run_timing_events,
            })
        }
        ("code-intel-run-commit.v1", "run.commit") => Some(ArtifactContract {
            artifact_schema: "code-intel-run-commit.v1",
            artifact_type: "run.commit",
            max_bytes: 64 * 1024,
            validate_payload: validate_run_commit,
        }),
        ("code-intel-run-manifest.v1", "run.manifest") => Some(ArtifactContract {
            artifact_schema: "code-intel-run-manifest.v1",
            artifact_type: "run.manifest",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_run_manifest,
        }),
        ("code-intel-session-evidence.v1", "verification.session-evidence") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-session-evidence.v1",
                artifact_type: "verification.session-evidence",
                max_bytes: 128 * 1024 * 1024,
                validate_payload: validate_session_evidence,
            })
        }
        ("code-intel-anchor-verification.v1", "verification.anchors") => Some(ArtifactContract {
            artifact_schema: "code-intel-anchor-verification.v1",
            artifact_type: "verification.anchors",
            max_bytes: MAX_ARTIFACT_BYTES,
            validate_payload: validate_anchor_verification,
        }),
        ("code-intel-delivery-light-speed.v1", "delivery.light-speed-report") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-delivery-light-speed.v1",
                artifact_type: "delivery.light-speed-report",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_light_speed_report,
            })
        }
        ("code-intel-delivery-light-speed-markdown.v1", "delivery.light-speed-report-view") => {
            Some(ArtifactContract {
                artifact_schema: "code-intel-delivery-light-speed-markdown.v1",
                artifact_type: "delivery.light-speed-report-view",
                max_bytes: 1024 * 1024,
                validate_payload: validate_light_speed_markdown,
            })
        }
        _ => None,
    }
}

fn method_decision_family_contract(schema: &str, artifact_type: &str) -> Option<ArtifactContract> {
    match (schema, artifact_type) {
        ("code-intel-method-catalog.v1", "method.catalog") => Some(ArtifactContract {
            artifact_schema: "code-intel-method-catalog.v1",
            artifact_type: "method.catalog",
            max_bytes: 256 * 1024,
            validate_payload: validate_method_catalog,
        }),
        ("code-intel-method-card.v1", "method.card") => Some(ArtifactContract {
            artifact_schema: "code-intel-method-card.v1",
            artifact_type: "method.card",
            max_bytes: 256 * 1024,
            validate_payload: validate_method_card,
        }),
        ("code-intel-decision-record.v1", "decision.record") => Some(ArtifactContract {
            artifact_schema: "code-intel-decision-record.v1",
            artifact_type: "decision.record",
            max_bytes: 1024 * 1024,
            validate_payload: validate_decision_record_schema,
        }),
        _ => None,
    }
}

fn validate_evidence_payload(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "observed evidence payload")?;
    exact_object_keys(&value, &["schema", "data"], "observed evidence payload")?;
    if value["schema"] != "code-intel-evidence-payload.v1"
        || value["data"].as_object().is_none_or(|data| data.is_empty())
    {
        return Err("observed evidence payload contract is invalid".into());
    }
    Ok(())
}

fn validate_sentrux_command_observation(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "Sentrux command observation")?;
    exact_object_keys(
        &value,
        &["schema", "snapshotIdentity", "commands"],
        "Sentrux command observation",
    )?;
    if value["schema"] != "code-intel-sentrux-command-observation.v1"
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
    {
        return Err("Sentrux command observation header is invalid".into());
    }
    let commands = value["commands"]
        .as_array()
        .filter(|commands| commands.len() == 2)
        .ok_or("Sentrux command observation must contain gate and check")?;
    let mut seen = BTreeSet::new();
    for command in commands {
        let mut expected_fields = vec!["id", "argv", "exitCode", "success", "stdout", "stderr"];
        if command.get("structuredData").is_some() {
            expected_fields.push("structuredData");
        }
        exact_object_keys(command, &expected_fields, "Sentrux command result")?;
        let id = command["id"]
            .as_str()
            .filter(|id| matches!(*id, "gate" | "check"))
            .ok_or("Sentrux command id is invalid")?;
        if !seen.insert(id) {
            return Err("Sentrux command ids must be unique".into());
        }
        if !sentrux_command_result_is_valid(command, id) {
            return Err("Sentrux command result is invalid".into());
        }
    }
    Ok(())
}

fn sentrux_command_result_is_valid(command: &Value, id: &str) -> bool {
    let known_argv = command["argv"] == json!(["sentrux", id, "."])
        || command["argv"] == json!(["code-intel", "sentrux", id, "."]);
    let exit_code_ok = command["exitCode"].is_null() || command["exitCode"].as_i64().is_some();
    let structured_data_ok = command
        .get("structuredData")
        .is_none_or(|value| value.is_null() || value.is_object() || value.is_array());
    known_argv
        && exit_code_ok
        && command["success"].is_boolean()
        && command["stdout"].is_string()
        && command["stderr"].is_string()
        && structured_data_ok
}

fn validate_sentrux_capability_artifact(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "Sentrux capability artifact")?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "contractVersion",
            "capabilityId",
            "operation",
            "runId",
            "snapshotIdentity",
            "provider",
            "status",
            "authority",
            "inputs",
            "outputs",
            "failure",
            "freshness",
            "decisionConsumers",
        ],
        "Sentrux capability artifact",
    )?;
    if value["schema"] != "code-intel-sentrux-capability-artifact.v1"
        || value["contractVersion"] != 1
        || !value["capabilityId"]
            .as_str()
            .is_some_and(|id| !id.is_empty() && id.starts_with("sentrux."))
        || !value["operation"]
            .as_str()
            .is_some_and(|operation| !operation.is_empty())
        || !value["runId"]
            .as_str()
            .is_some_and(|run_id| !run_id.is_empty())
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || !value["provider"].is_object()
        || !matches!(
            value["status"].as_str(),
            Some(
                "succeeded" | "degraded" | "unavailable" | "skipped" | "not_applicable" | "failed"
            )
        )
        || !matches!(
            value["authority"].as_str(),
            Some("authoritative" | "fallback" | "compatibility" | "declared_only")
        )
        || !value["inputs"].is_object()
        || !value["outputs"].is_object()
        || if value["status"] == "succeeded" {
            !value["failure"].is_null()
        } else {
            !value["failure"].is_object()
        }
        || !value["freshness"].is_object()
        || !value["decisionConsumers"].is_array()
    {
        return Err("Sentrux capability artifact header or envelope fields are invalid".into());
    }
    if value["status"] == "succeeded" && value["outputs"].as_object().is_none_or(|v| v.is_empty()) {
        return Err("successful Sentrux capability artifact must contain outputs".into());
    }
    Ok(())
}

fn validate_retirement_manifest(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "retirement manifest")?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "snapshotIdentity",
            "retirementId",
            "approvalSubject",
            "independentApproval",
        ],
        "retirement manifest",
    )?;
    if value["schema"] != "code-intel-compatibility-retirement-manifest.v1"
        || !value["snapshotIdentity"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
        || !value["retirementId"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
        || !value["approvalSubject"].is_object()
        || !value["independentApproval"].is_object()
    {
        return Err("retirement manifest contract is invalid".into());
    }
    let subject = &value["approvalSubject"];
    exact_object_keys(
        subject,
        &[
            "legacyBranch",
            "replacement",
            "parity",
            "registryReconciliation",
            "compatibilityWindow",
            "rollback",
            "usageObservation",
            "necessityEvidence",
            "dependencyStates",
            "lineReductionEvidence",
        ],
        "retirement approvalSubject",
    )?;
    exact_object_keys(
        &subject["legacyBranch"],
        &[
            "capabilityId",
            "branchId",
            "callPath",
            "affectedFiles",
            "owner",
            "registryParticipantId",
        ],
        "retirement legacyBranch",
    )?;
    let branch_id = subject["legacyBranch"]["branchId"]
        .as_str()
        .ok_or("retirement legacyBranch branchId is invalid")?;
    normalized_retirement_call_path(&subject["legacyBranch"]["callPath"], branch_id)?;
    retirement_portable_paths(
        &subject["legacyBranch"]["affectedFiles"],
        "retirement legacyBranch.affectedFiles",
    )?;
    exact_object_keys(
        &subject["replacement"],
        &[
            "capabilityId",
            "implementationId",
            "dependencies",
            "atomEvidence",
        ],
        "retirement replacement",
    )?;
    exact_object_keys(
        &subject["parity"],
        &["golden", "contract", "effects"],
        "retirement parity",
    )?;
    exact_object_keys(
        &subject["rollback"],
        &["command", "executionEvidence"],
        "retirement rollback",
    )?;
    if subject["lineReductionEvidence"] != false
        || !subject["replacement"]["dependencies"].is_array()
        || !subject["dependencyStates"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        || !subject["rollback"]["command"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
    {
        return Err("retirement approval subject is invalid".into());
    }
    for reference in [
        &subject["replacement"]["atomEvidence"],
        &subject["parity"]["golden"],
        &subject["parity"]["contract"],
        &subject["parity"]["effects"],
        &subject["registryReconciliation"],
        &subject["compatibilityWindow"],
        &subject["rollback"]["executionEvidence"],
        &subject["usageObservation"],
        &subject["necessityEvidence"],
        &value["independentApproval"],
    ] {
        validate_retirement_evidence_ref(reference)?;
    }
    for reference in subject["dependencyStates"].as_array().unwrap() {
        validate_retirement_evidence_ref(reference)?;
    }
    Ok(())
}

fn validate_retirement_evidence_ref(value: &Value) -> Result<(), String> {
    exact_object_keys(
        value,
        &[
            "schema",
            "artifactSchema",
            "type",
            "path",
            "sha256",
            "consumedSnapshotIdentity",
        ],
        "retirement evidence ref",
    )?;
    if value["schema"] != "code-intel-artifact-ref.v1"
        || value["artifactSchema"] != "code-intel-compatibility-retirement-evidence.v1"
        || value["type"] != "compatibility.retirement-evidence"
        || !value["path"].as_str().is_some_and(|v| !v.is_empty())
        || !value["consumedSnapshotIdentity"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
        || !value["sha256"]
            .as_str()
            .is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("retirement evidence ref is invalid".into());
    }
    Ok(())
}

fn validate_retirement_evidence(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "retirement evidence")?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "snapshotIdentity",
            "id",
            "evidenceClass",
            "retirementId",
            "legacyBranchId",
            "replacementCapabilityId",
            "details",
        ],
        "retirement evidence",
    )?;
    const CLASSES: [&str; 11] = [
        "replacement_atom",
        "golden_parity",
        "contract_parity",
        "effect_parity",
        "registry_reconciliation",
        "compatibility_window",
        "rollback_execution",
        "usage_observation",
        "independent_approval",
        "c00_necessity",
        "dependency_approval",
    ];
    if value["schema"] != "code-intel-compatibility-retirement-evidence.v1"
        || !CLASSES.contains(&value["evidenceClass"].as_str().unwrap_or(""))
        || !value["details"].is_object()
        || [
            "snapshotIdentity",
            "id",
            "retirementId",
            "legacyBranchId",
            "replacementCapabilityId",
        ]
        .iter()
        .any(|field| !value[field].as_str().is_some_and(|v| !v.is_empty()))
    {
        return Err("retirement evidence contract is invalid".into());
    }
    Ok(())
}

fn validate_retirement_decision(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "retirement decision")?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "snapshotIdentity",
            "retirementId",
            "legacyBranch",
            "replacement",
            "approvalSubjectSha256",
            "decision",
            "blockers",
            "authorityBoundary",
            "gainLedgerProjection",
        ],
        "retirement decision",
    )?;
    if value["schema"] != "code-intel-compatibility-retirement-decision.v1"
        || !matches!(value["decision"].as_str(), Some("approved" | "blocked"))
        || value["authorityBoundary"] != "approval_only_no_deletion_authority"
        || !value["approvalSubjectSha256"]
            .as_str()
            .is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
        || !value["blockers"].is_array()
        || !value["gainLedgerProjection"].is_object()
    {
        return Err("retirement decision contract is invalid".into());
    }
    Ok(())
}

fn validate_retirement_ticket_template(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "retirement ticket template")?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "snapshotIdentity",
            "ticketId",
            "retirementId",
            "legacyBranch",
            "replacement",
            "affectedFiles",
            "evidence",
            "source",
            "owner",
            "verifier",
            "observationExpiry",
            "status",
            "authorityBoundary",
        ],
        "retirement ticket template",
    )?;
    exact_object_keys(
        &value["legacyBranch"],
        &["capabilityId", "branchId", "callPath"],
        "ticket legacyBranch",
    )?;
    exact_object_keys(
        &value["replacement"],
        &["capabilityId", "dependencies"],
        "ticket replacement",
    )?;
    exact_object_keys(
        &value["evidence"],
        &[
            "golden",
            "contract",
            "effects",
            "usage",
            "rollbackRehearsal",
            "deletionDiff",
        ],
        "ticket evidence",
    )?;
    exact_object_keys(
        &value["source"],
        &["retirementDecision", "retirementManifest"],
        "ticket source",
    )?;
    if !retirement_ticket_template_header_is_valid(&value) {
        return Err("retirement ticket template contract is invalid".into());
    }
    for key in ["capabilityId", "branchId", "callPath"] {
        if !value["legacyBranch"][key]
            .as_str()
            .is_some_and(|v| !v.is_empty())
        {
            return Err("retirement ticket legacy branch is invalid".into());
        }
    }
    if !value["replacement"]["capabilityId"]
        .as_str()
        .is_some_and(|v| !v.is_empty())
        || !closed_unique_strings(&value["replacement"]["dependencies"], false)
        || !closed_unique_strings(&value["affectedFiles"], true)
    {
        return Err("retirement ticket replacement/files are invalid".into());
    }
    for key in [
        "golden",
        "contract",
        "effects",
        "usage",
        "rollbackRehearsal",
    ] {
        validate_retirement_evidence_ref(&value["evidence"][key])?;
    }
    validate_ticket_ref(
        &value["evidence"]["deletionDiff"],
        "code-intel-compatibility-retirement-deletion-diff.v1",
        "compatibility.retirement-deletion-diff",
    )?;
    validate_ticket_ref(
        &value["source"]["retirementDecision"],
        "code-intel-compatibility-retirement-decision.v1",
        "compatibility.retirement-decision",
    )?;
    validate_ticket_ref(
        &value["source"]["retirementManifest"],
        "code-intel-compatibility-retirement-manifest.v1",
        "compatibility.retirement-manifest",
    )?;
    Ok(())
}

fn retirement_ticket_template_header_is_valid(value: &Value) -> bool {
    let nonempty_identity_fields = [
        "snapshotIdentity",
        "ticketId",
        "retirementId",
        "owner",
        "verifier",
    ]
    .iter()
    .all(|key| value[key].as_str().is_some_and(|v| !v.is_empty()));
    value["schema"] == "code-intel-compatibility-retirement-ticket-template.v1"
        && value["status"] == "draft"
        && value["authorityBoundary"] == "template_only_no_approval_or_deletion_authority"
        && nonempty_identity_fields
        && value["owner"] != value["verifier"]
        && value["observationExpiry"].as_u64().is_some()
        && value["affectedFiles"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
}

fn validate_ticket_ref(value: &Value, schema: &str, kind: &str) -> Result<(), String> {
    exact_object_keys(
        value,
        &[
            "schema",
            "artifactSchema",
            "type",
            "path",
            "sha256",
            "consumedSnapshotIdentity",
        ],
        "ticket Artifact Ref",
    )?;
    if value["schema"] != "code-intel-artifact-ref.v1"
        || value["artifactSchema"] != schema
        || value["type"] != kind
        || !value["path"].as_str().is_some_and(|v| !v.is_empty())
        || !value["consumedSnapshotIdentity"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
        || !value["sha256"]
            .as_str()
            .is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("ticket Artifact Ref is invalid".into());
    }
    Ok(())
}

fn closed_unique_strings(value: &Value, portable_paths: bool) -> bool {
    let Some(values) = value.as_array().filter(|v| !v.is_empty()) else {
        return false;
    };
    let mut seen = BTreeSet::new();
    values.iter().all(|value| {
        value.as_str().is_some_and(|text| {
            !text.is_empty()
                && (!portable_paths
                    || (!text.contains('\\')
                        && !text.starts_with('/')
                        && !text.split('/').any(|part| part == "..")))
                && seen.insert(text)
        })
    })
}

fn validate_retirement_deletion_diff(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "retirement deletion diff")?;
    validate_retirement_deletion_diff_value(&value)
}

pub(crate) fn validate_retirement_deletion_diff_value(value: &Value) -> Result<(), String> {
    exact_object_keys(
        value,
        &[
            "schema",
            "snapshotIdentity",
            "retirementId",
            "legacyBranchId",
            "affectedFiles",
            "deletionsOnly",
            "summary",
            "patch",
        ],
        "retirement deletion diff",
    )?;
    if value["schema"] != "code-intel-compatibility-retirement-deletion-diff.v1"
        || [
            "snapshotIdentity",
            "retirementId",
            "legacyBranchId",
            "summary",
        ]
        .iter()
        .any(|key| !value[key].as_str().is_some_and(|v| !v.is_empty()))
        || value["deletionsOnly"] != true
    {
        return Err("retirement deletion diff contract is invalid".into());
    }
    let affected = retirement_portable_paths(&value["affectedFiles"], "affectedFiles")?;
    let patch = &value["patch"];
    exact_object_keys(
        patch,
        &["algorithm", "sha256", "files"],
        "retirement deletion patch",
    )?;
    if patch["algorithm"] != "replayable-delete-only-v1" || !is_lower_sha(&patch["sha256"]) {
        return Err("retirement deletion patch contract is invalid".into());
    }
    let files = patch["files"]
        .as_array()
        .filter(|files| !files.is_empty())
        .ok_or("retirement deletion patch files must not be empty")?;
    let patch_sha = sha256_hex(
        &serde_json::to_vec(files).map_err(|error| format!("serialize deletion patch: {error}"))?,
    );
    if patch["sha256"] != patch_sha {
        return Err("retirement deletion patch SHA-256 mismatch".into());
    }
    let mut touched = Vec::with_capacity(files.len());
    for file in files {
        exact_object_keys(
            file,
            &[
                "path",
                "baseBlobSha256",
                "resultBlobSha256",
                "baseText",
                "resultText",
                "hunks",
            ],
            "retirement deletion patch file",
        )?;
        let path = file["path"]
            .as_str()
            .ok_or("retirement deletion patch path is invalid")?;
        validate_portable_path(path, "retirement deletion patch path")?;
        touched.push(path.to_string());
        let base = file["baseText"]
            .as_str()
            .filter(|text| !text.contains('\r'))
            .ok_or("retirement deletion baseText must use normalized LF text")?;
        let result = file["resultText"]
            .as_str()
            .filter(|text| !text.contains('\r'))
            .ok_or("retirement deletion resultText must use normalized LF text")?;
        if !is_lower_sha(&file["baseBlobSha256"])
            || !is_lower_sha(&file["resultBlobSha256"])
            || file["baseBlobSha256"] != sha256_hex(base.as_bytes())
            || file["resultBlobSha256"] != sha256_hex(result.as_bytes())
        {
            return Err("retirement deletion blob SHA-256 mismatch".into());
        }
        replay_delete_only(base, result, &file["hunks"])?;
    }
    if touched != affected {
        return Err("retirement deletion touched paths differ from affectedFiles".into());
    }
    Ok(())
}

fn replay_delete_only(base: &str, result: &str, hunks: &Value) -> Result<(), String> {
    let hunks = hunks
        .as_array()
        .filter(|hunks| !hunks.is_empty())
        .ok_or("retirement deletion patch requires at least one hunk")?;
    let base_lines = base.split('\n').collect::<Vec<_>>();
    let mut rebuilt = Vec::<&str>::new();
    let mut cursor = 0usize;
    let mut deleted_before = 0usize;
    for hunk in hunks {
        exact_object_keys(
            hunk,
            &[
                "oldStart",
                "oldLines",
                "newStart",
                "newLines",
                "deletedLines",
                "addedLines",
            ],
            "retirement deletion hunk",
        )?;
        let old_start = hunk["oldStart"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or("retirement deletion hunk oldStart is invalid")?;
        let old_lines = hunk["oldLines"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or("retirement deletion hunk oldLines is invalid")?;
        let new_start = hunk["newStart"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or("retirement deletion hunk newStart is invalid")?;
        if hunk["newLines"] != 0
            || !hunk["addedLines"]
                .as_array()
                .is_some_and(|lines| lines.is_empty())
        {
            return Err("retirement deletion patch contains added or replacement lines".into());
        }
        let deleted = hunk["deletedLines"]
            .as_array()
            .filter(|lines| lines.len() == old_lines)
            .ok_or("retirement deletion hunk line count mismatch")?;
        let start = old_start - 1;
        if start < cursor
            || start + old_lines > base_lines.len()
            || new_start != old_start.saturating_sub(deleted_before)
        {
            return Err("retirement deletion hunks overlap or use invalid coordinates".into());
        }
        rebuilt.extend_from_slice(&base_lines[cursor..start]);
        for (actual, expected) in base_lines[start..start + old_lines].iter().zip(deleted) {
            if expected.as_str() != Some(*actual) {
                return Err("retirement deletion hunk does not match base text".into());
            }
        }
        cursor = start + old_lines;
        deleted_before += old_lines;
    }
    rebuilt.extend_from_slice(&base_lines[cursor..]);
    if rebuilt.join("\n") != result {
        return Err("retirement deletion patch does not reproduce result text".into());
    }
    Ok(())
}

pub(crate) fn normalized_retirement_call_path(
    value: &Value,
    branch_id: &str,
) -> Result<String, String> {
    let text = value
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or("retirement callPath is missing")?;
    let (path, branch) = text
        .split_once("::")
        .filter(|(_, branch)| !branch.contains("::"))
        .ok_or("retirement callPath must use <portable-path>::<branch-id>")?;
    validate_portable_path(path, "retirement callPath")?;
    if branch != branch_id || text != format!("{path}::{branch_id}") {
        return Err("retirement callPath is not canonical for the approved branch".into());
    }
    Ok(text.to_string())
}

pub(crate) fn retirement_portable_paths(value: &Value, label: &str) -> Result<Vec<String>, String> {
    let values = value
        .as_array()
        .filter(|values| !values.is_empty())
        .ok_or_else(|| format!("{label} must be a non-empty array"))?;
    let mut paths = Vec::with_capacity(values.len());
    for value in values {
        let path = value
            .as_str()
            .ok_or_else(|| format!("{label} contains an invalid path"))?;
        validate_portable_path(path, label)?;
        paths.push(path.to_string());
    }
    if paths.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(format!("{label} must be sorted and unique"));
    }
    Ok(paths)
}

// Guards retirement callPath/deletion-patch paths (see call sites above). Rejects
// a trailing '/' and '.'/'..' components directly -- distinct rule set from
// path_syntax_is_portable below, which guards general Artifact Ref paths and
// instead leans on component normalization to catch traversal.
fn validate_portable_path(path: &str, label: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\\')
        || path.starts_with('/')
        || path.ends_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path.contains(':')
    {
        Err(format!("{label} contains a non-portable path"))
    } else {
        Ok(())
    }
}

fn is_lower_sha(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn parse_contract_json(bytes: &[u8], label: &str) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("{label} is not UTF-8: {e}"))?;
    reject_duplicate_json_keys(text)?;
    serde_json::from_str(text).map_err(|e| format!("{label} is not JSON: {e}"))
}

fn validate_hospital_report(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("hospital report is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("hospital report is not JSON: {error}"))?;
    let mut expected = vec![
        "schema",
        "domainVerdict",
        "generatedAt",
        "repo",
        "mode",
        "artifacts",
        "triage",
        "state_machine",
        "modalities",
        "policies",
        "report_quality",
        "diagnosis",
        "treatment",
        "protocols",
        "tools",
        "surgery_plan",
    ];
    // "audit" is the additive optional block from code-intel-hospital.v1; reports
    // without it must keep passing unchanged.
    if let Some(audit) = value.get("audit") {
        expected.push("audit");
        validate_hospital_audit_block(audit)?;
    }
    exact_object_keys(&value, &expected, "hospital report")?;
    if value["schema"] != "code-intel-hospital.v1"
        || !matches!(
            value["domainVerdict"].as_str(),
            Some("pass" | "fail" | "unknown")
        )
        || !matches!(
            value.pointer("/triage/status").and_then(Value::as_str),
            Some("green" | "amber" | "red" | "unknown")
        )
        || !matches!(
            value.pointer("/triage/disposition").and_then(Value::as_str),
            Some("admit" | "observe")
        )
        || !matches!(
            value
                .pointer("/triage/next_protocol")
                .and_then(Value::as_str),
            Some("triage" | "diagnose" | "govern" | "surgery_plan" | "post_op")
        )
    {
        return Err("hospital report verdict/triage contract is invalid".into());
    }
    validate_surgery_plan_value(&value["surgery_plan"])
}

fn validate_hospital_audit_block(value: &Value) -> Result<(), String> {
    exact_object_keys(
        value,
        &[
            "status",
            "artifact",
            "overall",
            "findings_total",
            "by_severity",
        ],
        "hospital report audit block",
    )?;
    let status_valid = matches!(value["status"].as_str(), Some("absent" | "present"));
    let artifact_valid = value["artifact"].is_null() || value["artifact"].is_string();
    let overall_valid = value["overall"].is_null()
        || value["overall"]
            .as_f64()
            .is_some_and(|score| (0.0..=10.0).contains(&score));
    let findings_total_valid =
        value["findings_total"].is_null() || value["findings_total"].as_u64().is_some();
    let by_severity_valid = value["by_severity"].is_null()
        || value["by_severity"].as_object().is_some_and(|counts| {
            counts.iter().all(|(severity, count)| {
                matches!(
                    severity.as_str(),
                    "critical" | "high" | "medium" | "low" | "info"
                ) && count.as_u64().is_some()
            })
        });
    if !(status_valid
        && artifact_valid
        && overall_valid
        && findings_total_valid
        && by_severity_valid)
    {
        return Err("hospital report audit block contract is invalid".into());
    }
    Ok(())
}

/// Structural only: `AuditReport::parse` already enforces UTF-8, rejects
/// duplicate JSON keys, and applies the closed-object
/// `code-intel-audit-report.v1` shape. Registry-level validation
/// (`report.validate(&registry)`, which needs
/// `orchestration/audit/departments.v1.json` loaded from the repository
/// root) is deliberately not run here: payload validators receive only the
/// artifact bytes, no repo context, so that cross-artifact check is the
/// CLI's job (`code-intel audit --operation validate --repo <root> --report
/// <path>`), not the persist-time contract's.
fn validate_audit_report(bytes: &[u8]) -> Result<(), String> {
    crate::audit_report::AuditReport::parse(bytes).map(|_| ())
}

fn validate_surgery_plan(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("surgery plan is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("surgery plan is not JSON: {error}"))?;
    validate_surgery_plan_value(&value)
}

fn validate_surgery_plan_value(value: &Value) -> Result<(), String> {
    exact_object_keys(
        value,
        &[
            "schema",
            "status",
            "admission",
            "primary_target",
            "operating_plan",
            "verification",
            "discharge_criteria",
        ],
        "surgery plan",
    )?;
    if !surgery_plan_shape_is_valid(value) {
        return Err("surgery plan contract is invalid".into());
    }
    Ok(())
}

fn surgery_plan_shape_is_valid(value: &Value) -> bool {
    value["schema"] == "code-intel-surgery-plan.v1"
        && matches!(value["status"].as_str(), Some("planned" | "not_required"))
        && value["admission"].is_object()
        && value["primary_target"].is_object()
        && value["operating_plan"].is_array()
        && value["verification"].is_array()
        && value["discharge_criteria"].is_array()
}

fn validate_hospital_markdown(bytes: &[u8]) -> Result<(), String> {
    validate_markdown_view(bytes, "# Code Intel Hospital Report")
}

fn validate_surgery_markdown(bytes: &[u8]) -> Result<(), String> {
    validate_markdown_view(bytes, "# Code Intel Surgery Plan")
}

fn validate_project_orientation(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("project orientation is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("project orientation is not JSON: {error}"))?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "snapshotIdentity",
            "identity",
            "purpose",
            "languages",
            "boundaries",
            "entryPoints",
            "commands",
            "activeChange",
            "evidenceAvailability",
            "risks",
            "unknowns",
            "confidence",
        ],
        "project orientation",
    )?;
    if !project_orientation_shape_is_valid(&value) {
        return Err("project orientation contract is invalid".into());
    }
    for (label, claim) in [
        ("identity", &value["identity"]),
        ("purpose", &value["purpose"]),
        ("activeChange", &value["activeChange"]),
        ("confidence", &value["confidence"]),
    ] {
        validate_claim_provenance(&claim["provenance"], label)?;
    }
    for field in [
        "languages",
        "boundaries",
        "entryPoints",
        "commands",
        "evidenceAvailability",
        "risks",
        "unknowns",
    ] {
        for (index, claim) in value[field]
            .as_array()
            .expect("project_orientation_shape_is_valid proved this field is an array")
            .iter()
            .enumerate()
        {
            validate_claim_provenance(&claim["provenance"], &format!("{field}[{index}]"))?;
        }
    }
    Ok(())
}

fn project_orientation_shape_is_valid(value: &Value) -> bool {
    value["schema"] == "code-intel-project-orientation.v1"
        && value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        && value["identity"].is_object()
        && value["purpose"].is_object()
        && value["languages"].is_array()
        && value["boundaries"].is_array()
        && value["entryPoints"].is_array()
        && value["commands"].is_array()
        && value["activeChange"].is_object()
        && value["evidenceAvailability"].is_array()
        && value["risks"].is_array()
        && value["unknowns"]
            .as_array()
            .is_some_and(|unknowns| !unknowns.is_empty())
        && matches!(
            value.pointer("/confidence/level").and_then(Value::as_str),
            Some("low" | "medium" | "high")
        )
}

fn validate_claim_provenance(value: &Value, label: &str) -> Result<(), String> {
    let entries = value
        .as_array()
        .filter(|entries| !entries.is_empty())
        .ok_or_else(|| format!("{label} provenance must be a nonempty array"))?;
    let mut identities = BTreeSet::new();
    for entry in entries {
        exact_object_keys(
            entry,
            &["artifactType", "artifactSha256", "jsonPointer"],
            &format!("{label} provenance entry"),
        )?;
        if !entry["artifactType"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
            || !entry["artifactSha256"].as_str().is_some_and(valid_digest)
            || !entry["jsonPointer"]
                .as_str()
                .is_some_and(|value| value.starts_with('/'))
        {
            return Err(format!("{label} provenance entry is invalid"));
        }
        let identity = serde_json::to_string(entry)
            .map_err(|error| format!("serialize {label} provenance entry: {error}"))?;
        if !identities.insert(identity) {
            return Err(format!("{label} provenance entries must be unique"));
        }
    }
    Ok(())
}

fn validate_understanding_quadrant(bytes: &[u8]) -> Result<(), String> {
    let value = parse_understanding_quadrant(bytes)?;
    validate_understanding_quadrant_shape(&value)?;
    validate_understanding_quadrant_identity(&value)?;
    let (expected_unknowns, expected_counts) = validate_understanding_quadrant_items(&value)?;
    validate_understanding_quadrant_summary(&value, expected_unknowns, &expected_counts)
}

fn parse_understanding_quadrant(bytes: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("understanding quadrant is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    serde_json::from_str(text)
        .map_err(|error| format!("understanding quadrant is not JSON: {error}"))
}

fn validate_understanding_quadrant_shape(value: &Value) -> Result<(), String> {
    exact_object_keys(
        value,
        &[
            "schema",
            "snapshotIdentity",
            "sourceOrientation",
            "classificationPolicy",
            "items",
            "visibleUnknowns",
            "counts",
        ],
        "understanding quadrant",
    )?;
    exact_object_keys(
        &value["sourceOrientation"],
        &["artifactSchema", "artifactType", "sha256"],
        "understanding quadrant source",
    )?;
    exact_object_keys(
        &value["classificationPolicy"],
        &[
            "schema",
            "scoreRange",
            "systemCriticalityThreshold",
            "evidenceConfidenceThreshold",
            "thresholdRule",
            "unknownCriticalityRule",
            "methodConsumerPolicy",
        ],
        "understanding quadrant policy",
    )?;
    exact_object_keys(
        &value["classificationPolicy"]["scoreRange"],
        &["minimum", "maximum"],
        "understanding quadrant score range",
    )?;
    exact_object_keys(
        &value["counts"],
        &[
            "Known Core",
            "Critical Unknown",
            "Supporting Context",
            "Deferred Unknown",
        ],
        "understanding quadrant counts",
    )
}

fn validate_understanding_quadrant_identity(value: &Value) -> Result<(), String> {
    if !understanding_quadrant_identity_is_valid(value) {
        return Err("understanding quadrant identity/policy contract is invalid".into());
    }
    Ok(())
}

fn understanding_quadrant_identity_is_valid(value: &Value) -> bool {
    let source_orientation_valid = value.pointer("/sourceOrientation/artifactSchema")
        == Some(&json!("code-intel-project-orientation.v1"))
        && value.pointer("/sourceOrientation/artifactType") == Some(&json!("project.orientation"))
        && value
            .pointer("/sourceOrientation/sha256")
            .and_then(Value::as_str)
            .is_some_and(valid_digest);
    let classification_policy_valid = value.pointer("/classificationPolicy/schema")
        == Some(&json!("code-intel-understanding-quadrant-policy.v1"))
        && value.pointer("/classificationPolicy/scoreRange/minimum") == Some(&json!(0))
        && value.pointer("/classificationPolicy/scoreRange/maximum") == Some(&json!(100))
        && value.pointer("/classificationPolicy/systemCriticalityThreshold") == Some(&json!(50))
        && value.pointer("/classificationPolicy/evidenceConfidenceThreshold") == Some(&json!(50))
        && value.pointer("/classificationPolicy/thresholdRule")
            == Some(&json!("greater_than_or_equal_is_upper_band"))
        && value.pointer("/classificationPolicy/unknownCriticalityRule")
            == Some(&json!(
                "critical_by_default_except_declared_supporting_context"
            ))
        && value.pointer("/classificationPolicy/methodConsumerPolicy")
            == Some(&json!(
                "C01_cards_and_C02_selection_may_consume_but_cannot_rewrite"
            ));
    value["schema"] == "code-intel-understanding-quadrant.v1"
        && value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        && source_orientation_valid
        && classification_policy_valid
}

fn validate_understanding_quadrant_items(
    value: &Value,
) -> Result<(Vec<Value>, BTreeMap<&'static str, u64>), String> {
    let items = value["items"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or("understanding quadrant items must be nonempty")?;
    let mut prior = None::<String>;
    let mut expected_unknowns = Vec::new();
    let mut expected_counts = BTreeMap::<&'static str, u64>::new();
    for item in items {
        let (id, quadrant, is_unknown) = validate_understanding_quadrant_item(item, &prior)?;
        prior = Some(id.clone());
        *expected_counts.entry(quadrant).or_default() += 1;
        if is_unknown {
            expected_unknowns.push(Value::String(id));
        }
    }
    Ok((expected_unknowns, expected_counts))
}

fn validate_understanding_quadrant_item(
    item: &Value,
    prior: &Option<String>,
) -> Result<(String, &'static str, bool), String> {
    exact_object_keys(
        item,
        &[
            "id",
            "subject",
            "sourceState",
            "systemCriticality",
            "evidenceConfidence",
            "quadrant",
            "statement",
            "provenance",
        ],
        "understanding quadrant item",
    )?;
    exact_object_keys(
        &item["systemCriticality"],
        &["score", "band"],
        "system criticality",
    )?;
    exact_object_keys(
        &item["evidenceConfidence"],
        &["score", "band"],
        "evidence confidence",
    )?;
    let id = item["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("understanding quadrant item id is missing")?;
    if prior.as_deref().is_some_and(|prior| prior >= id) {
        return Err("understanding quadrant items are not uniquely sorted by id".into());
    }
    let criticality = item
        .pointer("/systemCriticality/score")
        .and_then(Value::as_u64);
    let confidence = item
        .pointer("/evidenceConfidence/score")
        .and_then(Value::as_u64);
    let (criticality, confidence) = match (criticality, confidence) {
        (Some(criticality @ 0..=100), Some(confidence @ 0..=100)) => (criticality, confidence),
        _ => return Err("understanding quadrant score is outside 0..=100".into()),
    };
    let expected = expected_understanding_quadrant(criticality, confidence);
    validate_claim_provenance(&item["provenance"], id)?;
    let source_state = item["sourceState"].as_str();
    if item
        .pointer("/systemCriticality/band")
        .and_then(Value::as_str)
        != Some(expected.0)
        || item
            .pointer("/evidenceConfidence/band")
            .and_then(Value::as_str)
            != Some(expected.1)
        || item["quadrant"] != expected.2
        || !matches!(source_state, Some("known" | "unknown"))
    {
        return Err("understanding quadrant item classification is incoherent".into());
    }
    Ok((id.to_string(), expected.2, source_state == Some("unknown")))
}

pub(crate) fn expected_understanding_quadrant(
    criticality: u64,
    confidence: u64,
) -> (&'static str, &'static str, &'static str) {
    match (criticality >= 50, confidence >= 50) {
        (true, true) => ("critical", "high", "Known Core"),
        (true, false) => ("critical", "low", "Critical Unknown"),
        (false, true) => ("supporting", "high", "Supporting Context"),
        (false, false) => ("supporting", "low", "Deferred Unknown"),
    }
}

fn validate_understanding_quadrant_summary(
    value: &Value,
    expected_unknowns: Vec<Value>,
    expected_counts: &BTreeMap<&'static str, u64>,
) -> Result<(), String> {
    if value["visibleUnknowns"] != Value::Array(expected_unknowns) {
        return Err("understanding quadrant hides or invents unknowns".into());
    }
    for quadrant in [
        "Known Core",
        "Critical Unknown",
        "Supporting Context",
        "Deferred Unknown",
    ] {
        if value["counts"][quadrant].as_u64()
            != Some(expected_counts.get(quadrant).copied().unwrap_or(0))
        {
            return Err("understanding quadrant counts do not match items".into());
        }
    }
    Ok(())
}

fn validate_orientation_benchmark_observations(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("orientation benchmark observations are not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("orientation benchmark observations are not JSON: {error}"))?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "snapshotIdentity",
            "method",
            "environment",
            "fixtures",
        ],
        "orientation benchmark observations",
    )?;
    if !orientation_benchmark_observations_header_is_valid(&value) {
        return Err("orientation benchmark observation contract is invalid".into());
    }
    for fixture in value["fixtures"].as_array().unwrap() {
        exact_object_keys(
            &fixture["expected"],
            &[
                "activeChange",
                "fileCount",
                "providerStatus",
                "unknownFields",
                "unsupportedFiles",
            ],
            "orientation benchmark expected fields",
        )?;
        if !fixture["expected"]["fileCount"].is_u64()
            || !matches!(
                fixture["expected"]["providerStatus"].as_str(),
                Some("available" | "unavailable")
            )
            || !fixture["expected"]["unknownFields"]
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
            || !fixture["expected"]["unsupportedFiles"]
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
        {
            return Err("orientation benchmark expected fields are invalid".into());
        }
        for temperature in ["cold", "warm"] {
            let samples = fixture["samples"][temperature]
                .as_array()
                .ok_or_else(|| "orientation benchmark samples are invalid".to_string())?;
            for sample in samples {
                if sample
                    .pointer("/artifact/bytes")
                    .and_then(Value::as_u64)
                    .is_none()
                    || !sample
                        .pointer("/artifact/sha256")
                        .and_then(Value::as_str)
                        .is_some_and(valid_digest)
                    || !sample
                        .pointer("/coverage/unsupportedFiles")
                        .and_then(Value::as_array)
                        .is_some_and(|items| items.iter().all(Value::is_string))
                {
                    return Err("orientation benchmark sample measurement is invalid".into());
                }
            }
        }
    }
    Ok(())
}

fn orientation_benchmark_observations_header_is_valid(value: &Value) -> bool {
    value["schema"] == "code-intel-project-orientation-benchmark-observations.v1"
        && value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        && value.pointer("/method/clock") == Some(&Value::String("std::time::Instant".into()))
        && value.pointer("/method/execution")
            == Some(&Value::String("sequential_child_process".into()))
        && value.pointer("/method/concurrency").and_then(Value::as_u64) == Some(1)
        && value.pointer("/method/llm") == Some(&Value::String("disabled".into()))
        && value
            .pointer("/environment/cleanMachine")
            .and_then(Value::as_bool)
            == Some(false)
        && value["fixtures"]
            .as_array()
            .is_some_and(|items| items.len() == 9)
}

fn validate_orientation_benchmark_report(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("orientation benchmark report is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("orientation benchmark report is not JSON: {error}"))?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "verdict",
            "target",
            "corpus",
            "method",
            "environment",
            "latency",
            "artifactSize",
            "quality",
            "costCenters",
            "limitations",
        ],
        "orientation benchmark report",
    )?;
    if !orientation_benchmark_report_is_valid(&value) {
        return Err("orientation benchmark report contract is invalid".into());
    }
    Ok(())
}

fn orientation_benchmark_report_is_valid(value: &Value) -> bool {
    let quality_metrics_in_range = [
        "fieldCorrectness",
        "unresolvedCoverage",
        "unsupportedCoverage",
        "deterministicReplayRate",
        "provenanceCompleteness",
    ]
    .into_iter()
    .all(|field| {
        value["quality"][field]
            .as_f64()
            .is_some_and(|metric| (0.0..=1.0).contains(&metric))
    });
    value["schema"] == "code-intel-project-orientation-benchmark.v1"
        && matches!(value["verdict"].as_str(), Some("pass" | "fail"))
        && value.pointer("/target/llm") == Some(&Value::String("disabled".into()))
        && value
            .pointer("/latency/typical/p50WallTimeMs")
            .and_then(Value::as_u64)
            .is_some()
        && value
            .pointer("/latency/typical/p95WallTimeMs")
            .and_then(Value::as_u64)
            .is_some()
        && value
            .pointer("/artifactSize/typical/p95Bytes")
            .and_then(Value::as_u64)
            .is_some()
        && quality_metrics_in_range
        && value["costCenters"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
}

fn validate_orientation_benchmark_markdown(bytes: &[u8]) -> Result<(), String> {
    validate_markdown_view(bytes, "# Project Orientation Benchmark")
}

fn validate_run_commit(bytes: &[u8]) -> Result<(), String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("run commit is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("run commit is not JSON: {error}"))?;
    exact_object_keys(
        &value,
        &["schema", "runIdentity", "snapshotIdentity", "manifest"],
        "run commit",
    )?;
    exact_object_keys(
        &value["manifest"],
        &["path", "sha256"],
        "run commit manifest",
    )?;
    let manifest_sha = value["manifest"]["sha256"].as_str();
    if value["schema"] != "code-intel-run-commit.v1"
        || !value["runIdentity"]
            .as_str()
            .is_some_and(valid_run_identity)
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || !manifest_sha.is_some_and(valid_digest)
        || value["manifest"]["path"].as_str()
            != manifest_sha
                .map(|sha| format!("objects/sha256/{sha}"))
                .as_deref()
    {
        return Err("run commit contract is invalid".into());
    }
    Ok(())
}

fn validate_run_manifest(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("run manifest is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("run manifest is not JSON: {error}"))?;
    let mut manifest_fields = vec![
        "schema",
        "runIdentity",
        "snapshotIdentity",
        "outcome",
        "nodes",
    ];
    if value.get("budget").is_some() {
        manifest_fields.push("budget");
    }
    exact_object_keys(&value, &manifest_fields, "run manifest")?;
    if let Some(budget) = value.get("budget") {
        if !budget.is_object() {
            return Err("run manifest budget must be an object".into());
        }
        validate_run_budget(budget)?;
    }
    if value["schema"] != "code-intel-run-manifest.v1"
        || !value["runIdentity"]
            .as_str()
            .is_some_and(valid_run_identity)
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || !matches!(
            value["outcome"].as_str(),
            Some(
                "completed"
                    | "domain_failed"
                    | "domain_unknown"
                    | "timeout"
                    | "process_failed"
                    | "failed"
                    | "budget_stopped"
            )
        )
    {
        return Err("run manifest identity/outcome is invalid".into());
    }
    let nodes = value["nodes"]
        .as_object()
        .filter(|nodes| !nodes.is_empty())
        .ok_or("run manifest nodes must be a non-empty object")?;
    for node in nodes.values() {
        match node["status"].as_str() {
            Some("succeeded") => {
                exact_object_keys(
                    node,
                    &["status", "verdict", "artifacts"],
                    "succeeded run node",
                )?;
                if !matches!(
                    node["verdict"].as_str(),
                    Some("pass" | "unknown" | "not_applicable")
                ) || !node["artifacts"].is_array()
                {
                    return Err("succeeded run node is invalid".into());
                }
                for reference in node["artifacts"].as_array().unwrap() {
                    validate_artifact_ref_shape(reference)?;
                }
            }
            Some("domain_failed") => {
                exact_object_keys(
                    node,
                    &["status", "verdict", "diagnostic", "artifacts"],
                    "domain-failed run node",
                )?;
                if node["verdict"] != "fail"
                    || node["diagnostic"].as_str().is_none_or(str::is_empty)
                    || !node["artifacts"].is_array()
                {
                    return Err("domain-failed run node is invalid".into());
                }
                for reference in node["artifacts"].as_array().unwrap() {
                    validate_artifact_ref_shape(reference)?;
                }
            }
            Some("timeout") => {
                exact_object_keys(node, &["status", "diagnostic"], "timeout run node")?;
                if node["diagnostic"].as_str().is_none_or(str::is_empty) {
                    return Err("timeout run node is invalid".into());
                }
            }
            Some("process_failed") => {
                exact_object_keys(
                    node,
                    &["status", "failure", "diagnostic"],
                    "process-failed run node",
                )?;
                if !matches!(
                    node["failure"].as_str(),
                    Some("contract" | "unavailable" | "internal" | "io")
                ) || node["diagnostic"].as_str().is_none_or(str::is_empty)
                {
                    return Err("process-failed run node is invalid".into());
                }
            }
            Some("dependency_blocked") => {
                exact_object_keys(node, &["status", "blockedBy"], "blocked run node")?;
                if node["blockedBy"].as_array().is_none_or(Vec::is_empty) {
                    return Err("blocked run node is invalid".into());
                }
            }
            Some("not_dispatched") => {
                exact_object_keys(node, &["status", "reason"], "not-dispatched run node")?;
                if node["reason"].as_str().is_none_or(str::is_empty) {
                    return Err("not-dispatched run node is invalid".into());
                }
            }
            // #307: pre-dispatch oversize refusal. Distinct from
            // `not_dispatched` (budget exhausted by siblings) -- carries the
            // actual/limit bytes that tripped the policy.
            Some("skipped_oversize") => {
                exact_object_keys(
                    node,
                    &["status", "reason", "actualBytes", "byteLimit"],
                    "skipped-oversize run node",
                )?;
                if node["reason"].as_str().is_none_or(str::is_empty)
                    || node["actualBytes"].as_u64().is_none()
                    || node["byteLimit"].as_u64().is_none()
                {
                    return Err("skipped-oversize run node is invalid".into());
                }
            }
            _ => return Err("run manifest contains a non-terminal node".into()),
        }
    }
    Ok(())
}
fn validate_run_budget(value: &Value) -> Result<(), String> {
    exact_object_keys(
        value,
        &["limits", "consumed", "exceeded", "stoppedAt", "oversize"],
        "run budget",
    )?;
    for (name, dimension) in [
        ("limits", &value["limits"]),
        ("consumed", &value["consumed"]),
    ] {
        exact_object_keys(
            dimension,
            &["wallClockSeconds", "bytes"],
            &format!("run budget {name}"),
        )?;
        if dimension["wallClockSeconds"].as_u64().is_none() || dimension["bytes"].as_u64().is_none()
        {
            return Err(format!("run budget {name} is invalid"));
        }
    }
    if !value["exceeded"].is_boolean()
        || (!value["stoppedAt"].is_null() && value["stoppedAt"].as_str().is_none_or(str::is_empty))
    {
        return Err("run budget terminal fields are invalid".into());
    }
    // #307: the `oversize` sub-object. Empty `skippedNodes` is the common
    // case (no oversize refusals this run) and must validate cleanly.
    let oversize = &value["oversize"];
    exact_object_keys(
        oversize,
        &["thresholdPercent", "skippedNodes"],
        "run budget oversize",
    )?;
    let threshold = oversize["thresholdPercent"].as_u64();
    if !threshold.is_some_and(|value| (1..=100).contains(&value)) {
        return Err("run budget oversize thresholdPercent is invalid".into());
    }
    let skipped = oversize["skippedNodes"]
        .as_array()
        .ok_or("run budget oversize skippedNodes must be an array")?;
    for node in skipped {
        exact_object_keys(
            node,
            &["nodeId", "actualBytes", "byteLimit"],
            "run budget oversize skipped node",
        )?;
        if node["nodeId"].as_str().is_none_or(str::is_empty)
            || node["actualBytes"].as_u64().is_none()
            || node["byteLimit"].as_u64().is_none()
        {
            return Err("run budget oversize skipped node is invalid".into());
        }
    }
    Ok(())
}

fn validate_repository_iteration_provenance(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "repository iteration provenance")?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "purpose",
            "runIdentity",
            "snapshotIdentity",
            "repositoryKey",
            "publicationName",
            "producer",
        ],
        "repository iteration provenance",
    )?;
    exact_object_keys(
        &value["producer"],
        &["component", "contract", "version"],
        "repository iteration producer",
    )?;
    if !repository_iteration_provenance_is_valid(&value) {
        return Err("repository iteration provenance contract is invalid".into());
    }
    Ok(())
}

fn repository_iteration_provenance_is_valid(value: &Value) -> bool {
    value["schema"] == REPOSITORY_ITERATION_SCHEMA
        && value["purpose"] == REPOSITORY_ITERATION_PURPOSE
        && value["runIdentity"]
            .as_str()
            .is_some_and(valid_run_identity)
        && value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        && value["repositoryKey"]
            .as_str()
            .is_some_and(valid_authority_name)
        && value["publicationName"]
            .as_str()
            .is_some_and(valid_authority_name)
        && value["producer"]["component"] == REPOSITORY_ITERATION_PRODUCER_COMPONENT
        && value["producer"]["contract"] == REPOSITORY_ITERATION_PRODUCER_CONTRACT
        && value["producer"]["version"] == REPOSITORY_ITERATION_PRODUCER_VERSION
}

fn validate_method_catalog(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("method catalog is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("method catalog is not JSON: {error}"))?;
    exact_object_keys(
        &value,
        &["schema", "catalogVersion", "selectionPolicy", "cards"],
        "method catalog",
    )?;
    let cards = value["cards"]
        .as_array()
        .filter(|cards| !cards.is_empty())
        .ok_or("method catalog cards must be non-empty")?;
    if value["schema"] != "code-intel-method-catalog.v1"
        || value["selectionPolicy"] != "none_catalog_only"
        || value["catalogVersion"].as_str().is_none_or(str::is_empty)
    {
        return Err("method catalog contract is invalid".into());
    }
    let mut ids = BTreeSet::new();
    for card in cards {
        exact_object_keys(card, &["id", "path"], "method catalog entry")?;
        let id = card["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("method catalog id is invalid")?;
        if !ids.insert(id) || card["path"] != format!("cards/{id}.v1.json") {
            return Err("method catalog entry is invalid or duplicated".into());
        }
    }
    Ok(())
}

fn validate_method_card(bytes: &[u8]) -> Result<(), String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("method card is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("method card is not JSON: {error}"))?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "id",
            "version",
            "name",
            "problemSignals",
            "requiredEvidence",
            "assumptions",
            "deterministicSteps",
            "outputs",
            "confidenceRules",
            "cost",
            "contraindications",
            "implementationPorts",
            "source",
            "applicabilityBoundary",
            "relatedMethodIds",
            "executionPolicy",
        ],
        "method card",
    )?;
    if value["schema"] != "code-intel-method-card.v1"
        || value["id"].as_str().is_none_or(str::is_empty)
        || value["version"].as_str().is_none_or(str::is_empty)
        || value["executionPolicy"] != "catalog_only_no_selection_or_execution"
        || value["deterministicSteps"]
            .as_array()
            .is_none_or(Vec::is_empty)
    {
        return Err("method card contract is invalid".into());
    }
    Ok(())
}

fn validate_run_timing_events(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("run timing events are not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("run timing events are not JSON: {error}"))?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "measurementSnapshotIdentity",
            "telemetry",
            "baseline",
            "current",
        ],
        "run timing events",
    )?;
    exact_object_keys(
        &value["telemetry"],
        &["mode", "clock", "externalPlatform"],
        "run timing telemetry",
    )?;
    if !run_timing_telemetry_policy_is_valid(&value) {
        return Err("run timing telemetry policy is invalid".into());
    }
    for label in ["baseline", "current"] {
        let trace = &value[label];
        exact_object_keys(trace, &["commitRef", "events"], "run timing trace")?;
        let commit_ref = &trace["commitRef"];
        validate_artifact_ref_shape(commit_ref)?;
        if commit_ref["artifactSchema"] != "code-intel-run-commit.v1"
            || commit_ref["type"] != "run.commit"
            || commit_ref["consumedSnapshotIdentity"] != value["measurementSnapshotIdentity"]
        {
            return Err("run timing trace is not bound to an A07 commit Artifact Ref".into());
        }
        let events = trace["events"]
            .as_array()
            .filter(|events| !events.is_empty())
            .ok_or("run timing trace events must be non-empty")?;
        for event in events {
            exact_object_keys(
                event,
                &[
                    "id",
                    "kind",
                    "subject",
                    "startedAtMs",
                    "completedAtMs",
                    "mandatory",
                    "coordinationNeed",
                    "predecessors",
                ],
                "run timing event",
            )?;
            if !run_timing_event_is_valid(event) {
                return Err("run timing event contract is invalid".into());
            }
        }
    }
    Ok(())
}

fn run_timing_telemetry_policy_is_valid(value: &Value) -> bool {
    value["schema"] == "code-intel-run-timing-events.v1"
        && value["measurementSnapshotIdentity"]
            .as_str()
            .is_some_and(valid_digest)
        && value.pointer("/telemetry/mode") == Some(&Value::String("local_opt_in".into()))
        && value.pointer("/telemetry/clock") == Some(&Value::String("monotonic_elapsed_ms".into()))
        && value
            .pointer("/telemetry/externalPlatform")
            .and_then(Value::as_bool)
            == Some(false)
}

fn run_timing_event_is_valid(event: &Value) -> bool {
    let start = event["startedAtMs"].as_u64();
    let end = event["completedAtMs"].as_u64();
    event["id"].as_str().is_some_and(|v| !v.is_empty())
        && event["subject"].as_str().is_some_and(|v| !v.is_empty())
        && matches!(
            event["kind"].as_str(),
            Some(
                "technical_work"
                    | "test"
                    | "verification"
                    | "queue"
                    | "handoff"
                    | "understanding"
                    | "rework"
                    | "coordination"
            )
        )
        && start.is_some()
        && end.zip(start).is_some_and(|(end, start)| end > start)
        && event["mandatory"].as_bool().is_some()
        && event["predecessors"].is_array()
}

fn validate_light_speed_report(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("light-speed report is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("light-speed report is not JSON: {error}"))?;
    exact_object_keys(
        &value,
        &[
            "schema",
            "measurementSnapshotIdentity",
            "authority",
            "method",
            "rules",
            "baseline",
            "current",
            "delta",
            "limitations",
        ],
        "light-speed report",
    )?;
    if !light_speed_report_is_valid(&value) {
        return Err("light-speed report contract is invalid".into());
    }
    Ok(())
}

fn light_speed_report_is_valid(value: &Value) -> bool {
    value["schema"] == "code-intel-delivery-light-speed.v1"
        && value["measurementSnapshotIdentity"]
            .as_str()
            .is_some_and(valid_digest)
        && value["authority"] == "derived_measurement_no_schedule_commitment"
        && value["rules"]
            .as_array()
            .is_some_and(|rules| rules.len() == 7)
        && value["baseline"].is_object()
        && value["current"].is_object()
        && value["delta"].is_object()
        && value["limitations"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
}

fn validate_light_speed_markdown(bytes: &[u8]) -> Result<(), String> {
    validate_markdown_view(bytes, "# Delivery Light-Speed Measurement")
}

fn validate_session_evidence(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "session evidence")?;
    validate_session_evidence_value(&value)
}

#[cfg(not(test))]
fn validate_session_evidence_value(value: &Value) -> Result<(), String> {
    crate::session_evidence::validate_artifact_value(value)
}

// Many integration tests compile artifact_ref.rs as a stand-alone path module. They do not
// consume session evidence; keep that test-only compilation surface independent of the binary
// crate root. End-to-end session tests exercise the non-test binary and the full validator above.
#[cfg(test)]
fn validate_session_evidence_value(value: &Value) -> Result<(), String> {
    exact_object_keys(
        value,
        &[
            "schema",
            "status",
            "reviewAuthority",
            "snapshot",
            "source",
            "implementation",
            "privacy",
            "observability",
            "summary",
            "events",
            "signals",
        ],
        "session evidence",
    )?;
    if value["schema"] != "code-intel-session-evidence.v1"
        || !matches!(value["status"].as_str(), Some("complete" | "partial"))
        || value["reviewAuthority"] != "advisory_only"
        || !value["snapshot"].is_object()
        || !value["source"].is_object()
        || !value["implementation"].is_object()
        || !value["privacy"].is_object()
        || !value["observability"].is_object()
        || !value["summary"].is_object()
        || value["events"].as_array().is_none_or(Vec::is_empty)
        || !value["signals"].is_array()
    {
        return Err("session evidence contract is invalid".into());
    }
    Ok(())
}

/// Issue #151. Deliberately self-contained (no `crate::anchor_verification`
/// reference) rather than delegating to that module's own `AnchorState`
/// parsing, matching how this file already keeps its simple payload
/// validators independent of the modules that produce them -- several
/// integration tests compile this file as a stand-alone `#[path = ...]`
/// module (see `validate_session_evidence_value`'s `#[cfg(test)]` variant
/// above) where a sibling crate module would not resolve.
///
/// Beyond per-field shape, this re-tallies every anchor's `state` and
/// requires it to match the report's own `counts` object exactly: a
/// `counts` claiming `dropped: 0` while an anchor entry underneath it is
/// actually `"state":"dropped"` is rejected here, not merely well-formed.
fn validate_anchor_verification(bytes: &[u8]) -> Result<(), String> {
    let value = parse_contract_json(bytes, "anchor verification report")?;
    exact_object_keys(
        &value,
        &["schema", "counts", "sources"],
        "anchor verification report",
    )?;
    if value["schema"] != "code-intel-anchor-verification.v1" {
        return Err("anchor verification report has the wrong schema".into());
    }
    let counts = value
        .get("counts")
        .ok_or("anchor verification report missing \"counts\"")?;
    exact_object_keys(
        counts,
        &["verified", "approximate", "dropped"],
        "anchor verification counts",
    )?;
    if !counts["verified"].is_u64()
        || !counts["approximate"].is_u64()
        || !counts["dropped"].is_u64()
    {
        return Err("anchor verification counts must be non-negative integers".into());
    }
    let sources = value["sources"]
        .as_array()
        .ok_or("anchor verification report \"sources\" must be an array")?;
    let (mut verified, mut approximate, mut dropped) = (0u64, 0u64, 0u64);
    for source in sources {
        exact_object_keys(
            source,
            &["artifactType", "artifactPath", "anchorKind", "anchors"],
            "an anchor verification source",
        )?;
        let anchor_kind = source["anchorKind"].as_str();
        if !source["artifactType"].is_string()
            || !source["artifactPath"].is_string()
            || !matches!(anchor_kind, Some("file" | "symbol"))
        {
            return Err("an anchor verification source has invalid fields".into());
        }
        let anchors = source["anchors"]
            .as_array()
            .ok_or("an anchor verification source's \"anchors\" must be an array")?;
        if anchors.is_empty() {
            return Err("an anchor verification source must not report zero anchors".into());
        }
        for anchor in anchors {
            let has_location = match anchor_kind {
                Some("file") => anchor["path"].is_string(),
                Some("symbol") => {
                    anchor["file"].is_string()
                        && anchor["name"].is_string()
                        && anchor["claimedLine"].is_u64()
                }
                _ => false,
            };
            if !has_location {
                return Err("an anchor entry is missing its location fields".into());
            }
            match anchor["state"].as_str() {
                Some("verified") => verified += 1,
                Some("approximate") => {
                    if !anchor["resolvedLine"].is_u64() {
                        return Err(
                            "an \"approximate\" anchor is missing a numeric resolvedLine".into(),
                        );
                    }
                    approximate += 1;
                }
                Some("dropped") => {
                    if !anchor["reason"]
                        .as_str()
                        .is_some_and(|reason| !reason.is_empty())
                    {
                        return Err("a \"dropped\" anchor is missing a reason".into());
                    }
                    dropped += 1;
                }
                _ => return Err("an anchor has an unrecognized state".into()),
            }
        }
    }
    if counts["verified"].as_u64() != Some(verified)
        || counts["approximate"].as_u64() != Some(approximate)
        || counts["dropped"].as_u64() != Some(dropped)
    {
        return Err("anchor verification counts do not match the tallied anchors".into());
    }
    Ok(())
}

fn validate_markdown_view(bytes: &[u8], heading: &str) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("diagnosis view is not UTF-8: {error}"))?;
    if !text.starts_with(heading) || text.trim().is_empty() {
        return Err("diagnosis Markdown view contract is invalid".into());
    }
    Ok(())
}

fn validate_evidence_admission(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("evidence admission is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("evidence admission is not JSON: {error}"))?;
    let keys = value
        .as_object()
        .ok_or("evidence admission must be an object")?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = [
        "schema",
        "status",
        "domainVerdict",
        "admissionIdentity",
        "evidence",
        "verifiedPayload",
        "engineeringFacts",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if keys != expected {
        return Err("evidence admission fields are not exact".into());
    }
    if value["schema"] != "code-intel-evidence-admissibility-result.v1"
        || value["status"] != "admitted"
        || !matches!(
            value["domainVerdict"].as_str(),
            Some("observed" | "unknown")
        )
        || !value["admissionIdentity"]
            .as_str()
            .is_some_and(valid_digest)
        || !value["engineeringFacts"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        return Err("evidence admission identity/status/verdict is invalid".into());
    }
    let evidence = value["evidence"]
        .as_object()
        .ok_or("evidence admission lacks observed evidence")?;
    let verified = value["verifiedPayload"]
        .as_object()
        .ok_or("evidence admission lacks verified payload")?;
    if evidence
        .get("consumedSnapshotIdentity")
        .and_then(Value::as_str)
        != verified
            .get("consumedSnapshotIdentity")
            .and_then(Value::as_str)
        || verified.get("artifactSchema").and_then(Value::as_str)
            != Some("code-intel-evidence-payload.v1")
        || verified.get("type").and_then(Value::as_str) != Some("observed.evidence.payload")
        || !verified
            .get("sha256")
            .and_then(Value::as_str)
            .is_some_and(valid_digest)
        || !verified.get("data").is_some_and(Value::is_object)
    {
        return Err("evidence admission verified payload is invalid or incoherent".into());
    }
    Ok(())
}

fn native_code_contract(
    schema: &str,
    artifact_type: &str,
) -> Option<(&'static str, &'static str, fn(&[u8]) -> Result<(), String>)> {
    match (schema, artifact_type) {
        ("code-evidence-files.v1", "code_evidence.files") => Some((
            "code-evidence-files.v1",
            "code_evidence.files",
            validate_native_files,
        )),
        ("code-evidence-symbols.v1", "code_evidence.symbols") => Some((
            "code-evidence-symbols.v1",
            "code_evidence.symbols",
            validate_native_symbols,
        )),
        ("code-evidence-chunks.v1", "code_evidence.chunks") => Some((
            "code-evidence-chunks.v1",
            "code_evidence.chunks",
            validate_native_chunks,
        )),
        ("code-evidence-symbol-chunks.v1", "code_evidence.symbol_chunks") => Some((
            "code-evidence-symbol-chunks.v1",
            "code_evidence.symbol_chunks",
            validate_native_symbol_chunks,
        )),
        ("code-evidence-imports.v1", "code_evidence.imports") => Some((
            "code-evidence-imports.v1",
            "code_evidence.imports",
            validate_native_imports,
        )),
        ("code-evidence-scorecard.v1", "code_evidence.scorecard") => Some((
            "code-evidence-scorecard.v1",
            "code_evidence.scorecard",
            validate_native_scorecard,
        )),
        ("code-evidence-coverage.v1", "code_evidence.coverage") => Some((
            "code-evidence-coverage.v1",
            "code_evidence.coverage",
            validate_native_coverage,
        )),
        ("agent-code-slice-ranking.v1", "code_evidence.agent_slice") => Some((
            "agent-code-slice-ranking.v1",
            "code_evidence.agent_slice",
            validate_native_ranking,
        )),
        _ => None,
    }
}

fn validate_native_files(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(bytes, "code-evidence-files.v1", "files", 2, &["path"])
}

fn validate_native_symbols(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(bytes, "code-evidence-symbols.v1", "symbols", 2, &[])
}

fn validate_native_chunks(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(bytes, "code-evidence-chunks.v1", "chunks", 2, &[])
}

fn validate_native_symbol_chunks(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(bytes, "code-evidence-symbol-chunks.v1", "mappings", 2, &[])
}

fn validate_native_imports(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(
        bytes,
        "code-evidence-imports.v1",
        "imports",
        2,
        &["file", "target"],
    )
}

fn validate_native_ranking(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(bytes, "agent-code-slice-ranking.v1", "files", 3, &[])
}

fn parse_native_object(bytes: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("native code evidence artifact is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("native code evidence artifact is invalid JSON: {error}"))?;
    value
        .as_object()
        .ok_or_else(|| "native code evidence artifact must be an object".to_string())?;
    Ok(value)
}

fn validate_native_array_artifact(
    bytes: &[u8],
    expected_schema: &str,
    payload: &str,
    expected_fields: usize,
    element_string_fields: &[&str],
) -> Result<(), String> {
    let value = parse_native_object(bytes)?;
    let object = value.as_object().expect("parse validated object");
    if value["schema"] != expected_schema
        || object.len() != expected_fields
        || !value[payload].is_array()
    {
        return Err(format!("{expected_schema} artifact shape is invalid"));
    }
    let elements = value[payload]
        .as_array()
        .expect("shape check validated array");
    if elements.iter().any(|element| {
        element_string_fields
            .iter()
            .any(|field| !element[*field].is_string())
    }) {
        return Err(format!("{expected_schema} artifact shape is invalid"));
    }
    Ok(())
}

fn validate_native_scorecard(bytes: &[u8]) -> Result<(), String> {
    let value = parse_native_object(bytes)?;
    if value["schema"] != "code-evidence-scorecard.v1"
        || !value
            .as_object()
            .is_some_and(|object| object.contains_key("metrics"))
        || value["status"] != "ok"
    {
        return Err("native code evidence scorecard is invalid".into());
    }
    Ok(())
}

fn validate_native_coverage(bytes: &[u8]) -> Result<(), String> {
    let value = parse_native_object(bytes)?;
    if value["schema"] != "code-evidence-coverage.v1"
        || value["parserKind"] != "line-heuristic"
        || value["relationshipPrecision"] != "unknown"
        || value["callGraph"] != "unknown"
    {
        return Err("native code evidence coverage overclaims precision".into());
    }
    Ok(())
}

pub(crate) fn validate_decision_record_schema(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("decision record artifact is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("decision record artifact is invalid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "decision record artifact must be an object".to_string())?;
    let expected = [
        "schema",
        "id",
        "bindingDigest",
        "gap",
        "request",
        "response",
        "evidenceBinding",
        "snapshotIdentity",
        "acceptedChoice",
        "authorityEvent",
        "consequences",
        "affectedBranches",
        "recordedAt",
        "freshness",
        "reopenRule",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if object.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
        || value["schema"] != "code-intel-decision-record.v1"
        || !value["id"].as_str().is_some_and(|id| {
            id.strip_prefix("decision-record-v1:")
                .is_some_and(valid_digest)
        })
        || !value["bindingDigest"].as_str().is_some_and(valid_digest)
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || !value["recordedAt"].is_u64()
    {
        return Err("decision record identity/schema fields are invalid".to_string());
    }
    let evidence = value["evidenceBinding"]
        .as_object()
        .ok_or_else(|| "decision record evidenceBinding must be an object".to_string())?;
    if evidence.keys().map(String::as_str).collect::<BTreeSet<_>>()
        != ["refs", "digest"].into_iter().collect()
        || !value["evidenceBinding"]["digest"]
            .as_str()
            .is_some_and(valid_digest)
        || !value["evidenceBinding"]["refs"]
            .as_array()
            .is_some_and(|refs| !refs.is_empty())
    {
        return Err("decision record evidenceBinding fields are invalid".to_string());
    }
    let branches = value["affectedBranches"]
        .as_array()
        .ok_or_else(|| "decision record affectedBranches must be an array".to_string())?;
    let mut seen = BTreeSet::new();
    if branches.is_empty()
        || !branches.iter().all(|branch| {
            branch
                .as_str()
                .is_some_and(|branch| !branch.is_empty() && seen.insert(branch))
        })
        || !value["consequences"].as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| item.as_str().is_some_and(|item| !item.is_empty()))
        })
    {
        return Err("decision record branch/consequence fields are invalid".to_string());
    }
    let freshness = value["freshness"]
        .as_object()
        .ok_or_else(|| "decision record freshness must be an object".to_string())?;
    if freshness
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != ["evidenceExpiresAt", "state"].into_iter().collect()
        || !value["freshness"]["evidenceExpiresAt"].is_u64()
        || value["freshness"]["state"] != "current"
    {
        return Err("decision record freshness fields are invalid".to_string());
    }
    let reopen = value["reopenRule"]
        .as_object()
        .ok_or_else(|| "decision record reopenRule must be an object".to_string())?;
    if reopen.keys().map(String::as_str).collect::<BTreeSet<_>>()
        != [
            "evidenceDigestChanged",
            "snapshotChanged",
            "evidenceExpired",
        ]
        .into_iter()
        .collect()
        || value["reopenRule"]["evidenceDigestChanged"] != true
        || value["reopenRule"]["snapshotChanged"] != true
        || value["reopenRule"]["evidenceExpired"] != true
    {
        return Err("decision record reopenRule fields are invalid".to_string());
    }
    Ok(())
}

fn validate_repository_snapshot(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("repository snapshot payload is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("repository snapshot payload is invalid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "repository snapshot payload must be an object".to_string())?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = ["schema", "snapshot", "dirtyOverlay", "repository"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    if actual != expected || value["schema"] != "code-intel-repository-snapshot.v1" {
        return Err("repository snapshot payload fields/schema are invalid".to_string());
    }
    validate_repository_snapshot_identity(&value["snapshot"])?;
    let repository = value["repository"]
        .as_object()
        .ok_or_else(|| "repository snapshot repository is invalid".to_string())?;
    if repository.len() != 1
        || !matches!(
            repository.get("kind").and_then(Value::as_str),
            Some("git" | "git_unborn" | "unversioned")
        )
    {
        return Err("repository snapshot repository kind is invalid".to_string());
    }
    let overlay = value["dirtyOverlay"]
        .as_object()
        .ok_or_else(|| "repository snapshot dirtyOverlay is invalid".to_string())?;
    let overlay_keys = overlay.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_overlay = ["present", "digest", "paths", "members", "ignoredPolicy"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    if overlay_keys != expected_overlay
        || !value["dirtyOverlay"]["present"].is_boolean()
        || value["dirtyOverlay"]["ignoredPolicy"] != "excluded_by_git_ignore"
    {
        return Err("repository snapshot dirtyOverlay fields are invalid".to_string());
    }
    let digest_valid = value["dirtyOverlay"]["digest"].is_null()
        || value["dirtyOverlay"]["digest"]
            .as_str()
            .is_some_and(valid_digest);
    if !digest_valid || !valid_path_array(&value["dirtyOverlay"]["paths"]) {
        return Err("repository snapshot dirtyOverlay digest/paths are invalid".to_string());
    }
    let members = value["dirtyOverlay"]["members"]
        .as_object()
        .ok_or_else(|| "repository snapshot dirtyOverlay members are invalid".to_string())?;
    let expected_members = [
        "trackedModified",
        "trackedDeleted",
        "untracked",
        "renamed",
        "typeChanged",
        "staged",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if members.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected_members
        || members.values().any(|value| !valid_path_array(value))
    {
        return Err("repository snapshot dirtyOverlay members are invalid".to_string());
    }
    Ok(())
}

// Mirrors `capability_inventory::validate_workflow_proposal`'s contract:
// both guard the same artifact before it crosses the same boundary, once at
// publish time inside the adapter and once here at consumption/staging time.
fn validate_workflow_recommendation(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("workflow recommendation is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("workflow recommendation is invalid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "workflow recommendation must be an object".to_string())?;
    let expected = [
        "schema",
        "kind",
        "recommendation",
        "evidence",
        "confidence",
        "alternatives",
        "provenance",
        "effects",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if object.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
        || value["schema"] != "code-intel-advisory-workflow-recommendation.v1"
        || value["kind"] != "proposal"
        || !matches!(
            value["confidence"].as_str(),
            Some("low" | "medium" | "high")
        )
        || value["evidence"].as_array().map_or(true, Vec::is_empty)
        || value["alternatives"]
            .as_array()
            .map_or(true, |items| items.len() < 3)
        || value["effects"]
            .as_array()
            .map_or(true, |items| !items.is_empty())
        || value
            .pointer("/provenance/capabilityId")
            .and_then(Value::as_str)
            != Some("advisory.workflow-recommend")
    {
        return Err("workflow recommendation violates the advisory proposal boundary".into());
    }
    Ok(())
}

fn validate_doctor_observation(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("doctor observation is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("doctor observation is invalid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "doctor observation must be an object".to_string())?;
    let expected = [
        "schema",
        "snapshotIdentity",
        "environmentPolicy",
        "bootstrap",
        "repository",
        "tools",
        "providers",
        "manifest",
        "diagnostics",
        "engineeringFacts",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if object.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
        || value["schema"] != "code-intel-doctor-observation.v1"
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || !value
            .pointer("/environmentPolicy/sha256")
            .and_then(Value::as_str)
            .is_some_and(valid_digest)
        || value.pointer("/bootstrap/authority") != Some(&Value::String("observation_only".into()))
        || value["engineeringFacts"]
            .as_array()
            .map_or(true, |facts| !facts.is_empty())
    {
        return Err("doctor observation top-level contract is invalid".into());
    }
    let policy = value
        .pointer("/environmentPolicy/policy")
        .ok_or_else(|| "doctor observation environment policy is missing".to_string())?;
    let policy_digest = sha256_hex(
        &serde_json::to_vec(policy)
            .map_err(|error| format!("serialize doctor environment policy: {error}"))?,
    );
    if value
        .pointer("/environmentPolicy/sha256")
        .and_then(Value::as_str)
        != Some(policy_digest.as_str())
    {
        return Err("doctor observation environment policy digest mismatch".into());
    }
    exact_object_keys(
        &value["repository"],
        &["presence", "readiness", "conformance", "admissibility"],
        "doctor repository",
    )?;
    for tool in value["tools"]
        .as_array()
        .ok_or("doctor tools must be an array")?
    {
        exact_object_keys(
            tool,
            &[
                "name",
                "required",
                "presence",
                "readiness",
                "conformance",
                "admissibility",
            ],
            "doctor tool",
        )?;
    }
    for provider in value["providers"]
        .as_array()
        .ok_or("doctor providers must be an array")?
    {
        exact_object_keys(
            provider,
            &[
                "id",
                "presence",
                "readiness",
                "conformance",
                "admissibility",
            ],
            "doctor provider",
        )?;
    }
    let observations = std::iter::once(&value["repository"])
        .chain(value["tools"].as_array().into_iter().flatten())
        .chain(value["providers"].as_array().into_iter().flatten());
    for observation in observations {
        if !matches!(
            observation["presence"].as_str(),
            Some("present" | "missing")
        ) || !matches!(
            observation["readiness"].as_str(),
            Some("ready" | "unavailable")
        ) || !matches!(
            observation["conformance"].as_str(),
            Some("conforming" | "nonconforming" | "not_evaluated")
        ) || observation["admissibility"] != "not_evaluated"
        {
            return Err(
                "doctor observation collapses presence/readiness/conformance/admissibility".into(),
            );
        }
    }
    Ok(())
}

fn exact_object_keys(value: &Value, expected: &[&str], context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))?;
    require_exact_keys(object, expected, context)
        .map_err(|_| format!("{context} fields are invalid"))
}

fn validate_survival_scan(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("survival scan payload is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("survival scan payload is invalid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "survival scan payload must be an object".to_string())?;
    let expected = [
        "schema",
        "status",
        "snapshotIdentity",
        "repository",
        "inventory",
        "providerDiagnosis",
        "completeness",
        "structuralVerdict",
        "limitations",
        "engineeringFacts",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if object.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
        || value["schema"] != "code-intel-repository-survival-scan-result.v1"
        || value["status"] != "completed"
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || value["completeness"] != "reduced"
        || value["structuralVerdict"] != "unknown"
    {
        return Err("survival scan top-level contract is invalid".into());
    }
    let repository = value["repository"]
        .as_object()
        .ok_or_else(|| "survival scan repository is invalid".to_string())?;
    if repository
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != ["kind", "identity", "revision", "dirty", "sourceSha256"]
            .into_iter()
            .collect()
        || !matches!(
            value["repository"]["kind"].as_str(),
            Some("git" | "git_unborn" | "unversioned")
        )
        || !value["repository"]["sourceSha256"]
            .as_str()
            .is_some_and(valid_digest)
        || !value["repository"]["dirty"].is_boolean()
    {
        return Err("survival scan repository contract is invalid".into());
    }
    let inventory = value["inventory"]
        .as_object()
        .ok_or_else(|| "survival scan inventory is invalid".to_string())?;
    if inventory
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != ["fileCount", "extensions", "sourceSha256"]
            .into_iter()
            .collect()
        || !value["inventory"]["fileCount"].is_u64()
        || !value["inventory"]["extensions"].is_object()
        || !value["inventory"]["sourceSha256"]
            .as_str()
            .is_some_and(valid_digest)
    {
        return Err("survival scan inventory contract is invalid".into());
    }
    if value["providerDiagnosis"]["status"] != "provider_unavailable"
        || value["providerDiagnosis"]["domainVerdict"] != "unknown"
        || !value["limitations"]
            .as_array()
            .is_some_and(|items| items.len() >= 2)
        || !value["engineeringFacts"]
            .as_array()
            .is_some_and(|items| items.len() == 3)
    {
        return Err("survival scan reduced-evidence boundary is invalid".into());
    }
    Ok(())
}

fn validate_repository_snapshot_identity(value: &Value) -> Result<(), String> {
    let snapshot = value
        .as_object()
        .ok_or_else(|| "repository snapshot identity must be an object".to_string())?;
    let expected = [
        "identity",
        "repoIdentity",
        "head",
        "workingTreePolicy",
        "scope",
        "inputDigest",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if snapshot.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected {
        return Err("repository snapshot identity fields are invalid".to_string());
    }
    let repo_identity = value["repoIdentity"].as_str().unwrap_or("");
    let repo_identity_valid = ["git-lineage-v1:", "content-v1:"]
        .iter()
        .any(|prefix| repo_identity.strip_prefix(prefix).is_some_and(valid_digest));
    let scope = value["scope"].as_array();
    if !value["identity"].as_str().is_some_and(valid_digest)
        || !repo_identity_valid
        || !value["head"].as_str().is_some_and(|head| !head.is_empty())
        || !matches!(
            value["workingTreePolicy"].as_str(),
            Some("head_only" | "explicit_overlay")
        )
        || !scope.is_some_and(|items| {
            !items.is_empty() && {
                let mut seen = BTreeSet::new();
                items.iter().all(|item| {
                    item.as_str()
                        .is_some_and(|text| !text.is_empty() && seen.insert(text))
                })
            }
        })
        || !value["inputDigest"].as_str().is_some_and(valid_digest)
    {
        return Err("repository snapshot identity values are invalid".to_string());
    }
    Ok(())
}

fn valid_authority_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.ends_with(['.', ' '])
        && !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | ':'))
}

fn valid_path_array(value: &Value) -> bool {
    let Some(values) = value.as_array() else {
        return false;
    };
    let mut seen = BTreeSet::new();
    values.iter().all(|value| {
        value
            .as_str()
            .is_some_and(|value| !value.is_empty() && seen.insert(value))
    })
}

fn validate_inventory(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("inventory payload is not UTF-8: {error}"))?;
    let records = if text.contains('\0') {
        text.split('\0')
            .filter(|record| !record.is_empty())
            .collect::<Vec<_>>()
    } else {
        text.lines()
            .filter(|record| !record.is_empty())
            .collect::<Vec<_>>()
    };
    let mut previous: Option<String> = None;
    for record in records {
        let normalized =
            portable_relative_path(record).map_err(|error| error.message().to_string())?;
        if previous.as_ref().is_some_and(|value| value >= &normalized) {
            return Err("inventory payload paths must be unique and sorted".to_string());
        }
        previous = Some(normalized);
    }
    Ok(())
}

fn portable_relative_path(value: &str) -> Result<String, ArtifactError> {
    if !path_syntax_is_portable(value) {
        return Err(ArtifactError::Contract(
            "Artifact Ref path is not portable root-relative syntax".to_string(),
        ));
    }
    let path = Path::new(value);
    let mut normalized = Vec::new();
    for component in path.components() {
        let name = match component {
            Component::Normal(name) => name.to_str().ok_or_else(|| {
                ArtifactError::Contract("Artifact Ref path is not UTF-8".to_string())
            })?,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(ArtifactError::Contract(
                    "Artifact Ref path contains a non-portable component".to_string(),
                ))
            }
        };
        if !path_component_is_unambiguous(name) {
            return Err(ArtifactError::Contract(
                "Artifact Ref path contains a Windows-ambiguous component".to_string(),
            ));
        }
        normalized.push(name);
    }
    if normalized.is_empty() {
        return Err(ArtifactError::Contract(
            "Artifact Ref path must name a file".to_string(),
        ));
    }
    Ok(normalized.join("/"))
}

// Guards the Artifact Ref path field (see caller above), ahead of component
// normalization -- distinct rule set from validate_portable_path above,
// which guards retirement callPath/deletion-patch paths directly.
fn path_syntax_is_portable(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\0')
        && !value.contains('\\')
        && !value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains(':')
        && value.split('/').all(|component| !component.is_empty())
}

fn path_component_is_unambiguous(name: &str) -> bool {
    !name.is_empty()
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && !name
            .chars()
            .any(|character| ('\u{0300}'..='\u{036f}').contains(&character))
        && !windows_reserved(name)
}

fn windows_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    matches!(
        stem.as_str(),
        "con" | "prn" | "aux" | "nul" | "conin$" | "conout$"
    ) || stem
        .strip_prefix("com")
        .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        || stem
            .strip_prefix("lpt")
            .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        || stem
            .strip_prefix("com")
            .is_some_and(|n| matches!(n, "¹" | "²" | "³"))
        || stem
            .strip_prefix("lpt")
            .is_some_and(|n| matches!(n, "¹" | "²" | "³"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Registered contracts that do not yet publish a JSON Schema.
    ///
    /// What `every_registered_contract_publishes_a_schema_file` enforces: an
    /// entry that gains a schema file fails (delete the entry), an entry that
    /// stops being registered fails (delete the entry), and a contract outside
    /// this list with no schema fails (publish the schema).
    ///
    /// What it does **not** enforce: a developer can still grow the list by
    /// appending an id and widening the array length. That edit is visible in
    /// review, but it is not gated. Gating it needs a merge-base or CI-held
    /// baseline to compare against — tracked in #210, not claimed here.
    ///
    /// Tracked by https://github.com/2233admin/code-intel-pipeline/issues/206.
    const AWAITING_SCHEMA: [&str; 5] = [
        "code-intel-anchor-verification.v1",
        "code-intel-file-inventory.v1",
        "code-intel-method-catalog.v1",
        "code-intel-sentrux-command-observation.v1",
        "code-intel-surgery-plan.v1",
    ];

    fn schemas_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../orchestration/schemas")
    }

    /// Every schema id published under `orchestration/schemas`, read once.
    ///
    /// One directory listing rather than a `Path::exists` per registered
    /// contract: the per-id form is a filesystem call inside a loop over 41
    /// contracts, which is the very finding class this repository is trying
    /// to burn down.
    fn published_schema_ids() -> BTreeSet<String> {
        let ids = fs::read_dir(schemas_dir())
            .expect("orchestration/schemas is readable")
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.strip_suffix(".schema.json"))
                    .map(str::to_string)
            })
            .collect::<BTreeSet<_>>();
        assert!(
            !ids.is_empty(),
            "orchestration/schemas listed no schema files — the check would pass vacuously"
        );
        ids
    }

    /// The production half of this file, whitespace-squeezed so that a pattern
    /// reads the same whether or not rustfmt wrapped it across lines.
    fn production_source() -> String {
        let source = include_str!("artifact_ref.rs");
        // `rfind`, not `find`: production code above carries its own
        // `#[cfg(test)]` items, and cutting at the first one would hide whole
        // contract families from the scan.
        let source = &source[..source
            .rfind("mod tests {")
            .expect("the test module bounds the production registry")];
        source.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// `const NAME: &str = "value";` items, so a match arm written with
    /// constants resolves to the same pair as one written with literals.
    fn string_consts(source: &str) -> BTreeMap<String, String> {
        let mut consts = BTreeMap::new();
        for chunk in source.split("const ").skip(1) {
            let Some((name, rest)) = chunk.split_once(": &str = ") else {
                continue;
            };
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            {
                continue;
            }
            let Some(value) = rest
                .strip_prefix('"')
                .and_then(|rest| rest.split('"').next())
            else {
                continue;
            };
            consts.insert(name.to_string(), value.to_string());
        }
        consts
    }

    /// The family functions `registered_contract` dispatches to, named by
    /// `registered_contract` itself rather than by a list kept here.
    ///
    /// A new family reaches the scanner the moment the dispatcher calls it,
    /// which is the only way it can reach production too.
    fn registry_family_names(source: &str) -> Vec<String> {
        let dispatcher = function_body(source, "registered_contract");
        // Each chunk but the last ends with the identifier that precedes the
        // next call, which is the family being dispatched to.
        let chunks = dispatcher
            .split("(schema, artifact_type)")
            .collect::<Vec<_>>();
        let mut names = Vec::new();
        for chunk in chunks.iter().take(chunks.len().saturating_sub(1)) {
            let name = chunk
                .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .next()
                .unwrap_or_default();
            if !name.is_empty() && !names.contains(&name.to_string()) {
                names.push(name.to_string());
            }
        }
        names
    }

    /// One function's body, from its signature to the next item.
    fn function_body<'a>(source: &'a str, name: &str) -> &'a str {
        let start = source
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("fn {name} is gone — the registry scanner needs updating"));
        let rest = &source[start..];
        let end = rest[1..]
            .find(" } fn ")
            .map(|offset| offset + 2)
            .unwrap_or(rest.len());
        &rest[..end]
    }

    /// The `(schema, type)` pairs the family functions match on, read out of
    /// this file's own source.
    ///
    /// The registry is `match` arms, so there is no value to iterate — the arms
    /// *are* the list. Reading them is what stops this check from becoming a
    /// second hand-maintained copy of the registry, which is precisely the
    /// drift it exists to catch. Every parsed pair is fed back through
    /// `registered_contract`, so a parse that drifts from the real arms fails
    /// loudly instead of silently checking nothing.
    ///
    /// An arm whose pattern this cannot resolve is a hard failure, not a skip:
    /// silently dropping one arm would leave its contract exempt from the
    /// schema check while the total count stayed healthy.
    fn registered_contract_arms() -> Vec<(String, String)> {
        let source = production_source();
        let consts = string_consts(&source);
        let families = registry_family_names(&source);
        assert!(
            families.len() >= 7,
            "registered_contract dispatches to only {} families — the scanner lost the dispatcher",
            families.len()
        );

        let mut arms = Vec::new();
        for family in &families {
            let body = function_body(&source, family);
            let mut cursor = 0;
            while let Some(hit) = body[cursor..].find(") =>") {
                let close = cursor + hit;
                cursor = close + ") =>".len();
                let Some(open) = body[..close].rfind('(') else {
                    continue;
                };
                let inside = &body[open + 1..close];
                let parts = inside
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>();
                if parts.len() != 2 {
                    continue;
                }
                let resolved = parts
                    .iter()
                    .map(|part| resolve_pattern_atom(part, &consts))
                    .collect::<Vec<_>>();
                match resolved.as_slice() {
                    [Some(schema), Some(artifact_type)] => {
                        arms.push((schema.clone(), artifact_type.clone()))
                    }
                    _ => panic!(
                        "{family}: match arm ({inside}) uses a form the registry scanner cannot resolve — write arms as string literals or `const NAME: &str` items, otherwise this contract silently escapes the schema check"
                    ),
                }
            }
        }
        arms
    }

    /// A match-pattern atom as either a string literal or a named constant.
    fn resolve_pattern_atom(atom: &str, consts: &BTreeMap<String, String>) -> Option<String> {
        match atom.strip_prefix('"') {
            Some(literal) => literal.split('"').next().map(str::to_string),
            None => consts.get(atom).cloned(),
        }
    }

    /// The schema ids the native code-evidence umbrella actually admits.
    ///
    /// Resolved through the `oneOf` branches and their `$defs` targets rather
    /// than by searching the file text: an id mentioned in a title, an example
    /// or an unrelated field is not coverage.
    fn native_umbrella_schema_ids() -> BTreeSet<String> {
        let text =
            fs::read_to_string(schemas_dir().join("code-evidence-native-artifacts.v1.schema.json"))
                .expect("the native code-evidence umbrella schema is published");
        let umbrella: Value =
            serde_json::from_str(&text).expect("the native umbrella schema is valid JSON");
        let branches = umbrella["oneOf"]
            .as_array()
            .expect("the native umbrella schema is a oneOf");
        let ids = branches
            .iter()
            .map(|branch| {
                let reference = branch["$ref"]
                    .as_str()
                    .unwrap_or_else(|| panic!("umbrella oneOf branch is not a $ref: {branch}"));
                let target = reference.strip_prefix("#/$defs/").unwrap_or_else(|| {
                    panic!("umbrella oneOf branch points outside $defs: {reference}")
                });
                umbrella["$defs"][target]["properties"]["schema"]["const"]
                    .as_str()
                    .unwrap_or_else(|| {
                        panic!("umbrella $defs/{target} does not pin a schema const")
                    })
                    .to_string()
            })
            .collect::<BTreeSet<_>>();
        assert!(
            !ids.is_empty(),
            "the native umbrella schema admitted no contract — the check would pass vacuously"
        );
        ids
    }

    /// Every schema/type pair `registered_contract` accepts must have a
    /// published JSON Schema, so a contract cannot be added on the Rust side
    /// while the artifact registry consumers keep validating against nothing.
    #[test]
    fn every_registered_contract_publishes_a_schema_file() {
        let arms = registered_contract_arms();
        assert!(
            arms.len() >= 40,
            "only {} contract arms parsed — the scanner stopped seeing the registry",
            arms.len()
        );

        // The parse is only trustworthy if the registry agrees with it.
        for (schema, artifact_type) in &arms {
            let contract =
                registered_contract(&json!({"artifactSchema": schema, "type": artifact_type}))
                    .unwrap_or_else(|error| {
                        panic!(
                            "{schema}/{artifact_type} parsed but is not registered: {}",
                            error.message()
                        )
                    });
            assert_eq!(contract.artifact_schema, schema);
            assert_eq!(contract.artifact_type, artifact_type);
        }
        // The one arm written with constants rather than literals: proof that
        // constant-form arms are resolved, not skipped.
        assert!(
            arms.iter()
                .any(|(schema, _)| schema == REPOSITORY_ITERATION_SCHEMA),
            "the constant-form arm did not resolve — constant arms are being dropped"
        );

        let registered = arms
            .iter()
            .map(|(schema, _)| schema.as_str())
            .collect::<BTreeSet<_>>();

        let published = published_schema_ids();
        let umbrella = native_umbrella_schema_ids();

        let mut unpublished = Vec::new();
        for schema in &registered {
            if published.contains(*schema) {
                continue;
            }
            // A markdown view carries no schema of its own; the JSON contract
            // it renders carries it, and that contract is checked on its own.
            if let Some(stem) = schema.strip_suffix("-markdown.v1") {
                assert!(
                    registered.contains(format!("{stem}.v1").as_str()),
                    "{schema} renders {stem}.v1, which is not a registered contract"
                );
                continue;
            }
            // The native code-evidence family is published as one `oneOf`
            // umbrella rather than a file per artifact.
            if umbrella.contains(*schema) {
                continue;
            }
            if AWAITING_SCHEMA.contains(schema) {
                continue;
            }
            unpublished.push(*schema);
        }
        assert!(
            unpublished.is_empty(),
            "registered contracts with no published schema: {unpublished:?} — publish orchestration/schemas/<id>.schema.json, or add the id to AWAITING_SCHEMA with an issue"
        );

        for schema in AWAITING_SCHEMA {
            assert!(
                registered.contains(schema),
                "{schema} is exempted but no longer registered — delete the exemption"
            );
            assert!(
                !published.contains(schema),
                "{schema} now publishes a schema — delete it from AWAITING_SCHEMA"
            );
        }
    }

    #[test]
    fn portable_path_rejects_cross_platform_aliases() {
        for path in [
            "",
            ".",
            "./a",
            "../a",
            "/a",
            "//server/a",
            r"C:\\a",
            "a:b",
            "a\\b",
            "con",
            "AUX.txt",
            "a.",
            "a ",
            "a//b",
            "a/",
            "CONIN$",
            "conout$.txt",
            "COM¹",
            "LPT².log",
            "e\u{301}.txt",
        ] {
            assert!(portable_relative_path(path).is_err(), "{path}");
        }
        assert_eq!(
            portable_relative_path("nested/子.bin").unwrap(),
            "nested/子.bin"
        );
    }

    #[test]
    fn verified_artifact_owns_bytes_after_hardlink_content_changes() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("code-intel-a03-owned-{nonce}"));
        fs::create_dir(&root).unwrap();
        let outside = root.with_extension("outside");
        fs::write(&outside, b"portable evidence\n").unwrap();
        fs::hard_link(&outside, root.join("payload.bin")).unwrap();
        let snapshot = "a".repeat(64);
        let reference = json!({
            "schema":"code-intel-artifact-ref.v1",
            "artifactSchema":"fixture.v1",
            "type":"fixture.data",
            "path":"payload.bin",
            "sha256":"924278019c18519b69088648b6d5b4f58fc96afa66204bab1274a5a4ee2bd2c2",
            "consumedSnapshotIdentity":snapshot
        });
        let verified = verify_artifact_ref(
            &root,
            &snapshot,
            ArtifactContract {
                artifact_schema: "fixture.v1",
                artifact_type: "fixture.data",
                max_bytes: 1024,
                validate_payload: |_| Ok(()),
            },
            &reference,
        )
        .unwrap();
        fs::write(&outside, b"changed evidence!\n").unwrap();
        assert_eq!(verified.bytes, b"portable evidence\n");
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }

    fn valid_snapshot_payload() -> Value {
        json!({
            "schema":"code-intel-repository-snapshot.v1",
            "snapshot":{
                "identity":"a".repeat(64),
                "repoIdentity":format!("content-v1:{}", "b".repeat(64)),
                "head":"unversioned",
                "workingTreePolicy":"explicit_overlay",
                "scope":["."],
                "inputDigest":"c".repeat(64)
            },
            "dirtyOverlay":{
                "present":false,
                "digest":null,
                "paths":[],
                "members":{"trackedModified":[],"trackedDeleted":[],"untracked":[],"renamed":[],"typeChanged":[],"staged":[]},
                "ignoredPolicy":"excluded_by_git_ignore"
            },
            "repository":{"kind":"unversioned"}
        })
    }

    fn valid_understanding_quadrant_payload() -> Value {
        json!({
            "schema":"code-intel-understanding-quadrant.v1",
            "snapshotIdentity":"a".repeat(64),
            "sourceOrientation":{
                "artifactSchema":"code-intel-project-orientation.v1",
                "artifactType":"project.orientation",
                "sha256":"b".repeat(64)
            },
            "classificationPolicy":{
                "schema":"code-intel-understanding-quadrant-policy.v1",
                "scoreRange":{"minimum":0,"maximum":100},
                "systemCriticalityThreshold":50,
                "evidenceConfidenceThreshold":50,
                "thresholdRule":"greater_than_or_equal_is_upper_band",
                "unknownCriticalityRule":"critical_by_default_except_declared_supporting_context",
                "methodConsumerPolicy":"C01_cards_and_C02_selection_may_consume_but_cannot_rewrite"
            },
            "items":[{
                "id":"unknown:dependencies.runtime",
                "subject":"dependencies.runtime",
                "sourceState":"unknown",
                "systemCriticality":{"score":90,"band":"critical"},
                "evidenceConfidence":{"score":0,"band":"low"},
                "quadrant":"Critical Unknown",
                "statement":"Runtime dependency authority is absent.",
                "provenance":[{"artifactType":"repository.survival-scan","artifactSha256":"c".repeat(64),"jsonPointer":"/unknowns/0"}]
            }],
            "visibleUnknowns":["unknown:dependencies.runtime"],
            "counts":{"Known Core":0,"Critical Unknown":1,"Supporting Context":0,"Deferred Unknown":0}
        })
    }

    #[test]
    fn understanding_quadrant_rejects_null_provenance_and_policy_constant_tampering() {
        let valid = valid_understanding_quadrant_payload();
        validate_understanding_quadrant(&serde_json::to_vec(&valid).unwrap()).unwrap();

        let mut null_provenance = valid.clone();
        null_provenance["items"][0]["provenance"] = json!([null]);
        assert!(
            validate_understanding_quadrant(&serde_json::to_vec(&null_provenance).unwrap())
                .is_err()
        );

        for (pointer, tampered) in [
            (
                "/classificationPolicy/schema",
                json!("code-intel-understanding-quadrant-policy.v2"),
            ),
            ("/classificationPolicy/scoreRange/maximum", json!(999)),
            (
                "/classificationPolicy/systemCriticalityThreshold",
                json!(51),
            ),
            (
                "/classificationPolicy/evidenceConfidenceThreshold",
                json!(49),
            ),
            (
                "/classificationPolicy/unknownCriticalityRule",
                json!("optimistic"),
            ),
        ] {
            let mut document = valid.clone();
            *document.pointer_mut(pointer).unwrap() = tampered;
            assert!(
                validate_understanding_quadrant(&serde_json::to_vec(&document).unwrap()).is_err(),
                "accepted policy tamper at {pointer}"
            );
        }
    }

    #[test]
    fn understanding_quadrant_rejects_duplicate_items_hidden_unknowns_and_wrong_counts() {
        let valid = valid_understanding_quadrant_payload();

        let mut duplicate_item = valid.clone();
        duplicate_item["items"] = json!([valid["items"][0].clone(), valid["items"][0].clone()]);
        assert!(
            validate_understanding_quadrant(&serde_json::to_vec(&duplicate_item).unwrap())
                .unwrap_err()
                .contains("uniquely sorted")
        );

        let mut hidden_unknown = valid.clone();
        hidden_unknown["visibleUnknowns"] = json!([]);
        assert!(
            validate_understanding_quadrant(&serde_json::to_vec(&hidden_unknown).unwrap())
                .unwrap_err()
                .contains("hides or invents unknowns")
        );

        let mut wrong_counts = valid;
        wrong_counts["counts"]["Critical Unknown"] = json!(0);
        assert!(
            validate_understanding_quadrant(&serde_json::to_vec(&wrong_counts).unwrap())
                .unwrap_err()
                .contains("counts do not match")
        );
    }

    #[test]
    fn registered_repository_snapshot_json_rejects_duplicate_extra_wrong_and_unknown_schema() {
        let valid = serde_json::to_vec(&valid_snapshot_payload()).unwrap();
        validate_repository_snapshot(&valid).unwrap();

        let duplicate = br#"{"schema":"code-intel-repository-snapshot.v1","schema":"code-intel-repository-snapshot.v1"}"#;
        assert!(validate_repository_snapshot(duplicate)
            .unwrap_err()
            .contains("duplicate"));

        let mut extra = valid_snapshot_payload();
        extra["extra"] = json!(true);
        assert!(validate_repository_snapshot(&serde_json::to_vec(&extra).unwrap()).is_err());

        let mut wrong = valid_snapshot_payload();
        wrong["schema"] = json!("code-intel-repository-snapshot.v2");
        assert!(validate_repository_snapshot(&serde_json::to_vec(&wrong).unwrap()).is_err());

        let unknown_ref = json!({"artifactSchema":"unknown-json.v1","type":"repository.snapshot"});
        assert!(registered_contract(&unknown_ref).is_err());
    }

    #[test]
    fn registered_repository_snapshot_enforces_every_nested_schema_constraint() {
        let mut invalid_repo = valid_snapshot_payload();
        invalid_repo["snapshot"]["repoIdentity"] = json!("INVALID");
        assert!(validate_repository_snapshot(&serde_json::to_vec(&invalid_repo).unwrap()).is_err());

        let mut empty_scope = valid_snapshot_payload();
        empty_scope["snapshot"]["scope"] = json!([]);
        assert!(validate_repository_snapshot(&serde_json::to_vec(&empty_scope).unwrap()).is_err());

        let mut nested_extra = valid_snapshot_payload();
        nested_extra["snapshot"]["extra"] = json!(true);
        assert!(validate_repository_snapshot(&serde_json::to_vec(&nested_extra).unwrap()).is_err());

        let mut overlay_extra = valid_snapshot_payload();
        overlay_extra["dirtyOverlay"]["members"]["extra"] = json!([]);
        assert!(
            validate_repository_snapshot(&serde_json::to_vec(&overlay_extra).unwrap()).is_err()
        );

        let mut overlay_duplicate = valid_snapshot_payload();
        overlay_duplicate["dirtyOverlay"]["paths"] = json!(["a", "a"]);
        assert!(
            validate_repository_snapshot(&serde_json::to_vec(&overlay_duplicate).unwrap()).is_err()
        );

        let mut invalid_member = valid_snapshot_payload();
        invalid_member["dirtyOverlay"]["members"]["untracked"] = json!([""]);
        assert!(
            validate_repository_snapshot(&serde_json::to_vec(&invalid_member).unwrap()).is_err()
        );
    }

    #[test]
    fn native_code_contracts_bind_each_ref_pair_to_its_payload_schema() {
        let cases = [
            (
                "code-evidence-files.v1",
                "code_evidence.files",
                json!({"schema":"code-evidence-files.v1","files":[]}),
            ),
            (
                "code-evidence-symbols.v1",
                "code_evidence.symbols",
                json!({"schema":"code-evidence-symbols.v1","symbols":[]}),
            ),
            (
                "code-evidence-chunks.v1",
                "code_evidence.chunks",
                json!({"schema":"code-evidence-chunks.v1","chunks":[]}),
            ),
            (
                "code-evidence-symbol-chunks.v1",
                "code_evidence.symbol_chunks",
                json!({"schema":"code-evidence-symbol-chunks.v1","mappings":[]}),
            ),
            (
                "code-evidence-imports.v1",
                "code_evidence.imports",
                json!({"schema":"code-evidence-imports.v1","imports":[]}),
            ),
            (
                "code-evidence-scorecard.v1",
                "code_evidence.scorecard",
                json!({"schema":"code-evidence-scorecard.v1","status":"ok","metrics":{}}),
            ),
            (
                "code-evidence-coverage.v1",
                "code_evidence.coverage",
                json!({"schema":"code-evidence-coverage.v1","parserKind":"line-heuristic","relationshipPrecision":"unknown","callGraph":"unknown"}),
            ),
            (
                "agent-code-slice-ranking.v1",
                "code_evidence.agent_slice",
                json!({"schema":"agent-code-slice-ranking.v1","strategy":"native-evidence-default","files":[]}),
            ),
        ];

        for (index, (schema, artifact_type, payload)) in cases.iter().enumerate() {
            let reference = json!({"artifactSchema":schema,"type":artifact_type});
            let contract = registered_contract(&reference).expect("all eight pairs are registered");
            (contract.validate_payload)(&serde_json::to_vec(payload).unwrap())
                .expect("matching payload must pass");

            let wrong_payload = &cases[(index + 1) % cases.len()].2;
            assert!(
                (contract.validate_payload)(&serde_json::to_vec(wrong_payload).unwrap()).is_err(),
                "{schema}/{artifact_type} accepted payload {}",
                wrong_payload["schema"]
            );
        }

        let files_ref = json!({
            "artifactSchema":"code-evidence-files.v1",
            "type":"code_evidence.files"
        });
        let files_contract = registered_contract(&files_ref).unwrap();
        let symbols_payload = br#"{"schema":"code-evidence-symbols.v1","symbols":[]}"#;
        assert!((files_contract.validate_payload)(symbols_payload).is_err());
    }

    #[test]
    fn native_files_and_imports_contracts_reject_malformed_elements() {
        let files_ref = json!({
            "artifactSchema":"code-evidence-files.v1",
            "type":"code_evidence.files"
        });
        let files_contract = registered_contract(&files_ref).unwrap();
        let files_ok = json!({"schema":"code-evidence-files.v1","files":[{"path":"src/lib.rs"}]});
        (files_contract.validate_payload)(&serde_json::to_vec(&files_ok).unwrap())
            .expect("files elements with a string path must pass");
        for files_bad in [
            json!({"schema":"code-evidence-files.v1","files":[{"path":1}]}),
            json!({"schema":"code-evidence-files.v1","files":["src/lib.rs"]}),
        ] {
            assert!(
                (files_contract.validate_payload)(&serde_json::to_vec(&files_bad).unwrap())
                    .is_err(),
                "files payload without a string path passed: {files_bad}"
            );
        }

        let imports_ref = json!({
            "artifactSchema":"code-evidence-imports.v1",
            "type":"code_evidence.imports"
        });
        let imports_contract = registered_contract(&imports_ref).unwrap();
        let imports_ok = json!({
            "schema":"code-evidence-imports.v1",
            "imports":[{"file":"src/lib.rs","target":"./util"}]
        });
        (imports_contract.validate_payload)(&serde_json::to_vec(&imports_ok).unwrap())
            .expect("imports elements with string file and target must pass");
        for imports_bad in [
            json!({"schema":"code-evidence-imports.v1","imports":[{"file":"src/lib.rs"}]}),
            json!({"schema":"code-evidence-imports.v1","imports":[{"file":"src/lib.rs","target":7}]}),
        ] {
            assert!(
                (imports_contract.validate_payload)(&serde_json::to_vec(&imports_bad).unwrap())
                    .is_err(),
                "imports payload without string file/target passed: {imports_bad}"
            );
        }
    }

    #[test]
    fn sentrux_command_observation_v1_preserves_optional_structured_data_contract() {
        let reference = json!({
            "artifactSchema":"code-intel-sentrux-command-observation.v1",
            "type":"provider.sentrux.command-observation"
        });
        let contract = registered_contract(&reference).expect("Sentrux observation is registered");
        let base = json!({
            "schema":"code-intel-sentrux-command-observation.v1",
            "snapshotIdentity":"a".repeat(64),
            "commands":[
                {
                    "id":"gate",
                    "argv":["code-intel", "sentrux", "gate", "."],
                    "exitCode":0,
                    "success":true,
                    "stdout":"ok",
                    "stderr":""
                },
                {
                    "id":"check",
                    "argv":["code-intel", "sentrux", "check", "."],
                    "exitCode":0,
                    "success":true,
                    "stdout":"ok",
                    "stderr":""
                }
            ]
        });
        (contract.validate_payload)(&serde_json::to_vec(&base).unwrap())
            .expect("pre-structuredData v1 observation must remain valid");

        for structured_data in [Value::Null, json!({"quality_signal": 9200}), json!([1, 2])] {
            let mut with_structured_data = base.clone();
            for command in with_structured_data["commands"].as_array_mut().unwrap() {
                command["structuredData"] = structured_data.clone();
            }
            (contract.validate_payload)(&serde_json::to_vec(&with_structured_data).unwrap())
                .expect("null, object, and array structuredData must be valid");
        }

        let mut scalar_structured_data = base.clone();
        scalar_structured_data["commands"][0]["structuredData"] = json!(true);
        assert!(
            (contract.validate_payload)(&serde_json::to_vec(&scalar_structured_data).unwrap())
                .is_err(),
            "scalar structuredData must be rejected"
        );

        let mut unexpected_field = base;
        unexpected_field["commands"][0]["unexpected"] = json!(true);
        assert!(
            (contract.validate_payload)(&serde_json::to_vec(&unexpected_field).unwrap()).is_err(),
            "v1 command observations must still reject unknown fields"
        );
    }

    #[test]
    fn sentrux_capability_artifact_contract_accepts_success_and_requires_failure_details() {
        let reference = json!({
            "artifactSchema":"code-intel-sentrux-capability-artifact.v1",
            "type":"provider.sentrux.capability-artifact"
        });
        let contract = registered_contract(&reference).expect("Sentrux artifact is registered");
        let base = json!({
            "schema":"code-intel-sentrux-capability-artifact.v1",
            "contractVersion":1,
            "capabilityId":"sentrux.scan",
            "operation":"scan",
            "runId":"run-1",
            "snapshotIdentity":"a".repeat(64),
            "provider":{
                "mode":"builtin",
                "id":"sentrux.builtin",
                "version":"1.0.0",
                "digest":"b".repeat(64)
            },
            "status":"succeeded",
            "authority":"authoritative",
            "inputs":{},
            "outputs":{"artifacts":[]},
            "failure":null,
            "freshness":{
                "status":"current",
                "evaluatedAt":"2026-08-18T00:00:00Z",
                "consumedSnapshotIdentity":"a".repeat(64)
            },
            "decisionConsumers":["release_gate"]
        });
        (contract.validate_payload)(&serde_json::to_vec(&base).unwrap())
            .expect("successful Sentrux artifact with null failure must pass");

        let mut failed = base.clone();
        failed["status"] = json!("failed");
        failed["failure"] = json!({
            "kind":"provider_error",
            "message":"command failed",
            "retryable":true
        });
        (contract.validate_payload)(&serde_json::to_vec(&failed).unwrap())
            .expect("failed Sentrux artifact with failure details must pass");

        failed["failure"] = Value::Null;
        assert!(
            (contract.validate_payload)(&serde_json::to_vec(&failed).unwrap()).is_err(),
            "failed Sentrux artifact must not silently omit failure details"
        );
    }

    fn deletion_file(path: &str, base: &str, result: &str, added: Vec<&str>) -> Value {
        json!({
            "path":path,
            "baseBlobSha256":sha256_hex(base.as_bytes()),
            "resultBlobSha256":sha256_hex(result.as_bytes()),
            "baseText":base,
            "resultText":result,
            "hunks":[{
                "oldStart":1,"oldLines":1,"newStart":1,"newLines":added.len(),
                "deletedLines":["legacy"],"addedLines":added
            }]
        })
    }

    fn deletion_diff(files: Vec<Value>, affected: Vec<&str>) -> Value {
        let patch_sha = sha256_hex(&serde_json::to_vec(&files).unwrap());
        json!({
            "schema":"code-intel-compatibility-retirement-deletion-diff.v1",
            "snapshotIdentity":"a".repeat(64),"retirementId":"ret-1","legacyBranchId":"legacy.branch",
            "affectedFiles":affected,"deletionsOnly":true,"summary":"delete only; summary has no authority",
            "patch":{"algorithm":"replayable-delete-only-v1","sha256":patch_sha,"files":files}
        })
    }

    #[test]
    fn retirement_deletion_patch_replays_pure_deletion_and_rejects_forged_addition() {
        let valid = deletion_diff(
            vec![deletion_file(
                "legacy/run-code-intel.ps1",
                "legacy\nkeep\n",
                "keep\n",
                vec![],
            )],
            vec!["legacy/run-code-intel.ps1"],
        );
        validate_retirement_deletion_diff_value(&valid).unwrap();

        let forged = deletion_diff(
            vec![deletion_file(
                "legacy/run-code-intel.ps1",
                "legacy\nkeep\n",
                "new-executable-code\nkeep\n",
                vec!["new-executable-code"],
            )],
            vec!["legacy/run-code-intel.ps1"],
        );
        let error = validate_retirement_deletion_diff_value(&forged).unwrap_err();
        assert!(error.contains("added or replacement"));
    }

    #[test]
    fn retirement_deletion_patch_rejects_hidden_touched_path_even_with_valid_hashes() {
        let hidden = deletion_diff(
            vec![
                deletion_file(
                    "legacy/run-code-intel.ps1",
                    "legacy\nkeep\n",
                    "keep\n",
                    vec![],
                ),
                deletion_file("second-branch.ps1", "legacy\nkeep\n", "keep\n", vec![]),
            ],
            vec!["legacy/run-code-intel.ps1"],
        );
        let error = validate_retirement_deletion_diff_value(&hidden).unwrap_err();
        assert!(error.contains("touched paths differ"));
    }

    fn minimal_hospital_report() -> Value {
        json!({
            "schema": "code-intel-hospital.v1",
            "domainVerdict": "pass",
            "generatedAt": null,
            "repo": null,
            "mode": null,
            "artifacts": null,
            "triage": {
                "status": "green",
                "disposition": "observe",
                "next_protocol": "post_op"
            },
            "state_machine": null,
            "modalities": null,
            "policies": null,
            "report_quality": null,
            "diagnosis": null,
            "treatment": null,
            "protocols": null,
            "tools": null,
            "surgery_plan": {
                "schema": "code-intel-surgery-plan.v1",
                "status": "not_required",
                "admission": {},
                "primary_target": {},
                "operating_plan": [],
                "verification": [],
                "discharge_criteria": []
            }
        })
    }

    #[test]
    fn hospital_report_accepts_the_optional_audit_block_and_rejects_malformed_ones() {
        let base = minimal_hospital_report();
        validate_hospital_report(&serde_json::to_vec(&base).unwrap()).unwrap();

        let mut with_audit = base.clone();
        with_audit["audit"] = json!({
            "status": "present",
            "artifact": "audit-report.json",
            "overall": 7.0,
            "findings_total": 1,
            "by_severity": { "medium": 1 }
        });
        validate_hospital_report(&serde_json::to_vec(&with_audit).unwrap()).unwrap();

        let mut score_out_of_range = with_audit.clone();
        score_out_of_range["audit"]["overall"] = json!(11.0);
        let error = validate_hospital_report(&serde_json::to_vec(&score_out_of_range).unwrap())
            .unwrap_err();
        assert!(error.contains("audit block"));

        let mut unknown_severity = with_audit.clone();
        unknown_severity["audit"]["by_severity"] = json!({ "catastrophic": 1 });
        let error =
            validate_hospital_report(&serde_json::to_vec(&unknown_severity).unwrap()).unwrap_err();
        assert!(error.contains("audit block"));

        let mut extra_key = with_audit.clone();
        extra_key["audit"]["surprise"] = json!(true);
        let error = validate_hospital_report(&serde_json::to_vec(&extra_key).unwrap()).unwrap_err();
        assert!(error.contains("audit block"));
    }

    #[test]
    fn audit_report_contract_accepts_the_fixture_and_rejects_an_unknown_field() {
        let fixture = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audit/audit-report.v1.example.json"),
        )
        .unwrap();
        validate_audit_report(&fixture).unwrap();

        let mut value: Value = serde_json::from_slice(&fixture).unwrap();
        value["bogus"] = json!(true);
        let error = validate_audit_report(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.contains("unrecognized field"), "{error}");
    }
    #[test]
    fn design_proposal_contracts_bind_exact_pairs_limits_and_shared_validators() {
        let snapshot = json!({
            "identity": "a".repeat(64),
            "repoIdentity": format!("content-v1:{}", "b".repeat(64)),
            "head": "head",
            "workingTreePolicy": "explicit_overlay",
            "scope": ["."],
            "inputDigest": "c".repeat(64)
        });
        let context = json!({
            "schema": "code-intel-design-context.v1",
            "type": "design.context",
            "snapshot": snapshot.clone(),
            "evidenceRefs": [],
            "methods": [],
            "constraints": [],
            "knownUnknowns": []
        });
        let fixture = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/design-proposal/valid-two-option.json"),
        )
        .unwrap();
        let mut candidate: Value = serde_json::from_slice(&fixture).unwrap();
        candidate["snapshot"] = snapshot;
        let mut proposal = candidate.clone();
        proposal["schema"] = json!("code-intel-design-proposal.v1");
        proposal["kind"] = json!("proposal");

        let cases = [
            (
                "code-intel-design-context.v1",
                "design.context",
                serde_json::to_vec(&context).unwrap(),
                validate_context_payload as fn(&[u8]) -> Result<(), String>,
            ),
            (
                "code-intel-design-proposal-candidate.v1",
                "design.proposal-candidate",
                serde_json::to_vec(&candidate).unwrap(),
                validate_candidate_payload as fn(&[u8]) -> Result<(), String>,
            ),
            (
                "code-intel-design-proposal.v1",
                "design.proposal",
                serde_json::to_vec(&proposal).unwrap(),
                validate_proposal_payload as fn(&[u8]) -> Result<(), String>,
            ),
        ];

        for (schema, artifact_type, bytes, validator) in cases {
            let reference = json!({"artifactSchema": schema, "type": artifact_type});
            let contract = registered_contract(&reference).unwrap();
            assert_eq!(contract.artifact_schema, schema);
            assert_eq!(contract.artifact_type, artifact_type);
            assert_eq!(contract.max_bytes, 8 * 1024 * 1024);
            assert_eq!(contract.validate_payload as usize, validator as usize);
            validator(&bytes).unwrap();
        }

        let candidate_ref = json!({
            "artifactSchema": "code-intel-design-proposal-candidate.v1",
            "type": "design.proposal-candidate"
        });
        let candidate_contract = registered_contract(&candidate_ref).unwrap();
        let mut candidate_bad = candidate.clone();
        candidate_bad["options"][0]["boundaryChanges"] = json!([]);
        assert!((candidate_contract.validate_payload)(&serde_json::to_vec(&candidate_bad).unwrap()).is_err());

        let proposal_ref = json!({
            "artifactSchema": "code-intel-design-proposal.v1",
            "type": "design.proposal"
        });
        let proposal_contract = registered_contract(&proposal_ref).unwrap();
        let mut proposal_bad = proposal;
        proposal_bad["options"][0]["validationPlan"] = json!([]);
        assert!((proposal_contract.validate_payload)(&serde_json::to_vec(&proposal_bad).unwrap()).is_err());
    }
}
