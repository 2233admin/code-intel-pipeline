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
    exact_keys(
        &value,
        &[
            "schema",
            "id",
            "decision",
            "approver",
            "evidenceIds",
            "issuedAt",
            "expiresAt",
            "attestation",
        ],
        "authority event",
    )
    .map_err(|error| adapter_error_message(&error).to_string())?;
    exact_keys(
        &value["approver"],
        &["id", "role"],
        "authority event approver",
    )
    .map_err(|error| adapter_error_message(&error).to_string())?;
    exact_keys(
        &value["attestation"],
        &["scheme", "digest"],
        "authority event attestation",
    )
    .map_err(|error| adapter_error_message(&error).to_string())?;
    if value["schema"] != "code-intel-authority-event.v1"
        || value["decision"] != "approved"
        || nonempty(&value, "id").is_err()
        || nonempty(&value["approver"], "id").is_err()
        || nonempty(&value["approver"], "role").is_err()
        || validate_string_array(
            &value["evidenceIds"],
            "authority event evidenceIds",
            1,
            true,
        )
        .is_err()
        || value["issuedAt"].as_u64().is_none()
        || value["expiresAt"].as_u64().is_none()
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
    if value["schema"] != V2_SCHEMA || value["kind"] != "proposal" {
        return Err(AdapterError::Contract(
            "workflow recommendation v2 violates the proposal boundary".into(),
        ));
    }
    if value["effects"]
        .as_array()
        .is_none_or(|items| !items.is_empty())
    {
        return Err(AdapterError::Contract(
            "workflow recommendation effects must be an empty array".into(),
        ));
    }
    let evidence = value["evidence"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| AdapterError::Contract("workflow evidence must be non-empty".into()))?;
    for item in evidence {
        exact_keys(item, &["kind", "value"], "workflow evidence")?;
        nonempty(item, "kind")?;
        nonempty(item, "value")?;
    }
    if !matches!(
        value["confidence"].as_str(),
        Some("low" | "medium" | "high")
    ) {
        return Err(AdapterError::Contract(
            "workflow recommendation confidence is invalid".into(),
        ));
    }
    validate_provenance(&value["provenance"])?;
    validate_conflict(&value["conflict"])?;
    validate_manual_override(&value["manualOverride"])?;
    validate_handoffs(&value["handoffs"])?;

    match &value["recommendation"] {
        Value::Null => {}
        Value::Object(_) => validate_rendered_candidate(&value["recommendation"])?,
        _ => {
            return Err(AdapterError::Contract(
                "workflow recommendation must be a candidate or null".into(),
            ))
        }
    }
    if value["recommendation"].is_null() != value["conflict"].is_object() {
        return Err(AdapterError::Contract(
            "workflow recommendation conflict state is incoherent".into(),
        ));
    }
    let alternatives = value["alternatives"]
        .as_array()
        .filter(|items| items.len() >= 3)
        .ok_or_else(|| {
            AdapterError::Contract("workflow alternatives must contain three candidates".into())
        })?;
    for candidate in alternatives {
        validate_rendered_candidate(candidate)?;
    }
    Ok(())
}

fn validate_provenance(provenance: &Value) -> Result<(), AdapterError> {
    exact_keys(
        provenance,
        &[
            "capabilityId",
            "implementation",
            "repository",
            "catalog",
            "sourceVersions",
        ],
        "workflow recommendation provenance",
    )?;
    if provenance["capabilityId"] != "advisory.workflow-recommend.v2"
        || provenance["implementation"] != "workflow_recommendation.rs"
        || provenance["catalog"] != CATALOG_PATH
    {
        return Err(AdapterError::Contract(
            "workflow recommendation provenance identity is invalid".into(),
        ));
    }
    nonempty(provenance, "repository")?;
    let sources = provenance["sourceVersions"]
        .as_array()
        .filter(|items| items.len() >= 3)
        .ok_or_else(|| {
            AdapterError::Contract("workflow provenance sourceVersions is incomplete".into())
        })?;
    for source in sources {
        validate_source(source)?;
    }
    for (uri, version, revision) in [
        (
            "https://github.com/Fission-AI/OpenSpec",
            "1.8.0",
            "d57889664cab4f2f061d236ec3ff82a5578701bb",
        ),
        (
            "https://github.com/github/spec-kit",
            "0.16.1",
            "ad4104b56c219b0a27bac06547d1a3c7d6a0dbd6",
        ),
    ] {
        if !sources.iter().any(|source| {
            source["uri"] == uri
                && source["version"] == version
                && source["revision"] == revision
                && source["license"] == "MIT"
        }) {
            return Err(AdapterError::Contract(format!(
                "workflow provenance is missing required source {uri}@{version}"
            )));
        }
    }
    Ok(())
}

fn validate_conflict(conflict: &Value) -> Result<(), AdapterError> {
    if conflict.is_null() {
        return Ok(());
    }
    exact_keys(
        conflict,
        &["kind", "roots", "resolution"],
        "workflow conflict",
    )?;
    if !matches!(
        conflict["kind"].as_str(),
        Some("competing-normative-roots" | "incompatible-required-capabilities")
    ) {
        return Err(AdapterError::Contract(
            "workflow conflict kind is invalid".into(),
        ));
    }
    validate_string_array(&conflict["roots"], "workflow conflict roots", 2, true)?;
    nonempty(conflict, "resolution")?;
    Ok(())
}

