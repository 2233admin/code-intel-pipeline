use crate::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const REPORT_SCHEMA: &str = "code-intel-beta-feature-report.v1";

pub(crate) struct Options<'a> {
    pub(crate) action: &'a str,
    pub(crate) feature: Option<&'a str>,
    pub(crate) request: Option<&'a Path>,
    pub(crate) artifact_root: Option<&'a Path>,
    pub(crate) json: bool,
}

pub(crate) fn run(options: &Options<'_>) -> Result<()> {
    match options.action.to_ascii_lowercase().as_str() {
        "list" => print_output(&list(), options.json),
        "build" => {
            let feature = options
                .feature
                .ok_or("feature build requires --feature <name>")?;
            let request = options
                .request
                .ok_or("feature build requires --request <route-result.json>")?;
            let artifact_root = options
                .artifact_root
                .ok_or("feature build requires --artifact-root <dir>")?;
            let route = read_json(request)?;
            let report = build(feature, &route)?;
            let artifacts = write_report(feature, artifact_root, &report)?;
            print_output(
                &json!({
                    "ok": true,
                    "feature": feature,
                    "beta": true,
                    "artifacts": artifacts,
                    "report": report
                }),
                options.json,
            )
        }
        other => Err(format!("unsupported feature action: {other}").into()),
    }
}

pub(crate) fn list() -> Value {
    json!({
        "schema": "code-intel-beta-feature-api.v1",
        "beta": true,
        "features": [
            {
                "id": "competitive-intelligence",
                "provider": "compete",
                "purpose": "Summarize competitive gaps and improvement recommendations without scoring",
                "required": false
            },
            {
                "id": "react-diagnostics",
                "provider": "react-doctor",
                "purpose": "Summarize React project problems and improvement recommendations without scoring",
                "required": false
            }
        ]
    })
}

pub(crate) fn build(feature: &str, route: &Value) -> Result<Value> {
    validate_route(feature, route)?;
    Ok(match feature {
        "competitive-intelligence" => build_competitive_intelligence(route),
        "react-diagnostics" => build_react_diagnostics(route),
        _ => return Err(format!("unknown beta feature: {feature}").into()),
    })
}

fn validate_route(feature: &str, route: &Value) -> Result<()> {
    let expected_provider = match feature {
        "competitive-intelligence" => "compete",
        "react-diagnostics" => "react-doctor",
        _ => return Err(format!("unknown beta feature: {feature}").into()),
    };
    if route.get("schema").and_then(Value::as_str) != Some("code-intel-evidence-route-result.v1") {
        return Err("feature input must be a code-intel-evidence-route-result.v1".into());
    }
    if route.get("provider").and_then(Value::as_str) != Some(expected_provider) {
        return Err(
            format!("feature {feature} requires a {expected_provider} route result").into(),
        );
    }
    Ok(())
}

fn base_report(feature: &str, route: &Value, summary: Value) -> Value {
    json!({
        "schema": REPORT_SCHEMA,
        "feature": feature,
        "beta": true,
        "snapshotIdentity": route.get("snapshotIdentity").cloned().unwrap_or(Value::Null),
        "status": route.get("status").cloned().unwrap_or_else(|| json!("unknown")),
        "summary": summary,
        "issues": [],
        "recommendations": [],
        "coverage": Value::Null,
        "source": {
            "provider": route.get("provider").cloned().unwrap_or(Value::Null),
            "advisoryOnly": true,
            "admissibility": route.get("admissibility").cloned().unwrap_or(Value::Null),
            "failureCategory": route.get("failureCategory").cloned().unwrap_or(Value::Null),
            "evaluatedAt": route.get("evaluatedAt").cloned().unwrap_or(Value::Null)
        }
    })
}

fn build_competitive_intelligence(route: &Value) -> Value {
    let report = route.pointer("/evidence/report").unwrap_or(&Value::Null);
    let summary = text_value(report.pointer("/executive_summary/summary"))
        .map(Value::String)
        .unwrap_or_else(|| route_reason(route));
    let mut out = base_report("competitive-intelligence", route, summary);
    if route.get("status").and_then(Value::as_str) != Some("observed") {
        return out;
    }

    let mut issues = Vec::new();
    if let Some(findings) = string_array_value(report.pointer("/executive_summary/key_findings")) {
        for (index, finding) in findings.into_iter().enumerate() {
            issues.push(json!({
                "id": format!("compete-finding-{}", index + 1),
                "kind": "finding",
                "title": finding,
                "description": Value::Null,
                "severity": "advisory",
                "category": "competitive-intelligence",
                "location": Value::Null,
                "sourceEvidence": Value::Null
            }));
        }
    }
    if let Some(gaps) = report.get("opportunity_gaps").and_then(Value::as_array) {
        for (index, gap) in gaps.iter().enumerate() {
            issues.push(json!({
                "id": format!("compete-gap-{}", index + 1),
                "kind": "opportunity-gap",
                "title": gap.get("title").and_then(Value::as_str).unwrap_or("Competitive opportunity gap"),
                "description": text_value(gap.get("description")),
                "severity": enum_value(gap.get("impact")).unwrap_or("advisory"),
                "category": "competitive-intelligence",
                "location": Value::Null,
                "sourceEvidence": gap
            }));
        }
    }

    let mut recommendations = Vec::new();
    if let Some(items) = report.get("recommendations").and_then(Value::as_array) {
        for (index, item) in items.iter().enumerate() {
            recommendations.push(json!({
                "id": format!("compete-recommendation-{}", index + 1),
                "title": item.get("title").and_then(Value::as_str).unwrap_or("Review competitive finding"),
                "rationale": text_value(item.get("rationale")),
                "priority": enum_value(item.get("priority")).unwrap_or("unspecified"),
                "sourceIssueIds": [],
                "confidence": item.get("confidence").cloned().unwrap_or(Value::Null)
            }));
        }
    }
    out["issues"] = Value::Array(issues);
    out["recommendations"] = Value::Array(recommendations);
    out
}

