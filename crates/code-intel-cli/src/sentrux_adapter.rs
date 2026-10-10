use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

#[path = "sentrux_gate_policy.rs"]
mod sentrux_gate_policy;

pub(crate) const AUTHORITATIVE_RULE_KINDS: [&str; 6] = [
    "max_cc",
    "max_cycles",
    "max_coupling",
    "no_god_files",
    "layer_order",
    "boundary_dependency",
];
const COMMAND_RULE_KINDS: [&str; 2] = ["sentrux_gate", "sentrux_check"];

pub(crate) fn translate(
    native: &Value,
    evaluated_at: u64,
    max_age_seconds: u64,
) -> Result<Value, String> {
    validate_native(native)?;
    if max_age_seconds == 0 {
        return Err("Sentrux freshness policy max age must be positive".to_string());
    }

    let expected = native["expectedSnapshotIdentity"].as_str().unwrap();
    let consumed = native["sourceSnapshotIdentity"].as_str().unwrap();
    let observed_at = native["observedAt"].as_u64().unwrap();
    let status = native["status"].as_str().unwrap();
    let mut rules = normalize_rules(&native["authoritativeRules"])?;
    let (handshake_complete, advisories) = validate_gate_results(native, &mut rules)?;
    let known = rules
        .iter()
        .filter_map(|rule| rule["kind"].as_str())
        .filter(|kind| known_rule(kind))
        .collect::<BTreeSet<_>>();
    let has_unknown = rules.iter().any(|rule| rule["status"] == "unsupported");
    let all_known_evaluated = rules.iter().all(|rule| {
        !known_rule(rule["kind"].as_str().unwrap_or("")) || rule["status"] == "evaluated"
    });
    // Legacy rule labels alone are never a current policy admission handshake.
    let complete_command_observation = COMMAND_RULE_KINDS.iter().all(|kind| known.contains(kind));
    let complete = status == "complete"
        && !has_unknown
        && all_known_evaluated
        && complete_command_observation
        && handshake_complete;
    let completeness = if complete { "complete" } else { "partial" };
    let failure = if status == "crashed" {
        native["nativeFailure"].clone()
    } else if complete {
        json!({"kind":"none"})
    } else {
        json!({
            "kind":"domain_unknown",
            "message":"Sentrux authoritative rule normalization is incomplete"
        })
    };
    let freshness = if expected != consumed {
        "snapshot_mismatch"
    } else if observed_at <= evaluated_at && evaluated_at - observed_at <= max_age_seconds {
        "current"
    } else {
        "stale"
    };
    let effects = sorted_effects(&native["declaredEffects"])?;
    let request = json!({
        "schema":"code-intel-evidence-admissibility-request.v1",
        "expectedSnapshotIdentity":native["expectedSnapshotIdentity"],
        "policy":{"evaluatedAt":evaluated_at,"maxAgeSeconds":max_age_seconds},
        "observation":{
            "schema":"code-intel-observed-evidence.v1",
            "provider":{
                "id":"structural-evidence.sentrux",
                "implementation":native["implementation"]
            },
            "source":{"revision":native["sourceRevision"]},
            "consumedSnapshotIdentity":native["sourceSnapshotIdentity"],
            "observedAt":observed_at,
            "completeness":completeness,
            "claimedComplete":complete,
            "payload":native["payload"],
            "provenance":{
                "collectionId":format!("sentrux-{}-{observed_at}", native["sourceRevision"].as_str().unwrap()),
                "command":"provider sentrux-adapt",
                "startedAt":native["collectedAt"],
                "completedAt":observed_at
            },
            "failure":failure
        }
    });

    Ok(json!({
        "schema":"code-intel-sentrux-adapter-result.v2",
        "gatePolicy":native["gatePolicy"],
        "port":{
            "schema":"code-intel-structural-evidence-port.v2",
            "gatePolicy":native["gatePolicy"],
            "gateResults":native["gateResults"],
            "advisories":advisories,
            "status":status,
            "completeness":completeness,
            "freshness":freshness,
            "expectedSnapshotIdentity":expected,
            "sourceSnapshotIdentity":consumed,
            "provider":{
                "implementationId":native["implementation"]["id"],
                "rollbackIdentity":native["rollbackIdentity"]
            },
            "provenance":{
                "sourceRevision":native["sourceRevision"],
                "observedAt":observed_at
            },
            "effects":{
                "declared":effects,
                "observed":effects,
                "match":true
            },
            "rules":rules,
            "payload":native["payload"],
            "diagnosisEligible":false
        },
        "evidence":{"request":request},
        "factPromotion":{
            "eligible":false,
            "requires":"evidence.admissibility-validate",
            "engineeringFacts":[]
        }
    }))
}

