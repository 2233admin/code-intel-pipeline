use std::collections::BTreeSet;

use serde_json::{json, Value};

pub(crate) fn translate(
    native: &Value,
    evaluated_at: u64,
    max_age_seconds: u64,
) -> Result<Value, String> {
    validate_native(native)?;
    if max_age_seconds == 0 {
        return Err("graph freshness policy max age must be positive".to_string());
    }

    let mode = native["providerMode"].as_str().unwrap();
    let status = native["status"].as_str().unwrap();
    let observed_at = native["observedAt"].as_u64().unwrap();
    let expected = native["expectedSnapshotIdentity"].as_str().unwrap();
    let consumed = native["sourceSnapshotIdentity"].as_str().unwrap();
    let snapshot_current = expected == consumed;
    let time_current = observed_at <= evaluated_at && evaluated_at - observed_at <= max_age_seconds;
    let freshness = if !snapshot_current {
        "snapshot_mismatch"
    } else if time_current {
        "current"
    } else {
        "stale"
    };
    let completeness = if status == "current" {
        "complete"
    } else {
        "partial"
    };
    let failure = match status {
        "current" => json!({"kind":"none"}),
        "partial" => json!({"kind":"domain_unknown","message":"architecture graph is partial"}),
        "missing" => {
            json!({"kind":"provider_unavailable","message":"architecture graph is missing"})
        }
        _ => unreachable!("validated graph status"),
    };
    let fallback_identity = native["fallback"]
        .as_object()
        .map(|fallback| fallback["identity"].clone());
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
                "id":if mode == "internal" { "architecture-graph.internal" } else { "architecture-graph.external-fallback" },
                "implementation":native["implementation"]
            },
            "source":{"revision":native["sourceRevision"]},
            "consumedSnapshotIdentity":native["sourceSnapshotIdentity"],
            "observedAt":observed_at,
            "completeness":completeness,
            "claimedComplete":completeness == "complete",
            "payload":native["payload"],
            "provenance":{
                "collectionId":format!("graph-{mode}-{}-{observed_at}", native["sourceRevision"].as_str().unwrap()),
                "command":if mode == "internal" { "code-intel graph adapter:internal" } else { "code-intel graph adapter:explicit-fallback" },
                "startedAt":native["collectedAt"],
                "completedAt":observed_at
            },
            "failure":failure
        }
    });

    Ok(json!({
        "schema":"code-intel-graph-adapter-result.v1",
        "port":{
            "schema":"code-intel-architecture-graph-port.v1",
            "status":status,
            "completeness":completeness,
            "freshness":freshness,
            "expectedSnapshotIdentity":expected,
            "sourceSnapshotIdentity":consumed,
            "provider":{
                "mode":mode,
                "implementationId":native["implementation"]["id"],
                "fallbackIdentity":fallback_identity
            },
            "provenance":provenance,
            "payload":native["payload"],
            "anatomyUsable":false
        },
        "evidence":{"request":request},
        "factPromotion":{"eligible":false,"requires":"evidence.admissibility-validate","engineeringFacts":[]}
    }))
}

