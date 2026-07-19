use crate::{providers, Result};
use serde_json::{json, Value};
use std::path::Path;

pub struct Options<'a> {
    pub action: &'a str,
    pub provider: Option<&'a str>,
    pub operation: Option<&'a str>,
    pub repo: Option<&'a Path>,
    pub json: bool,
}

pub fn run(options: &Options<'_>) -> Result<()> {
    let action = options.action.to_ascii_lowercase();
    let value = match action.as_str() {
        "list" => list_routes(options.provider),
        "plan" => plan_route(options)?,
        "validate" => validate_routes(),
        other => return Err(format!("unknown route action: {other}").into()),
    };

    if options.json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print_human(&value);
    }

    Ok(())
}

fn list_routes(provider: Option<&str>) -> Value {
    let provider_api = providers::list(provider);
    let routes: Vec<Value> = provider_api["operations"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(route_json)
        .collect();

    json!({
        "ok": true,
        "schema": "code-intel-route-registry.v1",
        "routes": routes
    })
}

fn plan_route(options: &Options<'_>) -> Result<Value> {
    let provider_plan = providers::plan(&providers::Options {
        action: "Plan",
        provider: options.provider,
        operation: options.operation,
        repo: options.repo,
        language: "zh",
        full: false,
        write: true,
        json: true,
        request: None,
        artifact_root: None,
        evaluated_at: None,
        max_age_seconds: None,
    })?;

    Ok(json!({
        "ok": true,
        "route": route_json(provider_plan["operation"].clone()),
        "command": provider_plan["command"]
    }))
}

fn validate_routes() -> Value {
    let provider_validation = providers::validate();
    json!({
        "ok": provider_validation["ok"],
        "routes": provider_validation["operations"],
        "errors": provider_validation["errors"]
    })
}

fn route_json(operation: Value) -> Value {
    json!({
        "provider": operation["provider"],
        "operation": operation["operation"],
        "stage": operation["stage"],
        "method": operation["method"],
        "path": operation["route"],
        "commandTemplate": operation["commandTemplate"],
        "artifact": operation["artifact"],
        "status": operation["status"],
        "protocol": operation["protocol"]
    })
}

fn print_human(value: &Value) {
    if let Some(routes) = value.get("routes").and_then(|value| value.as_array()) {
        for route in routes {
            println!(
                "{}:{} {} {} -> {}",
                route["provider"].as_str().unwrap_or(""),
                route["operation"].as_str().unwrap_or(""),
                route["method"].as_str().unwrap_or(""),
                route["path"].as_str().unwrap_or(""),
                route["commandTemplate"].as_str().unwrap_or("")
            );
        }
        return;
    }

    println!("{value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_registry_validates() {
        let result = validate_routes();
        assert_eq!(result["ok"].as_bool(), Some(true));
        assert!(result["routes"].as_u64().unwrap_or(0) >= 5);
    }

    #[test]
    fn plans_understand_graph_command() {
        let repo = Path::new("C:/repo");
        let result = plan_route(&Options {
            action: "Plan",
            provider: Some("understand"),
            operation: Some("graph"),
            repo: Some(repo),
            json: true,
        })
        .unwrap();

        assert!(result["command"]
            .as_str()
            .unwrap()
            .contains("code-intel.exe provider --action Invoke --provider understand"));
        assert!(result["command"]
            .as_str()
            .unwrap()
            .contains("--repo C:/repo"));
    }

    #[test]
    fn lists_on_demand_evidence_routes() {
        let routes = list_routes(None);
        let routes = routes["routes"].as_array().unwrap();
        assert!(routes
            .iter()
            .any(|route| { route["provider"] == "compete" && route["operation"] == "adapt" }));
        assert!(routes
            .iter()
            .any(|route| { route["provider"] == "react-doctor" && route["operation"] == "scan" }));
    }
}
