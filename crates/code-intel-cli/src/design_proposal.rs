use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::{publish_named, AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;

const CONTEXT_SCHEMA: &str = "code-intel-design-context.v1";
const CANDIDATE_SCHEMA: &str = "code-intel-design-proposal-candidate.v1";
const RESULT_SCHEMA: &str = "code-intel-design-proposal.v1";
const CONTEXT_TYPE: &str = "design.context";
const CANDIDATE_TYPE: &str = "design.proposal-candidate";
const RESULT_TYPE: &str = "design.proposal";
const CAPABILITY: &str = "advisory.design-proposal.compat";
const METHODS_ROOT: &str = "orchestration/methods";

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
        return Err(contract("proposal_invalid_shape", "context mode requires zero input artifacts"));
    }
    let options = request["options"].as_object().expect("validated options");
    let methods = requested_strings(options.get("methodIds"), "options.methodIds")?;
    let constraints = optional_strings(options.get("constraints"), "options.constraints")?;
    let mut known_unknowns =
        optional_strings(options.get("knownUnknowns"), "options.knownUnknowns")?;

    let catalog = load_catalog()?;
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
    validate_context_shape(&context)?;
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
    let (context_artifact, candidate_artifact) = match verified_inputs {
        [context, candidate]
            if context.artifact_schema() == CONTEXT_SCHEMA
                && context.artifact_type() == CONTEXT_TYPE
                && candidate.artifact_schema() == CANDIDATE_SCHEMA
                && candidate.artifact_type() == CANDIDATE_TYPE => (context, candidate),
        _ => {
            return Err(contract(
                "proposal_invalid_shape",
                "validate mode requires exactly one design.context and one design.proposal-candidate",
            ))
        }
    };
    let context: Value = serde_json::from_slice(context_artifact.bytes()).map_err(|error| {
        contract(
            "proposal_invalid_shape",
            format!("design context is not valid JSON: {error}"),
        )
    })?;
    let candidate: Value = serde_json::from_slice(candidate_artifact.bytes()).map_err(|error| {
        contract(
            "proposal_invalid_shape",
            format!("proposal candidate is not valid JSON: {error}"),
        )
    })?;

    if let Err(error) = validate_context_shape(&context)
        .and_then(|_| validate_candidate_shape(&candidate))
        .and_then(|_| validate_snapshot(&candidate, &context))
        .and_then(|_| validate_methods(&candidate, &context))
        .and_then(|_| validate_evidence_refs(&candidate, &context))
        .and_then(|_| validate_recommendation(&candidate["recommendation"], candidate["options"].as_array().unwrap()))
    {
        return Ok(failure_output(error_message(&error)));
    }

    let request_identity = request["snapshot"]["identity"].as_str();
    let candidate_identity = candidate["snapshot"]["identity"].as_str();
    let context_identity = context["snapshot"]["identity"].as_str();
    if request_identity != candidate_identity || request_identity != context_identity {
        return Ok(failure_output("proposal_snapshot_mismatch: request, context, and candidate snapshots differ"));
    }
    if context_artifact.consumed_snapshot_identity() != context_identity.unwrap_or("")
        || candidate_artifact.consumed_snapshot_identity() != candidate_identity.unwrap_or("")
    {
        return Ok(failure_output(
            "proposal_snapshot_mismatch: verified input snapshot differs from payload snapshot",
        ));
    }

    let mut result = candidate;
    result["schema"] = Value::String(RESULT_SCHEMA.into());
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