pub(crate) fn validate_admitted_payload(payload: &Value, adapter: &Value) -> Result<(), String> {
    exact(payload, &["schema", "data"], "Sentrux evidence payload")?;
    if payload["schema"] != "code-intel-evidence-payload.v1" {
        return Err("Sentrux evidence payload schema is invalid".to_string());
    }
    let evidence = &payload["data"]["structuralEvidence"];
    exact(
        evidence,
        &[
            "schema",
            "gatePolicy",
            "gateResults",
            "advisories",
            "snapshotIdentity",
            "provider",
            "provenance",
            "effects",
            "completeness",
            "rules",
        ],
        "Sentrux structural evidence",
    )?;
    // The payload is content-addressed, so its bytes must be a function of the
    // snapshot alone; the collection wall-clock lives in the port and in the
    // A04 observation instead. See `builtin_provider_evidence::payload_provenance`.
    exact(
        &evidence["provenance"],
        &["sourceRevision"],
        "Sentrux structural evidence provenance",
    )?;
    sentrux_gate_policy::validate_identity(&evidence["gatePolicy"])?;
    if evidence["schema"] != "code-intel-structural-evidence-payload.v2"
        || evidence["gatePolicy"] != adapter["port"]["gatePolicy"]
        || evidence["gateResults"] != adapter["port"]["gateResults"]
        || evidence["advisories"] != adapter["port"]["advisories"]
        || evidence["snapshotIdentity"] != adapter["port"]["sourceSnapshotIdentity"]
        || evidence["provider"] != adapter["port"]["provider"]
        || evidence["provenance"]["sourceRevision"]
            != adapter["port"]["provenance"]["sourceRevision"]
        || evidence["effects"] != adapter["port"]["effects"]
        || evidence["completeness"] != adapter["port"]["completeness"]
        || evidence["rules"] != adapter["port"]["rules"]
    {
        return Err(
            "Sentrux admitted payload does not match the structural evidence port".to_string(),
        );
    }
    Ok(())
}

fn validate_gate_results(
    native: &Value,
    rules: &mut [Value],
) -> Result<(bool, Vec<Value>), String> {
    let results = native["gateResults"]
        .as_array()
        .filter(|results| results.len() == 2)
        .ok_or("Sentrux gateResults must contain gate and check admissions")?;
    let mut seen = BTreeSet::new();
    let mut complete = true;
    let mut advisories = Vec::new();
    for entry in results {
        exact(entry, &["kind", "admission"], "Sentrux gate result binding")?;
        let kind = entry["kind"]
            .as_str()
            .filter(|kind| COMMAND_RULE_KINDS.contains(kind))
            .ok_or("Sentrux gate result kind is invalid")?;
        if !seen.insert(kind) {
            return Err("Sentrux gate result kinds must be unique".into());
        }
        let result = &entry["admission"];
        let verdict = if result.is_null() {
            "unknown"
        } else {
            sentrux_gate_policy::validate_result(result)?;
            if result["policy"] != native["gatePolicy"]
                || result["current"]["sourceCommit"] != native["sourceRevision"]
                || result["current"]["scope"] != "."
                || (kind == "sentrux_check" && result["ruleScope"] != "static_and_ratchet")
                || (kind == "sentrux_gate" && result["ruleScope"] != "baseline_ratchet")
            {
                return Err(
                    "Sentrux gate result policy/source/scope does not match the provider".into(),
                );
            }
            for advisory in result["advisories"].as_array().unwrap() {
                if !advisories.contains(advisory) {
                    advisories.push(advisory.clone());
                }
            }
            result["verdict"].as_str().unwrap()
        };
        let Some(rule) = rules.iter_mut().find(|rule| rule["kind"] == kind) else {
            complete = false;
            continue;
        };
        if verdict == "unknown" {
            complete = false;
            *rule = json!({
                "kind":kind,"status":"not_evaluated","verdict":"unknown",
                "failure":{"kind":"domain_unknown","message":"same-policy typed gate admission is unavailable"}
            });
        } else if rule["status"] == "evaluated" && rule["verdict"] != verdict {
            return Err("Sentrux rule verdict contradicts its typed gate result".into());
        } else if rule["status"] != "evaluated" {
            complete = false;
        } else if (verdict == "pass" && rule.get("details").is_some())
            || (verdict == "fail" && rule["details"]["violations"] != result["blockingViolations"])
        {
            return Err("Sentrux rule violation details contradict its typed gate result".into());
        }
    }
    Ok((complete, advisories))
}

