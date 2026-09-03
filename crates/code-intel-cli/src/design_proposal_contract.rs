use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::adapter_contract::AdapterError;
use crate::capability::reject_duplicate_json_keys;

#[cfg(test)]
#[path = "method_catalog.rs"]
mod method_catalog;
#[cfg(not(test))]
use crate::method_catalog;

const CONTEXT_SCHEMA: &str = "code-intel-design-context.v1";
const CANDIDATE_SCHEMA: &str = "code-intel-design-proposal-candidate.v1";
const RESULT_SCHEMA: &str = "code-intel-design-proposal.v1";
const CONTEXT_TYPE: &str = "design.context";
const CAPABILITY: &str = "advisory.design-proposal.compat";
const METHODS_ROOT: &str = "orchestration/methods";

pub(crate) fn parse_payload(bytes: &[u8], label: &str) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("{label} is not UTF-8: {error}"))?;
    reject_duplicate_json_keys(text)?;
    serde_json::from_str(text).map_err(|error| format!("{label} is not valid JSON: {error}"))
}

pub(crate) fn validate_context_payload(bytes: &[u8]) -> Result<(), String> {
    let value = parse_payload(bytes, "design context")?;
    validate_context_shape(&value).map_err(|error| error_message(&error))
}

pub(crate) fn validate_candidate_payload(bytes: &[u8]) -> Result<(), String> {
    let value = parse_payload(bytes, "design proposal candidate")?;
    validate_payload_contract(&value, CANDIDATE_SCHEMA, "design_proposal_candidate")
        .map_err(|error| error_message(&error))
}

pub(crate) fn validate_proposal_payload(bytes: &[u8]) -> Result<(), String> {
    let value = parse_payload(bytes, "design proposal")?;
    validate_payload_contract(&value, RESULT_SCHEMA, "proposal")
        .map_err(|error| error_message(&error))
}

pub(crate) fn validate_payload_contract(
    value: &Value,
    expected_schema: &str,
    expected_kind: &str,
) -> Result<(), AdapterError> {
    validate_proposal_shape(value, expected_schema, expected_kind)?;
    validate_option_requirements(value)
}

pub(crate) fn validate_candidate_shape(candidate: &Value) -> Result<(), AdapterError> {
    validate_proposal_shape(candidate, CANDIDATE_SCHEMA, "design_proposal_candidate")
}

