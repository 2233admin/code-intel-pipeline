use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::admissibility;
use crate::method_catalog::MethodCatalog;

const MAX_RULE_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectError(String);

impl fmt::Display for SelectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SelectError {}

#[derive(Debug, Clone)]
struct ContraRule {
    signal_id: String,
    card_text: String,
}

#[derive(Debug, Clone)]
struct Rule {
    method_id: String,
    signal_ids: BTreeSet<String>,
    contraindications: Vec<ContraRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BoundFact {
    signal_ids: BTreeSet<String>,
    evidence_kinds: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RuleTable {
    rules: Vec<Rule>,
}

pub(crate) fn load_rule_table(
    path: &Path,
    catalog: &MethodCatalog,
) -> Result<RuleTable, SelectError> {
    let metadata = fs::metadata(path)
        .map_err(|error| SelectError(format!("inspect method selection rules: {error}")))?;
    if !metadata.is_file() || metadata.len() > MAX_RULE_BYTES {
        return Err(SelectError(
            "method selection rules must be a bounded regular file".to_string(),
        ));
    }
    let document: Value = serde_json::from_slice(
        &fs::read(path)
            .map_err(|error| SelectError(format!("read method selection rules: {error}")))?,
    )
    .map_err(|error| SelectError(format!("parse method selection rules: {error}")))?;
    validate_rules(&document, catalog)
}

fn validate_rules(document: &Value, catalog: &MethodCatalog) -> Result<RuleTable, SelectError> {
    exact_object(document, "rule table", &["schema", "rules"])?;
    if document["schema"] != "code-intel-method-selection-rules.v1" {
        return Err(SelectError(
            "method selection rule schema is invalid".to_string(),
        ));
    }
    let cards = catalog
        .cards()
        .iter()
        .map(|card| (card["id"].as_str().unwrap(), card))
        .collect::<BTreeMap<_, _>>();
    let entries = nonempty_array(&document["rules"], "rule table.rules")?;
    let mut rules = Vec::with_capacity(entries.len());
    let mut previous: Option<&str> = None;
    for (index, entry) in entries.iter().enumerate() {
        let context = format!("rules[{index}]");
        exact_object(
            entry,
            &context,
            &["methodId", "signalIds", "contraindications"],
        )?;
        let method_id = nonempty_string(&entry["methodId"], &format!("{context}.methodId"))?;
        if previous.is_some_and(|prior| prior >= method_id) {
            return Err(SelectError(
                "method selection rules must be strictly sorted by methodId".to_string(),
            ));
        }
        previous = Some(method_id);
        let card = cards.get(method_id).ok_or_else(|| {
            SelectError(format!("rule references unknown C01 method {method_id}"))
        })?;
        let declared_signals = card["problemSignals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        let signal_ids = string_set(&entry["signalIds"], &format!("{context}.signalIds"), true)?;
        if signal_ids
            .iter()
            .any(|signal| !declared_signals.contains(signal.as_str()))
        {
            return Err(SelectError(format!(
                "{context} positive signals must reference C01 problemSignals"
            )));
        }
        let declared_contraindications = card["contraindications"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item.as_str().unwrap())
            .collect::<BTreeSet<_>>();
        let contra_entries = entry["contraindications"]
            .as_array()
            .ok_or_else(|| SelectError(format!("{context}.contraindications must be an array")))?;
        let mut contraindications = Vec::new();
        let mut contra_signals = BTreeSet::new();
        for (position, contra) in contra_entries.iter().enumerate() {
            let contra_context = format!("{context}.contraindications[{position}]");
            exact_object(contra, &contra_context, &["signalId", "cardText"])?;
            let signal_id =
                nonempty_string(&contra["signalId"], &format!("{contra_context}.signalId"))?;
            let card_text =
                nonempty_string(&contra["cardText"], &format!("{contra_context}.cardText"))?;
            if !contra_signals.insert(signal_id) || !declared_contraindications.contains(card_text)
            {
                return Err(SelectError(format!(
                    "{contra_context} must uniquely reference a C01 contraindication"
                )));
            }
            contraindications.push(ContraRule {
                signal_id: signal_id.to_string(),
                card_text: card_text.to_string(),
            });
        }
        rules.push(Rule {
            method_id: method_id.to_string(),
            signal_ids,
            contraindications,
        });
    }
    if rules.len() != cards.len() {
        return Err(SelectError(
            "rule table must contain exactly one rule for every C01 method card".to_string(),
        ));
    }
    Ok(RuleTable { rules })
}

pub(crate) fn select(
    request: &Value,
    artifact_root: &Path,
    catalog: &MethodCatalog,
    table: &RuleTable,
) -> Result<Value, SelectError> {
    validate_request(request)?;
    let snapshot = request["snapshotIdentity"].as_str().unwrap();
    let evaluated_at = request["evaluatedAt"].as_u64().unwrap();
    let max_age = request["maxEvidenceAgeSeconds"].as_u64().unwrap();
    let admissions = validate_admissions(
        &request["admissions"],
        snapshot,
        evaluated_at,
        max_age,
        artifact_root,
    )?;
    let (signals, available, source_admissions) = validate_facts(&request["facts"], &admissions)?;
    let gaps = string_set(&request["evidenceGaps"], "evidenceGaps", false)?;
    if available.iter().any(|kind| gaps.contains(kind)) {
        return Err(SelectError(
            "an evidence kind cannot be both available and an evidence gap".to_string(),
        ));
    }

    let cards = catalog
        .cards()
        .iter()
        .map(|card| (card["id"].as_str().unwrap(), card))
        .collect::<BTreeMap<_, _>>();
    let mut matches = Vec::new();
    for rule in &table.rules {
        let matched = rule
            .signal_ids
            .intersection(&signals)
            .cloned()
            .collect::<Vec<_>>();
        if matched.is_empty() {
            continue;
        }
        let card = cards[rule.method_id.as_str()];
        let missing = card["requiredEvidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .filter(|kind| !available.contains(*kind) || gaps.contains(*kind))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let triggered = rule
            .contraindications
            .iter()
            .filter(|contra| signals.contains(&contra.signal_id))
            .map(|contra| contra.card_text.clone())
            .collect::<Vec<_>>();
        let outcome = if !triggered.is_empty() {
            "none"
        } else if !missing.is_empty() {
            "unknown"
        } else {
            "proposal"
        };
        let confidence = if outcome != "proposal" {
            "unknown"
        } else if matched.len() > 1 {
            "high"
        } else {
            "medium"
        };
        matches.push(json!({
            "methodId":rule.method_id,
            "outcome":outcome,
            "matchedSignals":matched,
            "missingEvidence":missing,
            "cost":card["cost"],
            "triggeredContraindications":triggered,
            "declaredContraindications":card["contraindications"],
            "confidenceRules":card["confidenceRules"],
            "selectionConfidence":confidence,
            "matchScore":matched.len()
        }));
    }
    let outcome = if matches.iter().any(|item| item["outcome"] == "proposal") {
        "proposal"
    } else if matches.iter().any(|item| item["outcome"] == "unknown") {
        "unknown"
    } else {
        "none"
    };
    let top_score = matches
        .iter()
        .filter(|item| item["outcome"] == "proposal")
        .filter_map(|item| item["matchScore"].as_u64())
        .max();
    let tie = top_score.is_some_and(|score| {
        matches
            .iter()
            .filter(|item| item["outcome"] == "proposal" && item["matchScore"] == score)
            .count()
            > 1
    });
    Ok(json!({
        "schema":"code-intel-method-selection-result.v1",
        "outcome":outcome,
        "tie":tie,
        "matches":matches,
        "sourceAdmissionIds":source_admissions,
        "executionPolicy":"advisory_proposal_only_no_execution_or_decision"
    }))
}

fn validate_request(request: &Value) -> Result<(), SelectError> {
    exact_object(
        request,
        "request",
        &[
            "schema",
            "snapshotIdentity",
            "evaluatedAt",
            "maxEvidenceAgeSeconds",
            "admissions",
            "facts",
            "evidenceGaps",
        ],
    )?;
    if request["schema"] != "code-intel-method-selection-request.v1"
        || !digest(&request["snapshotIdentity"])
        || request["evaluatedAt"].as_u64().is_none()
        || !request["maxEvidenceAgeSeconds"]
            .as_u64()
            .is_some_and(|value| value > 0)
        || !request["admissions"].is_array()
        || !request["facts"].is_array()
        || !request["evidenceGaps"].is_array()
    {
        return Err(SelectError(
            "method selection request is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_admissions(
    value: &Value,
    snapshot: &str,
    evaluated_at: u64,
    max_age: u64,
    artifact_root: &Path,
) -> Result<BTreeMap<String, BTreeMap<String, BoundFact>>, SelectError> {
    let array = value.as_array().unwrap();
    let mut admissions = BTreeMap::new();
    for (index, envelope) in array.iter().enumerate() {
        let context = format!("admissions[{index}]");
        exact_object(envelope, &context, &["request", "result"])?;
        let request = &envelope["request"];
        if request["expectedSnapshotIdentity"] != snapshot
            || request["policy"]["evaluatedAt"].as_u64() != Some(evaluated_at)
            || request["policy"]["maxAgeSeconds"].as_u64() != Some(max_age)
        {
            return Err(SelectError(format!(
                "{context} A04 request differs from the C02 snapshot/freshness policy"
            )));
        }
        let validated =
            admissibility::validate_for_consumer(request, artifact_root).map_err(|message| {
                SelectError(format!("{context} A04 validation failed: {message}"))
            })?;
        if validated.result() != &envelope["result"] {
            return Err(SelectError(format!(
                "{context} A04 result is forged or does not match runtime validation"
            )));
        }
        if validated.result()["status"] != "admitted"
            || validated.result()["domainVerdict"] != "observed"
        {
            return Err(SelectError(format!(
                "{context} is not admitted observed evidence"
            )));
        }
        let identity = validated.result()["admissionIdentity"]
            .as_str()
            .expect("A04 admitted result has an identity")
            .to_string();
        let facts = payload_facts(validated.payload(), &context)?;
        if facts.is_empty() {
            return Err(SelectError(format!("{context} is an unused admission")));
        }
        if admissions.insert(identity.clone(), facts).is_some() {
            return Err(SelectError(format!(
                "duplicate admission identity {identity}"
            )));
        }
    }
    Ok(admissions)
}

fn payload_facts(
    payload: &Value,
    context: &str,
) -> Result<BTreeMap<String, BoundFact>, SelectError> {
    let data = payload["data"]
        .as_object()
        .ok_or_else(|| SelectError(format!("{context} A04 payload data is invalid")))?;
    let values = data
        .get("methodSelectionFacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SelectError(format!(
                "{context} A04 payload has no methodSelectionFacts evidence"
            ))
        })?;
    let mut facts = BTreeMap::new();
    for (index, fact) in values.iter().enumerate() {
        let fact_context = format!("{context}.payload.methodSelectionFacts[{index}]");
        exact_object(fact, &fact_context, &["id", "signalIds", "evidenceKinds"])?;
        let id = nonempty_string(&fact["id"], &format!("{fact_context}.id"))?.to_string();
        let bound = BoundFact {
            signal_ids: string_set(
                &fact["signalIds"],
                &format!("{fact_context}.signalIds"),
                true,
            )?,
            evidence_kinds: string_set(
                &fact["evidenceKinds"],
                &format!("{fact_context}.evidenceKinds"),
                true,
            )?,
        };
        if facts.insert(id.clone(), bound).is_some() {
            return Err(SelectError(format!(
                "{fact_context} duplicates payload fact {id}"
            )));
        }
    }
    Ok(facts)
}

fn validate_facts(
    value: &Value,
    admissions: &BTreeMap<String, BTreeMap<String, BoundFact>>,
) -> Result<(BTreeSet<String>, BTreeSet<String>, Vec<String>), SelectError> {
    let mut fact_ids = BTreeSet::new();
    let mut signals = BTreeSet::new();
    let mut evidence = BTreeSet::new();
    let mut used_admissions = BTreeSet::new();
    for (index, fact) in value.as_array().unwrap().iter().enumerate() {
        let context = format!("facts[{index}]");
        exact_object(
            fact,
            &context,
            &["id", "signalIds", "evidenceKinds", "admissionIds"],
        )?;
        let id = nonempty_string(&fact["id"], &format!("{context}.id"))?;
        if !fact_ids.insert(id) {
            return Err(SelectError(format!("duplicate fact id {id}")));
        }
        let fact_signals = string_set(&fact["signalIds"], &format!("{context}.signalIds"), true)?;
        let fact_evidence = string_set(
            &fact["evidenceKinds"],
            &format!("{context}.evidenceKinds"),
            true,
        )?;
        for admission in string_set(
            &fact["admissionIds"],
            &format!("{context}.admissionIds"),
            true,
        )? {
            let admitted_facts = admissions.get(&admission).ok_or_else(|| {
                SelectError(format!(
                    "{context} references unknown admission {admission}"
                ))
            })?;
            let bound = admitted_facts.get(id).ok_or_else(|| {
                SelectError(format!(
                    "{context} fact {id} is not present in A04 admitted payload {admission}"
                ))
            })?;
            if bound.signal_ids != fact_signals || bound.evidence_kinds != fact_evidence {
                return Err(SelectError(format!(
                    "{context} signal/evidence labels differ from A04 admitted payload {admission}"
                )));
            }
            used_admissions.insert(admission);
        }
        signals.extend(fact_signals);
        evidence.extend(fact_evidence);
    }
    let admitted = admissions.keys().cloned().collect::<BTreeSet<_>>();
    if used_admissions != admitted {
        let unused = admitted
            .difference(&used_admissions)
            .cloned()
            .collect::<Vec<_>>();
        return Err(SelectError(format!(
            "unused admission identities are not referenced by facts: {}",
            unused.join(",")
        )));
    }
    Ok((signals, evidence, used_admissions.into_iter().collect()))
}

fn exact_object(value: &Value, context: &str, fields: &[&str]) -> Result<(), SelectError> {
    allowed_object(value, context, fields, &[])
}

fn allowed_object(
    value: &Value,
    context: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<(), SelectError> {
    let object = value
        .as_object()
        .ok_or_else(|| SelectError(format!("{context} must be an object")))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let required = required.iter().copied().collect::<BTreeSet<_>>();
    let optional = optional.iter().copied().collect::<BTreeSet<_>>();
    let allowed = required.union(&optional).copied().collect::<BTreeSet<_>>();
    if !required.is_subset(&actual) || !actual.is_subset(&allowed) {
        return Err(SelectError(format!(
            "{context} fields differ from v1 contract"
        )));
    }
    Ok(())
}

fn nonempty_array<'a>(value: &'a Value, context: &str) -> Result<&'a [Value], SelectError> {
    value
        .as_array()
        .filter(|items| !items.is_empty())
        .map(Vec::as_slice)
        .ok_or_else(|| SelectError(format!("{context} must be a non-empty array")))
}

fn string_set(
    value: &Value,
    context: &str,
    require_nonempty: bool,
) -> Result<BTreeSet<String>, SelectError> {
    let array = value
        .as_array()
        .ok_or_else(|| SelectError(format!("{context} must be an array")))?;
    if require_nonempty && array.is_empty() {
        return Err(SelectError(format!("{context} must not be empty")));
    }
    let mut set = BTreeSet::new();
    for item in array {
        let text = nonempty_string(item, context)?;
        if !set.insert(text.to_string()) {
            return Err(SelectError(format!("{context} contains duplicate {text}")));
        }
    }
    Ok(set)
}

fn nonempty_string<'a>(value: &'a Value, context: &str) -> Result<&'a str, SelectError> {
    value
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| SelectError(format!("{context} must be a non-empty string")))
}

fn digest(value: &Value) -> bool {
    value.as_str().is_some_and(is_digest)
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
