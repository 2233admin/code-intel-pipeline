use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path};

use serde_json::Value;

const MAX_DOCUMENT_BYTES: u64 = 256 * 1024;
const CARD_FIELDS: [&str; 17] = [
    "schema",
    "id",
    "version",
    "name",
    "problemSignals",
    "requiredEvidence",
    "assumptions",
    "deterministicSteps",
    "outputs",
    "confidenceRules",
    "cost",
    "contraindications",
    "implementationPorts",
    "source",
    "applicabilityBoundary",
    "relatedMethodIds",
    "executionPolicy",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CatalogError(String);

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CatalogError {}

#[derive(Debug, Clone)]
pub(crate) struct MethodCatalog {
    cards: Vec<Value>,
}

impl MethodCatalog {
    pub(crate) fn cards(&self) -> &[Value] {
        &self.cards
    }
}

pub(crate) fn load_catalog(methods_root: &Path) -> Result<MethodCatalog, CatalogError> {
    let index = read_json(&methods_root.join("catalog.v1.json"))?;
    validate_index(&index)?;
    let mut documents = Vec::new();
    for entry in index["cards"].as_array().unwrap() {
        let relative = entry["path"].as_str().unwrap();
        validate_relative_card_path(relative)?;
        documents.push((
            relative.to_string(),
            read_json(&methods_root.join(relative))?,
        ));
    }
    let expected = documents
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    let cards_dir = methods_root.join("cards");
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(&cards_dir).map_err(|error| {
        CatalogError(format!(
            "read method card directory {}: {error}",
            cards_dir.display()
        ))
    })? {
        let entry =
            entry.map_err(|error| CatalogError(format!("read method card entry: {error}")))?;
        let file_type = entry
            .file_type()
            .map_err(|error| CatalogError(format!("inspect method card entry: {error}")))?;
        if !file_type.is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("json")
        {
            return Err(CatalogError(format!(
                "method card directory contains an unregistered non-card entry {}",
                entry.path().display()
            )));
        }
        actual.insert(format!("cards/{}", entry.file_name().to_string_lossy()));
    }
    if actual != expected {
        return Err(CatalogError(
            "catalog card paths differ from checked-in JSON card files".to_string(),
        ));
    }
    validate_documents(&index, &documents)
}

pub(crate) fn validate_documents(
    index: &Value,
    documents: &[(String, Value)],
) -> Result<MethodCatalog, CatalogError> {
    validate_index(index)?;
    let entries = index["cards"].as_array().unwrap();
    if entries.len() != documents.len() {
        return Err(CatalogError(
            "catalog entry count differs from loaded card count".to_string(),
        ));
    }

    let mut by_path = BTreeMap::new();
    for (path, card) in documents {
        validate_relative_card_path(path)?;
        if by_path.insert(path.as_str(), card).is_some() {
            return Err(CatalogError(format!("duplicate loaded card path {path}")));
        }
    }

    let mut cards = Vec::with_capacity(entries.len());
    let mut ids = BTreeSet::new();
    for entry in entries {
        let path = entry["path"].as_str().unwrap();
        let expected_id = entry["id"].as_str().unwrap();
        let card = by_path
            .get(path)
            .ok_or_else(|| CatalogError(format!("catalog path {path} was not loaded")))?;
        validate_card(card)?;
        let actual_id = card["id"].as_str().unwrap();
        if actual_id != expected_id {
            return Err(CatalogError(format!(
                "catalog id {expected_id} does not match card id {actual_id}"
            )));
        }
        if !ids.insert(actual_id.to_string()) {
            return Err(CatalogError(format!("duplicate method id {actual_id}")));
        }
        cards.push((*card).clone());
    }

    for card in &cards {
        let id = card["id"].as_str().unwrap();
        for related in strings(&card["relatedMethodIds"], "relatedMethodIds", false)? {
            if related == id || !ids.contains(related) {
                return Err(CatalogError(format!(
                    "{id}.relatedMethodIds references unknown or self method {related}"
                )));
            }
        }
    }
    Ok(MethodCatalog { cards })
}

fn validate_index(index: &Value) -> Result<(), CatalogError> {
    exact_object(
        index,
        "catalog",
        &["schema", "catalogVersion", "selectionPolicy", "cards"],
    )?;
    require_exact_string(index, "schema", "code-intel-method-catalog.v1", "catalog")?;
    require_nonempty_string(index, "catalogVersion", "catalog")?;
    require_exact_string(index, "selectionPolicy", "none_catalog_only", "catalog")?;
    let cards = nonempty_array(&index["cards"], "catalog.cards")?;
    let mut previous: Option<&str> = None;
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for (position, entry) in cards.iter().enumerate() {
        let context = format!("catalog.cards[{position}]");
        exact_object(entry, &context, &["id", "path"])?;
        let id = require_portable_id(entry, "id", &context)?;
        let path = require_nonempty_string(entry, "path", &context)?;
        validate_relative_card_path(path)?;
        if path != format!("cards/{id}.v1.json") {
            return Err(CatalogError(format!(
                "{context}.path must be the versioned path for method id {id}"
            )));
        }
        if previous.is_some_and(|prior| prior >= id) {
            return Err(CatalogError(
                "catalog cards must be strictly sorted by stable id".to_string(),
            ));
        }
        previous = Some(id);
        if !ids.insert(id) || !paths.insert(path) {
            return Err(CatalogError(format!(
                "duplicate catalog id or path at {context}"
            )));
        }
    }
    Ok(())
}

fn validate_card(card: &Value) -> Result<(), CatalogError> {
    exact_object(card, "card", &CARD_FIELDS)?;
    require_exact_string(card, "schema", "code-intel-method-card.v1", "card")?;
    let method_id = require_portable_id(card, "id", "card")?;
    require_nonempty_string(card, "version", method_id)?;
    require_nonempty_string(card, "name", method_id)?;
    require_exact_string(
        card,
        "executionPolicy",
        "catalog_only_no_selection_or_execution",
        method_id,
    )?;

    let evidence =
        validate_described_ids(&card["requiredEvidence"], method_id, "requiredEvidence")?;
    validate_described_ids(&card["problemSignals"], method_id, "problemSignals")?;
    let outputs = validate_described_ids(&card["outputs"], method_id, "outputs")?;
    strings(
        &card["assumptions"],
        &format!("{method_id}.assumptions"),
        true,
    )?;
    strings(
        &card["contraindications"],
        &format!("{method_id}.contraindications"),
        true,
    )?;
    validate_steps(card, method_id, &evidence, &outputs)?;
    validate_confidence(card, method_id)?;
    validate_cost(card, method_id)?;
    validate_ports(card, method_id)?;
    validate_source(card, method_id)?;
    validate_boundary(card, method_id)?;
    strings(
        &card["relatedMethodIds"],
        &format!("{method_id}.relatedMethodIds"),
        false,
    )?;
    Ok(())
}

fn validate_described_ids(
    value: &Value,
    method_id: &str,
    field: &str,
) -> Result<BTreeSet<String>, CatalogError> {
    let entries = nonempty_array(value, &format!("{method_id}.{field}"))?;
    let mut ids = BTreeSet::new();
    for (position, entry) in entries.iter().enumerate() {
        let context = format!("{method_id}.{field}[{position}]");
        exact_object(entry, &context, &["id", "description"])?;
        let id = require_portable_id(entry, "id", &context)?;
        require_nonempty_string(entry, "description", &context)?;
        if !ids.insert(id.to_string()) {
            return Err(CatalogError(format!(
                "duplicate id {id} in {method_id}.{field}"
            )));
        }
    }
    Ok(ids)
}

fn validate_steps(
    card: &Value,
    method_id: &str,
    evidence: &BTreeSet<String>,
    outputs: &BTreeSet<String>,
) -> Result<(), CatalogError> {
    let steps = nonempty_array(
        &card["deterministicSteps"],
        &format!("{method_id}.deterministicSteps"),
    )?;
    let mut prior_steps = BTreeSet::new();
    let mut produced = BTreeSet::new();
    for (position, step) in steps.iter().enumerate() {
        let context = format!("{method_id}.deterministicSteps[{position}]");
        exact_object(step, &context, &["id", "action", "requires", "produces"])?;
        let id = require_portable_id(step, "id", &context)?;
        if prior_steps.contains(id) {
            return Err(CatalogError(format!(
                "duplicate step id {id} in {method_id}"
            )));
        }
        require_nonempty_string(step, "action", &context)?;
        for requirement in strings(&step["requires"], &format!("{context}.requires"), false)? {
            if let Some(evidence_id) = requirement.strip_prefix("evidence:") {
                if !evidence.contains(evidence_id) {
                    return Err(CatalogError(format!(
                        "{context} references unknown evidence {evidence_id}"
                    )));
                }
            } else if let Some(step_id) = requirement.strip_prefix("step:") {
                if !prior_steps.contains(step_id) {
                    return Err(CatalogError(format!(
                        "{context} references unknown or forward step {step_id}"
                    )));
                }
            } else {
                return Err(CatalogError(format!(
                    "{context} requirement must use evidence: or step:"
                )));
            }
        }
        for output in strings(&step["produces"], &format!("{context}.produces"), true)? {
            if !outputs.contains(output) {
                return Err(CatalogError(format!(
                    "{context} produces unknown output {output}"
                )));
            }
            produced.insert(output.to_string());
        }
        prior_steps.insert(id.to_string());
    }
    if &produced != outputs {
        return Err(CatalogError(format!(
            "{method_id} deterministic steps must produce every declared output"
        )));
    }
    Ok(())
}

fn validate_confidence(card: &Value, method_id: &str) -> Result<(), CatalogError> {
    let rules = nonempty_array(
        &card["confidenceRules"],
        &format!("{method_id}.confidenceRules"),
    )?;
    let mut levels = BTreeSet::new();
    for (position, rule) in rules.iter().enumerate() {
        let context = format!("{method_id}.confidenceRules[{position}]");
        exact_object(rule, &context, &["level", "whenAll"])?;
        let level = require_nonempty_string(rule, "level", &context)?;
        if !["unknown", "low", "medium", "high"].contains(&level) || !levels.insert(level) {
            return Err(CatalogError(format!(
                "invalid or duplicate confidence level in {context}"
            )));
        }
        strings(&rule["whenAll"], &format!("{context}.whenAll"), true)?;
    }
    Ok(())
}

fn validate_cost(card: &Value, method_id: &str) -> Result<(), CatalogError> {
    let cost = &card["cost"];
    let context = format!("{method_id}.cost");
    exact_object(cost, &context, &["relative", "drivers"])?;
    let relative = require_nonempty_string(cost, "relative", &context)?;
    if !["low", "medium", "high"].contains(&relative) {
        return Err(CatalogError(format!("{context}.relative is invalid")));
    }
    strings(&cost["drivers"], &format!("{context}.drivers"), true)?;
    Ok(())
}

fn validate_ports(card: &Value, method_id: &str) -> Result<(), CatalogError> {
    let ports = nonempty_array(
        &card["implementationPorts"],
        &format!("{method_id}.implementationPorts"),
    )?;
    let mut ids = BTreeSet::new();
    for (position, port) in ports.iter().enumerate() {
        let context = format!("{method_id}.implementationPorts[{position}]");
        exact_object(port, &context, &["id", "kind", "contract"])?;
        let id = require_portable_id(port, "id", &context)?;
        if !ids.insert(id) {
            return Err(CatalogError(format!("duplicate implementation port {id}")));
        }
        let kind = require_nonempty_string(port, "kind", &context)?;
        if ![
            "manual",
            "deterministic_tool",
            "statistical_engine",
            "modeling_tool",
        ]
        .contains(&kind)
        {
            return Err(CatalogError(format!(
                "invalid implementation port kind in {context}"
            )));
        }
        strings(&port["contract"], &format!("{context}.contract"), true)?;
    }
    Ok(())
}

fn validate_source(card: &Value, method_id: &str) -> Result<(), CatalogError> {
    let source = &card["source"];
    let context = format!("{method_id}.source");
    exact_object(source, &context, &["title", "version", "reference"])?;
    for field in ["title", "version", "reference"] {
        require_nonempty_string(source, field, &context)?;
    }
    Ok(())
}

fn validate_boundary(card: &Value, method_id: &str) -> Result<(), CatalogError> {
    let boundary = &card["applicabilityBoundary"];
    let context = format!("{method_id}.applicabilityBoundary");
    exact_object(boundary, &context, &["inScope", "outOfScope"])?;
    strings(&boundary["inScope"], &format!("{context}.inScope"), true)?;
    strings(
        &boundary["outOfScope"],
        &format!("{context}.outOfScope"),
        true,
    )?;
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, CatalogError> {
    let metadata = fs::metadata(path)
        .map_err(|error| CatalogError(format!("inspect {}: {error}", path.display())))?;
    if !metadata.is_file() || metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(CatalogError(format!(
            "method catalog document {} is not a bounded regular file",
            path.display()
        )));
    }
    let bytes = fs::read(path)
        .map_err(|error| CatalogError(format!("read {}: {error}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| CatalogError(format!("parse {}: {error}", path.display())))
}

fn validate_relative_card_path(path: &str) -> Result<(), CatalogError> {
    let value = Path::new(path);
    let components = value.components().collect::<Vec<_>>();
    let file_name = match components.as_slice() {
        [Component::Normal(parent), Component::Normal(file)]
            if parent.to_str() == Some("cards") =>
        {
            file.to_str()
        }
        _ => None,
    };
    let Some(method_id) = file_name.and_then(|name| name.strip_suffix(".v1.json")) else {
        return Err(CatalogError(format!(
            "invalid relative method card path {path}"
        )));
    };
    if method_id.is_empty()
        || method_id.len() > 96
        || !method_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(CatalogError(format!(
            "invalid relative method card path {path}"
        )));
    }
    Ok(())
}

fn exact_object(value: &Value, context: &str, fields: &[&str]) -> Result<(), CatalogError> {
    let object = value
        .as_object()
        .ok_or_else(|| CatalogError(format!("{context} must be an object")))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = fields.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
        let unknown = actual.difference(&expected).copied().collect::<Vec<_>>();
        return Err(CatalogError(format!(
            "{context} fields differ from v1 schema; missing={missing:?}; unknown={unknown:?}"
        )));
    }
    Ok(())
}

fn require_exact_string(
    value: &Value,
    field: &str,
    expected: &str,
    context: &str,
) -> Result<(), CatalogError> {
    let actual = require_nonempty_string(value, field, context)?;
    if actual != expected {
        return Err(CatalogError(format!(
            "{context}.{field} must equal {expected}"
        )));
    }
    Ok(())
}

fn require_nonempty_string<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a str, CatalogError> {
    value[field]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| CatalogError(format!("{context}.{field} must be a non-empty string")))
}

fn require_portable_id<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a str, CatalogError> {
    let id = require_nonempty_string(value, field, context)?;
    if id.len() > 96
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(CatalogError(format!(
            "{context}.{field} is not a portable id"
        )));
    }
    Ok(id)
}

fn nonempty_array<'a>(value: &'a Value, context: &str) -> Result<&'a [Value], CatalogError> {
    value
        .as_array()
        .filter(|items| !items.is_empty())
        .map(Vec::as_slice)
        .ok_or_else(|| CatalogError(format!("{context} must be a non-empty array")))
}

fn strings<'a>(
    value: &'a Value,
    context: &str,
    require_nonempty: bool,
) -> Result<Vec<&'a str>, CatalogError> {
    let array = value
        .as_array()
        .ok_or_else(|| CatalogError(format!("{context} must be an array")))?;
    if require_nonempty && array.is_empty() {
        return Err(CatalogError(format!("{context} must not be empty")));
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(array.len());
    for item in array {
        let text = item
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| CatalogError(format!("{context} must contain non-empty strings")))?;
        if !seen.insert(text) {
            return Err(CatalogError(format!(
                "{context} contains duplicate value {text}"
            )));
        }
        result.push(text);
    }
    Ok(result)
}