fn normalize_rules(value: &Value) -> Result<Vec<Value>, String> {
    let rules = value
        .as_array()
        .ok_or("Sentrux authoritative rules must be an array")?;
    let mut normalized = BTreeMap::new();
    for rule in rules {
        exact_with_optional(
            rule,
            &["kind", "status", "verdict", "failure"],
            &["details"],
            "Sentrux authoritative rule",
        )?;
        if let Some(details) = rule.get("details") {
            validate_rule_details(details)?;
        }
        let kind = rule["kind"]
            .as_str()
            .filter(|kind| !kind.is_empty())
            .ok_or("Sentrux authoritative rule kind is invalid")?;
        if normalized.contains_key(kind) {
            return Err("Sentrux authoritative rule kinds must be unique".to_string());
        }
        let normalized_rule = if known_rule(kind) {
            validate_known_rule(rule)?;
            rule.clone()
        } else {
            json!({
                "kind":kind,
                "status":"unsupported",
                "verdict":"unknown",
                "failure":{
                    "kind":"domain_unknown",
                    "message":"unrecognized authoritative Sentrux rule kind"
                }
            })
        };
        normalized.insert(kind.to_string(), normalized_rule);
    }
    Ok(normalized.into_values().collect())
}

fn known_rule(kind: &str) -> bool {
    AUTHORITATIVE_RULE_KINDS.contains(&kind) || COMMAND_RULE_KINDS.contains(&kind)
}

fn validate_known_rule(rule: &Value) -> Result<(), String> {
    let status = rule["status"].as_str().unwrap_or("");
    let verdict = rule["verdict"].as_str().unwrap_or("");
    let failure = &rule["failure"];
    let failure_kind = failure["kind"].as_str().unwrap_or("");
    match status {
        "evaluated"
            if matches!(verdict, "pass" | "fail")
                && failure_kind == "none"
                && failure.as_object().is_some_and(|object| object.len() == 1) =>
        {
            Ok(())
        }
        "not_evaluated"
            if verdict == "unknown"
                && failure_kind == "domain_unknown"
                && failure["message"]
                    .as_str()
                    .is_some_and(|message| !message.is_empty())
                && failure.as_object().is_some_and(|object| object.len() == 2) =>
        {
            Ok(())
        }
        _ => Err("Sentrux authoritative rule status/verdict/failure is inconsistent".to_string()),
    }
}