pub(crate) fn validate_admitted_payload(payload: &Value, adapter: &Value) -> Result<(), String> {
    exact(payload, &["schema", "data"], "graph evidence payload")?;
    if payload["schema"] != "code-intel-evidence-payload.v1" {
        return Err("graph evidence payload schema is invalid".to_string());
    }
    let graph = &payload["data"]["architectureGraph"];
    exact(
        graph,
        &[
            "schema",
            "snapshotIdentity",
            "provider",
            "provenance",
            "completeness",
            "graph",
        ],
        "architecture graph port payload",
    )?;
    if graph["schema"] != "code-intel-architecture-graph-evidence.v1"
        || graph["snapshotIdentity"] != adapter["port"]["sourceSnapshotIdentity"]
        || graph["completeness"] != adapter["port"]["completeness"]
    {
        return Err("architecture graph port snapshot/completeness mismatch".to_string());
    }
    exact(
        &graph["provider"],
        &["mode", "implementationId", "fallbackIdentity"],
        "architecture graph provider",
    )?;
    if graph["provider"] != adapter["port"]["provider"] {
        return Err("architecture graph provider/fallback identity mismatch".to_string());
    }
    exact(
        &graph["provenance"],
        &["sourceRevision", "observedAt"],
        "architecture graph provenance",
    )?;
    if graph["provenance"]["sourceRevision"] != adapter["port"]["provenance"]["sourceRevision"]
        || graph["provenance"]["observedAt"] != adapter["port"]["provenance"]["observedAt"]
    {
        return Err("architecture graph provenance mismatch".to_string());
    }
    match adapter["port"]["status"].as_str().unwrap() {
        "missing" if !graph["graph"].is_null() => {
            Err("missing architecture graph must carry null graph data".to_string())
        }
        "current" if !graph_document(&graph["graph"]) => {
            Err("current architecture graph data is invalid".to_string())
        }
        "partial" if !(graph["graph"].is_null() || graph_document(&graph["graph"])) => {
            Err("partial architecture graph data is invalid".to_string())
        }
        _ => Ok(()),
    }
}

fn graph_document(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let required = ["schema", "summary", "nodes", "edges", "symbols"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    required.is_subset(&actual)
        && value["schema"] == "code-intel-understand-graph.v1"
        && value["summary"].is_object()
        && value["nodes"].is_array()
        && value["edges"].is_array()
        && value["symbols"].is_array()
}

fn validate_native(native: &Value) -> Result<(), String> {
    exact(
        native,
        &[
            "schema",
            "providerMode",
            "status",
            "implementation",
            "sourceRevision",
            "expectedSnapshotIdentity",
            "sourceSnapshotIdentity",
            "collectedAt",
            "observedAt",
            "payload",
            "fallback",
        ],
        "graph provider native result",
    )?;
    if native["schema"] != "code-intel-graph-provider-native.v1"
        || !matches!(
            native["providerMode"].as_str(),
            Some("internal" | "external")
        )
        || !matches!(
            native["status"].as_str(),
            Some("current" | "partial" | "missing")
        )
        || !digest(&native["expectedSnapshotIdentity"])
        || !digest(&native["sourceSnapshotIdentity"])
        || native["collectedAt"].as_u64().is_none()
        || native["observedAt"].as_u64().is_none()
        || native["observedAt"].as_u64().unwrap() < native["collectedAt"].as_u64().unwrap()
    {
        return Err("graph provider native identity/status/time is invalid".to_string());
    }
    exact(
        &native["implementation"],
        &["id", "version", "digest"],
        "graph provider implementation",
    )?;
    if !nonempty(&native["implementation"]["id"])
        || !nonempty(&native["implementation"]["version"])
        || !digest(&native["implementation"]["digest"])
        || !nonempty(&native["sourceRevision"])
    {
        return Err("graph provider implementation/source is invalid".to_string());
    }
    crate::capability::validate_artifact_ref_shape(&native["payload"])?;
    if native["payload"]["artifactSchema"] != "code-intel-evidence-payload.v1"
        || native["payload"]["type"] != "observed.evidence.payload"
        || native["payload"]["consumedSnapshotIdentity"] != native["sourceSnapshotIdentity"]
    {
        return Err("graph provider payload contract/snapshot is invalid".to_string());
    }
    if native["providerMode"] == "internal" {
        if !native["fallback"].is_null() {
            return Err("internal graph provider cannot declare fallback identity".to_string());
        }
    } else {
        exact(
            &native["fallback"],
            &["identity", "activation", "reason"],
            "external graph fallback",
        )?;
        if !nonempty(&native["fallback"]["identity"])
            || !nonempty(&native["fallback"]["reason"])
            || !matches!(
                native["fallback"]["activation"].as_str(),
                Some("explicit_fallback" | "legacy_rollback")
            )
        {
            return Err("external graph requires explicit fallback/rollback identity".to_string());
        }
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
