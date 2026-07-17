use super::{
    load_json, rejected, status_route, validate_artifact_ref, validate_native, Context,
    EvidenceError,
};
use serde_json::{json, Value};
use std::collections::HashSet;

const SCHEMA: &str = "code-intel-react-doctor-native-result.v1";
const VERSION: &str = "0.7.8";
const INTEGRITY: &str =
    "sha512-G3spmtZJE/gWWPRJ3rpgUWTPRDJpEmdRja7iNZ7RAXlfpEO+NWVzPTca/cPI9hLwPo2Aq5/BZggo5JDBrwGrlA==";

pub(super) fn adapt(request: &Value, context: &Context) -> Value {
    let (snapshot, status) = match validate_native(request, SCHEMA, context) {
        Ok(value) => value,
        Err(error) => {
            return rejected(
                "react-doctor",
                super::snapshot_identity(request),
                error.category,
                &error.reason,
                context.evaluated_at,
            );
        }
    };
    if let Err(error) = validate_identity(request) {
        return rejected(
            "react-doctor",
            Some(snapshot),
            error.category,
            &error.reason,
            context.evaluated_at,
        );
    }
    match status {
        "provider_unavailable" => {
            return status_route(
                "react-doctor",
                Some(snapshot),
                "unknown",
                "unknown",
                Some("provider_unavailable"),
                "React Doctor or npm was unavailable",
                context,
                Vec::new(),
                Value::Null,
            );
        }
        "local_tool_error" => {
            return rejected(
                "react-doctor",
                Some(snapshot),
                "local_tool_error",
                "React Doctor exited unsuccessfully",
                context.evaluated_at,
            );
        }
        "completed" => {}
        _ => {
            return rejected(
                "react-doctor",
                Some(snapshot),
                "local_tool_error",
                "invalid React Doctor native status",
                context.evaluated_at,
            );
        }
    }

    let reference = match request.get("report") {
        Some(value) => value,
        None => {
            return rejected(
                "react-doctor",
                Some(snapshot),
                "local_tool_error",
                "completed React Doctor result has no report",
                context.evaluated_at,
            );
        }
    };
    let report = match validate_artifact_ref(reference, snapshot, context)
        .and_then(|path| load_json(&path))
    {
        Ok(value) => value,
        Err(error) => {
            return rejected(
                "react-doctor",
                Some(snapshot),
                error.category,
                &error.reason,
                context.evaluated_at,
            );
        }
    };
    match analyze_report(&report) {
        Ok(Analysis::NotApplicable) => status_route(
            "react-doctor",
            Some(snapshot),
            "not_applicable",
            "not_applicable",
            None,
            "React runtime was not detected",
            context,
            vec![reference.clone()],
            compact_evidence(&report, Vec::new()),
        ),
        Ok(Analysis::Observed {
            complete,
            diagnostics,
            coverage,
        }) => {
            let (state, verdict, reason) = if !complete {
                (
                    "unknown",
                    "unknown",
                    "partial React Doctor coverage admitted as unknown",
                )
            } else if diagnostics.is_empty() {
                (
                    "observed",
                    "pass",
                    "complete React Doctor scan with no diagnostics",
                )
            } else {
                (
                    "observed",
                    "fail",
                    "complete React Doctor scan reported diagnostics",
                )
            };
            status_route(
                "react-doctor",
                Some(snapshot),
                state,
                verdict,
                None,
                reason,
                context,
                vec![reference.clone()],
                compact_evidence(&report, coverage),
            )
        }
        Err(error) => rejected(
            "react-doctor",
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
        .ok_or_else(|| EvidenceError::local("missing React Doctor tool identity"))?;
    let command = tool
        .get("command")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::local("missing React Doctor command provenance"))?;
    let arguments: Vec<&str> = command.iter().filter_map(Value::as_str).collect();
    let expected = [
        "npx",
        "--yes",
        "react-doctor@0.7.8",
        "--json",
        "--no-telemetry",
    ];
    if tool.get("version").and_then(Value::as_str) != Some(VERSION)
        || tool.get("integrity").and_then(Value::as_str) != Some(INTEGRITY)
        || arguments != expected
    {
        return Err(EvidenceError::local(
            "React Doctor supply-chain identity or execution flags mismatch",
        ));
    }
    Ok(())
}

enum Analysis {
    NotApplicable,
    Observed {
        complete: bool,
        diagnostics: Vec<Value>,
        coverage: Vec<Value>,
    },
}

fn analyze_report(report: &Value) -> Result<Analysis, EvidenceError> {
    if report.get("schemaVersion").and_then(Value::as_i64) != Some(3) {
        return Err(EvidenceError::local(
            "unsupported React Doctor JSON schema; expected v3",
        ));
    }
    if is_not_applicable(report) {
        return Ok(Analysis::NotApplicable);
    }
    if report.get("ok").and_then(Value::as_bool) != Some(true)
        || !report.get("error").is_some_and(Value::is_null)
    {
        return Err(EvidenceError::local(
            "React Doctor report contains an execution error",
        ));
    }
    let mode = report
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| EvidenceError::local("React Doctor report lacks mode"))?;
    if !["full", "diff", "staged", "baseline"].contains(&mode) {
        return Err(EvidenceError::local("invalid React Doctor report mode"));
    }
    let projects = report
        .get("projects")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::local("React Doctor report lacks projects"))?;
    let diagnostics = report
        .get("diagnostics")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::local("React Doctor report lacks diagnostics"))?;

    let mut complete = true;
    let mut coverage = Vec::new();
    let mut project_ids = Vec::new();
    for project in projects {
        let analyzed_files = string_array(project, "analyzedFiles")?;
        let skipped_checks = string_array(project, "skippedChecks")?;
        let analyzed_count = project
            .get("analyzedFileCount")
            .and_then(Value::as_u64)
            .ok_or_else(|| EvidenceError::local("project lacks analyzedFileCount"))?;
        let project_complete = project
            .get("complete")
            .and_then(Value::as_bool)
            .ok_or_else(|| EvidenceError::local("project lacks complete"))?;
        if analyzed_count != analyzed_files.len() as u64 {
            return Err(EvidenceError::local(
                "React Doctor analyzedFileCount does not match analyzedFiles",
            ));
        }
        validate_paths(&analyzed_files)?;
        let project_diagnostics = project
            .get("diagnostics")
            .and_then(Value::as_array)
            .ok_or_else(|| EvidenceError::local("project lacks diagnostics"))?;
        for diagnostic in project_diagnostics {
            project_ids.push(validate_diagnostic(diagnostic)?);
        }
        complete &= project_complete && skipped_checks.is_empty();
        coverage.push(json!({
            "directory": required_string(project, "directory")?,
            "packageRoot": required_string(project, "packageRoot")?,
            "framework": required_string(project, "framework")?,
            "analyzedFiles": analyzed_files,
            "analyzedFileCount": analyzed_count,
            "complete": project_complete,
            "skippedChecks": skipped_checks,
            "skippedCheckReasons": project.get("skippedCheckReasons").cloned().unwrap_or(Value::Null)
        }));
    }

    let mut top_ids = Vec::new();
    for diagnostic in diagnostics {
        top_ids.push(validate_diagnostic(diagnostic)?);
    }
    if top_ids != project_ids || top_ids.iter().collect::<HashSet<_>>().len() != top_ids.len() {
        return Err(EvidenceError::local(
            "React Doctor flattened diagnostics do not match project diagnostics",
        ));
    }
    Ok(Analysis::Observed {
        complete,
        diagnostics: diagnostics.clone(),
        coverage,
    })
}

