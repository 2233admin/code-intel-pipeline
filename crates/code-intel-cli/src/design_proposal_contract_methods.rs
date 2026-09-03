use serde_json::Value;

use crate::adapter_contract::AdapterError;

use super::{
    contract, error_message, evidence_set, exact_keys, load_catalog, section_refs, string_array,
    valid_digest, valid_evidence_ref, valid_repo_identity, value_refs, CONTEXT_SCHEMA,
    CONTEXT_TYPE,
};

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
    for method in candidate["methods"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
    {
        let id = method["id"].as_str().unwrap();
        let card = catalog.cards().iter().find(|card| card["id"] == id);
        let refs = method["evidenceRefs"].as_array().unwrap();
        let refs_are_available = refs.iter().all(|reference| {
            reference
                .as_str()
                .is_some_and(|reference| context_evidence.contains(reference))
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
            let provided_ids = provided_ids
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if provided_ids.len() != required_ids.len()
                || required_ids
                    .iter()
                    .any(|required| !provided_ids.contains(required))
            {
                return Err(contract(
                    "proposal_method_not_applicable",
                    format!(
                        "candidate evidence IDs do not match required evidence for method {id}"
                    ),
                ));
            }
            if context_evidence.len() < required_ids.len() {
                return Err(contract(
                    "proposal_method_not_applicable",
                    format!("context does not represent all required evidence for method {id}"),
                ));
            }
            if !context_methods
                .iter()
                .any(|value| value.as_str() == Some(id))
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

pub(crate) fn validate_evidence_refs(
    candidate: &Value,
    context: &Value,
) -> Result<(), AdapterError> {
    let context_refs = evidence_set(context)?;
    let mut references = Vec::new();
    references.extend(section_refs(candidate, "baseline")?);
    references.extend(section_refs(candidate, "delta")?);
    for method in candidate["methods"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
    {
        references.extend(value_refs(method.get("evidenceRefs"))?);
    }
    for option in candidate["options"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
    {
        references.extend(value_refs(option.get("evidenceRefs"))?);
    }
    if context_refs.is_empty() && !references.is_empty() {
        return Err(contract(
            "proposal_evidence_missing",
            "context contains no evidence references",
        ));
    }
    for reference in references {
        if !valid_evidence_ref(&reference) {
            return Err(contract(
                "proposal_evidence_missing",
                "evidence reference is malformed",
            ));
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
    let object = context
        .as_object()
        .ok_or_else(|| contract("proposal_invalid_shape", "context must be an object"))?;
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
        return Err(contract(
            "proposal_invalid_shape",
            "context schema or type is invalid",
        ));
    }
    validate_snapshot_object(&context["snapshot"], "context.snapshot")?;
    if !context["evidenceRefs"].as_array().is_some_and(|values| {
        values
            .iter()
            .all(|value| value.as_str().is_some_and(valid_evidence_ref))
    }) || !string_array(&context["methods"], "context.methods")
        || !string_array(&context["constraints"], "context.constraints")
        || !string_array(&context["knownUnknowns"], "context.knownUnknowns")
    {
        return Err(contract(
            "proposal_invalid_shape",
            "context arrays must contain strings",
        ));
    }
    Ok(())
}

pub(crate) fn validate_snapshot_object(value: &Value, name: &str) -> Result<(), AdapterError> {
    let object = value.as_object().ok_or_else(|| {
        contract(
            "proposal_invalid_shape",
            format!("{name} must be an object"),
        )
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
            scope.as_array().is_some_and(|scope| {
                !scope.is_empty()
                    && scope
                        .iter()
                        .all(|item| item.as_str().is_some_and(|item| !item.is_empty()))
            })
        })
    {
        return Err(contract(
            "proposal_invalid_shape",
            format!("{name} snapshot fields are invalid"),
        ));
    }
    Ok(())
}
