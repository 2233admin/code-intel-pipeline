// Validation is part of the owning workflow adapter contract, so reuse its
// types and constants through one module boundary.
use super::*;

pub(super) fn validate_catalog(catalog: &Value) -> Result<(), AdapterError> {
    exact_keys(
        catalog,
        &["schema", "candidates"],
        "workflow adapter catalog",
    )?;
    if catalog["schema"] != "code-intel-workflow-adapter-catalog.v1" {
        return Err(AdapterError::Contract(
            "workflow adapter catalog schema is invalid".into(),
        ));
    }
    let candidates = catalog["candidates"]
        .as_array()
        .filter(|items| items.len() >= 3)
        .ok_or_else(|| AdapterError::Contract("workflow adapter catalog is incomplete".into()))?;
    let mut ids = BTreeSet::new();
    let mut adapters = BTreeSet::new();
    for candidate in candidates {
        exact_keys(
            candidate,
            &[
                "candidate",
                "adapter",
                "stack",
                "source",
                "capabilities",
                "configurationRoots",
                "entryActions",
                "setupActions",
                "maintenanceActions",
                "runtimeBoundary",
            ],
            "workflow adapter candidate",
        )?;
        let id = nonempty(candidate, "candidate")?;
        let adapter = nonempty(candidate, "adapter")?;
        if !ADAPTERS.contains(&adapter)
            || !ids.insert(id)
            || !adapters.insert(adapter)
            || candidate["stack"] != "spec-driven"
            || candidate["runtimeBoundary"] != "reference-only-no-runtime-dependency"
        {
            return Err(AdapterError::Contract(
                "workflow adapter catalog contains an invalid candidate".into(),
            ));
        }
        validate_source(&candidate["source"])?;
        for field in ["entryActions", "setupActions", "maintenanceActions"] {
            let actions = candidate[field].as_array().ok_or_else(|| {
                AdapterError::Contract(format!("workflow adapter {field} must be an array"))
            })?;
            for action in actions {
                validate_action(action)?;
            }
        }
    }
    if adapters != ADAPTERS.into_iter().collect() {
        return Err(AdapterError::Contract(
            "workflow adapter catalog must contain the governed adapter set".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_v2_bytes(bytes: &[u8]) -> Result<(), String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    validate_v2(&value).map_err(|error| adapter_error_message(&error).to_string())
}

pub(crate) fn validate_authority_event_bytes(bytes: &[u8]) -> Result<(), String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "authority event must be an object".to_string())?;
    let expected = [
        "schema",
        "id",
        "decision",
        "approver",
        "evidenceIds",
        "issuedAt",
        "expiresAt",
        "attestation",
    ];
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err("authority event fields are not exact".into());
    }
    if value["schema"] != "code-intel-authority-event.v1"
        || value["decision"] != "approved"
        || value["id"].as_str().is_none_or(str::is_empty)
        || value["approver"]["id"].as_str().is_none_or(str::is_empty)
        || value["approver"]["role"].as_str().is_none_or(str::is_empty)
        || value["evidenceIds"].as_array().is_none_or(Vec::is_empty)
    {
        return Err("authority event content is invalid".into());
    }
    let expected_digest = crate::artifact_ref::content_contract::authority_event_digest(&value)?;
    if value["attestation"]["scheme"] != "repository-governed-sha256-v1"
        || value["attestation"]["digest"].as_str() != Some(expected_digest.as_str())
    {
        return Err("authority event attestation is invalid".into());
    }
    Ok(())
}

pub(super) fn validate_v2(value: &Value) -> Result<(), AdapterError> {
    exact_keys(
        value,
        &[
            "schema",
            "kind",
            "recommendation",
            "evidence",
            "confidence",
            "alternatives",
            "provenance",
            "effects",
            "conflict",
            "manualOverride",
            "handoffs",
        ],
        "workflow recommendation v2",
    )?;
    if value["schema"] != V2_SCHEMA
        || value["kind"] != "proposal"
        || value["effects"]
            .as_array()
            .is_none_or(|items| !items.is_empty())
        || value["evidence"].as_array().is_none_or(Vec::is_empty)
        || value["alternatives"]
            .as_array()
            .is_none_or(|items| items.len() < 3)
        || value["provenance"]["capabilityId"] != "advisory.workflow-recommend.v2"
        || value["provenance"]["implementation"] != "workflow_recommendation.rs"
    {
        return Err(AdapterError::Contract(
            "workflow recommendation v2 violates the proposal boundary".into(),
        ));
    }
    if value["recommendation"].is_null() != value["conflict"].is_object() {
        return Err(AdapterError::Contract(
            "workflow recommendation conflict state is incoherent".into(),
        ));
    }
    for candidate in value["alternatives"].as_array().unwrap() {
        validate_rendered_candidate(candidate)?;
    }
    Ok(())
}