fn is_not_applicable(report: &Value) -> bool {
    report.get("reactDetected").and_then(Value::as_bool) == Some(false)
        || (report.get("ok").and_then(Value::as_bool) == Some(false)
            && report.pointer("/error/name").and_then(Value::as_str)
                == Some("ProjectNotFoundError"))
}

fn validate_diagnostic(value: &Value) -> Result<String, EvidenceError> {
    for key in [
        "id",
        "normalizedFilePath",
        "filePath",
        "plugin",
        "rule",
        "category",
        "severity",
    ] {
        required_string(value, key)?;
    }
    let normalized = required_string(value, "normalizedFilePath")?;
    validate_paths(&[normalized.clone()])?;
    if normalized.contains('\\') {
        return Err(EvidenceError::local(
            "diagnostic normalizedFilePath uses backslashes",
        ));
    }
    let severity = required_string(value, "severity")?;
    if severity != "error" && severity != "warning" {
        return Err(EvidenceError::local("invalid diagnostic severity"));
    }
    string_array(value, "tags")?;
    for key in ["line", "column"] {
        if value.get(key).and_then(Value::as_u64).is_none() {
            return Err(EvidenceError::local(format!(
                "diagnostic lacks numeric {key}"
            )));
        }
    }
    Ok(required_string(value, "id")?)
}

fn compact_evidence(report: &Value, coverage: Vec<Value>) -> Value {
    json!({
        "schemaVersion": 3,
        "version": report.get("version").cloned().unwrap_or(Value::Null),
        "mode": report.get("mode").cloned().unwrap_or(Value::Null),
        "reactDetected": report.get("reactDetected").cloned().unwrap_or(Value::Null),
        "error": report.get("error").cloned().unwrap_or(Value::Null),
        "coverage": coverage,
        "diagnostics": report.get("diagnostics").cloned().unwrap_or_else(|| json!([])),
        "summary": report.get("summary").cloned().unwrap_or(Value::Null)
    })
}

fn required_string(value: &Value, key: &str) -> Result<String, EvidenceError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| EvidenceError::local(format!("missing {key}")))
}

fn string_array(value: &Value, key: &str) -> Result<Vec<String>, EvidenceError> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::local(format!("missing {key}")))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToString::to_string)
                .ok_or_else(|| EvidenceError::local(format!("{key} must contain strings")))
        })
        .collect()
}

fn validate_paths(paths: &[String]) -> Result<(), EvidenceError> {
    if paths.iter().any(|path| {
        let path = std::path::Path::new(path);
        path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
    }) {
        return Err(EvidenceError::local(
            "React Doctor report contains an unsafe path",
        ));
    }
    Ok(())
}
