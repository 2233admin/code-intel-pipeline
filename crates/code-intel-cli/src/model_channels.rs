use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

const FAILURE_CATEGORIES: [&str; 9] = [
    "consent_required",
    "model_unavailable",
    "provider_unavailable",
    "provider_quota",
    "config_error",
    "local_tool_error",
    "adapter_protocol_error",
    "external_data_forbidden",
    "paid_usage_forbidden",
];
const COST_SCOPES: [&str; 4] = [
    "local_compute",
    "subscription_cli",
    "free_or_internal_quota",
    "metered_api",
];
const CONSENT_STATUSES: [&str; 3] = ["unanswered", "granted", "denied"];
const INVENTORY_DIAGNOSTICS: [&str; 26] = [
    "candidate_verified",
    "version_probe_passed",
    "fallback_candidate_selected",
    "candidate_timed_out",
    "candidate_verification_failed",
    "endpoint_configured",
    "endpoint_not_configured",
    "model_declared_by_user",
    "model_not_declared",
    "endpoint_value_not_collected",
    "model_catalog_observed",
    "auth_method_api_key",
    "auth_method_oauth",
    "auth_method_subscription",
    "auth_method_unknown",
    "installation_present",
    "installation_not_found",
    "config_present_content_not_read",
    "config_not_found",
    "presence_only",
    "credential_values_not_collected",
    "endpoint_values_not_collected",
    "endpoint_not_probed",
    "endpoint_probe_passed",
    "endpoint_probe_failed",
    "model_not_in_catalog",
];

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let parsed = match parse_cli(raw) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let request = match read_json(&parsed.request) {
        Ok(request) => request,
        Err((code, message)) => {
            eprintln!("{message}");
            return code;
        }
    };
    let result = match parsed.operation.as_str() {
        "inventory-validate" => validate_inventory(&request).map(|_| request),
        "route" => route(&request),
        _ => unreachable!("operation was validated by parse_cli"),
    };
    let result = match result {
        Ok(result) => result,
        Err(message) => {
            eprintln!("{message}");
            return 65;
        }
    };
    let output = serde_json::to_vec_pretty(&result).expect("model channel result serializes");
    if let Some(path) = parsed.out.as_deref() {
        if let Err(error) = fs::write(path, &output) {
            eprintln!("cannot write {}: {error}", path.display());
            return 74;
        }
    }
    println!("{}", String::from_utf8(output).expect("JSON is UTF-8"));
    if result.get("status").and_then(Value::as_str) == Some("consent_required") {
        2
    } else {
        0
    }
}

struct Cli {
    operation: String,
    request: PathBuf,
    out: Option<PathBuf>,
}

fn parse_cli(raw: &[String]) -> Result<Cli, String> {
    let operation = raw
        .first()
        .map(String::as_str)
        .ok_or("usage: model <inventory-validate|route> --request <json> [--out <json>]")?;
    if !matches!(operation, "inventory-validate" | "route") {
        return Err(format!("unknown model operation: {operation}"));
    }
    let mut request = None;
    let mut out = None;
    let mut index = 1;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(flag, "--request" | "--out") {
            return Err(format!("unknown model argument: {flag}"));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires exactly one value"))?;
        let slot = if flag == "--request" {
            &mut request
        } else {
            &mut out
        };
        if slot.replace(PathBuf::from(value)).is_some() {
            return Err(format!("duplicate model argument: {flag}"));
        }
        index += 2;
    }
    Ok(Cli {
        operation: operation.to_string(),
        request: request.ok_or("model operation requires --request")?,
        out,
    })
}

fn read_json(path: &Path) -> Result<Value, (i32, String)> {
    let bytes =
        fs::read(path).map_err(|error| (74, format!("cannot read {}: {error}", path.display())))?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err((64, "model request exceeds 8 MiB".into()));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| (64, format!("model request is not UTF-8: {error}")))?;
    crate::capability::reject_duplicate_json_keys(text).map_err(|error| (64, error))?;
    serde_json::from_str(text).map_err(|error| (64, format!("invalid JSON: {error}")))
}

