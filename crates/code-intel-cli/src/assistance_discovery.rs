use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::{json, Value};

const KINDS: [&str; 4] = [
    "internal_atom",
    "established_method",
    "external_tool",
    "documentation",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryError(String);

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DiscoveryError {}

pub(crate) fn discover(request: &Value) -> Result<Value, DiscoveryError> {
    exact_object(request, "request", &["schema", "gap", "candidates"])?;
    require_exact(
        request,
        "schema",
        "code-intel-assistance-discovery-request.v1",
        "request",
    )?;
    let gap = parse_gap(&request["gap"])?;
    let candidates = request["candidates"]
        .as_array()
        .ok_or_else(|| DiscoveryError("request.candidates must be an array".into()))?;
    if candidates.is_empty() {
        return Err(DiscoveryError(
            "assistance discovery requires at least one candidate".into(),
        ));
    }

    let mut ids = BTreeSet::new();
    let mut dossiers = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let dossier = parse_candidate(candidate, index)?;
        let id = dossier["id"].as_str().unwrap().to_string();
        if !ids.insert(id.clone()) {
            return Err(DiscoveryError(format!("duplicate candidate id {id}")));
        }
        let kind = dossier["kind"].as_str().unwrap();
        let order = KINDS.iter().position(|value| value == &kind).unwrap();
        dossiers.insert((order, id), dossier);
    }

    Ok(json!({
        "schema": "code-intel-assistance-discovery-result.v1",
        "status": "completed",
        "gapId": gap.id,
        "capability": gap.capability,
        "gapEvidenceRefs": gap.evidence_refs,
        "dossiers": dossiers.into_values().collect::<Vec<_>>(),
        "proposalOnly": true,
        "selectionPolicy": "evidence_and_constraints_not_popularity",
        "effects": [],
        "authorityEvents": [],
        "adoptionDecisions": [],
        "committedEngineeringPlans": [],
    }))
}

struct Gap {
    id: String,
    capability: String,
    evidence_refs: Vec<String>,
}

fn parse_gap(value: &Value) -> Result<Gap, DiscoveryError> {
    exact_object(
        value,
        "gap",
        &[
            "schema",
            "id",
            "capability",
            "description",
            "constraints",
            "evidenceRefs",
        ],
    )?;
    require_exact(
        value,
        "schema",
        "code-intel-engineering-capability-gap.v1",
        "gap",
    )?;
    let id = nonempty(value, "id", "gap")?.to_string();
    let capability = nonempty(value, "capability", "gap")?.to_string();
    nonempty(value, "description", "gap")?;
    string_list(&value["constraints"], "gap.constraints", true)?;
    let evidence_refs = string_list(&value["evidenceRefs"], "gap.evidenceRefs", true)?;
    Ok(Gap {
        id,
        capability,
        evidence_refs,
    })
}

fn parse_candidate(value: &Value, index: usize) -> Result<Value, DiscoveryError> {
    let context = format!("candidates[{index}]");
    exact_object(
        value,
        &context,
        &[
            "id",
            "kind",
            "name",
            "fit",
            "license",
            "security",
            "integration",
            "reversibility",
            "evidenceRefs",
        ],
    )?;
    let id = nonempty(value, "id", &context)?;
    let kind = nonempty(value, "kind", &context)?;
    if !KINDS.contains(&kind) {
        return Err(DiscoveryError(format!("{context}.kind is invalid")));
    }
    let name = nonempty(value, "name", &context)?;
    let fit = assessment(
        &value["fit"],
        &format!("{context}.fit"),
        "status",
        &["strong", "partial", "weak", "unknown"],
    )?;
    let license = assessment(
        &value["license"],
        &format!("{context}.license"),
        "status",
        &["acceptable", "review_required", "not_applicable", "unknown"],
    )?;
    let security = assessment(
        &value["security"],
        &format!("{context}.security"),
        "status",
        &["acceptable", "review_required", "unacceptable", "unknown"],
    )?;
    let integration = assessment(
        &value["integration"],
        &format!("{context}.integration"),
        "effort",
        &["low", "medium", "high", "unknown"],
    )?;
    let reversibility = assessment(
        &value["reversibility"],
        &format!("{context}.reversibility"),
        "status",
        &["high", "medium", "low", "unknown"],
    )?;
    let evidence_refs = string_list(
        &value["evidenceRefs"],
        &format!("{context}.evidenceRefs"),
        true,
    )?;
    reject_popularity_only(&fit, &evidence_refs, &context)?;

    Ok(json!({
        "schema": "code-intel-assistance-dossier.v1",
        "id": id,
        "kind": kind,
        "name": name,
        "fit": fit,
        "license": license,
        "security": security,
        "integration": integration,
        "reversibility": reversibility,
        "evidenceRefs": evidence_refs,
        "disposition": "proposal",
        "authorityState": "unresolved",
    }))
}

