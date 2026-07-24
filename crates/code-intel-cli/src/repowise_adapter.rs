use std::collections::BTreeSet;

use serde_json::{json, Value};

const SNAPSHOT_DIGEST_LEN: usize = 64;

pub(crate) fn translate(
    native: &Value,
    evaluated_at: u64,
    max_age_seconds: u64,
) -> Result<Value, String> {
    validate_native(native)?;
    if max_age_seconds == 0 {
        return Err("Repowise freshness policy max age must be positive".to_string());
    }

    let cli_available = native["cli"]["status"] == "available";
    let health_status = if cli_available {
        native["health"]["status"].as_str().unwrap()
    } else {
        "unavailable"
    };
    let health = json!({
        "kind":"health",
        "status":health_status,
        "evidence":false,
        "effects":["process_probe"]
    });

    let mut evidence = Vec::new();
    let index = translate_channel(
        native,
        "index",
        evaluated_at,
        max_age_seconds,
        cli_available,
        &["read_repository", "write_repowise_index"],
        &mut evidence,
    )?;
    let docs = translate_channel(
        native,
        "docs",
        evaluated_at,
        max_age_seconds,
        cli_available,
        &[
            "read_repowise_index",
            "network_provider",
            "model_inference",
            "write_repowise_docs",
        ],
        &mut evidence,
    )?;

    Ok(json!({
        "schema":"code-intel-repowise-adapter-result.v1",
        "provider":"repowise",
        "health":health,
        "index":index,
        "docs":docs,
        "evidence":evidence,
        "factPromotion":{
            "eligible":false,
            "requires":"evidence.admissibility-validate",
            "engineeringFacts":[]
        }
    }))
}

fn translate_channel(
    native: &Value,
    channel: &str,
    evaluated_at: u64,
    max_age_seconds: u64,
    cli_available: bool,
    effects: &[&str],
    evidence: &mut Vec<Value>,
) -> Result<Value, String> {
    let input = &native[channel];
    let native_status = input["status"].as_str().unwrap();
    if !cli_available {
        return Ok(channel_summary(
            "unavailable",
            "none",
            "not_observed",
            effects,
            "local_tool_error",
        ));
    }

    let (completeness, failure_kind, failure_message) = match (channel, native_status) {
        ("index", "current" | "stale") | ("docs", "complete") => ("complete", "none", None),
        ("docs", "partial") => (
            "partial",
            "domain_unknown",
            Some("Repowise docs are incomplete"),
        ),
        ("docs", "quota") => (
            "partial",
            "provider_unavailable",
            Some("Repowise docs provider quota unavailable"),
        ),
        ("docs", "not_requested") => {
            return Ok(channel_summary(
                native_status,
                "none",
                "not_applicable",
                effects,
                "none",
            ));
        }
        ("index", "missing" | "unavailable") | ("docs", "unavailable") => {
            return Ok(channel_summary(
                native_status,
                "none",
                "not_observed",
                effects,
                "provider_unavailable",
            ));
        }
        _ => return Err(format!("Repowise {channel} status is invalid")),
    };

    let observed_at = input["observedAt"]
        .as_u64()
        .ok_or_else(|| format!("Repowise {channel} observedAt is invalid"))?;
    let freshness = freshness_state(observed_at, evaluated_at, max_age_seconds);
    let payload = input
        .get("payload")
        .filter(|value| value.is_object())
        .ok_or_else(|| format!("Repowise {channel} payload is required"))?;
    let failure = match failure_message {
        Some(message) => json!({"kind":failure_kind,"message":message}),
        None => json!({"kind":"none"}),
    };
    let request = json!({
        "schema":"code-intel-evidence-admissibility-request.v1",
        "expectedSnapshotIdentity":native["snapshotIdentity"],
        "policy":{"evaluatedAt":evaluated_at,"maxAgeSeconds":max_age_seconds},
        "observation":{
            "schema":"code-intel-observed-evidence.v1",
            "provider":{
                "id":format!("repowise.{channel}"),
                "implementation":{
                    "id":"provider.repowise-adapt",
                    "version":native["implementation"]["version"],
                    "digest":native["implementation"]["digest"]
                }
            },
            "source":{"revision":native["sourceRevision"]},
            "consumedSnapshotIdentity":native["snapshotIdentity"],
            "observedAt":observed_at,
            "completeness":completeness,
            "claimedComplete":completeness == "complete",
            "payload":payload,
            "provenance":{
                "collectionId":format!("repowise-{channel}-{}-{observed_at}", native["sourceRevision"].as_str().unwrap()),
                "command":format!("repowise adapter:{channel}"),
                "startedAt":native["collectedAt"],
                "completedAt":observed_at
            },
            "failure":failure
        }
    });
    evidence.push(json!({"channel":channel,"request":request}));

    Ok(channel_summary(
        native_status,
        completeness,
        freshness,
        effects,
        failure_kind,
    ))
}