fn validate_inventory(value: &Value) -> Result<(), String> {
    let object = exact_object(value, &["schema", "candidates", "configurationBrokers"])?;
    exact_string(
        object,
        "schema",
        "code-intel-model-channel-inventory-result.v1",
    )?;
    let candidates = exact_array(object, "candidates")?;
    let mut ids = std::collections::BTreeSet::new();
    for candidate in candidates {
        let candidate = exact_object(
            candidate,
            &[
                "id",
                "channelKind",
                "provider",
                "model",
                "costScope",
                "endpointConfigured",
                "discovered",
                "executableVerified",
                "authPresent",
                "modelAvailable",
                "externalEgress",
                "source",
                "diagnostics",
            ],
        )?;
        let id = portable_id(candidate, "id")?;
        if !ids.insert(id) {
            return Err("candidate ids must be unique".into());
        }
        enum_string(
            candidate,
            "channelKind",
            &[
                "local_compatible",
                "ollama",
                "claude_cli",
                "opencode_cli",
                "codex_cli",
            ],
        )?;
        nullable_string(candidate, "provider")?;
        nullable_string(candidate, "model")?;
        enum_string(candidate, "costScope", &COST_SCOPES)?;
        bool_field(candidate, "endpointConfigured")?;
        bool_field(candidate, "discovered")?;
        bool_field(candidate, "executableVerified")?;
        enum_string(
            candidate,
            "authPresent",
            &["unknown", "present", "absent", "not_applicable"],
        )?;
        enum_string(
            candidate,
            "modelAvailable",
            &["unknown", "available", "unavailable"],
        )?;
        bool_field(candidate, "externalEgress")?;
        enum_string(
            candidate,
            "source",
            &["user_input", "local_discovery", "cc_switch", "cli_config"],
        )?;
        enum_string_array(candidate, "diagnostics", &INVENTORY_DIAGNOSTICS)?;
    }
    let brokers = exact_array(object, "configurationBrokers")?;
    let mut broker_ids = std::collections::BTreeSet::new();
    for broker in brokers {
        let broker = exact_object(
            broker,
            &["id", "kind", "discovered", "configPresent", "diagnostics"],
        )?;
        let id = portable_id(broker, "id")?;
        if !broker_ids.insert(id) {
            return Err("configuration broker ids must be unique".into());
        }
        enum_string(broker, "kind", &["cc_switch", "manual_config"])?;
        bool_field(broker, "discovered")?;
        bool_field(broker, "configPresent")?;
        enum_string_array(broker, "diagnostics", &INVENTORY_DIAGNOSTICS)?;
    }
    Ok(())
}