fn validate_rendered_candidate(candidate: &Value) -> Result<(), AdapterError> {
    exact_keys(
        candidate,
        &[
            "candidate",
            "stack",
            "adapter",
            "verdict",
            "score",
            "reasons",
            "presence",
            "adoption",
            "entryActions",
            "setupActions",
            "maintenanceActions",
            "source",
            "capabilities",
        ],
        "rendered workflow candidate",
    )?;
    for field in ["entryActions", "setupActions", "maintenanceActions"] {
        for action in candidate[field].as_array().ok_or_else(|| {
            AdapterError::Contract(format!("rendered candidate {field} must be an array"))
        })? {
            validate_action(action)?;
            if action["availability"] != "available"
                && action["invocations"] != json!({"codex":null,"generic":null,"cli":null})
            {
                return Err(AdapterError::Contract(
                    "uncallable action must not advertise an invocation".into(),
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_v1(value: &Value) -> Result<(), AdapterError> {
    exact_keys(
        value,
        &[
            "schema",
            "kind",
            "recommendation",
            "evidence",
            "confidence",
            "alternatives",
            "provenance",
            "effects",
        ],
        "workflow recommendation v1",
    )?;
    if value["schema"] != V1_SCHEMA
        || value["kind"] != "proposal"
        || value["effects"]
            .as_array()
            .is_none_or(|items| !items.is_empty())
        || value["alternatives"]
            .as_array()
            .is_none_or(|items| items.len() != 3)
        || value["provenance"]["capabilityId"] != "advisory.workflow-recommend"
    {
        return Err(AdapterError::Contract(
            "workflow recommendation v1 violates the compatibility boundary".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_action(action: &Value) -> Result<(), AdapterError> {
    exact_keys(
        action,
        &[
            "schema",
            "classification",
            "intent",
            "actionId",
            "availability",
            "invocations",
            "prerequisites",
            "effects",
        ],
        "workflow action",
    )?;
    if action["schema"] != "code-intel-workflow-entry-action.v1"
        || action["actionId"].as_str().is_none_or(str::is_empty)
        || action["prerequisites"].as_array().is_none()
        || action["effects"].as_array().is_none()
    {
        return Err(AdapterError::Contract(
            "workflow action content is invalid".into(),
        ));
    }
    exact_keys(
        &action["invocations"],
        &["codex", "generic", "cli"],
        "workflow action invocations",
    )?;
    Ok(())
}

fn validate_source(source: &Value) -> Result<(), AdapterError> {
    exact_keys(
        source,
        &["uri", "version", "revision", "license"],
        "workflow adapter source",
    )?;
    if nonempty(source, "uri")?.is_empty()
        || nonempty(source, "version")?.is_empty()
        || nonempty(source, "revision")?.is_empty()
        || source["license"] != "MIT"
    {
        return Err(AdapterError::Contract(
            "workflow adapter source is invalid".into(),
        ));
    }
    Ok(())
}

fn exact_keys(value: &Value, expected: &[&str], label: &str) -> Result<(), AdapterError> {
    let object = value
        .as_object()
        .ok_or_else(|| AdapterError::Contract(format!("{label} must be an object")))?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(AdapterError::Contract(format!(
            "{label} fields are not exact"
        )));
    }
    Ok(())
}

fn nonempty<'a>(value: &'a Value, key: &str) -> Result<&'a str, AdapterError> {
    value[key]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| AdapterError::Contract(format!("{key} must be non-empty")))
}

fn adapter_error_message(error: &AdapterError) -> &str {
    match error {
        AdapterError::InvalidOptions(message)
        | AdapterError::Contract(message)
        | AdapterError::Unavailable(message)
        | AdapterError::Internal(message)
        | AdapterError::Io(message) => message,
    }
}
