use std::path::Path;

use serde_json::{json, Value};

use super::{publish_named, AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;
use crate::artifact_ref::design_proposal_contract;
pub(crate) use design_proposal_contract::{validate_candidate_payload, validate_context_payload, validate_proposal_payload};

const CONTEXT_SCHEMA: &str = "code-intel-design-context.v1";
const CANDIDATE_SCHEMA: &str = "code-intel-design-proposal-candidate.v1";
const RESULT_SCHEMA: &str = "code-intel-design-proposal.v1";
const CONTEXT_TYPE: &str = "design.context";
const CANDIDATE_TYPE: &str = "design.proposal-candidate";
const RESULT_TYPE: &str = "design.proposal";
const CAPABILITY: &str = "advisory.design-proposal.compat";

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options.keys().any(|key| {
        !matches!(
            key.as_str(),
            "repoPath" | "mode" | "methodIds" | "constraints" | "knownUnknowns"
        )
    }) {
        return Err(AdapterError::InvalidOptions(
            "design proposal accepts only repoPath/mode/methodIds/constraints/knownUnknowns".into(),
        ));
    }
    match options.get("mode").and_then(Value::as_str) {
        Some("context") => build_context(request, verified_inputs, out),
        Some("validate") => validate_and_publish(request, verified_inputs, out),
        Some(other) => Err(AdapterError::InvalidOptions(format!(
            "unsupported design proposal mode: {other}"
        ))),
        None => Err(AdapterError::InvalidOptions(
            "options.mode is required".into(),
        )),
    }
}