pub(crate) fn route(request: &Value) -> Result<Value, String> {
    let object = exact_object(request, &["schema", "inventory", "policy", "workload"])?;
    exact_string(object, "schema", "code-intel-model-routing-request.v1")?;
    let inventory = object
        .get("inventory")
        .expect("exact object contains inventory");
    validate_inventory(inventory)?;
    let policy = exact_object(
        object.get("policy").expect("exact object contains policy"),
        &[
            "consumptionAuthorization",
            "externalData",
            "paidSpend",
            "selection",
        ],
    )?;
    let consumption = exact_object(
        policy
            .get("consumptionAuthorization")
            .expect("exact policy"),
        &["status", "scopes"],
    )?;
    let consumption_status = enum_string(consumption, "status", &CONSENT_STATUSES)?;
    let authorized_scopes = enum_string_array(consumption, "scopes", &COST_SCOPES)?;
    let external_status = status_object(policy, "externalData")?;
    let paid_status = status_object(policy, "paidSpend")?;
    let selection = exact_object(
        policy.get("selection").expect("exact policy"),
        &["pinnedAdapter", "fallbackPolicy"],
    )?;
    let pinned = nullable_portable_id(selection, "pinnedAdapter")?;
    let fallback = enum_string(selection, "fallbackPolicy", &["denied", "allowed"])?;
    let workload = exact_object(
        object
            .get("workload")
            .expect("exact object contains workload"),
        &["requiresExternalData"],
    )?;
    let requires_external = bool_field(workload, "requiresExternalData")?;

    let candidates = inventory["candidates"].as_array().expect("validated array");
    let mut ordered: Vec<&Value> = Vec::with_capacity(candidates.len());
    if let Some(pinned_id) = pinned {
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate["id"] == pinned_id)
        {
            ordered.push(candidate);
        }
    }
    ordered.extend(
        candidates
            .iter()
            .filter(|candidate| pinned.map_or(true, |pinned_id| candidate["id"] != pinned_id)),
    );
    let pinned_seen = pinned.map(|id| candidates.iter().any(|candidate| candidate["id"] == id));
    let mut attempts = Vec::new();
    let mut selected = None;
    let mut pinned_failed = pinned_seen == Some(false);
    for candidate in ordered {
        let id = candidate["id"].as_str().expect("validated id");
        let is_pinned = pinned == Some(id);
        if pinned.is_some() && !is_pinned && (fallback == "denied" || !pinned_failed) {
            attempts.push(attempt(
                id,
                "discovered",
                false,
                Some("config_error"),
                "fallback_not_authorized",
            ));
            continue;
        }
        let (state, category, reason) = evaluate_candidate(
            candidate,
            consumption_status,
            &authorized_scopes,
            external_status,
            paid_status,
            requires_external,
        );
        let eligible = category.is_none();
        attempts.push(attempt(id, state, eligible, category, reason));
        if eligible && selected.is_none() {
            selected = Some(json!({
                "candidateId": id,
                "channelKind": candidate["channelKind"],
                "provider": candidate["provider"],
                "model": candidate["model"],
                "costScope": candidate["costScope"],
                "readinessState": "ready"
            }));
            break;
        }
        if is_pinned {
            pinned_failed = true;
        }
    }
    if pinned_seen == Some(false) {
        attempts.insert(
            0,
            attempt(
                pinned.unwrap(),
                "discovered",
                false,
                Some("config_error"),
                "pinned_adapter_not_found",
            ),
        );
    }
    let consent_blocked = attempts.iter().any(|attempt| {
        attempt["failureCategory"] == "consent_required"
            || attempt["failureCategory"] == "external_data_forbidden"
            || attempt["failureCategory"] == "paid_usage_forbidden"
    });
    let status = if selected.is_some() {
        "ready"
    } else if consent_blocked {
        "consent_required"
    } else {
        "deterministic_degraded"
    };
    let manual_action = match status {
        "consent_required" => Value::String("obtain_explicit_authorization".into()),
        "deterministic_degraded" => Value::String("provide_or_enable_model_channel".into()),
        _ => Value::Null,
    };
    Ok(json!({
        "schema": "code-intel-model-routing-result.v1",
        "status": status,
        "selected": selected,
        "authorization": {
            "consumptionAuthorization": {
                "status": consumption_status,
                "scopes": authorized_scopes
            },
            "externalData": {"status": external_status},
            "paidSpend": {"status": paid_status}
        },
        "attempts": attempts,
        "manualAction": manual_action
    }))
}

fn evaluate_candidate<'a>(
    candidate: &'a Value,
    consumption_status: &str,
    authorized_scopes: &[&str],
    external_status: &str,
    paid_status: &str,
    requires_external: bool,
) -> (&'static str, Option<&'static str>, &'static str) {
    if !candidate["discovered"].as_bool().unwrap() {
        return ("discovered", Some("provider_unavailable"), "not_discovered");
    }
    if candidate["channelKind"] == "local_compatible"
        && !candidate["endpointConfigured"].as_bool().unwrap()
    {
        return (
            "executable_verified",
            Some("config_error"),
            "endpoint_not_configured",
        );
    }
    if !candidate["executableVerified"].as_bool().unwrap() {
        return (
            "executable_verified",
            Some("local_tool_error"),
            "executable_not_verified",
        );
    }
    if candidate["authPresent"] == "absent" {
        return (
            "auth_present",
            Some("provider_unavailable"),
            "authentication_absent",
        );
    }
    if candidate["authPresent"] == "unknown" {
        return (
            "auth_present",
            Some("provider_unavailable"),
            "authentication_unverified",
        );
    }
    if candidate["modelAvailable"] != "available" {
        return (
            "model_available",
            Some("model_unavailable"),
            "model_not_available",
        );
    }
    if requires_external
        && candidate["externalEgress"].as_bool().unwrap()
        && external_status != "granted"
    {
        return (
            "egress_allowed",
            Some("external_data_forbidden"),
            "external_data_not_authorized",
        );
    }
    let scope = candidate["costScope"].as_str().unwrap();
    if consumption_status != "granted" || !authorized_scopes.contains(&scope) {
        return (
            "spend_allowed",
            Some("consent_required"),
            "consumption_scope_not_authorized",
        );
    }
    if scope == "metered_api" && paid_status != "granted" {
        return (
            "spend_allowed",
            Some("paid_usage_forbidden"),
            "paid_spend_not_authorized",
        );
    }
    ("ready", None, "eligible")
}

