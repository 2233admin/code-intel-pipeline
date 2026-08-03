//! Runtime adapter for `assistance.discovery`.
//!
//! Candidates are resolved from the committed catalog rather than accepted
//! inline: a fit, license, security, or reversibility rating invented at call
//! time would defeat the review the catalog exists to record, and the
//! discovery core can only reject popularity-only reasoning it can see the
//! basis for. The result stays proposal-only — no adoption, no install, no
//! write into the target repository.
//!
//! The decision core lives in `assistance_discovery.rs` and stays free of
//! adapter and filesystem types so its own test can include it standalone.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::assistance_discovery;
use super::{publish_named, AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;

pub(crate) const CATALOG_PATH: &str = "orchestration/agent-assistance-catalog.v1.json";

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "assistance.discovery does not accept input artifacts".into(),
        ));
    }
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options
        .keys()
        .any(|key| !matches!(key.as_str(), "gap" | "candidateIds"))
    {
        return Err(AdapterError::InvalidOptions(
            "assistance.discovery accepts only gap/candidateIds".into(),
        ));
    }
    let gap = options
        .get("gap")
        .ok_or_else(|| AdapterError::InvalidOptions("options.gap is required".into()))?;
    let requested = options
        .get("candidateIds")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| {
            AdapterError::InvalidOptions("options.candidateIds must be a non-empty array".into())
        })?;

    let catalog = catalog()?;
    let mut candidates = Vec::with_capacity(requested.len());
    let mut seen = BTreeSet::new();
    for value in requested {
        let id = value
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| {
                AdapterError::InvalidOptions(
                    "options.candidateIds contains an invalid value".into(),
                )
            })?;
        if !seen.insert(id) {
            return Err(AdapterError::InvalidOptions(format!(
                "options.candidateIds repeats {id}"
            )));
        }
        let candidate = catalog.get(id).ok_or_else(|| {
            AdapterError::InvalidOptions(format!("candidate {id} is not in {CATALOG_PATH}"))
        })?;
        candidates.push(candidate.clone());
    }

    let result = assistance_discovery::discover(&json!({
        "schema": "code-intel-assistance-discovery-request.v1",
        "gap": gap,
        "candidates": candidates,
    }))
    .map_err(|error| AdapterError::Contract(error.to_string()))?;
    let bytes = serde_json::to_vec(&result).map_err(|error| {
        AdapterError::Internal(format!("serialize assistance discovery result: {error}"))
    })?;
    publish_named(out, "assistance-discovery-result.json", &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-assistance-discovery-result.v1".into(),
            artifact_type: "assistance.discovery".into(),
            relative_path: "assistance-discovery-result.json".into(),
            bytes,
        }],
        observed_effects: vec![],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn catalog() -> Result<BTreeMap<String, Value>, AdapterError> {
    let path = super::pipeline_root().join(CATALOG_PATH);
    let bytes = fs::read(&path).map_err(|error| {
        AdapterError::Unavailable(format!(
            "assistance catalog is unavailable: {}: {error}",
            path.display()
        ))
    })?;
    let catalog: Value = serde_json::from_slice(&bytes).map_err(|error| {
        AdapterError::Contract(format!("assistance catalog is not one JSON object: {error}"))
    })?;
    if catalog["schema"] != "code-intel-agent-assistance-catalog.v1" {
        return Err(AdapterError::Contract(
            "assistance catalog schema is not code-intel-agent-assistance-catalog.v1".into(),
        ));
    }
    let entries = catalog["entries"].as_array().ok_or_else(|| {
        AdapterError::Contract("assistance catalog entries must be an array".into())
    })?;
    let mut resolved = BTreeMap::new();
    for entry in entries {
        let candidate = entry
            .get("candidate")
            .cloned()
            .ok_or_else(|| AdapterError::Contract("catalog entry lacks a candidate".into()))?;
        let id = candidate["id"]
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| AdapterError::Contract("catalog candidate lacks an id".into()))?
            .to_string();
        if resolved.insert(id.clone(), candidate).is_some() {
            return Err(AdapterError::Contract(format!(
                "assistance catalog repeats candidate {id}"
            )));
        }
    }
    Ok(resolved)
}