fn build_context(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(design_proposal_contract::contract("proposal_invalid_shape", "context mode requires zero input artifacts"));
    }
    let options = request["options"].as_object().expect("validated options");
    let methods = design_proposal_contract::requested_strings(options.get("methodIds"), "options.methodIds")?;
    let constraints = design_proposal_contract::optional_strings(options.get("constraints"), "options.constraints")?;
    let mut known_unknowns =
        design_proposal_contract::optional_strings(options.get("knownUnknowns"), "options.knownUnknowns")?;
    if !methods.is_empty() && verified_inputs.is_empty() {
        known_unknowns.extend(
            methods
                .iter()
                .map(|method| format!("required evidence for method {method} is not supplied")),
        );
        return Ok(unknown_output(
            "proposal_validation_unknown: requested methods have no verified evidence inputs",
        ));
    }
    let catalog = design_proposal_contract::load_catalog()?;
    for method in &methods {
        if !catalog
            .cards()
            .iter()
            .any(|card| card.get("id").and_then(Value::as_str) == Some(method))
        {
            return Ok(unknown_output(format!(
                "proposal_method_not_applicable: method is not in the loaded catalog: {method}"
            )));
        }
        known_unknowns.push(format!("required evidence for method {method} is not supplied"));
    }

    let context = json!({
        "schema": CONTEXT_SCHEMA,
        "type": CONTEXT_TYPE,
        "snapshot": request["snapshot"].clone(),
        "evidenceRefs": verified_inputs.iter().map(|input| {
            format!("artifact://sha256/{}", input.sha256())
        }).collect::<Vec<_>>(),
        "methods": methods,
        "constraints": constraints,
        "knownUnknowns": known_unknowns,
    });
    design_proposal_contract::validate_context_shape(&context)?;
    let bytes = serde_json::to_vec(&context)
        .map_err(|error| AdapterError::Internal(format!("serialize design context: {error}")))?;
    publish_named(out, "design-context.json", &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: CONTEXT_SCHEMA.into(),
            artifact_type: CONTEXT_TYPE.into(),
            relative_path: "design-context.json".into(),
            bytes,
        }],
        observed_effects: vec!["repo_read".into(), "local_write".into()],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn validate_and_publish(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if verified_inputs.len() != 2 {
        return Err(design_proposal_contract::contract(
            "proposal_invalid_shape",
            "validate mode requires exactly one design.context and one design.proposal-candidate",
        ));
    }
    let mut context_artifact = None;
    let mut candidate_artifact = None;
    for artifact in verified_inputs {
        match (artifact.artifact_schema(), artifact.artifact_type()) {
            (CONTEXT_SCHEMA, CONTEXT_TYPE) if context_artifact.is_none() => {
                context_artifact = Some(artifact)
            }
            (CANDIDATE_SCHEMA, CANDIDATE_TYPE) if candidate_artifact.is_none() => {
                candidate_artifact = Some(artifact)
            }
            _ => {
                return Err(design_proposal_contract::contract(
                    "proposal_invalid_shape",
                    "validate inputs must contain one design.context and one design.proposal-candidate",
                ))
            }
        }
    }
    let (context_artifact, candidate_artifact) = match (context_artifact, candidate_artifact) {
        (Some(context), Some(candidate)) => (context, candidate),
        _ => {
            return Err(design_proposal_contract::contract(
                "proposal_invalid_shape",
                "validate inputs must contain one design.context and one design.proposal-candidate",
            ))
        }
    };
    let context: Value = serde_json::from_slice(context_artifact.bytes()).map_err(|error| {
        design_proposal_contract::contract(
            "proposal_invalid_shape",
            format!("design context is not valid JSON: {error}"),
        )
    })?;
    let candidate: Value = serde_json::from_slice(candidate_artifact.bytes()).map_err(|error| {
        design_proposal_contract::contract(
            "proposal_invalid_shape",
            format!("proposal candidate is not valid JSON: {error}"),
        )
    })?;

    if let Err(error) = design_proposal_contract::validate_snapshot_object(&request["snapshot"], "request.snapshot")
        .and_then(|_| design_proposal_contract::validate_context_shape(&context))
        .and_then(|_| design_proposal_contract::validate_candidate_shape(&candidate))
        .and_then(|_| design_proposal_contract::validate_snapshot(&candidate, &context))
        .and_then(|_| design_proposal_contract::validate_methods(&candidate, &context))
        .and_then(|_| design_proposal_contract::validate_evidence_refs(&candidate, &context))
        .and_then(|_| design_proposal_contract::validate_option_requirements(&candidate))
        .and_then(|_| design_proposal_contract::validate_recommendation(&candidate["recommendation"], candidate["options"].as_array().unwrap()))
    {
        return Ok(failure_output(design_proposal_contract::error_message(&error)));
    }

    if request["snapshot"] != context["snapshot"] || request["snapshot"] != candidate["snapshot"] {
        return Ok(failure_output(
            "proposal_snapshot_mismatch: request, context, and candidate snapshots differ",
        ));
    }
    let snapshot_identity = request["snapshot"]["identity"].as_str().unwrap_or("");
    if context_artifact.consumed_snapshot_identity() != snapshot_identity
        || candidate_artifact.consumed_snapshot_identity() != snapshot_identity
    {
        return Ok(failure_output(
            "proposal_snapshot_mismatch: verified input snapshot differs from payload snapshot",
        ));
    }
    let mut result = candidate;
    result["schema"] = Value::String(RESULT_SCHEMA.into());
    result["kind"] = Value::String("proposal".into());
    result["snapshot"] = request["snapshot"].clone();
    let bytes = serde_json::to_vec(&result)
        .map_err(|error| AdapterError::Internal(format!("serialize design proposal: {error}")))?;
    publish_named(out, "design-proposal.json", &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: RESULT_SCHEMA.into(),
            artifact_type: RESULT_TYPE.into(),
            relative_path: "design-proposal.json".into(),
            bytes,
        }],
        observed_effects: vec!["repo_read".into(), "local_write".into()],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}
fn failure_output(message: impl Into<String>) -> AdapterOutput {
    AdapterOutput {
        artifacts: Vec::new(),
        observed_effects: Vec::new(),
        domain_verdict: AdapterDomainVerdict::Fail,
        domain_failure: Some(message.into()),
    }
}

fn unknown_output(message: impl Into<String>) -> AdapterOutput {
    AdapterOutput {
        artifacts: Vec::new(),
        observed_effects: Vec::new(),
        domain_verdict: AdapterDomainVerdict::Unknown,
        domain_failure: Some(message.into()),
    }
}

