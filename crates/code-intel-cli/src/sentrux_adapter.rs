use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

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
    let rules = normalize_rules(&native["authoritativeRules"])?;
    let known = rules
        .iter()
        .filter_map(|rule| rule["kind"].as_str())
        .filter(|kind| known_rule(kind))
        .collect::<BTreeSet<_>>();
    let has_unknown = rules.iter().any(|rule| rule["status"] == "unsupported");
    let all_known_evaluated = rules.iter().all(|rule| {
        !known_rule(rule["kind"].as_str().unwrap_or("")) || rule["status"] == "evaluated"
    });
    let complete_legacy_rules = AUTHORITATIVE_RULE_KINDS
        .iter()
        .all(|kind| known.contains(kind));
    let complete_command_observation = COMMAND_RULE_KINDS.iter().all(|kind| known.contains(kind));
    let complete = status == "complete"
        && !has_unknown
        && all_known_evaluated
        && (complete_legacy_rules || complete_command_observation);
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
        "schema":"code-intel-sentrux-adapter-result.v1",
        "port":{
            "schema":"code-intel-structural-evidence-port.v1",
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
            "snapshotIdentity",
            "provider",
            "provenance",
            "effects",
            "completeness",
            "rules",
        ],
        "Sentrux structural evidence",
    )?;
    if evidence["schema"] != "code-intel-structural-evidence-payload.v1"
        || evidence["snapshotIdentity"] != adapter["port"]["sourceSnapshotIdentity"]
        || evidence["provider"] != adapter["port"]["provider"]
        || evidence["provenance"] != adapter["port"]["provenance"]
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

fn normalize_rules(value: &Value) -> Result<Vec<Value>, String> {
    let rules = value
        .as_array()
        .ok_or("Sentrux authoritative rules must be an array")?;
    let mut normalized = BTreeMap::new();
    for rule in rules {
        exact(
            rule,
            &["kind", "status", "verdict", "failure"],
            "Sentrux authoritative rule",
        )?;
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
    if native["schema"] != "code-intel-sentrux-provider-native.v1"
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
    if !nonempty(&native["implementation"]["id"])
        || !nonempty(&native["implementation"]["version"])
        || !digest(&native["implementation"]["digest"])
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