fn validate_candidate_shape(candidate: &Value) -> Result<(), AdapterError> {
    let object = candidate.as_object().ok_or_else(|| {
        contract("proposal_invalid_shape", "candidate must be an object")
    })?;
    exact_keys(
        object,
        &[
            "schema",
            "kind",
            "authority",
            "snapshot",
            "request",
            "baseline",
            "delta",
            "methods",
            "options",
            "recommendation",
            "risks",
            "validationPlan",
            "limitations",
        ],
        "proposal_invalid_shape",
        "candidate",
    )?;
    if candidate["schema"] != CANDIDATE_SCHEMA || candidate["kind"] != "design_proposal_candidate" {
        return Err(contract(
            "proposal_invalid_shape",
            "candidate schema or kind is invalid",
        ));
    }
    match candidate["authority"].as_str() {
        Some("advisory_only") => {}
        Some(_) => return Err(contract("proposal_authority_escalation", "candidate authority is not advisory_only")),
        None => return Err(contract("proposal_invalid_shape", "candidate authority must be a string")),
    }
    validate_snapshot_object(&candidate["snapshot"], "candidate.snapshot")?;
    let request = candidate["request"].as_object().ok_or_else(|| {
        contract("proposal_invalid_shape", "candidate.request must be an object")
    })?;
    exact_keys(request, &["mode", "capability", "schema"], "proposal_invalid_shape", "candidate.request")?;
    if request["mode"] != "validate"
        || request["capability"] != CAPABILITY
        || request["schema"] != "code-intel-design-proposal-request.v1"
    {
        return Err(contract(
            "proposal_invalid_shape",
            "candidate.request provenance is invalid",
        ));
    }
    for field in ["baseline", "delta"] {
        let section = candidate[field].as_object().ok_or_else(|| {
            contract("proposal_invalid_shape", format!("candidate.{field} must be an object"))
        })?;
        exact_keys(section, &["summary", "evidenceRefs"], "proposal_invalid_shape", &format!("candidate.{field}"))?;
        if !nonempty_string(section.get("summary")) || !nonempty_evidence_refs(section.get("evidenceRefs")) {
            return Err(contract(
                "proposal_invalid_shape",
                format!("candidate.{field} requires a non-empty summary and evidenceRefs"),
            ));
        }
    }
    let methods = candidate["methods"].as_array().ok_or_else(|| {
        contract("proposal_invalid_shape", "candidate.methods must be an array")
    })?;
    for method in methods {
        let method = method.as_object().ok_or_else(|| {
            contract("proposal_invalid_shape", "candidate.methods entries must be objects")
        })?;
        exact_keys(method, &["id", "evidenceRefs"], "proposal_invalid_shape", "candidate.methods[]")?;
        if !nonempty_string(method.get("id")) || !nonempty_evidence_refs(method.get("evidenceRefs")) {
            return Err(contract("proposal_invalid_shape", "candidate.methods entry is incomplete"));
        }
    }
    let options = candidate["options"].as_array().ok_or_else(|| {
        contract("proposal_invalid_shape", "candidate.options must be an array")
    })?;
    validate_options(options)?;
    for field in ["risks", "validationPlan", "limitations"] {
        if !string_array(&candidate[field], &format!("candidate.{field}")) {
            return Err(contract("proposal_invalid_shape", format!("candidate.{field} must be a string array")));
        }
    }
    validate_recommendation(&candidate["recommendation"], options)
}

fn validate_options(options: &[Value]) -> Result<(), AdapterError> {
    if !(2..=3).contains(&options.len()) {
        return Err(contract(
            "proposal_option_count",
            "candidate.options must contain exactly two or three options",
        ));
    }
    let mut ids = BTreeSet::new();
    for option in options {
        let option = option.as_object().ok_or_else(|| {
            contract("proposal_invalid_shape", "candidate.options entries must be objects")
        })?;
        exact_keys(
            option,
            &[
                "id",
                "title",
                "summary",
                "boundaryChanges",
                "tradeoffs",
                "assumptions",
                "evidenceRefs",
                "validationPlan",
                "reversibility",
            ],
            "proposal_invalid_shape",
            "candidate.options[]",
        )?;
        let id = option.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).ok_or_else(|| {
            contract("proposal_invalid_shape", "candidate.options[].id must be non-empty")
        })?;
        if !ids.insert(id) {
            return Err(contract("proposal_invalid_shape", "candidate option IDs must be unique"));
        }
        for field in ["title", "summary"] {
            if !nonempty_string(option.get(field)) {
                return Err(contract("proposal_invalid_shape", format!("candidate.options[].{field} must be non-empty")));
            }
        }
        for field in ["boundaryChanges", "tradeoffs", "assumptions", "validationPlan"] {
            if !string_array(option.get(field).unwrap_or(&Value::Null), &format!("candidate.options[].{field}")) {
                return Err(contract("proposal_invalid_shape", format!("candidate.options[].{field} must be a string array")));
            }
        }
        if !nonempty_evidence_refs(option.get("evidenceRefs")) {
            return Err(contract("proposal_invalid_shape", "candidate.options[].evidenceRefs must be non-empty"));
        }
        let reversibility = option["reversibility"].as_object().ok_or_else(|| {
            contract("proposal_invalid_shape", "candidate.options[].reversibility must be an object")
        })?;
        exact_keys(reversibility, &["status", "basis"], "proposal_invalid_shape", "candidate.options[].reversibility")?;
        if !nonempty_string(reversibility.get("status")) || !nonempty_string(reversibility.get("basis")) {
            return Err(contract("proposal_invalid_shape", "candidate.options[].reversibility is incomplete"));
        }
    }
    Ok(())
}