fn build_react_diagnostics(route: &Value) -> Value {
    let mut out = base_report("react-diagnostics", route, react_summary(route));
    out["coverage"] = route
        .pointer("/evidence/coverage")
        .cloned()
        .unwrap_or(Value::Null);
    if route.get("status").and_then(Value::as_str) != Some("observed") {
        return out;
    }

    let diagnostics = route
        .pointer("/evidence/diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (issues, recommendations): (Vec<_>, Vec<_>) = diagnostics
        .iter()
        .enumerate()
        .map(|(index, diagnostic)| react_outputs(index, diagnostic))
        .unzip();
    out["issues"] = Value::Array(issues);
    out["recommendations"] = Value::Array(recommendations);
    out
}

fn react_summary(route: &Value) -> Value {
    match route.get("status").and_then(Value::as_str) {
        Some("not_applicable") => json!("React was not detected; this feature is not applicable."),
        Some("observed") => observed_react_summary(route),
        _ => route_reason(route),
    }
}

fn observed_react_summary(route: &Value) -> Value {
    let count = route
        .pointer("/evidence/diagnostics")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if count == 0 {
        json!("React Doctor completed with no diagnostics.")
    } else {
        json!(format!("React Doctor reported {count} project problem(s)."))
    }
}

fn react_outputs(index: usize, diagnostic: &Value) -> (Value, Value) {
    let id = diagnostic
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("react-doctor-diagnostic");
    let rule = diagnostic
        .get("rule")
        .and_then(Value::as_str)
        .unwrap_or("unknown-rule");
    let title = diagnostic
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or(rule);
    let issue = json!({
        "id": id,
        "kind": "diagnostic",
        "title": title,
        "description": diagnostic.get("message").cloned().unwrap_or(Value::Null),
        "severity": diagnostic.get("severity").cloned().unwrap_or_else(|| json!("warning")),
        "category": diagnostic.get("category").cloned().unwrap_or_else(|| json!("react")),
        "location": {
            "path": diagnostic.get("normalizedFilePath").cloned().unwrap_or(Value::Null),
            "line": diagnostic.get("line").cloned().unwrap_or(Value::Null),
            "column": diagnostic.get("column").cloned().unwrap_or(Value::Null),
            "endLine": diagnostic.get("endLine").cloned().unwrap_or(Value::Null),
            "endColumn": diagnostic.get("endColumn").cloned().unwrap_or(Value::Null)
        },
        "sourceEvidence": diagnostic
    });
    let recommendation = json!({
        "id": format!("react-doctor-recommendation-{}", index + 1),
        "title": format!("Resolve {rule}"),
        "rationale": diagnostic.get("help").cloned().unwrap_or_else(|| json!(title)),
        "priority": diagnostic.get("severity").cloned().unwrap_or_else(|| json!("warning")),
        "sourceIssueIds": [id],
        "confidence": Value::Null
    });
    (issue, recommendation)
}

fn route_reason(route: &Value) -> Value {
    route
        .pointer("/admissibility/reason")
        .cloned()
        .unwrap_or_else(|| json!("Feature result is unknown."))
}

fn text_value(value: Option<&Value>) -> Option<String> {
    let value = value?;
    value
        .as_str()
        .or_else(|| value.get("value").and_then(Value::as_str))
        .map(ToString::to_string)
}

fn string_array_value(value: Option<&Value>) -> Option<Vec<String>> {
    let value = value?;
    let array = value
        .as_array()
        .or_else(|| value.get("value").and_then(Value::as_array))?;
    Some(
        array
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect(),
    )
}

fn enum_value(value: Option<&Value>) -> Option<&str> {
    let value = value?;
    value
        .as_str()
        .or_else(|| value.get("value").and_then(Value::as_str))
}

fn write_report(feature: &str, artifact_root: &Path, report: &Value) -> Result<Value> {
    fs::create_dir_all(artifact_root)?;
    let json_path = artifact_root.join(format!("{feature}.json"));
    let markdown_path = artifact_root.join(format!("{feature}.md"));
    fs::write(&json_path, serde_json::to_string_pretty(report)?)?;
    fs::write(&markdown_path, render_markdown(report))?;
    Ok(json!({
        "json": absolute_or_original(json_path),
        "markdown": absolute_or_original(markdown_path)
    }))
}

fn absolute_or_original(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn render_markdown(report: &Value) -> String {
    let title = match report.get("feature").and_then(Value::as_str) {
        Some("competitive-intelligence") => "Competitive Intelligence",
        _ => "React Diagnostics",
    };
    let mut text = format!(
        "# {title} (Experimental Beta)\n\nStatus: `{}`\n\n{}\n\n## Problems and findings\n",
        report
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        report
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("No summary is available.")
    );
    let issues = report
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if issues.is_empty() {
        text.push_str("\nNo observed problems were available. An unknown or partial result is not a healthy result.\n");
    } else {
        for issue in issues {
            text.push_str(&format!(
                "\n- **{}** — {}\n",
                issue
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Finding"),
                issue
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("No additional description.")
            ));
        }
    }
    text.push_str("\n## Improvement recommendations\n");
    let recommendations = report
        .get("recommendations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if recommendations.is_empty() {
        text.push_str("\nNo recommendations were available for this result.\n");
    } else {
        for recommendation in recommendations {
            text.push_str(&format!(
                "\n- **{}** — {}\n",
                recommendation
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Recommendation"),
                recommendation
                    .get("rationale")
                    .and_then(Value::as_str)
                    .unwrap_or("Review the linked finding.")
            ));
        }
    }
    text.push_str("\nThis Beta report is advisory and does not contain or affect a score.\n");
    text
}

fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
}

fn print_output(value: &Value, json_output: bool) -> Result<()> {
    if json_output {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else if let Some(features) = value.get("features").and_then(Value::as_array) {
        for feature in features {
            println!(
                "{} (beta)",
                feature.get("id").and_then(Value::as_str).unwrap_or("")
            );
        }
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(provider: &str, status: &str, evidence: Value) -> Value {
        json!({
            "schema": "code-intel-evidence-route-result.v1",
            "provider": provider,
            "snapshotIdentity": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "status": status,
            "advisoryOnly": true,
            "admissibility": {"admitted": true, "reason": "fixture"},
            "failureCategory": Value::Null,
            "evaluatedAt": 1,
            "evidence": evidence
        })
    }

    #[test]
    fn competitive_intelligence_reports_gaps_and_recommendations_without_score() {
        let result = build(
            "competitive-intelligence",
            &route(
                "compete",
                "observed",
                json!({"report": {
                    "executive_summary": {
                        "summary": {"value": "Market summary"},
                        "key_findings": {"value": ["Positioning is unclear"]}
                    },
                    "opportunity_gaps": [{
                        "title": "Missing workflow",
                        "description": {"value": "Rivals cover this workflow"},
                        "impact": {"value": "high"}
                    }],
                    "recommendations": [{
                        "title": "Add workflow",
                        "rationale": {"value": "Close the gap"},
                        "priority": {"value": "high"},
                        "confidence": "high"
                    }]
                }}),
            ),
        )
        .unwrap();
        assert_eq!(result["issues"].as_array().unwrap().len(), 2);
        assert_eq!(result["recommendations"].as_array().unwrap().len(), 1);
        assert!(result.get("score").is_none());
    }

    #[test]
    fn react_diagnostics_preserve_ids_locations_coverage_and_help() {
        let result = build(
            "react-diagnostics",
            &route(
                "react-doctor",
                "observed",
                json!({
                    "coverage": [{"complete": true, "skippedChecks": []}],
                    "diagnostics": [{
                        "id": "stable-id",
                        "normalizedFilePath": "src/App.tsx",
                        "line": 2,
                        "column": 3,
                        "endLine": 2,
                        "endColumn": 8,
                        "plugin": "react-doctor",
                        "rule": "no-test",
                        "category": "correctness",
                        "severity": "warning",
                        "tags": ["test"],
                        "message": "Add a test",
                        "help": "Cover this component"
                    }]
                }),
            ),
        )
        .unwrap();
        assert_eq!(result["issues"][0]["id"], "stable-id");
        assert_eq!(result["issues"][0]["location"]["path"], "src/App.tsx");
        assert_eq!(
            result["recommendations"][0]["rationale"],
            "Cover this component"
        );
        assert_eq!(result["coverage"][0]["complete"], true);
        assert!(result.get("score").is_none());
    }

    #[test]
    fn partial_results_remain_unknown_and_do_not_claim_health() {
        let result = build(
            "react-diagnostics",
            &route(
                "react-doctor",
                "unknown",
                json!({"coverage": [{"complete": false}], "diagnostics": []}),
            ),
        )
        .unwrap();
        assert_eq!(result["status"], "unknown");
        assert!(result["issues"].as_array().unwrap().is_empty());
        assert_ne!(
            result["summary"],
            "React Doctor completed with no diagnostics."
        );
    }
}