fn validate_manual_override(value: &Value) -> Result<(), AdapterError> {
    if value.is_null() {
        return Ok(());
    }
    exact_keys(value, &["from", "to", "reason"], "workflow manual override")?;
    for field in ["from", "to"] {
        if !matches!(
            value[field].as_str(),
            Some("openspec" | "spec-kit" | "lightweight")
        ) {
            return Err(AdapterError::Contract(
                "workflow manual override adapter is invalid".into(),
            ));
        }
    }
    nonempty(value, "reason")?;
    Ok(())
}

fn validate_handoffs(value: &Value) -> Result<(), AdapterError> {
    let handoffs = value
        .as_array()
        .ok_or_else(|| AdapterError::Contract("workflow handoffs must be an array".into()))?;
    for handoff in handoffs {
        exact_keys(
            handoff,
            &["intent", "availability", "missingCapability"],
            "workflow handoff",
        )?;
        if !matches!(handoff["intent"].as_str(), Some("ship" | "observe"))
            || handoff["availability"] != "unavailable"
        {
            return Err(AdapterError::Contract(
                "workflow handoff content is invalid".into(),
            ));
        }
        nonempty(handoff, "missingCapability")?;
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
    nonempty(candidate, "candidate")?;
    if candidate["stack"] != "spec-driven"
        || !matches!(
            candidate["adapter"].as_str(),
            Some("openspec" | "spec-kit" | "lightweight")
        )
        || !matches!(
            candidate["verdict"].as_str(),
            Some("recommended" | "alternative" | "active-continuation" | "manual-override")
        )
        || candidate["score"].as_u64().is_none_or(|score| score > 100)
    {
        return Err(AdapterError::Contract(
            "rendered workflow candidate identity is invalid".into(),
        ));
    }
    validate_string_array(
        &candidate["reasons"],
        "workflow candidate reasons",
        1,
        false,
    )?;
    validate_presence(&candidate["presence"])?;
    validate_adoption(&candidate["adoption"])?;
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
    validate_source(&candidate["source"])?;
    validate_string_array(
        &candidate["capabilities"],
        "workflow candidate capabilities",
        0,
        true,
    )?;
    Ok(())
}

fn validate_presence(value: &Value) -> Result<(), AdapterError> {
    exact_keys(value, &["state", "evidence"], "workflow candidate presence")?;
    if !matches!(
        value["state"].as_str(),
        Some("absent" | "configured" | "active")
    ) {
        return Err(AdapterError::Contract(
            "workflow candidate presence state is invalid".into(),
        ));
    }
    validate_string_array(
        &value["evidence"],
        "workflow candidate presence evidence",
        0,
        true,
    )
}

fn validate_adoption(value: &Value) -> Result<(), AdapterError> {
    exact_keys(
        value,
        &["state", "authorityEventRef"],
        "workflow candidate adoption",
    )?;
    if !matches!(value["state"].as_str(), Some("unresolved" | "approved"))
        || !(value["authorityEventRef"].is_null() || value["authorityEventRef"].is_string())
    {
        return Err(AdapterError::Contract(
            "workflow candidate adoption is invalid".into(),
        ));
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
        || !matches!(
            action["classification"].as_str(),
            Some("entry" | "setup" | "maintenance")
        )
        || !matches!(
            action["intent"].as_str(),
            Some(
                "explore"
                    | "plan"
                    | "clarify"
                    | "implement"
                    | "verify"
                    | "converge"
                    | "archive"
                    | "synchronize"
                    | "setup"
                    | "maintain"
            )
        )
        || nonempty(action, "actionId").is_err()
        || !matches!(
            action["availability"].as_str(),
            Some("available" | "conditional" | "unavailable")
        )
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
    if ["codex", "generic", "cli"].iter().any(|field| {
        let value = &action["invocations"][field];
        !(value.is_null() || value.is_string())
    }) {
        return Err(AdapterError::Contract(
            "workflow action invocation must be a string or null".into(),
        ));
    }
    validate_string_array(
        &action["prerequisites"],
        "workflow action prerequisites",
        0,
        false,
    )?;
    validate_string_array(&action["effects"], "workflow action effects", 0, true)?;
    Ok(())
}

fn validate_string_array(
    value: &Value,
    label: &str,
    min_items: usize,
    unique: bool,
) -> Result<(), AdapterError> {
    let values = value
        .as_array()
        .filter(|items| items.len() >= min_items)
        .ok_or_else(|| AdapterError::Contract(format!("{label} must be an array")))?;
    let mut observed = BTreeSet::new();
    for value in values {
        let text = value
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| AdapterError::Contract(format!("{label} contains an invalid value")))?;
        if unique && !observed.insert(text) {
            return Err(AdapterError::Contract(format!(
                "{label} contains a duplicate value"
            )));
        }
    }
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

#[cfg(test)]
#[path = "workflow_recommendation_contract_tests.rs"]
mod tests;
