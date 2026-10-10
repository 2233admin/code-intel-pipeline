//! Immutable admission policy shared by the engine and evidence consumers.
//! Target-repository policy files never select the policy evaluated here.
use serde_json::{json, Value};
#[path = "content_contract/sha256.rs"]
mod content_sha256;
pub(crate) use content_sha256::sha256_hex;
use std::collections::BTreeSet;
use std::sync::LazyLock;

const MANIFEST: &[u8] = include_bytes!("../../../orchestration/sentrux-gate-policy.v1.json");
const APPROVED_SHA256: &str = "f97cfed77d80be09acda1331bf449bb21e09ee58d91e19255313bbc5815d11f0";
const RESULT_SCHEMA: &str = "code-intel-sentrux-gate-result.v1";
const RATCHETS: [&str; 4] = [
    "quality_degraded",
    "coupling_increased",
    "cycles_increased",
    "god_files_increased",
];
const STATIC_RULES: [&str; 6] = [
    "max_cc",
    "no_god_files",
    "max_cycles",
    "max_coupling",
    "boundary_dependency",
    "layer_order",
];

static POLICY: LazyLock<Value> = LazyLock::new(|| {
    assert_eq!(
        sha256_hex(MANIFEST),
        APPROVED_SHA256,
        "embedded admission policy differs from the approved bytes"
    );
    serde_json::from_slice(MANIFEST).expect("approved admission policy is JSON")
});

fn manifest() -> &'static Value {
    &POLICY
}
pub(crate) fn identity() -> Value {
    let policy = manifest();
    json!({"policyId": policy["policyId"], "policyVersion": policy["policyVersion"],
        "sha256": APPROVED_SHA256})
}

pub(crate) fn measurement() -> &'static Value {
    &manifest()["measurement"]
}

pub(crate) fn validate_identity(value: &Value) -> Result<(), String> {
    let policy = manifest();
    if value.as_object().is_none_or(|object| object.len() != 3)
        || value["policyId"] != policy["policyId"]
        || value["policyVersion"] != policy["policyVersion"]
        || value["sha256"] != APPROVED_SHA256
    {
        return Err("missing or unapproved Sentrux gate policy identity".into());
    }
    Ok(())
}

pub(crate) fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn paths(value: &Value) -> Result<BTreeSet<&str>, String> {
    let entries = value
        .as_array()
        .ok_or("god-file comparison must contain path arrays")?;
    let mut result = BTreeSet::new();
    let mut previous = None;
    for entry in entries {
        let path = entry.as_str().ok_or("god-file path must be a string")?;
        if !valid_path(path) || previous.is_some_and(|last| last >= path) {
            return Err("god-file paths must be canonical, sorted and unique".into());
        }
        previous = Some(path);
        result.insert(path);
    }
    Ok(result)
}

fn violation_rule(value: &Value) -> Result<&str, String> {
    let object = value.as_object().ok_or("violation must be an object")?;
    let rule = value["rule"].as_str().ok_or("violation rule missing")?;
    if object.len() != 3
        || value["message"].as_str().is_none_or(str::is_empty)
        || value["targets"].as_array().is_none_or(|targets| {
            targets
                .iter()
                .any(|target| target.as_str().is_none_or(str::is_empty))
        })
    {
        return Err("malformed violation".into());
    }
    Ok(rule)
}

fn metadata(value: &Value, baseline: bool) -> Result<(), String> {
    if value
        .as_object()
        .is_none_or(|object| object.len() != if baseline { 4 } else { 2 })
        || value["scope"] != "."
        || value["sourceCommit"].as_str().is_none_or(str::is_empty)
    {
        return Err("gate result has invalid source or measurement scope".into());
    }
    if baseline {
        let digest = value["sha256"].as_str().unwrap_or("");
        if value["schema"] != measurement()["baselineSchema"]
            || digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("gate result has invalid baseline identity".into());
        }
    }
    Ok(())
}

