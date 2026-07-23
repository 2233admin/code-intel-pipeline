use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

const MAX_RULE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetectError(String);

impl fmt::Display for DetectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DetectError {}

#[derive(Debug, Clone)]
pub(crate) struct RuleTable {
    choice_kinds: BTreeSet<String>,
    fact_kinds: BTreeSet<String>,
}

pub(crate) fn load_rule_table(path: &Path) -> Result<RuleTable, DetectError> {
    let metadata = fs::metadata(path)
        .map_err(|error| DetectError(format!("inspect decision-gap rules: {error}")))?;
    if !metadata.is_file() || metadata.len() > MAX_RULE_BYTES {
        return Err(DetectError(
            "decision-gap rules must be a bounded regular file".to_string(),
        ));
    }
    let document: Value = serde_json::from_slice(
        &fs::read(path)
            .map_err(|error| DetectError(format!("read decision-gap rules: {error}")))?,
    )
    .map_err(|error| DetectError(format!("parse decision-gap rules: {error}")))?;
    exact_object(
        &document,
        "rule table",
        &["schema", "choiceKinds", "factKinds"],
    )?;
    if document["schema"] != "code-intel-decision-gap-rules.v1" {
        return Err(DetectError("decision-gap rule schema is invalid".into()));
    }
    let choice_kinds = string_set(&document["choiceKinds"], "choiceKinds", true)?;
    let fact_kinds = string_set(&document["factKinds"], "factKinds", true)?;
    if !choice_kinds.is_disjoint(&fact_kinds) {
        return Err(DetectError(
            "choiceKinds and factKinds must be disjoint".into(),
        ));
    }
    Ok(RuleTable {
        choice_kinds,
        fact_kinds,
    })
}