fn assessment(
    value: &Value,
    context: &str,
    rating_key: &str,
    allowed: &[&str],
) -> Result<Value, DiscoveryError> {
    exact_object(value, context, &[rating_key, "basis"])?;
    let rating = nonempty(value, rating_key, context)?;
    if !allowed.contains(&rating) {
        return Err(DiscoveryError(format!("{context}.{rating_key} is invalid")));
    }
    nonempty(value, "basis", context)?;
    Ok(value.clone())
}

fn reject_popularity_only(
    fit: &Value,
    evidence_refs: &[String],
    context: &str,
) -> Result<(), DiscoveryError> {
    let basis = fit["basis"].as_str().unwrap().to_ascii_lowercase();
    let popularity_basis = ["popular", "github star", "star count", "downloads"]
        .iter()
        .any(|term| basis.contains(term));
    let only_popularity_refs = evidence_refs.iter().all(|reference| {
        let reference = reference.to_ascii_lowercase();
        reference.contains("star")
            || reference.contains("download")
            || reference.contains("popular")
    });
    if popularity_basis && only_popularity_refs {
        return Err(DiscoveryError(format!(
            "{context} cannot rely on popularity alone"
        )));
    }
    Ok(())
}

fn exact_object(value: &Value, context: &str, allowed: &[&str]) -> Result<(), DiscoveryError> {
    let object = value
        .as_object()
        .ok_or_else(|| DiscoveryError(format!("{context} must be an object")))?;
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    if let Some(extra) = object.keys().find(|key| !allowed.contains(key.as_str())) {
        return Err(DiscoveryError(format!(
            "{context} contains unknown field {extra}"
        )));
    }
    if let Some(missing) = allowed.iter().find(|key| !object.contains_key(**key)) {
        return Err(DiscoveryError(format!(
            "{context} is missing field {missing}"
        )));
    }
    Ok(())
}

fn require_exact(
    value: &Value,
    key: &str,
    expected: &str,
    context: &str,
) -> Result<(), DiscoveryError> {
    if value.get(key).and_then(Value::as_str) != Some(expected) {
        return Err(DiscoveryError(format!("{context}.{key} is invalid")));
    }
    Ok(())
}

fn nonempty<'a>(value: &'a Value, key: &str, context: &str) -> Result<&'a str, DiscoveryError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| DiscoveryError(format!("{context}.{key} must be a non-empty string")))
}

fn string_list(
    value: &Value,
    context: &str,
    nonempty_required: bool,
) -> Result<Vec<String>, DiscoveryError> {
    let values = value
        .as_array()
        .ok_or_else(|| DiscoveryError(format!("{context} must be an array")))?;
    if nonempty_required && values.is_empty() {
        return Err(DiscoveryError(format!("{context} must not be empty")));
    }
    let mut unique = BTreeSet::new();
    for item in values {
        let item = item
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| DiscoveryError(format!("{context} contains an invalid value")))?;
        if !unique.insert(item.to_string()) {
            return Err(DiscoveryError(format!("{context} contains a duplicate")));
        }
    }
    Ok(unique.into_iter().collect())
}