fn validate_native(native: &Value) -> Result<(), String> {
    exact(
        native,
        &[
            "schema",
            "gatePolicy",
            "gateResults",
            "status",
            "implementation",
            "rollbackIdentity",
            "sourceRevision",
            "expectedSnapshotIdentity",
            "sourceSnapshotIdentity",
            "collectedAt",
            "observedAt",
            "declaredEffects",
            "observedEffects",
            "authoritativeRules",
            "nativeFailure",
            "payload",
        ],
        "Sentrux provider native result",
    )?;
    sentrux_gate_policy::validate_identity(&native["gatePolicy"])?;
    if native["schema"] != "code-intel-sentrux-provider-native.v2"
        || !matches!(
            native["status"].as_str(),
            Some("complete" | "partial" | "crashed")
        )
        || !digest(&native["expectedSnapshotIdentity"])
        || !digest(&native["sourceSnapshotIdentity"])
        || native["collectedAt"].as_u64().is_none()
        || native["observedAt"].as_u64().is_none()
        || native["observedAt"].as_u64().unwrap() < native["collectedAt"].as_u64().unwrap()
    {
        return Err("Sentrux native identity/status/time is invalid".to_string());
    }
    exact(
        &native["implementation"],
        &["id", "version", "digest"],
        "Sentrux provider implementation",
    )?;
    if !matches!(
        (
            native["implementation"]["id"].as_str(),
            native["implementation"]["version"].as_str()
        ),
        (Some("sentrux-native"), Some("3.0.0")) | (Some("sentrux.command-adapter"), Some("1.0.0"))
    ) || !digest(&native["implementation"]["digest"])
        || !nonempty(&native["rollbackIdentity"])
        || !nonempty(&native["sourceRevision"])
    {
        return Err("Sentrux implementation/rollback/source identity is invalid".to_string());
    }
    let declared = sorted_effects(&native["declaredEffects"])?;
    let observed = sorted_effects(&native["observedEffects"])?;
    if declared != observed {
        return Err("Sentrux observed effects do not match declared effects".to_string());
    }
    let failure = &native["nativeFailure"];
    if native["status"] == "crashed" {
        if failure["kind"] != "provider_unavailable"
            || !failure["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty())
            || failure.as_object().is_none_or(|object| object.len() != 2)
            || native["authoritativeRules"]
                .as_array()
                .is_none_or(|rules| !rules.is_empty())
        {
            return Err("crashed Sentrux provider failure semantics are invalid".to_string());
        }
    } else if failure != &json!({"kind":"none"}) {
        return Err("non-crashed Sentrux provider cannot report native failure".to_string());
    }
    crate::capability::validate_artifact_ref_shape(&native["payload"])?;
    if native["payload"]["artifactSchema"] != "code-intel-evidence-payload.v1"
        || native["payload"]["type"] != "observed.evidence.payload"
        || native["payload"]["consumedSnapshotIdentity"] != native["sourceSnapshotIdentity"]
    {
        return Err("Sentrux payload contract/snapshot is invalid".to_string());
    }
    Ok(())
}

fn sorted_effects(value: &Value) -> Result<Vec<&str>, String> {
    let effects = value.as_array().ok_or("Sentrux effects must be an array")?;
    let mut result = BTreeSet::new();
    for effect in effects {
        let effect = effect.as_str().unwrap_or("");
        if !matches!(effect, "repo_read" | "local_write" | "process_spawn") {
            return Err("Sentrux effect is invalid".to_string());
        }
        if !result.insert(effect) {
            return Err("Sentrux effects must be unique".to_string());
        }
    }
    if result.is_empty() {
        return Err("Sentrux must declare at least one effect".to_string());
    }
    Ok(result.into_iter().collect())
}

const MAX_RULE_DETAIL_VIOLATIONS: usize = 32;
const MAX_RULE_DETAIL_TARGETS: usize = 16;
const MAX_RULE_DETAIL_TEXT: usize = 1024;

/// Optional structured failure evidence on a rule: which concrete violations
/// produced the verdict and which files to open first. This is what lets the
/// Hospital name targets instead of a bare "architecture gate failure".
fn validate_rule_details(details: &Value) -> Result<(), String> {
    exact(details, &["violations"], "Sentrux rule details")?;
    let violations = details["violations"]
        .as_array()
        .ok_or("Sentrux rule detail violations must be an array")?;
    if violations.is_empty() || violations.len() > MAX_RULE_DETAIL_VIOLATIONS {
        return Err("Sentrux rule detail violations must be non-empty and bounded".to_string());
    }
    for violation in violations {
        exact(
            violation,
            &["rule", "message", "targets"],
            "Sentrux rule detail violation",
        )?;
        let rule_ok = violation["rule"]
            .as_str()
            .is_some_and(|text| !text.is_empty() && text.len() <= MAX_RULE_DETAIL_TEXT);
        let message_ok = violation["message"]
            .as_str()
            .is_some_and(|text| !text.is_empty() && text.len() <= MAX_RULE_DETAIL_TEXT);
        let targets = violation["targets"]
            .as_array()
            .ok_or("Sentrux rule detail targets must be an array")?;
        let targets_ok = targets.len() <= MAX_RULE_DETAIL_TARGETS
            && targets.iter().all(|target| {
                target
                    .as_str()
                    .is_some_and(|text| !text.is_empty() && text.len() <= MAX_RULE_DETAIL_TEXT)
            });
        if !rule_ok || !message_ok || !targets_ok {
            return Err("Sentrux rule detail violation fields are invalid".to_string());
        }
    }
    Ok(())
}

fn exact_with_optional(
    value: &Value,
    required: &[&str],
    optional: &[&str],
    label: &str,
) -> Result<(), String> {
    let actual = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let missing = required.iter().any(|field| !actual.contains(field));
    let unexpected = actual
        .iter()
        .any(|field| !required.contains(field) && !optional.contains(field));
    if missing || unexpected {
        return Err(format!("{label} fields are invalid"));
    }
    Ok(())
}

fn exact(value: &Value, fields: &[&str], label: &str) -> Result<(), String> {
    let actual = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = fields.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label} fields are invalid"))
    }
}

fn nonempty(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.is_empty())
}

fn digest(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}