/// Validate the whole admission claim, not merely its label or exit status.
/// Snapshot binding remains the provider boundary's responsibility.
pub(crate) fn validate_result(value: &Value) -> Result<(), String> {
    if value.as_object().is_none_or(|object| object.len() != 10) || value["schema"] != RESULT_SCHEMA
    {
        return Err("unsupported Sentrux gate result schema".into());
    }
    validate_identity(&value["policy"])?;
    if &value["measurement"] != measurement() {
        return Err("Sentrux gate measurement identity mismatch".into());
    }
    metadata(&value["current"], false)?;
    let scope = value["ruleScope"]
        .as_str()
        .ok_or("missing gate ruleScope")?;
    if !matches!(scope, "baseline_ratchet" | "static_and_ratchet") {
        return Err("unsupported gate ruleScope".into());
    }
    let baseline = value.get("baseline").ok_or("missing baseline identity")?;
    if !baseline.is_null() {
        metadata(baseline, true)?;
    }
    let blocking = value["blockingViolations"]
        .as_array()
        .ok_or("missing blocking violations")?;
    let advisories = value["advisories"].as_array().ok_or("missing advisories")?;
    let mut blocking_rules = BTreeSet::new();
    for violation in blocking {
        let rule = violation_rule(violation)?;
        if !RATCHETS[1..].contains(&rule)
            && !STATIC_RULES.contains(&rule)
            && !matches!(rule, "baseline_missing" | "baseline_engine_mismatch")
        {
            return Err("unknown or advisory rule in blocking violations".into());
        }
        blocking_rules.insert(rule);
    }
    for advisory in advisories {
        if violation_rule(advisory)? != "quality_degraded" {
            return Err("non-Quality rule cannot be advisory".into());
        }
    }
    if advisories.len() > 1 {
        return Err("duplicate Quality advisory".into());
    }
    let comparisons = value["comparisons"]
        .as_array()
        .ok_or("missing gate comparisons")?;
    let mut seen = BTreeSet::new();
    let mut failed = BTreeSet::new();
    for comparison in comparisons {
        let rule = comparison["rule"]
            .as_str()
            .ok_or("comparison rule missing")?;
        let is_ratchet = RATCHETS.contains(&rule);
        if comparison
            .as_object()
            .is_none_or(|object| object.len() != 5)
            || comparison.get("before").is_none()
            || (!is_ratchet && !STATIC_RULES.contains(&rule))
        {
            return Err("malformed or unknown gate comparison".into());
        }
        if is_ratchet && (!seen.insert(rule) || baseline.is_null()) {
            return Err("duplicate or ungrounded ratchet comparison".into());
        }
        let disposition = if rule == "quality_degraded" {
            "advisory"
        } else {
            "blocking"
        };
        if comparison["disposition"] != disposition {
            return Err("gate comparison disposition contradicts approved policy".into());
        }
        let regressed = if rule == "god_files_increased" {
            let before = paths(&comparison["before"])?;
            let after = paths(&comparison["after"])?;
            !after.is_subset(&before)
        } else if is_ratchet {
            let before = comparison["before"]
                .as_f64()
                .ok_or("comparison before must be numeric")?;
            let after = comparison["after"]
                .as_f64()
                .ok_or("comparison after must be numeric")?;
            if !before.is_finite()
                || !after.is_finite()
                || before < 0.0
                || after < 0.0
                || (rule == "quality_degraded"
                    && (before > 10000.0
                        || after > 10000.0
                        || before.fract() != 0.0
                        || after.fract() != 0.0))
                || (rule == "cycles_increased" && (before.fract() != 0.0 || after.fract() != 0.0))
            {
                return Err("gate comparison has invalid numeric values".into());
            }
            if rule == "quality_degraded" {
                after < before
            } else {
                after > before
            }
        } else {
            if scope != "static_and_ratchet"
                || !comparison["before"].is_null()
                || violation_rule(&comparison["after"])? != rule
                || !blocking.contains(&comparison["after"])
            {
                return Err("static comparison is not backed by a blocking violation".into());
            }
            true
        };
        if comparison["verdict"] != if regressed { "fail" } else { "pass" } {
            return Err("comparison verdict contradicts measured before/after".into());
        }
        if regressed {
            failed.insert(rule);
        }
    }
    if !baseline.is_null() && !RATCHETS.iter().all(|rule| seen.contains(rule)) {
        return Err("gate result omits a required ratchet comparison".into());
    }
    for rule in RATCHETS {
        let reported = if rule == "quality_degraded" {
            !advisories.is_empty()
        } else {
            blocking_rules.contains(rule)
        };
        if failed.contains(rule) != reported {
            return Err("gate diagnostics contradict ratchet comparisons".into());
        }
    }
    for rule in &blocking_rules {
        if STATIC_RULES.contains(rule) && !failed.contains(rule) {
            return Err("blocking static violation lacks a comparison".into());
        }
    }
    let missing = blocking_rules.contains("baseline_missing");
    let mismatch = blocking_rules.contains("baseline_engine_mismatch");
    if baseline.is_null() != (missing || mismatch) || (missing && mismatch) {
        return Err("baseline identity contradicts baseline diagnostics".into());
    }
    let verdict = if blocking.is_empty() {
        "pass"
    } else if missing && blocking.len() == 1 {
        "unknown"
    } else {
        "fail"
    };
    if value["verdict"] != verdict {
        return Err("gate verdict contradicts blocking diagnostics".into());
    }
    Ok(())
}