fn channel_summary(
    status: &str,
    completeness: &str,
    freshness: &str,
    effects: &[&str],
    failure_kind: &str,
) -> Value {
    json!({
        "status":status,
        "completeness":completeness,
        "freshness":freshness,
        "effects":effects,
        "failureKind":failure_kind
    })
}

fn freshness_state(observed_at: u64, evaluated_at: u64, max_age_seconds: u64) -> &'static str {
    if observed_at <= evaluated_at && evaluated_at - observed_at <= max_age_seconds {
        "current"
    } else {
        "stale"
    }
}

fn validate_native(native: &Value) -> Result<(), String> {
    exact_optional(
        native,
        &[
            "schema",
            "implementation",
            "sourceRevision",
            "snapshotIdentity",
            "collectedAt",
            "cli",
            "health",
            "index",
            "docs",
        ],
        &["diagnostic"],
        "Repowise native result",
    )?;
    if native["schema"] != "code-intel-repowise-native-result.v1" {
        return Err("Repowise native result schema is invalid".to_string());
    }
    exact(
        &native["implementation"],
        &["version", "digest"],
        "Repowise implementation",
    )?;
    nonempty(&native["implementation"]["version"], "Repowise version")?;
    digest(
        &native["implementation"]["digest"],
        "Repowise implementation digest",
    )?;
    nonempty(&native["sourceRevision"], "Repowise source revision")?;
    digest(&native["snapshotIdentity"], "Repowise snapshot identity")?;
    native["collectedAt"]
        .as_u64()
        .ok_or("Repowise collectedAt is invalid")?;
    exact(&native["cli"], &["status"], "Repowise CLI status")?;
    if !matches!(
        native["cli"]["status"].as_str(),
        Some("available" | "missing")
    ) {
        return Err("Repowise CLI status is invalid".to_string());
    }
    exact(&native["health"], &["status"], "Repowise health")?;
    if !matches!(
        native["health"]["status"].as_str(),
        Some("healthy" | "unavailable")
    ) {
        return Err("Repowise health status is invalid".to_string());
    }
    validate_channel_shape(&native["index"], "index")?;
    validate_channel_shape(&native["docs"], "docs")?;
    Ok(())
}

fn validate_channel_shape(value: &Value, channel: &str) -> Result<(), String> {
    exact_optional(
        value,
        &["status"],
        &["observedAt", "payload"],
        &format!("Repowise {channel}"),
    )?;
    let status = value["status"]
        .as_str()
        .ok_or_else(|| format!("Repowise {channel} status is invalid"))?;
    let allowed = if channel == "index" {
        ["current", "stale", "missing", "unavailable"].as_slice()
    } else {
        [
            "complete",
            "partial",
            "quota",
            "not_requested",
            "unavailable",
        ]
        .as_slice()
    };
    if !allowed.contains(&status) {
        return Err(format!("Repowise {channel} status is invalid"));
    }
    Ok(())
}

fn exact(value: &Value, fields: &[&str], label: &str) -> Result<(), String> {
    exact_optional(value, fields, &[], label)
}

fn exact_optional(
    value: &Value,
    required: &[&str],
    optional: &[&str],
    label: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let required = required.iter().copied().collect::<BTreeSet<_>>();
    let allowed = required
        .iter()
        .copied()
        .chain(optional.iter().copied())
        .collect::<BTreeSet<_>>();
    if required.is_subset(&actual) && actual.is_subset(&allowed) {
        Ok(())
    } else {
        Err(format!("{label} fields are invalid"))
    }
}

fn nonempty(value: &Value, label: &str) -> Result<(), String> {
    if value.as_str().is_some_and(|value| !value.is_empty()) {
        Ok(())
    } else {
        Err(format!("{label} is invalid"))
    }
}

fn digest(value: &Value, label: &str) -> Result<(), String> {
    if value.as_str().is_some_and(|value| {
        value.len() == SNAPSHOT_DIGEST_LEN
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }) {
        Ok(())
    } else {
        Err(format!("{label} is invalid"))
    }
}
