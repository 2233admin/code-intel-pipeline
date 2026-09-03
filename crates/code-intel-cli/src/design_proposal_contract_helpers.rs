use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::adapter_contract::AdapterError;

use super::{method_catalog, METHODS_ROOT};

pub(crate) fn load_catalog() -> Result<method_catalog::MethodCatalog, AdapterError> {
    let root = option_env!("CODE_INTEL_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    method_catalog::load_catalog(&root.join(METHODS_ROOT))
        .map_err(|error| AdapterError::Unavailable(error.to_string()))
}

pub(crate) fn section_refs<'a>(
    candidate: &'a Value,
    field: &str,
) -> Result<Vec<String>, AdapterError> {
    value_refs(candidate[field].get("evidenceRefs"))
}

pub(crate) fn value_refs(value: Option<&Value>) -> Result<Vec<String>, AdapterError> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| contract("proposal_evidence_missing", "evidenceRefs must be an array"))?;
    Ok(values
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect())
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

pub(crate) fn requested_strings(
    value: Option<&Value>,
    name: &str,
) -> Result<Vec<String>, AdapterError> {
    optional_strings(value, name)
}

pub(crate) fn nonempty_string_array(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value.as_array().is_some_and(|values| {
            !values.is_empty() && values.iter().all(|item| item.as_str().is_some())
        })
    })
}

pub(crate) fn optional_strings(
    value: Option<&Value>,
    name: &str,
) -> Result<Vec<String>, AdapterError> {
    match value {
        None => Ok(Vec::new()),
        Some(value) => {
            let values = value.as_array().ok_or_else(|| {
                AdapterError::InvalidOptions(format!("{name} must be a string array"))
            })?;
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
    value
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
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
    let values = context["evidenceRefs"].as_array().ok_or_else(|| {
        contract(
            "proposal_invalid_shape",
            "context.evidenceRefs must be an array",
        )
    })?;
    let mut references = BTreeSet::new();
    for value in values {
        let reference = value
            .as_str()
            .filter(|reference| valid_evidence_ref(reference))
            .ok_or_else(|| {
                contract(
                    "proposal_evidence_missing",
                    "context evidence reference is malformed",
                )
            })?;
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
        return Err(contract(
            rule,
            format!("{name} has unknown or missing fields"),
        ));
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