pub(crate) fn detect(request: &Value, rules: &RuleTable) -> Result<Value, DetectError> {
    exact_object(request, "request", &["schema", "branches"])?;
    if request["schema"] != "code-intel-decision-gap-detection-request.v1" {
        return Err(DetectError("decision-gap request schema is invalid".into()));
    }
    let branches = request["branches"]
        .as_array()
        .ok_or_else(|| DetectError("branches must be an array".into()))?;
    let mut branch_inputs = BTreeMap::new();
    for (index, branch) in branches.iter().enumerate() {
        let context = format!("branches[{index}]");
        exact_object(branch, &context, &["branchId", "status", "blockers"])?;
        let id = nonempty_string(&branch["branchId"], &format!("{context}.branchId"))?;
        let status = nonempty_string(&branch["status"], &format!("{context}.status"))?;
        if !matches!(status, "completed" | "pending") {
            return Err(DetectError(format!("{context}.status is invalid")));
        }
        if branch_inputs.insert(id.to_string(), branch).is_some() {
            return Err(DetectError(format!("duplicate branchId {id}")));
        }
    }

    let known_branches = branch_inputs.keys().cloned().collect::<BTreeSet<_>>();
    let mut blocker_ids = BTreeSet::new();
    let mut gaps = BTreeMap::new();
    let mut fact_discovery = BTreeMap::new();
    let mut branch_gap_ids: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut branch_fact_ids: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for (owner_branch, branch) in &branch_inputs {
        let blockers = branch["blockers"].as_array().ok_or_else(|| {
            DetectError(format!("branch {owner_branch}.blockers must be an array"))
        })?;
        for blocker in blockers {
            let parsed = parse_blocker(blocker, &known_branches)?;
            if !blocker_ids.insert(parsed.id.clone()) {
                return Err(DetectError(format!("duplicate blocker id {}", parsed.id)));
            }
            if !rules.choice_kinds.contains(&parsed.kind)
                && !rules.fact_kinds.contains(&parsed.kind)
            {
                return Err(DetectError(format!("unknown blocker kind {}", parsed.kind)));
            }
            let unresolved_facts = parsed
                .facts
                .iter()
                .filter(|fact| fact["status"] == "missing")
                .filter_map(|fact| fact["factId"].as_str())
                .chain(parsed.missing_fact_ids.iter().map(String::as_str))
                .map(str::to_string)
                .collect::<BTreeSet<_>>();
            if rules.fact_kinds.contains(&parsed.kind) && unresolved_facts.is_empty() {
                return Err(DetectError(format!(
                    "fact blocker {} must name an unresolved fact",
                    parsed.id
                )));
            }
            if !unresolved_facts.is_empty() {
                let affected = if parsed.affected_branches.is_empty() {
                    BTreeSet::from([owner_branch.clone()])
                } else {
                    parsed.affected_branches.clone()
                };
                for branch_id in &affected {
                    branch_fact_ids
                        .entry(branch_id.clone())
                        .or_default()
                        .insert(parsed.id.clone());
                }
                fact_discovery.insert(
                    parsed.id.clone(),
                    json!({
                        "blockerId": parsed.id,
                        "kind": parsed.kind,
                        "missingFactIds": unresolved_facts,
                        "affectedBranches": affected,
                    }),
                );
                continue;
            }
            if parsed.facts.is_empty() {
                return Err(DetectError(format!(
                    "choice blocker {} must name discoverable facts checked",
                    parsed.id
                )));
            }
            if parsed.options.len() < 2 {
                return Err(DetectError(format!(
                    "choice blocker {} must provide at least two options",
                    parsed.id
                )));
            }
            if !parsed.options.contains_key(&parsed.recommended_option_id) {
                return Err(DetectError(format!(
                    "choice blocker {} recommends an unknown option",
                    parsed.id
                )));
            }
            if parsed.affected_branches.is_empty() {
                return Err(DetectError(format!(
                    "choice blocker {} must affect at least one branch",
                    parsed.id
                )));
            }
            for branch_id in &parsed.affected_branches {
                branch_gap_ids
                    .entry(branch_id.clone())
                    .or_default()
                    .insert(parsed.id.clone());
            }
            gaps.insert(
                parsed.id.clone(),
                json!({
                    "schema": "code-intel-decision-gap.v1",
                    "id": parsed.id,
                    "kind": parsed.kind,
                    "blockedDecision": parsed.blocked_decision,
                    "discoverableFactsChecked": parsed.facts,
                    "options": parsed.options.into_values().collect::<Vec<_>>(),
                    "recommendedAnswer": {
                        "kind": "proposal",
                        "optionId": parsed.recommended_option_id,
                        "rationale": parsed.recommendation_rationale,
                    },
                    "affectedBranches": parsed.affected_branches,
                    "authorityRequired": true,
                    "authorityState": "unresolved",
                    "effects": [],
                }),
            );
        }
    }

    let branch_results = branch_inputs
        .into_iter()
        .map(|(branch_id, branch)| {
            let gap_ids = branch_gap_ids.remove(&branch_id).unwrap_or_default();
            let fact_ids = branch_fact_ids.remove(&branch_id).unwrap_or_default();
            let status = if !gap_ids.is_empty() {
                "blocked_decision_gap"
            } else if !fact_ids.is_empty() {
                "fact_discovery_required"
            } else {
                branch["status"].as_str().unwrap()
            };
            json!({
                "branchId": branch_id,
                "status": status,
                "blockedByGapIds": gap_ids,
                "factDiscoveryBlockerIds": fact_ids,
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "schema": "code-intel-decision-gap-detection-result.v1",
        "status": "completed",
        "gaps": gaps.into_values().collect::<Vec<_>>(),
        "factDiscovery": fact_discovery.into_values().collect::<Vec<_>>(),
        "branches": branch_results,
        "answersRecorded": false,
        "authorityEvents": [],
        "adoptionDecisions": [],
        "committedEngineeringPlans": [],
        "engineeringFacts": [],
    }))
}

struct ParsedBlocker {
    id: String,
    kind: String,
    blocked_decision: String,
    facts: Vec<Value>,
    missing_fact_ids: BTreeSet<String>,
    options: BTreeMap<String, Value>,
    recommended_option_id: String,
    recommendation_rationale: String,
    affected_branches: BTreeSet<String>,
}

fn parse_blocker(
    value: &Value,
    known_branches: &BTreeSet<String>,
) -> Result<ParsedBlocker, DetectError> {
    exact_object(
        value,
        "blocker",
        &[
            "id",
            "kind",
            "blockedDecision",
            "discoverableFactsChecked",
            "missingFactIds",
            "options",
            "recommendedOptionId",
            "recommendationRationale",
            "affectedBranches",
        ],
    )?;
    let id = nonempty_string(&value["id"], "blocker.id")?.to_string();
    let kind = nonempty_string(&value["kind"], "blocker.kind")?.to_string();
    let blocked_decision =
        nonempty_string(&value["blockedDecision"], "blocker.blockedDecision")?.to_string();
    let missing_fact_ids = string_set(&value["missingFactIds"], "blocker.missingFactIds", false)?;
    let affected_branches = string_set(
        &value["affectedBranches"],
        "blocker.affectedBranches",
        false,
    )?;
    if let Some(unknown) = affected_branches
        .iter()
        .find(|branch| !known_branches.contains(*branch))
    {
        return Err(DetectError(format!(
            "blocker {id} references unknown branch {unknown}"
        )));
    }
    let facts = parse_facts(&value["discoverableFactsChecked"])?;
    let options = parse_options(&value["options"])?;
    Ok(ParsedBlocker {
        id,
        kind,
        blocked_decision,
        facts,
        missing_fact_ids,
        options,
        recommended_option_id: nonempty_string(
            &value["recommendedOptionId"],
            "blocker.recommendedOptionId",
        )?
        .to_string(),
        recommendation_rationale: nonempty_string(
            &value["recommendationRationale"],
            "blocker.recommendationRationale",
        )?
        .to_string(),
        affected_branches,
    })
}

fn parse_facts(value: &Value) -> Result<Vec<Value>, DetectError> {
    let entries = value
        .as_array()
        .ok_or_else(|| DetectError("discoverableFactsChecked must be an array".into()))?;
    let mut facts = BTreeMap::new();
    for entry in entries {
        exact_object(entry, "fact check", &["factId", "status"])?;
        let fact_id = nonempty_string(&entry["factId"], "fact check.factId")?;
        let status = nonempty_string(&entry["status"], "fact check.status")?;
        if !matches!(status, "resolved" | "missing") {
            return Err(DetectError("fact check.status is invalid".into()));
        }
        if facts.insert(fact_id.to_string(), entry.clone()).is_some() {
            return Err(DetectError(format!("duplicate factId {fact_id}")));
        }
    }
    Ok(facts.into_values().collect())
}

fn parse_options(value: &Value) -> Result<BTreeMap<String, Value>, DetectError> {
    let entries = value
        .as_array()
        .ok_or_else(|| DetectError("options must be an array".into()))?;
    let mut options = BTreeMap::new();
    for entry in entries {
        exact_object(entry, "option", &["id", "label", "consequence"])?;
        let id = nonempty_string(&entry["id"], "option.id")?;
        nonempty_string(&entry["label"], "option.label")?;
        nonempty_string(&entry["consequence"], "option.consequence")?;
        if options.insert(id.to_string(), entry.clone()).is_some() {
            return Err(DetectError(format!("duplicate option id {id}")));
        }
    }
    Ok(options)
}

fn exact_object(value: &Value, context: &str, keys: &[&str]) -> Result<(), DetectError> {
    let object = value
        .as_object()
        .ok_or_else(|| DetectError(format!("{context} must be an object")))?;
    let expected = keys.iter().copied().collect::<BTreeSet<_>>();
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(DetectError(format!("{context} has invalid fields")));
    }
    Ok(())
}

fn nonempty_string<'a>(value: &'a Value, context: &str) -> Result<&'a str, DetectError> {
    value
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| DetectError(format!("{context} must be a non-empty string")))
}

fn string_set(
    value: &Value,
    context: &str,
    require_nonempty: bool,
) -> Result<BTreeSet<String>, DetectError> {
    let values = value
        .as_array()
        .ok_or_else(|| DetectError(format!("{context} must be an array")))?;
    if require_nonempty && values.is_empty() {
        return Err(DetectError(format!("{context} must not be empty")));
    }
    let mut result = BTreeSet::new();
    for value in values {
        let item = nonempty_string(value, context)?.to_string();
        if !result.insert(item.clone()) {
            return Err(DetectError(format!("{context} contains duplicate {item}")));
        }
    }
    Ok(result)
}
