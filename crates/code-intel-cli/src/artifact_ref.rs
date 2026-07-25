use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use serde_json::{json, Value};

use crate::capability::{reject_duplicate_json_keys, sha256_hex, validate_artifact_ref_shape};
use crate::stable_artifact::{self, FileId, StableReadError};

const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

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
    match (
        artifact.get("artifactSchema").and_then(Value::as_str),
        artifact.get("type").and_then(Value::as_str),
    ) {
        (Some("code-intel-file-inventory.v1"), Some("inventory.files")) => Ok(ArtifactContract {
            artifact_schema: "code-intel-file-inventory.v1",
            artifact_type: "inventory.files",
            max_bytes: MAX_ARTIFACT_BYTES,
            validate_payload: validate_inventory,
        }),
        (Some("code-intel-repository-snapshot.v1"), Some("repository.snapshot")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-repository-snapshot.v1",
                artifact_type: "repository.snapshot",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_repository_snapshot,
            })
        }
        (Some("code-intel-doctor-observation.v1"), Some("doctor.observation")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-doctor-observation.v1",
                artifact_type: "doctor.observation",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_doctor_observation,
            })
        }
        (
            Some("code-intel-repository-survival-scan-result.v1"),
            Some("repository.survival-scan"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-repository-survival-scan-result.v1",
            artifact_type: "repository.survival-scan",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_survival_scan,
        }),
        (Some("code-intel-evidence-admissibility-result.v1"), Some("evidence.admission")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-evidence-admissibility-result.v1",
                artifact_type: "evidence.admission",
                max_bytes: 16 * 1024 * 1024,
                validate_payload: validate_evidence_admission,
            })
        }
        (Some("code-intel-evidence-payload.v1"), Some("observed.evidence.payload")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-evidence-payload.v1",
                artifact_type: "observed.evidence.payload",
                max_bytes: 64 * 1024 * 1024,
                validate_payload: validate_evidence_payload,
            })
        }
        (
            Some("code-intel-sentrux-command-observation.v1"),
            Some("provider.sentrux.command-observation"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-sentrux-command-observation.v1",
            artifact_type: "provider.sentrux.command-observation",
            max_bytes: 2 * 1024 * 1024,
            validate_payload: validate_sentrux_command_observation,
        }),
        (Some("code-intel-hospital.v1"), Some("diagnosis.hospital")) => Ok(ArtifactContract {
            artifact_schema: "code-intel-hospital.v1",
            artifact_type: "diagnosis.hospital",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_hospital_report,
        }),
        (Some("code-intel-hospital-markdown.v1"), Some("diagnosis.hospital-view")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-hospital-markdown.v1",
                artifact_type: "diagnosis.hospital-view",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_hospital_markdown,
            })
        }
        (Some("code-intel-surgery-plan.v1"), Some("diagnosis.surgery-plan")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-surgery-plan.v1",
                artifact_type: "diagnosis.surgery-plan",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_surgery_plan,
            })
        }
        (Some("code-intel-surgery-plan-markdown.v1"), Some("diagnosis.surgery-plan-view")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-surgery-plan-markdown.v1",
                artifact_type: "diagnosis.surgery-plan-view",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_surgery_markdown,
            })
        }
        (Some("code-intel-project-orientation.v1"), Some("project.orientation")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-project-orientation.v1",
                artifact_type: "project.orientation",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_project_orientation,
            })
        }
        (Some("code-intel-understanding-quadrant.v1"), Some("understanding.quadrant")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-understanding-quadrant.v1",
                artifact_type: "understanding.quadrant",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_understanding_quadrant,
            })
        }
        (
            Some("code-intel-compatibility-retirement-manifest.v1"),
            Some("compatibility.retirement-manifest"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-compatibility-retirement-manifest.v1",
            artifact_type: "compatibility.retirement-manifest",
            max_bytes: 4 * 1024 * 1024,
            validate_payload: validate_retirement_manifest,
        }),
        (
            Some("code-intel-compatibility-retirement-evidence.v1"),
            Some("compatibility.retirement-evidence"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-compatibility-retirement-evidence.v1",
            artifact_type: "compatibility.retirement-evidence",
            max_bytes: 4 * 1024 * 1024,
            validate_payload: validate_retirement_evidence,
        }),
        (
            Some("code-intel-compatibility-retirement-decision.v1"),
            Some("compatibility.retirement-decision"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-compatibility-retirement-decision.v1",
            artifact_type: "compatibility.retirement-decision",
            max_bytes: 4 * 1024 * 1024,
            validate_payload: validate_retirement_decision,
        }),
        (
            Some("code-intel-compatibility-retirement-ticket-template.v1"),
            Some("compatibility.retirement-ticket-template"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-compatibility-retirement-ticket-template.v1",
            artifact_type: "compatibility.retirement-ticket-template",
            max_bytes: 4 * 1024 * 1024,
            validate_payload: validate_retirement_ticket_template,
        }),
        (
            Some("code-intel-compatibility-retirement-deletion-diff.v1"),
            Some("compatibility.retirement-deletion-diff"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-compatibility-retirement-deletion-diff.v1",
            artifact_type: "compatibility.retirement-deletion-diff",
            max_bytes: 4 * 1024 * 1024,
            validate_payload: validate_retirement_deletion_diff,
        }),
        (
            Some("code-intel-project-orientation-benchmark-observations.v1"),
            Some("benchmark.orientation-observations"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-project-orientation-benchmark-observations.v1",
            artifact_type: "benchmark.orientation-observations",
            max_bytes: 64 * 1024 * 1024,
            validate_payload: validate_orientation_benchmark_observations,
        }),
        (
            Some("code-intel-project-orientation-benchmark.v1"),
            Some("benchmark.orientation-report"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-project-orientation-benchmark.v1",
            artifact_type: "benchmark.orientation-report",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_orientation_benchmark_report,
        }),
        (
            Some("code-intel-project-orientation-benchmark-markdown.v1"),
            Some("benchmark.orientation-report-view"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-project-orientation-benchmark-markdown.v1",
            artifact_type: "benchmark.orientation-report-view",
            max_bytes: 1024 * 1024,
            validate_payload: validate_orientation_benchmark_markdown,
        }),
        (Some("code-intel-run-timing-events.v1"), Some("delivery.run-timing-events")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-run-timing-events.v1",
                artifact_type: "delivery.run-timing-events",
                max_bytes: 64 * 1024 * 1024,
                validate_payload: validate_run_timing_events,
            })
        }
        (Some("code-intel-run-commit.v1"), Some("run.commit")) => Ok(ArtifactContract {
            artifact_schema: "code-intel-run-commit.v1",
            artifact_type: "run.commit",
            max_bytes: 64 * 1024,
            validate_payload: validate_run_commit,
        }),
        (Some("code-intel-run-manifest.v1"), Some("run.manifest")) => Ok(ArtifactContract {
            artifact_schema: "code-intel-run-manifest.v1",
            artifact_type: "run.manifest",
            max_bytes: 8 * 1024 * 1024,
            validate_payload: validate_run_manifest,
        }),
        (Some("code-intel-session-evidence.v1"), Some("verification.session-evidence")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-session-evidence.v1",
                artifact_type: "verification.session-evidence",
                max_bytes: 128 * 1024 * 1024,
                validate_payload: validate_session_evidence,
            })
        }
        (Some("code-intel-method-catalog.v1"), Some("method.catalog")) => Ok(ArtifactContract {
            artifact_schema: "code-intel-method-catalog.v1",
            artifact_type: "method.catalog",
            max_bytes: 256 * 1024,
            validate_payload: validate_method_catalog,
        }),
        (Some("code-intel-method-card.v1"), Some("method.card")) => Ok(ArtifactContract {
            artifact_schema: "code-intel-method-card.v1",
            artifact_type: "method.card",
            max_bytes: 256 * 1024,
            validate_payload: validate_method_card,
        }),
        (Some("code-intel-delivery-light-speed.v1"), Some("delivery.light-speed-report")) => {
            Ok(ArtifactContract {
                artifact_schema: "code-intel-delivery-light-speed.v1",
                artifact_type: "delivery.light-speed-report",
                max_bytes: 8 * 1024 * 1024,
                validate_payload: validate_light_speed_report,
            })
        }
        (
            Some("code-intel-delivery-light-speed-markdown.v1"),
            Some("delivery.light-speed-report-view"),
        ) => Ok(ArtifactContract {
            artifact_schema: "code-intel-delivery-light-speed-markdown.v1",
            artifact_type: "delivery.light-speed-report-view",
            max_bytes: 1024 * 1024,
            validate_payload: validate_light_speed_markdown,
        }),
        (Some("code-intel-decision-record.v1"), Some("decision.record")) => Ok(ArtifactContract {
            artifact_schema: "code-intel-decision-record.v1",
            artifact_type: "decision.record",
            max_bytes: 1024 * 1024,
            validate_payload: validate_decision_record_schema,
        }),
        (Some(schema), Some(artifact_type))
            if native_code_contract(schema, artifact_type).is_some() =>
        {
            let (artifact_schema, artifact_type, validate_payload) =
                native_code_contract(schema, artifact_type).expect("guard matched native contract");
            Ok(ArtifactContract {
                artifact_schema,
                artifact_type,
                max_bytes: MAX_ARTIFACT_BYTES,
                validate_payload,
            })
        }
        _ => Err(ArtifactError::Contract(
            "Artifact Ref schema/type is not registered for capability input consumption"
                .to_string(),
        )),
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
        exact_object_keys(
            command,
            &["id", "argv", "exitCode", "success", "stdout", "stderr"],
            "Sentrux command result",
        )?;
        let id = command["id"]
            .as_str()
            .filter(|id| matches!(*id, "gate" | "check"))
            .ok_or("Sentrux command id is invalid")?;
        if !seen.insert(id) {
            return Err("Sentrux command ids must be unique".into());
        }
        if command["argv"] != json!(["sentrux", id, "."])
            || (!command["exitCode"].is_null() && command["exitCode"].as_i64().is_none())
            || !command["success"].is_boolean()
            || !command["stdout"].is_string()
            || !command["stderr"].is_string()
        {
            return Err("Sentrux command result is invalid".into());
        }
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
    if value["schema"] != "code-intel-compatibility-retirement-ticket-template.v1"
        || value["status"] != "draft"
        || value["authorityBoundary"] != "template_only_no_approval_or_deletion_authority"
        || [
            "snapshotIdentity",
            "ticketId",
            "retirementId",
            "owner",
            "verifier",
        ]
        .iter()
        .any(|key| !value[key].as_str().is_some_and(|v| !v.is_empty()))
        || value["owner"] == value["verifier"]
        || value["observationExpiry"].as_u64().is_none()
        || !value["affectedFiles"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
    {
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
    exact_object_keys(
        &value,
        &[
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
        ],
        "hospital report",
    )?;
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
    if value["schema"] != "code-intel-surgery-plan.v1"
        || !matches!(value["status"].as_str(), Some("planned" | "not_required"))
        || !value["admission"].is_object()
        || !value["primary_target"].is_object()
        || !value["operating_plan"].is_array()
        || !value["verification"].is_array()
        || !value["discharge_criteria"].is_array()
    {
        return Err("surgery plan contract is invalid".into());
    }
    Ok(())
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
    if value["schema"] != "code-intel-project-orientation.v1"
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || !value["identity"].is_object()
        || !value["purpose"].is_object()
        || !value["languages"].is_array()
        || !value["boundaries"].is_array()
        || !value["entryPoints"].is_array()
        || !value["commands"].is_array()
        || !value["activeChange"].is_object()
        || !value["evidenceAvailability"].is_array()
        || !value["risks"].is_array()
        || !value["unknowns"]
            .as_array()
            .is_some_and(|unknowns| !unknowns.is_empty())
        || !matches!(
            value.pointer("/confidence/level").and_then(Value::as_str),
            Some("low" | "medium" | "high")
        )
    {
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
        for (index, claim) in value[field].as_array().unwrap().iter().enumerate() {
            validate_claim_provenance(&claim["provenance"], &format!("{field}[{index}]"))?;
        }
    }
    Ok(())
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
    if value["schema"] != "code-intel-understanding-quadrant.v1"
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || value.pointer("/sourceOrientation/artifactSchema")
            != Some(&json!("code-intel-project-orientation.v1"))
        || value.pointer("/sourceOrientation/artifactType") != Some(&json!("project.orientation"))
        || !value
            .pointer("/sourceOrientation/sha256")
            .and_then(Value::as_str)
            .is_some_and(valid_digest)
        || value.pointer("/classificationPolicy/schema")
            != Some(&json!("code-intel-understanding-quadrant-policy.v1"))
        || value.pointer("/classificationPolicy/scoreRange/minimum") != Some(&json!(0))
        || value.pointer("/classificationPolicy/scoreRange/maximum") != Some(&json!(100))
        || value.pointer("/classificationPolicy/systemCriticalityThreshold") != Some(&json!(50))
        || value.pointer("/classificationPolicy/evidenceConfidenceThreshold") != Some(&json!(50))
        || value.pointer("/classificationPolicy/thresholdRule")
            != Some(&json!("greater_than_or_equal_is_upper_band"))
        || value.pointer("/classificationPolicy/unknownCriticalityRule")
            != Some(&json!(
                "critical_by_default_except_declared_supporting_context"
            ))
        || value.pointer("/classificationPolicy/methodConsumerPolicy")
            != Some(&json!(
                "C01_cards_and_C02_selection_may_consume_but_cannot_rewrite"
            ))
    {
        return Err("understanding quadrant identity/policy contract is invalid".into());
    }
    Ok(())
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

fn expected_understanding_quadrant(
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
    if value["schema"] != "code-intel-project-orientation-benchmark-observations.v1"
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || value.pointer("/method/clock") != Some(&Value::String("std::time::Instant".into()))
        || value.pointer("/method/execution")
            != Some(&Value::String("sequential_child_process".into()))
        || value.pointer("/method/concurrency").and_then(Value::as_u64) != Some(1)
        || value.pointer("/method/llm") != Some(&Value::String("disabled".into()))
        || value
            .pointer("/environment/cleanMachine")
            .and_then(Value::as_bool)
            != Some(false)
        || value["fixtures"]
            .as_array()
            .map_or(true, |items| items.len() != 9)
    {
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
    if value["schema"] != "code-intel-project-orientation-benchmark.v1"
        || !matches!(value["verdict"].as_str(), Some("pass" | "fail"))
        || value.pointer("/target/llm") != Some(&Value::String("disabled".into()))
        || value
            .pointer("/latency/typical/p50WallTimeMs")
            .and_then(Value::as_u64)
            .is_none()
        || value
            .pointer("/latency/typical/p95WallTimeMs")
            .and_then(Value::as_u64)
            .is_none()
        || value
            .pointer("/artifactSize/typical/p95Bytes")
            .and_then(Value::as_u64)
            .is_none()
        || [
            "fieldCorrectness",
            "unresolvedCoverage",
            "unsupportedCoverage",
            "deterministicReplayRate",
            "provenanceCompleteness",
        ]
        .into_iter()
        .any(|field| {
            value["quality"][field]
                .as_f64()
                .is_none_or(|metric| !(0.0..=1.0).contains(&metric))
        })
        || value["costCenters"].as_array().is_none_or(Vec::is_empty)
    {
        return Err("orientation benchmark report contract is invalid".into());
    }
    Ok(())
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
    exact_object_keys(
        &value,
        &[
            "schema",
            "runIdentity",
            "snapshotIdentity",
            "outcome",
            "nodes",
        ],
        "run manifest",
    )?;
    if value["schema"] != "code-intel-run-manifest.v1"
        || !value["runIdentity"]
            .as_str()
            .is_some_and(valid_run_identity)
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || !matches!(
            value["outcome"].as_str(),
            Some("completed" | "domain_failed" | "domain_unknown" | "process_failed")
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
            _ => return Err("run manifest contains a non-terminal node".into()),
        }
    }
    Ok(())
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
    if value["schema"] != "code-intel-run-timing-events.v1"
        || !value["measurementSnapshotIdentity"]
            .as_str()
            .is_some_and(valid_digest)
        || value.pointer("/telemetry/mode") != Some(&Value::String("local_opt_in".into()))
        || value.pointer("/telemetry/clock") != Some(&Value::String("monotonic_elapsed_ms".into()))
        || value
            .pointer("/telemetry/externalPlatform")
            .and_then(Value::as_bool)
            != Some(false)
    {
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
            let start = event["startedAtMs"].as_u64();
            let end = event["completedAtMs"].as_u64();
            if event["id"].as_str().is_none_or(str::is_empty)
                || event["subject"].as_str().is_none_or(str::is_empty)
                || !matches!(
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
                || start.is_none()
                || end.zip(start).is_none_or(|(end, start)| end <= start)
                || event["mandatory"].as_bool().is_none()
                || !event["predecessors"].is_array()
            {
                return Err("run timing event contract is invalid".into());
            }
        }
    }
    Ok(())
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
    if value["schema"] != "code-intel-delivery-light-speed.v1"
        || !value["measurementSnapshotIdentity"]
            .as_str()
            .is_some_and(valid_digest)
        || value["authority"] != "derived_measurement_no_schedule_commitment"
        || !value["rules"]
            .as_array()
            .is_some_and(|rules| rules.len() == 7)
        || !value["baseline"].is_object()
        || !value["current"].is_object()
        || !value["delta"].is_object()
        || value["limitations"].as_array().is_none_or(Vec::is_empty)
    {
        return Err("light-speed report contract is invalid".into());
    }
    Ok(())
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
    validate_native_array_artifact(bytes, "code-evidence-files.v1", "files", 2)
}

fn validate_native_symbols(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(bytes, "code-evidence-symbols.v1", "symbols", 2)
}

fn validate_native_chunks(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(bytes, "code-evidence-chunks.v1", "chunks", 2)
}

fn validate_native_symbol_chunks(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(bytes, "code-evidence-symbol-chunks.v1", "mappings", 2)
}

fn validate_native_imports(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(bytes, "code-evidence-imports.v1", "imports", 2)
}

fn validate_native_ranking(bytes: &[u8]) -> Result<(), String> {
    validate_native_array_artifact(bytes, "agent-code-slice-ranking.v1", "files", 3)
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
) -> Result<(), String> {
    let value = parse_native_object(bytes)?;
    let object = value.as_object().expect("parse validated object");
    if value["schema"] != expected_schema
        || object.len() != expected_fields
        || !value[payload].is_array()
    {
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
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("{context} fields are invalid"));
    }
    Ok(())
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

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_run_identity(value: &str) -> bool {
    value.strip_prefix("dag-v1:").is_some_and(|tail| {
        !tail.is_empty()
            && tail.len() % 2 == 0
            && tail
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
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
    if value.is_empty()
        || value.contains('\0')
        || value.contains('\\')
        || value.starts_with('/')
        || value.starts_with("//")
        || value.contains(':')
        || value.split('/').any(|component| component.is_empty())
    {
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
        if name.is_empty()
            || name.ends_with('.')
            || name.ends_with(' ')
            || name
                .chars()
                .any(|character| ('\u{0300}'..='\u{036f}').contains(&character))
            || windows_reserved(name)
        {
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
                "run-code-intel.ps1",
                "legacy\nkeep\n",
                "keep\n",
                vec![],
            )],
            vec!["run-code-intel.ps1"],
        );
        validate_retirement_deletion_diff_value(&valid).unwrap();

        let forged = deletion_diff(
            vec![deletion_file(
                "run-code-intel.ps1",
                "legacy\nkeep\n",
                "new-executable-code\nkeep\n",
                vec!["new-executable-code"],
            )],
            vec!["run-code-intel.ps1"],
        );
        let error = validate_retirement_deletion_diff_value(&forged).unwrap_err();
        assert!(error.contains("added or replacement"));
    }

    #[test]
    fn retirement_deletion_patch_rejects_hidden_touched_path_even_with_valid_hashes() {
        let hidden = deletion_diff(
            vec![
                deletion_file("run-code-intel.ps1", "legacy\nkeep\n", "keep\n", vec![]),
                deletion_file("second-branch.ps1", "legacy\nkeep\n", "keep\n", vec![]),
            ],
            vec!["run-code-intel.ps1"],
        );
        let error = validate_retirement_deletion_diff_value(&hidden).unwrap_err();
        assert!(error.contains("touched paths differ"));
    }
}
