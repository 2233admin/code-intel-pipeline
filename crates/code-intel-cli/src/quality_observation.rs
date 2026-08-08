use std::collections::BTreeSet;

use serde_json::{json, Value};

const REPORT_SCHEMA: &str = "code-intel-quality-gate-report.v1";
const OBSERVATION_SCHEMA: &str = "code-intel-runtime-ci-observation.v1";

pub(crate) fn generate(report: &Value) -> Result<Value, String> {
    validate_report(report)?;

    let gates = report["gates"].as_array().expect("validated");
    let statuses = gates
        .iter()
        .map(|gate| gate["status"].as_str().expect("validated"))
        .collect::<Vec<_>>();
    let quality_status = if statuses.iter().any(|status| *status == "failed") {
        "failed"
    } else if statuses.iter().any(|status| *status == "cancelled") {
        "cancelled"
    } else if statuses.iter().all(|status| *status == "passed") {
        "passed"
    } else {
        "unknown"
    };
    let observed = quality_status != "unknown";
    let summary = gates
        .iter()
        .map(|gate| {
            format!(
                "{}={}",
                gate["id"].as_str().expect("validated"),
                gate["status"].as_str().expect("validated")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    Ok(json!({
        "schema": OBSERVATION_SCHEMA,
        "provider": report["provider"],
        "provenance": report["provenance"],
        "snapshotIdentity": report["snapshotIdentity"],
        "observedAt": report["observedAt"],
        "completeness": "partial",
        "signals": {
            "tests": {"status": "unknown", "observed": false, "summary": "not supplied by quality collector"},
            "build": {"status": "unknown", "observed": false, "summary": "not supplied by quality collector"},
            "runtime": {"status": "unknown", "observed": false, "summary": "not supplied by quality collector"},
            "quality": {
                "status": quality_status,
                "observed": observed,
                "summary": summary
            }
        }
    }))
}

fn validate_report(report: &Value) -> Result<(), String> {
    require_object_keys(
        report,
        &[
            "schema",
            "provider",
            "provenance",
            "snapshotIdentity",
            "observedAt",
            "gates",
        ],
        "quality gate report",
    )?;
    if report["schema"].as_str() != Some(REPORT_SCHEMA) {
        return Err(format!(
            "quality gate report schema must be {REPORT_SCHEMA}"
        ));
    }
    digest(
        &report["snapshotIdentity"],
        "quality gate report.snapshotIdentity",
    )?;
    if !report["observedAt"].is_u64() {
        return Err("quality gate report.observedAt must be a non-negative integer".into());
    }
    validate_provider(&report["provider"])?;
    validate_provenance(&report["provenance"])?;

    let gates = report["gates"]
        .as_array()
        .ok_or_else(|| "quality gate report.gates must be an array".to_string())?;
    if gates.is_empty() {
        return Err("quality gate report.gates must not be empty".into());
    }
    for (index, gate) in gates.iter().enumerate() {
        let context = format!("quality gate report.gates[{index}]");
        require_object_keys(gate, &["id", "status", "observed", "summary"], &context)?;
        nonempty_string(&gate["id"], &format!("{context}.id"))?;
        let status = gate["status"]
            .as_str()
            .ok_or_else(|| format!("{context}.status must be a string"))?;
        if !matches!(status, "passed" | "failed" | "cancelled" | "unknown") {
            return Err(format!("{context}.status is invalid"));
        }
        let observed = gate["observed"]
            .as_bool()
            .ok_or_else(|| format!("{context}.observed must be a boolean"))?;
        if !observed && status != "unknown" {
            return Err(format!(
                "{context} cannot claim {status} when observed is false"
            ));
        }
        nonempty_string(&gate["summary"], &format!("{context}.summary"))?;
    }
    Ok(())
}

fn validate_provider(value: &Value) -> Result<(), String> {
    require_object_keys(
        value,
        &["id", "runId", "sourceRevision"],
        "quality gate report.provider",
    )?;
    for field in ["id", "runId", "sourceRevision"] {
        nonempty_string(
            &value[field],
            &format!("quality gate report.provider.{field}"),
        )?;
    }
    Ok(())
}

fn validate_provenance(value: &Value) -> Result<(), String> {
    require_object_keys(
        value,
        &[
            "collectorId",
            "collectorVersion",
            "collectionId",
            "collectedAt",
        ],
        "quality gate report.provenance",
    )?;
    for field in ["collectorId", "collectorVersion", "collectionId"] {
        nonempty_string(
            &value[field],
            &format!("quality gate report.provenance.{field}"),
        )?;
    }
    if !value["collectedAt"].is_u64() {
        return Err(
            "quality gate report.provenance.collectedAt must be a non-negative integer".into(),
        );
    }
    Ok(())
}

fn digest(value: &Value, context: &str) -> Result<(), String> {
    let value = value
        .as_str()
        .ok_or_else(|| format!("{context} must be a SHA-256 digest"))?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("{context} must be a lowercase SHA-256 digest"));
    }
    Ok(())
}

fn nonempty_string(value: &Value, context: &str) -> Result<(), String> {
    if !value.is_string() || value.as_str().is_some_and(str::is_empty) {
        return Err(format!("{context} must be a non-empty string"));
    }
    Ok(())
}

fn require_object_keys(value: &Value, expected: &[&str], context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("{context} fields are invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::generate;
    use serde_json::{json, Value};

    fn report(gates: Value) -> Value {
        json!({
            "schema": "code-intel-quality-gate-report.v1",
            "provider": {
                "id": "tdxcli-quality",
                "runId": "run-1",
                "sourceRevision": "abc123"
            },
            "provenance": {
                "collectorId": "tdxcli-quality-gate",
                "collectorVersion": "1",
                "collectionId": "collection-1",
                "collectedAt": 2000
            },
            "snapshotIdentity": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "observedAt": 2000,
            "gates": gates
        })
    }

    fn gate(id: &str, status: &str, observed: bool) -> Value {
        json!({
            "id": id,
            "status": status,
            "observed": observed,
            "summary": format!("{id} {status}")
        })
    }

    #[test]
    fn passed_gates_generate_a_quality_observation_without_faking_other_domains() {
        let output = generate(&report(json!([
            gate("tests", "passed", true),
            gate("clippy", "passed", true),
            gate("audit", "passed", true)
        ])))
        .unwrap();

        assert_eq!(output["signals"]["quality"]["status"], "passed");
        assert_eq!(output["signals"]["quality"]["observed"], true);
        assert_eq!(output["signals"]["tests"]["status"], "unknown");
        assert_eq!(output["completeness"], "partial");
    }

    #[test]
    fn any_failed_gate_is_failed_and_cancelled_is_preserved_when_no_failure_exists() {
        let failed = generate(&report(json!([
            gate("tests", "passed", true),
            gate("clippy", "failed", true),
            gate("audit", "cancelled", true)
        ])))
        .unwrap();
        assert_eq!(failed["signals"]["quality"]["status"], "failed");

        let cancelled = generate(&report(json!([
            gate("tests", "passed", true),
            gate("clippy", "cancelled", true)
        ])))
        .unwrap();
        assert_eq!(cancelled["signals"]["quality"]["status"], "cancelled");
    }

    #[test]
    fn malformed_or_unobserved_positive_gate_is_rejected() {
        let mut malformed = report(json!([gate("tests", "passed", false)]));
        assert!(generate(&malformed).is_err());
        malformed["gates"][0]["status"] = json!("unknown");
        assert!(generate(&malformed).is_ok());
    }
}
