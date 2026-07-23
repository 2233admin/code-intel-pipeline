use std::collections::BTreeSet;

use serde_json::{json, Value};

pub(crate) fn translate(
    native: &Value,
    evaluated_at: u64,
    max_age_seconds: u64,
) -> Result<Value, String> {
    validate_native(native)?;
    if max_age_seconds == 0 {
        return Err("CodeNexus freshness policy max age must be positive".to_string());
    }

    let mode = native["providerMode"].as_str().unwrap();
    let status = native["status"].as_str().unwrap();
    let observed_at = native["observedAt"].as_u64().unwrap();
    let expected = native["expectedSnapshotIdentity"].as_str().unwrap();
    let consumed = native["sourceSnapshotIdentity"].as_str().unwrap();
    let freshness = if expected != consumed {
        "snapshot_mismatch"
    } else if observed_at <= evaluated_at && evaluated_at - observed_at <= max_age_seconds {
        "current"
    } else {
        "stale"
    };
    let completeness = if status == "current" {
        "complete"
    } else {
        "partial"
    };
    let (failure_kind, failure) = match status {
        "current" => ("none", json!({"kind":"none"})),
        "partial" => (
            "domain_unknown",
            json!({"kind":"domain_unknown","message":"CodeNexus evidence is partial"}),
        ),
        "unavailable" => (
            "provider_unavailable",
            json!({"kind":"provider_unavailable","message":"CodeNexus provider is unavailable"}),
        ),
        _ => unreachable!("validated CodeNexus status"),
    };
    let provider = json!({
        "mode":mode,
        "providerId":native["providerId"],
        "implementationId":native["implementation"]["id"],
        "activation":native["activation"]
    });
    let provenance = json!({
        "sourceRevision":native["sourceRevision"],
        "observedAt":observed_at
    });
    let request = json!({
        "schema":"code-intel-evidence-admissibility-request.v1",
        "expectedSnapshotIdentity":native["expectedSnapshotIdentity"],
        "policy":{"evaluatedAt":evaluated_at,"maxAgeSeconds":max_age_seconds},
        "observation":{
            "schema":"code-intel-observed-evidence.v1",
            "provider":{
                "id":native["providerId"],
                "implementation":native["implementation"]
            },
            "source":{"revision":native["sourceRevision"]},
            "consumedSnapshotIdentity":native["sourceSnapshotIdentity"],
            "observedAt":observed_at,
            "completeness":completeness,
            "claimedComplete":completeness == "complete",
            "payload":native["payload"],
            "provenance":{
                "collectionId":format!("codenexus-{mode}-{}-{observed_at}", native["sourceRevision"].as_str().unwrap()),
                "command":if mode == "full" { "codenexus adapter:provider-artifact" } else { "codenexus adapter:lite-compatibility-artifact" },
                "startedAt":native["collectedAt"],
                "completedAt":observed_at
            },
            "failure":failure
        }
    });

    Ok(json!({
        "schema":"code-intel-codenexus-adapter-result.v1",
        "port":{
            "schema":"code-intel-codenexus-port.v1",
            "status":status,
            "completeness":completeness,
            "freshness":freshness,
            "expectedSnapshotIdentity":expected,
            "sourceSnapshotIdentity":consumed,
            "provider":provider,
            "provenance":provenance,
            "boundary":{
                "transport":"artifact_ref",
                "storageOwnership":"provider",
                "impactSemanticsOwnership":"provider"
            },
            "effects":native["effects"],
            "payload":native["payload"],
            "failureKind":failure_kind,
            "perceptionUsable":false
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
    exact(payload, &["schema", "data"], "CodeNexus evidence payload")?;
    if payload["schema"] != "code-intel-evidence-payload.v1" {
        return Err("CodeNexus evidence payload schema is invalid".to_string());
    }
    exact(&payload["data"], &["codenexus"], "CodeNexus payload data")?;
    let evidence = &payload["data"]["codenexus"];
    exact(
        evidence,
        &[
            "schema",
            "snapshotIdentity",
            "provider",
            "provenance",
            "completeness",
            "availability",
            "providerData",
        ],
        "CodeNexus port payload",
    )?;
    if evidence["schema"] != "code-intel-codenexus-evidence.v1"
        || evidence["snapshotIdentity"] != adapter["port"]["sourceSnapshotIdentity"]
        || evidence["completeness"] != adapter["port"]["completeness"]
    {
        return Err("CodeNexus payload snapshot/completeness mismatch".to_string());
    }
    exact(
        &evidence["provider"],
        &["mode", "providerId", "implementationId", "activation"],
        "CodeNexus payload provider",
    )?;
    if evidence["provider"] != adapter["port"]["provider"] {
        return Err("CodeNexus payload provider identity mismatch".to_string());
    }
    exact(
        &evidence["provenance"],
        &["sourceRevision", "observedAt"],
        "CodeNexus payload provenance",
    )?;
    if evidence["provenance"] != adapter["port"]["provenance"] {
        return Err("CodeNexus payload provenance mismatch".to_string());
    }
    let expected_availability = if adapter["port"]["status"] == "unavailable" {
        "provider_unavailable"
    } else {
        "available"
    };
    if evidence["availability"] != expected_availability {
        return Err("CodeNexus payload availability mismatch".to_string());
    }
    match adapter["port"]["status"].as_str().unwrap() {
        "unavailable" if !evidence["providerData"].is_null() => {
            Err("unavailable CodeNexus evidence must not fabricate provider data".to_string())
        }
        "current" if !evidence["providerData"].is_object() => {
            Err("current CodeNexus evidence requires opaque provider data".to_string())
        }
        "partial"
            if !(evidence["providerData"].is_null() || evidence["providerData"].is_object()) =>
        {
            Err("partial CodeNexus provider data is invalid".to_string())
        }
        _ => Ok(()),
    }
}

pub(crate) fn validate_adapter_result(adapter: &Value) -> Result<(), String> {
    exact(
        adapter,
        &["schema", "port", "evidence", "factPromotion"],
        "CodeNexus adapter result",
    )?;
    if adapter["schema"] != "code-intel-codenexus-adapter-result.v1" {
        return Err("CodeNexus adapter result schema is invalid".to_string());
    }
    let port = &adapter["port"];
    exact(
        port,
        &[
            "schema",
            "status",
            "completeness",
            "freshness",
            "expectedSnapshotIdentity",
            "sourceSnapshotIdentity",
            "provider",
            "provenance",
            "boundary",
            "effects",
            "payload",
            "failureKind",
            "perceptionUsable",
        ],
        "CodeNexus adapter port",
    )?;
    exact(
        &port["provider"],
        &["mode", "providerId", "implementationId", "activation"],
        "CodeNexus adapter provider",
    )?;
    exact(
        &port["provenance"],
        &["sourceRevision", "observedAt"],
        "CodeNexus adapter provenance",
    )?;
    exact(
        &port["boundary"],
        &["transport", "storageOwnership", "impactSemanticsOwnership"],
        "CodeNexus adapter boundary",
    )?;
    exact(
        &port["payload"],
        &[
            "schema",
            "artifactSchema",
            "type",
            "path",
            "sha256",
            "consumedSnapshotIdentity",
        ],
        "CodeNexus adapter payload ref",
    )?;
    exact(
        &adapter["evidence"],
        &["request"],
        "CodeNexus adapter evidence",
    )?;
    exact(
        &adapter["factPromotion"],
        &["eligible", "requires", "engineeringFacts"],
        "CodeNexus adapter fact promotion",
    )?;
    if adapter["factPromotion"]["eligible"] != false
        || adapter["factPromotion"]["requires"] != "evidence.admissibility-validate"
        || adapter["factPromotion"]["engineeringFacts"] != json!([])
    {
        return Err("CodeNexus adapter fact promotion contract is invalid".to_string());
    }
    Ok(())
}

fn validate_native(native: &Value) -> Result<(), String> {
    exact(
        native,
        &[
            "schema",
            "providerMode",
            "status",
            "providerId",
            "implementation",
            "sourceRevision",
            "expectedSnapshotIdentity",
            "sourceSnapshotIdentity",
            "collectedAt",
            "observedAt",
            "payload",
            "activation",
            "effects",
        ],
        "CodeNexus native result",
    )?;
    if native["schema"] != "code-intel-codenexus-native-result.v1"
        || !matches!(native["providerMode"].as_str(), Some("full" | "lite"))
        || !matches!(
            native["status"].as_str(),
            Some("current" | "partial" | "unavailable")
        )
        || !nonempty(&native["providerId"])
        || !nonempty(&native["sourceRevision"])
        || !digest(&native["expectedSnapshotIdentity"])
        || !digest(&native["sourceSnapshotIdentity"])
        || native["collectedAt"].as_u64().is_none()
        || native["observedAt"].as_u64().is_none()
        || native["observedAt"].as_u64().unwrap() < native["collectedAt"].as_u64().unwrap()
    {
        return Err("CodeNexus native identity/status/time is invalid".to_string());
    }
    exact(
        &native["implementation"],
        &["id", "version", "digest"],
        "CodeNexus implementation",
    )?;
    if !nonempty(&native["implementation"]["id"])
        || !nonempty(&native["implementation"]["version"])
        || !digest(&native["implementation"]["digest"])
    {
        return Err("CodeNexus implementation identity is invalid".to_string());
    }
    let mode = native["providerMode"].as_str().unwrap();
    let activation = native["activation"].as_str().unwrap_or("");
    let provider_id = native["providerId"].as_str().unwrap();
    let implementation_id = native["implementation"]["id"].as_str().unwrap();
    if mode == "full" && activation != "primary" {
        return Err("CodeNexus full provider must be primary".to_string());
    }
    if mode == "full"
        && (provider_id != "codenexus.full" || implementation_id == "invoke-codenexus-lite.ps1")
    {
        return Err("CodeNexus full provider identity is invalid".to_string());
    }
    if mode == "lite" && !matches!(activation, "explicit_fallback" | "legacy_rollback") {
        return Err("CodeNexus lite requires explicit fallback or legacy rollback".to_string());
    }
    if mode == "lite"
        && (provider_id != "codenexus.lite-compat"
            || implementation_id != "invoke-codenexus-lite.ps1")
    {
        return Err("CodeNexus lite compatibility identity is invalid".to_string());
    }
    validate_effects(&native["effects"], mode)?;
    crate::capability::validate_artifact_ref_shape(&native["payload"])?;
    if native["payload"]["artifactSchema"] != "code-intel-evidence-payload.v1"
        || native["payload"]["type"] != "observed.evidence.payload"
        || native["payload"]["consumedSnapshotIdentity"] != native["sourceSnapshotIdentity"]
    {
        return Err("CodeNexus payload contract/snapshot is invalid".to_string());
    }
    Ok(())
}

fn validate_effects(value: &Value, mode: &str) -> Result<(), String> {
    let values = value
        .as_array()
        .ok_or_else(|| "CodeNexus effects must be an array".to_string())?;
    let allowed = if mode == "full" {
        ["network_provider", "read_provider_artifact"].as_slice()
    } else {
        [
            "read_repository",
            "read_git_history",
            "read_sentrux_artifacts",
            "write_compatibility_artifact",
        ]
        .as_slice()
    };
    let mut seen = BTreeSet::new();
    for effect in values {
        let effect = effect
            .as_str()
            .ok_or_else(|| "CodeNexus effect is invalid".to_string())?;
        if !allowed.contains(&effect) || !seen.insert(effect) {
            return Err("CodeNexus effects are invalid for provider mode".to_string());
        }
    }
    if values.is_empty() {
        return Err("CodeNexus effects must not be empty".to_string());
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