pub(crate) fn validate_proposal_shape(
    candidate: &Value,
    expected_schema: &str,
    expected_kind: &str,
) -> Result<(), AdapterError> {
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
    if candidate["schema"] != expected_schema || candidate["kind"] != expected_kind {
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
        let has_evidence_ids = method.contains_key("evidenceIds");
        let expected_keys = if has_evidence_ids {
            &["id", "evidenceRefs", "evidenceIds"][..]
        } else {
            &["id", "evidenceRefs"][..]
        };
        exact_keys(method, expected_keys, "proposal_invalid_shape", "candidate.methods[]")?;
        if !nonempty_string(method.get("id")) || !nonempty_evidence_refs(method.get("evidenceRefs")) {
            return Err(contract("proposal_invalid_shape", "candidate.methods entry is incomplete"));
        }
        if has_evidence_ids && !nonempty_string_array(method.get("evidenceIds")) {
            return Err(contract(
                "proposal_invalid_shape",
                "candidate.methods[].evidenceIds must be a non-empty string array",
            ));
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

pub(crate) fn validate_options(options: &[Value]) -> Result<(), AdapterError> {
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
            if !option
                .get(field)
                .is_some_and(|value| string_array(value, &format!("candidate.options[].{field}")))
            {
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

pub(crate) fn validate_option_requirements(candidate: &Value) -> Result<(), AdapterError> {
    for option in candidate["options"].as_array().map_or(&[][..], Vec::as_slice) {
        for field in ["boundaryChanges", "validationPlan"] {
            if !nonempty_string_array(option.get(field)) {
                return Err(contract(
                    "proposal_invalid_shape",
                    format!("candidate.options[].{field} must be non-empty"),
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_recommendation(recommendation: &Value, options: &[Value]) -> Result<(), AdapterError> {
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

pub(crate) fn validate_methods(candidate: &Value, context: &Value) -> Result<(), AdapterError> {
    let catalog = load_catalog().map_err(|error| {
        contract(
            "proposal_validation_unknown",
            format!("method catalog unavailable: {}", error_message(&error)),
        )
    })?;
    let context_methods = context["methods"].as_array().map_or(&[][..], Vec::as_slice);
    let context_evidence = evidence_set(context)?;
    let mut unknown_methods = Vec::new();
    for method in candidate["methods"].as_array().map_or(&[][..], Vec::as_slice) {
        let id = method["id"].as_str().unwrap();
        let card = catalog.cards().iter().find(|card| card["id"] == id);
        let refs = method["evidenceRefs"].as_array().unwrap();
        let refs_are_available = refs.iter().all(|reference| {
            reference.as_str().is_some_and(|reference| context_evidence.contains(reference))
        });
        if let Some(card) = card {
            let required_evidence = card
                .get("requiredEvidence")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty());
            let required_ids_are_valid = required_evidence.is_some_and(|items| {
                items.iter().all(|item| {
                    item.as_object()
                        .and_then(|item| item.get("id"))
                        .and_then(Value::as_str)
                        .is_some_and(|id| !id.is_empty())
                })
            });
            if !required_ids_are_valid {
                return Err(contract(
                    "proposal_validation_unknown",
                    format!("method card requiredEvidence is unavailable for method {id}"),
                ));
            }
            let required_ids = required_evidence
                .unwrap()
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>();
            let Some(provided_ids) = method.get("evidenceIds").and_then(Value::as_array) else {
                return Err(contract(
                    "proposal_method_not_applicable",
                    format!("candidate does not identify required evidence for method {id}"),
                ));
            };
            let provided_ids = provided_ids.iter().filter_map(Value::as_str).collect::<Vec<_>>();
            if provided_ids.len() != required_ids.len()
                || required_ids.iter().any(|required| !provided_ids.contains(required))
            {
                return Err(contract(
                    "proposal_method_not_applicable",
                    format!("candidate evidence IDs do not match required evidence for method {id}"),
                ));
            }
            if context_evidence.len() < required_ids.len() {
                return Err(contract(
                    "proposal_method_not_applicable",
                    format!("context does not represent all required evidence for method {id}"),
                ));
            }
            if !context_methods.iter().any(|value| value.as_str() == Some(id))
                || !refs_are_available
            {
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

pub(crate) fn validate_evidence_refs(candidate: &Value, context: &Value) -> Result<(), AdapterError> {
    let context_refs = evidence_set(context)?;
    let mut references = Vec::new();
    references.extend(section_refs(candidate, "baseline")?);
    references.extend(section_refs(candidate, "delta")?);
    for method in candidate["methods"].as_array().map_or(&[][..], Vec::as_slice) {
        references.extend(value_refs(method.get("evidenceRefs"))?);
    }
    for option in candidate["options"].as_array().map_or(&[][..], Vec::as_slice) {
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

pub(crate) fn validate_snapshot(candidate: &Value, context: &Value) -> Result<(), AdapterError> {
    validate_snapshot_object(&candidate["snapshot"], "candidate.snapshot")?;
    validate_snapshot_object(&context["snapshot"], "context.snapshot")?;
    if candidate["snapshot"] != context["snapshot"] {
        return Err(contract(
            "proposal_snapshot_mismatch",
            "candidate and context snapshots differ",
        ));
    }
    Ok(())
}

pub(crate) fn validate_context_shape(context: &Value) -> Result<(), AdapterError> {
    let object = context.as_object().ok_or_else(|| {
        contract("proposal_invalid_shape", "context must be an object")
    })?;
    exact_keys(
        object,
        &[
            "schema",
            "type",
            "snapshot",
            "evidenceRefs",
            "methods",
            "constraints",
            "knownUnknowns",
        ],
        "proposal_invalid_shape",
        "context",
    )?;
    if context["schema"] != CONTEXT_SCHEMA || context["type"] != CONTEXT_TYPE {
        return Err(contract("proposal_invalid_shape", "context schema or type is invalid"));
    }
    validate_snapshot_object(&context["snapshot"], "context.snapshot")?;
    if !context["evidenceRefs"].as_array().is_some_and(|values| {
        values.iter().all(|value| value.as_str().is_some_and(valid_evidence_ref))
    })
        || !string_array(&context["methods"], "context.methods")
        || !string_array(&context["constraints"], "context.constraints")
        || !string_array(&context["knownUnknowns"], "context.knownUnknowns")
    {
        return Err(contract("proposal_invalid_shape", "context arrays must contain strings"));
    }
    Ok(())
}

pub(crate) fn validate_snapshot_object(value: &Value, name: &str) -> Result<(), AdapterError> {
    let object = value.as_object().ok_or_else(|| {
        contract("proposal_invalid_shape", format!("{name} must be an object"))
    })?;
    let expected = [
        "identity",
        "repoIdentity",
        "head",
        "workingTreePolicy",
        "scope",
        "inputDigest",
    ];
    exact_keys(object, &expected, "proposal_invalid_shape", name)?;
    if !object
        .get("identity")
        .and_then(Value::as_str)
        .is_some_and(valid_digest)
        || !object
            .get("repoIdentity")
            .and_then(Value::as_str)
            .is_some_and(valid_repo_identity)
        || !object
            .get("inputDigest")
            .and_then(Value::as_str)
            .is_some_and(valid_digest)
        || object
            .get("head")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || !matches!(
            object.get("workingTreePolicy").and_then(Value::as_str),
            Some("head_only" | "explicit_overlay")
        )
        || !object.get("scope").is_some_and(|scope| {
            scope
                .as_array()
                .is_some_and(|scope| !scope.is_empty() && scope.iter().all(|item| item.as_str().is_some_and(|item| !item.is_empty())))
        })
    {
        return Err(contract(
            "proposal_invalid_shape",
            format!("{name} snapshot fields are invalid"),
        ));
    }
    Ok(())
}

pub(crate) fn load_catalog() -> Result<method_catalog::MethodCatalog, AdapterError> {
    let root = option_env!("CODE_INTEL_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    method_catalog::load_catalog(&root.join(METHODS_ROOT))
        .map_err(|error| AdapterError::Unavailable(error.to_string()))
}

pub(crate) fn section_refs<'a>(candidate: &'a Value, field: &str) -> Result<Vec<String>, AdapterError> {
    value_refs(candidate[field].get("evidenceRefs"))
}

pub(crate) fn value_refs(value: Option<&Value>) -> Result<Vec<String>, AdapterError> {
    let values = value.and_then(Value::as_array).ok_or_else(|| {
        contract("proposal_evidence_missing", "evidenceRefs must be an array")
    })?;
    Ok(values.iter().filter_map(Value::as_str).map(ToOwned::to_owned).collect())
}

pub(crate) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(crate) fn valid_repo_identity(value: &str) -> bool {
    value
        .strip_prefix("git-lineage-v1:")
        .or_else(|| value.strip_prefix("content-v1:"))
        .is_some_and(valid_digest)
}

pub(crate) fn valid_evidence_ref(reference: &str) -> bool {
    let Some(digest) = reference.strip_prefix("artifact://sha256/") else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(crate) fn requested_strings(value: Option<&Value>, name: &str) -> Result<Vec<String>, AdapterError> {
    optional_strings(value, name)
}

pub(crate) fn nonempty_string_array(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value
            .as_array()
            .is_some_and(|values| !values.is_empty() && values.iter().all(|item| item.as_str().is_some()))
    })
}

pub(crate) fn optional_strings(value: Option<&Value>, name: &str) -> Result<Vec<String>, AdapterError> {
    match value {
        None => Ok(Vec::new()),
        Some(value) => {
            let values = value
                .as_array()
                .ok_or_else(|| AdapterError::InvalidOptions(format!("{name} must be a string array")))?;
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .filter(|item| !item.is_empty())
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| {
                            AdapterError::InvalidOptions(format!(
                                "{name} must contain non-empty strings"
                            ))
                        })
                })
                .collect()
        }
    }
}

pub(crate) fn string_array(value: &Value, _name: &str) -> bool {
    value
        .as_array()
        .is_some_and(|values| values.iter().all(|value| value.as_str().is_some()))
}

pub(crate) fn nonempty_string(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|value| !value.is_empty())
}

pub(crate) fn nonempty_evidence_refs(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value.as_array().is_some_and(|values| {
            !values.is_empty()
                && values
                    .iter()
                    .all(|value| value.as_str().is_some_and(valid_evidence_ref))
        })
    })
}

pub(crate) fn evidence_set(context: &Value) -> Result<BTreeSet<String>, AdapterError> {
    let values = context["evidenceRefs"]
        .as_array()
        .ok_or_else(|| contract("proposal_invalid_shape", "context.evidenceRefs must be an array"))?;
    let mut references = BTreeSet::new();
    for value in values {
        let reference = value
            .as_str()
            .filter(|reference| valid_evidence_ref(reference))
            .ok_or_else(|| contract("proposal_evidence_missing", "context evidence reference is malformed"))?;
        references.insert(reference.to_string());
    }
    Ok(references)
}

pub(crate) fn exact_keys(
    object: &Map<String, Value>,
    expected: &[&str],
    rule: &str,
    name: &str,
) -> Result<(), AdapterError> {
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(contract(rule, format!("{name} has unknown or missing fields")));
    }
    Ok(())
}

pub(crate) fn contract(rule: &str, detail: impl Into<String>) -> AdapterError {
    AdapterError::Contract(format!("{rule}: {}", detail.into()))
}

pub(crate) fn error_message(error: &AdapterError) -> String {
    match error {
        AdapterError::InvalidOptions(message)
        | AdapterError::Contract(message)
        | AdapterError::Unavailable(message)
        | AdapterError::Internal(message)
        | AdapterError::Io(message) => message.clone(),
    }
}