fn validate_recommendation(recommendation: &Value, options: &[Value]) -> Result<(), AdapterError> {
    let recommendation = recommendation.as_object().ok_or_else(|| {
        contract("proposal_invalid_shape", "candidate.recommendation must be an object")
    })?;
    exact_keys(recommendation, &["optionId", "rationale"], "proposal_invalid_shape", "candidate.recommendation")?;
    let option_id = recommendation.get("optionId").and_then(Value::as_str).filter(|id| !id.is_empty()).ok_or_else(|| {
        contract("proposal_option_reference_invalid", "recommendation.optionId must be non-empty")
    })?;
    if !options.iter().any(|option| option["id"] == option_id) {
        return Err(contract("proposal_option_reference_invalid", "recommendation.optionId does not resolve to an option"));
    }
    if !nonempty_string(recommendation.get("rationale")) {
        return Err(contract("proposal_invalid_shape", "recommendation.rationale must be non-empty"));
    }
    Ok(())
}

fn validate_methods(candidate: &Value, context: &Value) -> Result<(), AdapterError> {
    let catalog = load_catalog().map_err(|error| {
        contract("proposal_validation_unknown", format!("method catalog unavailable: {error}"))
    })?;
    let context_methods = context["methods"].as_array().unwrap_or(&[]);
    let context_evidence = evidence_set(context)?;
    let mut unknown_methods = Vec::new();
    for method in candidate["methods"].as_array().unwrap_or(&[]) {
        let id = method["id"].as_str().unwrap();
        let known = catalog.cards().iter().any(|card| card["id"] == id);
        let refs = method["evidenceRefs"].as_array().unwrap();
        let refs_are_available = refs.iter().all(|reference| {
            reference.as_str().is_some_and(|reference| context_evidence.contains(reference))
        });
        if known {
            if !context_methods.iter().any(|value| value.as_str() == Some(id)) || !refs_are_available {
                return Err(contract(
                    "proposal_method_not_applicable",
                    format!("required evidence or selected context is unavailable for method {id}"),
                ));
            }
        } else {
            unknown_methods.push(id.to_string());
        }
    }
    if let Some(id) = unknown_methods.first() {
        return Err(contract(
            "proposal_method_not_applicable",
            format!("method is not in the loaded catalog: {id}"),
        ));
    }
    Ok(())
}

fn validate_evidence_refs(candidate: &Value, context: &Value) -> Result<(), AdapterError> {
    let context_refs = evidence_set(context)?;
    let mut references = Vec::new();
    references.extend(section_refs(candidate, "baseline")?);
    references.extend(section_refs(candidate, "delta")?);
    for method in candidate["methods"].as_array().unwrap_or(&[]) {
        references.extend(value_refs(method.get("evidenceRefs"))?);
    }
    for option in candidate["options"].as_array().unwrap_or(&[]) {
        references.extend(value_refs(option.get("evidenceRefs"))?);
    }
    if context_refs.is_empty() && !references.is_empty() {
        return Err(contract("proposal_evidence_missing", "context contains no evidence references"));
    }
    for reference in references {
        if !valid_evidence_ref(&reference) {
            return Err(contract("proposal_evidence_missing", "evidence reference is malformed"));
        }
        if !context_refs.contains(&reference) {
            return Err(contract(
                "proposal_evidence_drifted",
                format!("evidence reference is not present in the context: {reference}"),
            ));
        }
    }
    Ok(())
}

fn validate_snapshot(candidate: &Value, context: &Value) -> Result<(), AdapterError> {
    validate_snapshot_object(&candidate["snapshot"], "candidate.snapshot")?;
    validate_snapshot_object(&context["snapshot"], "context.snapshot")?;
    if candidate["snapshot"]["identity"] != context["snapshot"]["identity"] {
        return Err(contract("proposal_snapshot_mismatch", "candidate and context snapshot identities differ"));
    }
    Ok(())
}