fn attempt(id: &str, state: &str, eligible: bool, category: Option<&str>, reason: &str) -> Value {
    debug_assert!(category.map_or(true, |value| FAILURE_CATEGORIES.contains(&value)));
    json!({
        "candidateId": id,
        "readinessState": state,
        "eligible": eligible,
        "failureCategory": category,
        "reason": reason
    })
}

fn status_object<'a>(parent: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    let object = exact_object(parent.get(field).expect("exact policy"), &["status"])?;
    enum_string(object, "status", &CONSENT_STATUSES)
}

fn exact_object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let object = value.as_object().ok_or("expected an object")?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(format!("object must contain exactly: {}", keys.join(", ")));
    }
    Ok(object)
}

fn exact_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    expected: &str,
) -> Result<&'a str, String> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a string"))?;
    if value != expected {
        return Err(format!("{key} must equal {expected}"));
    }
    Ok(value)
}
fn nonempty_string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{key} must be a non-empty string"))?;
    Ok(value)
}
fn portable_id<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    let value = nonempty_string(object, key)?;
    if value.len() > 128
        || !value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        })
    {
        return Err(format!("{key} must be a portable identifier"));
    }
    Ok(value)
}
fn nullable_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, String> {
    match object.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        _ => Err(format!("{key} must be null or a non-empty string")),
    }
}
fn nullable_portable_id<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, String> {
    match nullable_string(object, key)? {
        None => Ok(None),
        Some(value)
            if value.len() <= 128
                && value.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_alphanumeric()
                        || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
                }) =>
        {
            Ok(Some(value))
        }
        Some(_) => Err(format!("{key} must be a portable identifier")),
    }
}
fn enum_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<&'a str, String> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a string"))?;
    if !allowed.contains(&value) {
        return Err(format!("{key} is outside the closed vocabulary"));
    }
    Ok(value)
}
fn bool_field(object: &Map<String, Value>, key: &str) -> Result<bool, String> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{key} must be boolean"))
}
fn exact_array<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a Vec<Value>, String> {
    object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} must be an array"))
}
fn enum_string_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<Vec<&'a str>, String> {
    let array = exact_array(object, key)?;
    let mut result = Vec::new();
    let mut unique = std::collections::BTreeSet::new();
    for value in array {
        let value = value
            .as_str()
            .ok_or_else(|| format!("{key} must contain strings"))?;
        if !allowed.contains(&value) || !unique.insert(value) {
            return Err(format!(
                "{key} must contain unique closed-vocabulary values"
            ));
        }
        result.push(value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, scope: &str) -> Value {
        json!({"id":id,"channelKind":"ollama","provider":"ollama","model":"qwen","costScope":scope,"endpointConfigured":true,"discovered":true,"executableVerified":true,"authPresent":"not_applicable","modelAvailable":"available","externalEgress":false,"source":"local_discovery","diagnostics":[]})
    }
    fn request(
        candidates: Vec<Value>,
        consumption: &str,
        scopes: Vec<&str>,
        pinned: Value,
        fallback: &str,
    ) -> Value {
        json!({"schema":"code-intel-model-routing-request.v1","inventory":{"schema":"code-intel-model-channel-inventory-result.v1","candidates":candidates,"configurationBrokers":[]},"policy":{"consumptionAuthorization":{"status":consumption,"scopes":scopes},"externalData":{"status":"denied"},"paidSpend":{"status":"denied"},"selection":{"pinnedAdapter":pinned,"fallbackPolicy":fallback}},"workload":{"requiresExternalData":false}})
    }
    #[test]
    fn local_channel_requires_explicit_compute_authorization() {
        let result = route(&request(
            vec![candidate("ollama", "local_compute")],
            "unanswered",
            vec![],
            Value::Null,
            "denied",
        ))
        .unwrap();
        assert_eq!(result["status"], "consent_required");
        assert_eq!(result["attempts"][0]["readinessState"], "spend_allowed");
    }
    #[test]
    fn authorized_local_channel_is_ready() {
        let result = route(&request(
            vec![candidate("ollama", "local_compute")],
            "granted",
            vec!["local_compute"],
            Value::Null,
            "denied",
        ))
        .unwrap();
        assert_eq!(result["status"], "ready");
        assert_eq!(result["selected"]["candidateId"], "ollama");
    }
    #[test]
    fn pinned_adapter_does_not_fallback_without_permission() {
        let mut unavailable = candidate("pinned", "subscription_cli");
        unavailable["modelAvailable"] = json!("unavailable");
        let result = route(&request(
            vec![unavailable, candidate("other", "local_compute")],
            "granted",
            vec!["subscription_cli", "local_compute"],
            json!("pinned"),
            "denied",
        ))
        .unwrap();
        assert_eq!(result["status"], "deterministic_degraded");
        assert!(result["selected"].is_null());
        assert_eq!(result["attempts"][1]["reason"], "fallback_not_authorized");
    }
    #[test]
    fn ready_pinned_adapter_is_evaluated_before_inventory_order() {
        let result = route(&request(
            vec![
                candidate("other", "local_compute"),
                candidate("pinned", "local_compute"),
            ],
            "granted",
            vec!["local_compute"],
            json!("pinned"),
            "allowed",
        ))
        .unwrap();
        assert_eq!(result["selected"]["candidateId"], "pinned");
        assert_eq!(result["attempts"][0]["candidateId"], "pinned");
    }
    #[test]
    fn missing_pinned_adapter_falls_back_only_when_allowed() {
        let result = route(&request(
            vec![candidate("other", "local_compute")],
            "granted",
            vec!["local_compute"],
            json!("missing"),
            "allowed",
        ))
        .unwrap();
        assert_eq!(result["status"], "ready");
        assert_eq!(result["selected"]["candidateId"], "other");
        assert_eq!(result["attempts"][0]["reason"], "pinned_adapter_not_found");
    }
    #[test]
    fn unknown_authentication_is_not_ready() {
        let mut channel = candidate("claude", "subscription_cli");
        channel["channelKind"] = json!("claude_cli");
        channel["authPresent"] = json!("unknown");
        let result = route(&request(
            vec![channel],
            "granted",
            vec!["subscription_cli"],
            Value::Null,
            "denied",
        ))
        .unwrap();
        assert_eq!(result["status"], "deterministic_degraded");
        assert_eq!(result["attempts"][0]["readinessState"], "auth_present");
    }
    #[test]
    fn metered_api_needs_separate_paid_spend_consent() {
        let result = route(&request(
            vec![candidate("api", "metered_api")],
            "granted",
            vec!["metered_api"],
            Value::Null,
            "denied",
        ))
        .unwrap();
        assert_eq!(result["status"], "consent_required");
        assert_eq!(
            result["attempts"][0]["failureCategory"],
            "paid_usage_forbidden"
        );
    }
    #[test]
    fn repository_data_on_external_channel_needs_separate_egress_consent() {
        let mut channel = candidate("claude", "subscription_cli");
        channel["channelKind"] = json!("claude_cli");
        channel["authPresent"] = json!("present");
        channel["externalEgress"] = json!(true);
        let mut request = request(
            vec![channel],
            "granted",
            vec!["subscription_cli"],
            Value::Null,
            "denied",
        );
        request["workload"]["requiresExternalData"] = json!(true);
        let result = route(&request).unwrap();
        assert_eq!(result["status"], "consent_required");
        assert_eq!(
            result["attempts"][0]["failureCategory"],
            "external_data_forbidden"
        );
    }
    #[test]
    fn inventory_rejects_unknown_fields() {
        let mut value = json!({"schema":"code-intel-model-channel-inventory-result.v1","candidates":[],"configurationBrokers":[]});
        value["secret"] = json!("must-not-be-accepted");
        assert!(validate_inventory(&value).is_err());
    }
}