pub(crate) fn current_structural_policy(structural: &Value, snapshot: &Value) -> bool {
    if structural["schema"] != "code-intel-structural-evidence-payload.v2"
        || snapshot.as_str().is_none_or(str::is_empty)
        || structural["snapshotIdentity"] != *snapshot
        || validate_identity(&structural["gatePolicy"]).is_err()
    {
        return false;
    }
    let Some(results) = structural["gateResults"].as_array() else {
        return false;
    };
    let Some(rules) = structural["rules"].as_array() else {
        return false;
    };
    if results.len() != 2 {
        return false;
    }
    let Some(advisories) = structural["advisories"].as_array() else {
        return false;
    };
    for kind in ["sentrux_gate", "sentrux_check"] {
        let mut matches = results.iter().filter(|result| result["kind"] == kind);
        let mut rule_matches = rules.iter().filter(|rule| rule["kind"] == kind);
        let (Some(entry), Some(rule)) = (matches.next(), rule_matches.next()) else {
            return false;
        };
        if matches.next().is_some() || rule_matches.next().is_some() {
            return false;
        }
        let result = &entry["admission"];
        if validate_result(result).is_err()
            || result["policy"] != structural["gatePolicy"]
            || result["current"]["sourceCommit"] != structural["provenance"]["sourceRevision"]
            || (kind == "sentrux_gate" && result["ruleScope"] != "baseline_ratchet")
            || (kind == "sentrux_check" && result["ruleScope"] != "static_and_ratchet")
            || !matches!(result["verdict"].as_str(), Some("pass" | "fail"))
            || rule["verdict"] != result["verdict"]
        {
            return false;
        }
        if result["advisories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| !advisories.contains(item))
        {
            return false;
        }
    }
    advisories.iter().enumerate().all(|(index, advisory)| {
        !advisories[..index].contains(advisory)
            && results.iter().any(|entry| {
                entry["admission"]["advisories"]
                    .as_array()
                    .unwrap()
                    .contains(advisory)
            })
    })
}

/// Current consumers must not reinterpret historical or unrecognized evidence
/// as approved-policy authority. ArtifactRef verification still owns the hash
/// and snapshot lease; this check owns the current envelope/policy semantics.
pub(crate) fn current_capability(payload: &Value) -> bool {
    payload["schema"] == "code-intel-sentrux-capability-artifact.v2"
        && payload["contractVersion"] == 2
        && validate_identity(&payload["gatePolicy"]).is_ok()
        && payload["freshness"]["status"] == "current"
        && payload["snapshotIdentity"]
            .as_str()
            .is_some_and(|snapshot| {
                !snapshot.is_empty()
                    && payload["inputs"]["snapshotIdentity"] == snapshot
                    && payload["freshness"]["consumedSnapshotIdentity"] == snapshot
            })
        && match payload["provider"]["mode"].as_str() {
            Some("builtin") => {
                payload["provider"]["id"] == measurement()["engineId"]
                    && payload["provider"]["version"] == measurement()["engineVersion"]
            }
            Some("external") => {
                payload["provider"]["id"] == "sentrux.command-adapter"
                    && payload["provider"]["version"] == "1.0.0"
            }
            Some("lite_fallback") => {
                payload["provider"]["id"] == "sentrux.lite-capabilities"
                    && payload["provider"]["version"] == "1.0.0"
            }
            _ => false,
        }
}

pub(crate) fn validated_admission(payload: &Value) -> Option<&Value> {
    if !current_capability(payload) || payload["authority"] != "authoritative" {
        return None;
    }
    let command = &payload["outputs"]["command"];
    let result = command.get("admission")?;
    validate_result(result).ok()?;
    if result["policy"] != payload["gatePolicy"]
        || command["violations"] != result["blockingViolations"]
        || command["advisories"] != result["advisories"]
        || payload["outputs"]["verdict"] != result["verdict"]
        || !matches!(
            payload["status"].as_str(),
            Some("succeeded" | "failed" | "degraded")
        )
    {
        return None;
    }
    Some(result)
}

pub(crate) fn capability_admission(payload: &Value) -> Value {
    match validated_admission(payload) {
        Some(result) => json!({
            "verdict": result["verdict"],
            "policy": result["policy"],
            "ruleScope": result["ruleScope"],
            "blockingViolations": result["blockingViolations"],
            "advisories": result["advisories"],
        }),
        None => json!({
            "verdict":"unknown",
            "reason":"No authoritative current approved-policy typed gate result; command exit status and raw Quality are not admission.",
        }),
    }
}