fn validate_context_shape(context: &Value) -> Result<(), AdapterError> {
    let object = context.as_object().ok_or_else(|| {
        contract("proposal_invalid_shape", "context must be an object")
    })?;
    let allowed = [
        "schema",
        "type",
        "snapshot",
        "evidenceRefs",
        "methods",
        "constraints",
        "knownUnknowns",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(contract(
            "proposal_invalid_shape",
            "context has unknown fields",
        ));
    }
    for field in ["schema", "type", "snapshot", "evidenceRefs", "methods"] {
        if !object.contains_key(field) {
            return Err(contract(
                "proposal_invalid_shape",
                format!("context.{field} is required"),
            ));
        }
    }
    if context["schema"] != CONTEXT_SCHEMA || context["type"] != CONTEXT_TYPE {
        return Err(contract("proposal_invalid_shape", "context schema or type is invalid"));
    }
    validate_snapshot_object(&context["snapshot"], "context.snapshot")?;
    if !string_array(&context["evidenceRefs"], "context.evidenceRefs")
        || !string_array(&context["methods"], "context.methods")
        || !context
            .get("constraints")
            .is_none_or(|value| string_array(value, "context.constraints"))
        || !context
            .get("knownUnknowns")
            .is_none_or(|value| string_array(value, "context.knownUnknowns"))
    {
        return Err(contract("proposal_invalid_shape", "context arrays must contain strings"));
    }
    Ok(())
}

fn validate_snapshot_object(value: &Value, name: &str) -> Result<(), AdapterError> {
    if value
        .as_object()
        .and_then(|object| object.get("identity"))
        .and_then(Value::as_str)
        .is_some_and(|identity| !identity.is_empty())
    {
        Ok(())
    } else {
        Err(contract("proposal_invalid_shape", format!("{name}.identity must be a non-empty string")))
    }
}

fn load_catalog() -> Result<crate::method_catalog::MethodCatalog, AdapterError> {
    let root = option_env!("CODE_INTEL_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    crate::method_catalog::load_catalog(&root.join(METHODS_ROOT))
        .map_err(|error| AdapterError::Unavailable(error.to_string()))
}

fn evidence_set(context: &Value) -> Result<BTreeSet<String>, AdapterError> {
    Ok(context["evidenceRefs"]
        .as_array()
        .ok_or_else(|| contract("proposal_invalid_shape", "context.evidenceRefs must be an array"))?
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect())
}

fn section_refs<'a>(candidate: &'a Value, field: &str) -> Result<Vec<String>, AdapterError> {
    value_refs(candidate[field].get("evidenceRefs"))
}

fn value_refs(value: Option<&Value>) -> Result<Vec<String>, AdapterError> {
    let values = value.and_then(Value::as_array).ok_or_else(|| {
        contract("proposal_evidence_missing", "evidenceRefs must be an array")
    })?;
    Ok(values.iter().filter_map(Value::as_str).map(ToOwned::to_owned).collect())
}

fn requested_strings(value: Option<&Value>, name: &str) -> Result<Vec<String>, AdapterError> {
    optional_strings(value, name)
}

fn optional_strings(value: Option<&Value>, name: &str) -> Result<Vec<String>, AdapterError> {
    match value {
        None => Ok(Vec::new()),
        Some(value) => {
            let values = value.as_array().ok_or_else(|| AdapterError::InvalidOptions(format!("{name} must be a string array")))?;
            values.iter().map(|value| value.as_str().filter(|item| !item.is_empty()).map(ToOwned::to_owned).ok_or_else(|| AdapterError::InvalidOptions(format!("{name} must contain non-empty strings")))).collect()
        }
    }
}

fn string_array(value: &Value, _name: &str) -> bool {
    value.as_array().is_some_and(|values| values.iter().all(|value| value.as_str().is_some()))
}

fn nonempty_string(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|value| !value.is_empty())
}

fn nonempty_evidence_refs(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value.as_array().is_some_and(|values| {
            !values.is_empty() && values.iter().all(|value| value.as_str().is_some_and(|value| !value.is_empty()))
        })
    })
}

fn valid_evidence_ref(reference: &str) -> bool {
    let Some(digest) = reference.strip_prefix("artifact://sha256/") else {
        return false;
    };
    let digest = digest.split_once('#').map_or(digest, |(digest, _)| digest);
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn exact_keys(object: &Map<String, Value>, expected: &[&str], rule: &str, name: &str) -> Result<(), AdapterError> {
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(contract(rule, format!("{name} has unknown or missing fields")));
    }
    Ok(())
}

fn contract(rule: &str, detail: impl Into<String>) -> AdapterError {
    AdapterError::Contract(format!("{rule}: {}", detail.into()))
}

fn error_message(error: &AdapterError) -> String {
    match error {
        AdapterError::InvalidOptions(message)
        | AdapterError::Contract(message)
        | AdapterError::Unavailable(message)
        | AdapterError::Internal(message)
        | AdapterError::Io(message) => message.clone(),
    }
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

