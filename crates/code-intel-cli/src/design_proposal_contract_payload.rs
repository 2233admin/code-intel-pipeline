use serde_json::Value;

use crate::adapter_contract::AdapterError;
use crate::capability::reject_duplicate_json_keys;

use super::{
    error_message, validate_context_shape, validate_option_requirements, validate_proposal_shape,
    CANDIDATE_SCHEMA, RESULT_SCHEMA,
};

pub(crate) fn parse_payload(bytes: &[u8], label: &str) -> Result<Value, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("{label} is not UTF-8: {error}"))?;
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
