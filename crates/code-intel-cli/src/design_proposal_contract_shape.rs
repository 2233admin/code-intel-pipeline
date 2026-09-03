use std::collections::BTreeSet;

use serde_json::Value;

use crate::adapter_contract::AdapterError;

use super::{
    contract, exact_keys, nonempty_evidence_refs, nonempty_string, nonempty_string_array,
    string_array, validate_snapshot_object, CANDIDATE_SCHEMA, CAPABILITY,
};

pub(crate) fn validate_candidate_shape(candidate: &Value) -> Result<(), AdapterError> {
    validate_proposal_shape(candidate, CANDIDATE_SCHEMA, "design_proposal_candidate")
}

pub(crate) fn validate_proposal_shape(
    candidate: &Value,
    expected_schema: &str,
    expected_kind: &str,
) -> Result<(), AdapterError> {
    let object = candidate
        .as_object()
        .ok_or_else(|| contract("proposal_invalid_shape", "candidate must be an object"))?;
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
        Some(_) => {
            return Err(contract(
                "proposal_authority_escalation",
                "candidate authority is not advisory_only",
            ))
        }
        None => {
            return Err(contract(
                "proposal_invalid_shape",
                "candidate authority must be a string",
            ))
        }
    }
    validate_snapshot_object(&candidate["snapshot"], "candidate.snapshot")?;
    let request = candidate["request"].as_object().ok_or_else(|| {
        contract(
            "proposal_invalid_shape",
            "candidate.request must be an object",
        )
    })?;
    exact_keys(
        request,
        &["mode", "capability", "schema"],
        "proposal_invalid_shape",
        "candidate.request",
    )?;
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
            contract(
                "proposal_invalid_shape",
                format!("candidate.{field} must be an object"),
            )
        })?;
        exact_keys(
            section,
            &["summary", "evidenceRefs"],
            "proposal_invalid_shape",
            &format!("candidate.{field}"),
        )?;
        if !nonempty_string(section.get("summary"))
            || !nonempty_evidence_refs(section.get("evidenceRefs"))
        {
            return Err(contract(
                "proposal_invalid_shape",
                format!("candidate.{field} requires a non-empty summary and evidenceRefs"),
            ));
        }
    }
    let methods = candidate["methods"].as_array().ok_or_else(|| {
        contract(
            "proposal_invalid_shape",
            "candidate.methods must be an array",
        )
    })?;
    for method in methods {
        let method = method.as_object().ok_or_else(|| {
            contract(
                "proposal_invalid_shape",
                "candidate.methods entries must be objects",
            )
        })?;
        let has_evidence_ids = method.contains_key("evidenceIds");
        let expected_keys = if has_evidence_ids {
            &["id", "evidenceRefs", "evidenceIds"][..]
        } else {
            &["id", "evidenceRefs"][..]
        };
        exact_keys(
            method,
            expected_keys,
            "proposal_invalid_shape",
            "candidate.methods[]",
        )?;
        if !nonempty_string(method.get("id")) || !nonempty_evidence_refs(method.get("evidenceRefs"))
        {
            return Err(contract(
                "proposal_invalid_shape",
                "candidate.methods entry is incomplete",
            ));
        }
        if has_evidence_ids && !nonempty_string_array(method.get("evidenceIds")) {
            return Err(contract(
                "proposal_invalid_shape",
                "candidate.methods[].evidenceIds must be a non-empty string array",
            ));
        }
    }
    let options = candidate["options"].as_array().ok_or_else(|| {
        contract(
            "proposal_invalid_shape",
            "candidate.options must be an array",
        )
    })?;
    validate_options(options)?;
    for field in ["risks", "validationPlan", "limitations"] {
        if !string_array(&candidate[field], &format!("candidate.{field}")) {
            return Err(contract(
                "proposal_invalid_shape",
                format!("candidate.{field} must be a string array"),
            ));
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
            contract(
                "proposal_invalid_shape",
                "candidate.options entries must be objects",
            )
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
        let id = option
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                contract(
                    "proposal_invalid_shape",
                    "candidate.options[].id must be non-empty",
                )
            })?;
        if !ids.insert(id) {
            return Err(contract(
                "proposal_invalid_shape",
                "candidate option IDs must be unique",
            ));
        }
        for field in ["title", "summary"] {
            if !nonempty_string(option.get(field)) {
                return Err(contract(
                    "proposal_invalid_shape",
                    format!("candidate.options[].{field} must be non-empty"),
                ));
            }
        }
        for field in [
            "boundaryChanges",
            "tradeoffs",
            "assumptions",
            "validationPlan",
        ] {
            if !option
                .get(field)
                .is_some_and(|value| string_array(value, &format!("candidate.options[].{field}")))
            {
                return Err(contract(
                    "proposal_invalid_shape",
                    format!("candidate.options[].{field} must be a string array"),
                ));
            }
        }
        if !nonempty_evidence_refs(option.get("evidenceRefs")) {
            return Err(contract(
                "proposal_invalid_shape",
                "candidate.options[].evidenceRefs must be non-empty",
            ));
        }
        let reversibility = option["reversibility"].as_object().ok_or_else(|| {
            contract(
                "proposal_invalid_shape",
                "candidate.options[].reversibility must be an object",
            )
        })?;
        exact_keys(
            reversibility,
            &["status", "basis"],
            "proposal_invalid_shape",
            "candidate.options[].reversibility",
        )?;
        if !nonempty_string(reversibility.get("status"))
            || !nonempty_string(reversibility.get("basis"))
        {
            return Err(contract(
                "proposal_invalid_shape",
                "candidate.options[].reversibility is incomplete",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_option_requirements(candidate: &Value) -> Result<(), AdapterError> {
    for option in candidate["options"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
    {
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

pub(crate) fn validate_recommendation(
    recommendation: &Value,
    options: &[Value],
) -> Result<(), AdapterError> {
    let recommendation = recommendation.as_object().ok_or_else(|| {
        contract(
            "proposal_invalid_shape",
            "candidate.recommendation must be an object",
        )
    })?;
    exact_keys(
        recommendation,
        &["optionId", "rationale"],
        "proposal_invalid_shape",
        "candidate.recommendation",
    )?;
    let option_id = recommendation
        .get("optionId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            contract(
                "proposal_option_reference_invalid",
                "recommendation.optionId must be non-empty",
            )
        })?;
    if !options.iter().any(|option| option["id"] == option_id) {
        return Err(contract(
            "proposal_option_reference_invalid",
            "recommendation.optionId does not resolve to an option",
        ));
    }
    if !nonempty_string(recommendation.get("rationale")) {
        return Err(contract(
            "proposal_invalid_shape",
            "recommendation.rationale must be non-empty",
        ));
    }
    Ok(())
}
