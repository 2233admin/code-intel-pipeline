use super::{
    load_json, rejected, status_route, validate_artifact_ref, validate_native, Context,
    EvidenceError,
};
use serde_json::{json, Map, Value};

const SCHEMA: &str = "code-intel-compete-native-result.v1";
const REVISION: &str = "ec13028fc8da620c73a114ffe403a772b29a78cb";
const DATASETS: &[(&str, &str)] = &[
    ("product", "identity"),
    ("competitors", "competitors"),
    ("companies", "companies"),
    ("pricing", "pricing"),
    ("techstack", "techstack"),
    ("social", "presence"),
    ("marketing", "marketing"),
    ("seo", "seo"),
    ("features", "features"),
];

pub(super) fn adapt(request: &Value, context: &Context) -> Value {
    let (snapshot, status) = match validate_request(request, context) {
        Ok(value) => value,
        Err(error) => {
            return rejected(
                "compete",
                super::snapshot_identity(request),
                error.category,
                &error.reason,
                context.evaluated_at,
            );
        }
    };
    if let Some(route) = non_completed_route(snapshot, status, context) {
        return route;
    }

    collected_route(collect(request, snapshot, context), snapshot, context)
}

fn validate_request<'a>(
    request: &'a Value,
    context: &Context,
) -> Result<(&'a str, &'a str), EvidenceError> {
    let native = validate_native(request, SCHEMA, context)?;
    validate_identity(request)?;
    Ok(native)
}

fn non_completed_route(snapshot: &str, status: &str, context: &Context) -> Option<Value> {
    match status {
        "completed" => None,
        "provider_unavailable" => Some(status_route(
            "compete",
            Some(snapshot),
            "unknown",
            "unknown",
            Some("provider_unavailable"),
            "Compete Agent/web provider was unavailable",
            context,
            Vec::new(),
            Value::Null,
        )),
        "not_run" => Some(status_route(
            "compete",
            Some(snapshot),
            "unknown",
            "unknown",
            None,
            "Compete was not run",
            context,
            Vec::new(),
            Value::Null,
        )),
        "local_tool_error" => Some(rejected(
            "compete",
            Some(snapshot),
            "local_tool_error",
            "Compete upstream execution failed",
            context.evaluated_at,
        )),
        _ => Some(rejected(
            "compete",
            Some(snapshot),
            "local_tool_error",
            "invalid Compete native status",
            context.evaluated_at,
        )),
    }
}

type Collected = Result<(Vec<Value>, Vec<String>, Value), EvidenceError>;

fn collected_route(result: Collected, snapshot: &str, context: &Context) -> Value {
    match result {
        Ok((artifacts, missing, report)) if missing.is_empty() => status_route(
            "compete",
            Some(snapshot),
            "observed",
            "unknown",
            None,
            "complete schema-marked InsightKit evidence admitted as advisory",
            context,
            artifacts,
            json!({"complete": true, "missing": [], "report": report}),
        ),
        Ok((artifacts, missing, report)) => status_route(
            "compete",
            Some(snapshot),
            "unknown",
            "unknown",
            None,
            "partial Compete output cannot become observed evidence",
            context,
            artifacts,
            json!({"complete": false, "missing": missing, "report": report}),
        ),
        Err(error) => rejected(
            "compete",
            Some(snapshot),
            error.category,
            &error.reason,
            context.evaluated_at,
        ),
    }
}

fn validate_identity(request: &Value) -> Result<(), EvidenceError> {
    let tool = request
        .get("tool")
        .and_then(Value::as_object)
        .ok_or_else(|| EvidenceError::local("missing Compete tool identity"))?;
    if tool.get("revision").and_then(Value::as_str) != Some(REVISION)
        || tool.get("license").and_then(Value::as_str) != Some("MIT")
    {
        return Err(EvidenceError::local(
            "Compete supply-chain identity mismatch",
        ));
    }
    Ok(())
}

fn collect(
    request: &Value,
    snapshot: &str,
    context: &Context,
) -> Result<(Vec<Value>, Vec<String>, Value), EvidenceError> {
    let native_artifacts = request.get("artifacts").and_then(Value::as_object);
    let datasets = native_artifacts
        .and_then(|value| value.get("datasets"))
        .and_then(Value::as_object);
    let mut admitted = Vec::new();
    let mut missing = Vec::new();
    let mut report = Value::Null;

    for (name, top_level_key) in DATASETS {
        let Some(reference) = datasets.and_then(|value| value.get(*name)) else {
            missing.push(format!("{name}.json"));
            continue;
        };
        let path = validate_artifact_ref(reference, snapshot, context)?;
        validate_dataset(&load_json(&path)?, name, top_level_key)?;
        admitted.push(reference.clone());
    }

    collect_report(
        native_artifacts,
        "reportJson",
        "report.json",
        snapshot,
        context,
        &mut admitted,
        &mut missing,
        &mut report,
        true,
    )?;
    collect_report(
        native_artifacts,
        "reportHtml",
        "report.html",
        snapshot,
        context,
        &mut admitted,
        &mut missing,
        &mut report,
        false,
    )?;

    let provenance = native_artifacts
        .and_then(|value| value.get("provenance"))
        .and_then(Value::as_array);
    if provenance.is_none_or(Vec::is_empty) {
        missing.push("provenance".to_string());
    } else {
        for reference in provenance.unwrap() {
            validate_artifact_ref(reference, snapshot, context)?;
            admitted.push(reference.clone());
        }
    }
    Ok((admitted, missing, report))
}

#[allow(clippy::too_many_arguments)]
fn collect_report(
    artifacts: Option<&Map<String, Value>>,
    key: &str,
    label: &str,
    snapshot: &str,
    context: &Context,
    admitted: &mut Vec<Value>,
    missing: &mut Vec<String>,
    report: &mut Value,
    json_report: bool,
) -> Result<(), EvidenceError> {
    let Some(reference) = artifacts.and_then(|value| value.get(key)) else {
        missing.push(label.to_string());
        return Ok(());
    };
    let path = validate_artifact_ref(reference, snapshot, context)?;
    if json_report {
        let value = load_json(&path)?;
        validate_dataset(&value, "report", "competitor_analysis")?;
        if !value.get("executive_summary").is_some_and(Value::is_object) {
            return Err(EvidenceError::local(
                "Compete report.json lacks executive_summary",
            ));
        }
        *report = value;
    } else {
        let html = std::fs::read_to_string(&path)
            .map_err(|error| EvidenceError::local(format!("cannot read report.html: {error}")))?;
        if !html.to_ascii_lowercase().contains("<html") {
            return Err(EvidenceError::local(
                "Compete report.html is not an HTML document",
            ));
        }
    }
    admitted.push(reference.clone());
    Ok(())
}

fn validate_dataset(
    value: &Value,
    dataset: &str,
    top_level_key: &str,
) -> Result<(), EvidenceError> {
    if value.pointer("/meta/dataset").and_then(Value::as_str) != Some(dataset)
        || !value
            .get(top_level_key)
            .is_some_and(|value| value.is_array() || value.is_object())
    {
        return Err(EvidenceError::local(format!(
            "Compete {dataset}.json does not match its upstream schema marker"
        )));
    }
    Ok(())
}
