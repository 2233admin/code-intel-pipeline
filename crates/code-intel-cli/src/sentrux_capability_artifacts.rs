use std::path::Path;

use serde_json::{json, Value};

use crate::adapter_contract::{AdapterArtifact, AdapterError};
use crate::capability::{rfc3339_now, sha256_hex};

use super::{command_evidence, run_sentrux, SentruxCommand};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RouteKind {
    Command,
    ReuseScan,
    NotApplicable {
        failure_kind: &'static str,
        message: &'static str,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CapabilityRoute {
    capability_id: &'static str,
    operation: &'static str,
    command: &'static str,
    kind: RouteKind,
}

// This is the executor's canonical dispatch table. The matrix remains the
// inventory of record; keeping the table explicit here makes an omitted route
// observable in tests and in the emitted artifact set instead of becoming a
// silent loop omission.
const SENTRUX_CAPABILITY_ROUTES: [CapabilityRoute; 15] = [
    route("sentrux.gate", "gate", "gate", RouteKind::Command),
    route("sentrux.check", "check", "check", RouteKind::Command),
    route("sentrux.scan", "scan", "scan", RouteKind::Command),
    route("sentrux.health", "health", "health", RouteKind::Command),
    route("sentrux.dsm", "dsm", "dsm", RouteKind::Command),
    route(
        "sentrux.check_rules",
        "check_rules",
        "check_rules",
        RouteKind::Command,
    ),
    route(
        "sentrux.baseline_save",
        "gate_save",
        "gate_save",
        RouteKind::NotApplicable {
            failure_kind: "explicit_mutation_required",
            message: "baseline_save mutates the repository baseline and requires explicit authority",
        },
    ),
    route(
        "sentrux.rescan",
        "rescan",
        "rescan",
        RouteKind::ReuseScan,
    ),
    route(
        "sentrux.git_stats",
        "git_stats",
        "git_stats",
        RouteKind::Command,
    ),
    route(
        "sentrux.evolution",
        "evolution",
        "evolution",
        RouteKind::Command,
    ),
    route(
        "sentrux.test_gaps",
        "test_gaps",
        "test_gaps",
        RouteKind::Command,
    ),
    route(
        "sentrux.what_if",
        "what_if",
        "what_if",
        RouteKind::NotApplicable {
            failure_kind: "dag_scope_not_supported",
            message: "what_if requires an explicit change set and is not applicable to this repository DAG snapshot",
        },
    ),
    route(
        "sentrux.session_start",
        "session_start",
        "session_start",
        RouteKind::NotApplicable {
            failure_kind: "session_lifecycle_outside_dag",
            message: "session_start is an agent lifecycle event and is not applicable to this repository DAG run",
        },
    ),
    route(
        "sentrux.session_end",
        "session_end",
        "session_end",
        RouteKind::NotApplicable {
            failure_kind: "session_lifecycle_outside_dag",
            message: "session_end is an agent lifecycle event and is not applicable to this repository DAG run",
        },
    ),
    route(
        "sentrux.provider_discovery",
        "provider_discovery",
        "provider_discovery",
        RouteKind::Command,
    ),
];

const fn route(
    capability_id: &'static str,
    operation: &'static str,
    command: &'static str,
    kind: RouteKind,
) -> CapabilityRoute {
    CapabilityRoute {
        capability_id,
        operation,
        command,
        kind,
    }
}

pub(super) fn collect_sentrux_capabilities(
    repo: &Path,
    tool_path_prefix: Option<&Path>,
) -> Result<(SentruxCommand, SentruxCommand, Vec<Value>), AdapterError> {
    let mut gate = None;
    let mut check = None;
    let mut observations = Vec::with_capacity(SENTRUX_CAPABILITY_ROUTES.len());
    for route in SENTRUX_CAPABILITY_ROUTES {
        let observation = match route.kind {
            RouteKind::NotApplicable {
                failure_kind,
                message,
            } => not_applicable_observation(&route, tool_path_prefix, failure_kind, message),
            RouteKind::ReuseScan => {
                match run_sentrux(
                    repo,
                    tool_path_prefix,
                    if tool_path_prefix.is_some() {
                        route.command
                    } else {
                        "scan"
                    },
                ) {
                    Ok(command) => capability_observation(
                        &route,
                        tool_path_prefix,
                        &command,
                        Some("authoritative"),
                    ),
                    Err(error) => route_error_observation(&route, tool_path_prefix, &error),
                }
            }
            RouteKind::Command => {
                if tool_path_prefix.is_none()
                    && !matches!(
                        route.command,
                        "gate"
                            | "check"
                            | "scan"
                            | "health"
                            | "dsm"
                            | "check_rules"
                            | "git_stats"
                            | "evolution"
                            | "test_gaps"
                            | "provider_discovery"
                    )
                {
                    unavailable_observation(
                        &route,
                        tool_path_prefix,
                        "the built-in Rust provider has no route for this capability",
                    )
                } else {
                    match run_sentrux(repo, tool_path_prefix, route.command) {
                        Ok(command) => capability_observation(
                            &route,
                            tool_path_prefix,
                            &command,
                            Some("authoritative"),
                        ),
                        Err(error) if matches!(route.command, "gate" | "check") => {
                            return Err(error)
                        }
                        Err(error) => route_error_observation(&route, tool_path_prefix, &error),
                    }
                }
            }
        };
        if route.capability_id == "sentrux.gate" {
            if let Some(command) = observation_command(&observation) {
                gate = Some(command);
            }
        } else if route.capability_id == "sentrux.check" {
            if let Some(command) = observation_command(&observation) {
                check = Some(command);
            }
        }
        observations.push(observation);
    }
    Ok((
        gate.ok_or_else(|| AdapterError::Internal("Sentrux gate observation is missing".into()))?,
        check
            .ok_or_else(|| AdapterError::Internal("Sentrux check observation is missing".into()))?,
        observations,
    ))
}

pub(super) fn build_capability_artifacts(
    observations: &[Value],
    snapshot_identity: &str,
    run_id: &str,
    tool_path_prefix: Option<&Path>,
) -> Result<(Vec<AdapterArtifact>, Vec<Value>), AdapterError> {
    let provider = sentrux_capability_provider(tool_path_prefix);
    let mut artifacts = Vec::with_capacity(observations.len());
    let mut refs = Vec::with_capacity(observations.len());
    for observation in observations {
        let capability_id = observation["capabilityId"].as_str().ok_or_else(|| {
            AdapterError::Contract("Sentrux capability observation has no capabilityId".into())
        })?;
        let operation = observation["operation"].as_str().ok_or_else(|| {
            AdapterError::Contract("Sentrux capability observation has no operation".into())
        })?;
        let raw_status = observation["status"].as_str().unwrap_or("failed");
        let status = match raw_status {
            "not_run" | "not_applicable" => "not_applicable",
            "succeeded" | "degraded" | "unavailable" | "skipped" | "failed" => raw_status,
            other => {
                return Err(AdapterError::Contract(format!(
                    "unsupported Sentrux capability status: {other}"
                )))
            }
        };
        let artifact = json!({
            "schema":"code-intel-sentrux-capability-artifact.v1",
            "contractVersion":1,
            "capabilityId":capability_id,
            "operation":operation,
            "runId":run_id,
            "snapshotIdentity":snapshot_identity,
            "provider":provider.clone(),
            "status":status,
            "authority":artifact_authority(observation, status),
            "inputs":{"snapshotIdentity":snapshot_identity},
            "outputs":{
                "command":observation["command"],
                "verdict":observation["verdict"],
                "outputSummary":observation["outputSummary"]
            },
            "failure":capability_failure(observation, status),
            "freshness":{
                "status":"current",
                "evaluatedAt":rfc3339_now(),
                "consumedSnapshotIdentity":snapshot_identity
            },
            "decisionConsumers":sentrux_decision_consumers(capability_id)
        });
        let bytes = serde_json::to_vec(&artifact).map_err(|error| {
            AdapterError::Internal(format!("serialize Sentrux capability artifact: {error}"))
        })?;
        let relative_path = format!(
            "sentrux-capability-{}.json",
            capability_id.replace('.', "-")
        );
        refs.push(json!({
            "schema":"code-intel-artifact-ref.v1",
            "artifactSchema":"code-intel-sentrux-capability-artifact.v1",
            "type":"provider.sentrux.capability-artifact",
            "path":relative_path,
            "sha256":sha256_hex(&bytes),
            "consumedSnapshotIdentity":snapshot_identity
        }));
        artifacts.push(AdapterArtifact {
            artifact_schema: "code-intel-sentrux-capability-artifact.v1".into(),
            artifact_type: "provider.sentrux.capability-artifact".into(),
            relative_path,
            bytes,
        });
    }
    Ok((artifacts, refs))
}

fn sentrux_capability_provider(tool_path_prefix: Option<&Path>) -> Value {
    if tool_path_prefix.is_some() {
        json!({
            "mode":"external",
            "id":"sentrux.command-adapter",
            "version":"1.0.0",
            "digest":sha256_hex(include_bytes!("builtin_provider_evidence.rs"))
        })
    } else {
        json!({
            "mode":"builtin",
            "id":super::sentrux_gate::ENGINE_ID,
            "version":super::sentrux_gate::ENGINE_VERSION,
            "digest":sha256_hex(include_bytes!("sentrux_gate.rs"))
        })
    }
}

fn capability_failure(observation: &Value, status: &str) -> Value {
    if status == "succeeded" {
        return Value::Null;
    }
    let raw_kind = observation["failure"]["kind"].as_str().unwrap_or("unknown");
    let kind = match raw_kind {
        "explicit_mutation_required"
        | "dag_scope_not_supported"
        | "session_lifecycle_outside_dag"
        | "not_applicable" => "not_applicable",
        "provider_unavailable" | "capability_unavailable" => "provider_unavailable",
        "contract_error" | "invalid_options" => "config_error",
        "io_error" => "local_tool_error",
        _ => "provider_error",
    };
    let message = observation["failure"]["message"]
        .as_str()
        .or_else(|| observation["failure"]["kind"].as_str())
        .unwrap_or("Sentrux capability did not complete")
        .to_string();
    json!({
        "kind":kind,
        "message":message,
        "retryable":kind == "provider_unavailable" || kind == "local_tool_error"
    })
}

fn sentrux_decision_consumers(capability_id: &str) -> Value {
    match capability_id {
        "sentrux.gate" | "sentrux.check" => {
            json!(["diagnosis.hospital", "pr_gate", "release_gate"])
        }
        "sentrux.baseline_save" => json!(["sentrux.gate"]),
        "sentrux.dsm" | "sentrux.test_gaps" => json!([
            "evidence.sentrux",
            "diagnosis.hospital",
            "report",
            "change_impact",
            "test_selection",
            "pr_gate",
            "release_gate"
        ]),
        "sentrux.what_if" => json!(["change_impact", "pr_gate", "release_gate"]),
        "sentrux.session_start" => json!(["sentrux.rescan", "sentrux.session_end"]),
        "sentrux.session_end" => json!(["pr_gate", "release_gate"]),
        "sentrux.provider_discovery" => {
            json!(["doctor", "run_planner", "install_smoke", "release_gate"])
        }
        _ => json!([
            "evidence.sentrux",
            "diagnosis.hospital",
            "report",
            "release_gate"
        ]),
    }
}

fn capability_observation(
    route: &CapabilityRoute,
    tool_path_prefix: Option<&Path>,
    command: &SentruxCommand,
    authority: Option<&str>,
) -> Value {
    let (status, verdict, failure) = if command.success && !command.output_summary.complete() {
        (
            "degraded",
            "unknown",
            json!({
                "kind":"degraded",
                "message":format!(
                    "Sentrux {} output exceeded the bounded evidence limit; only metadata and preview were retained",
                    route.operation
                )
            }),
        )
    } else if command.success {
        ("succeeded", "pass", json!({"kind":"none"}))
    } else if !command.governed {
        ("succeeded", "unknown", json!({"kind":"none"}))
    } else {
        (
            "failed",
            "fail",
            json!({
                "kind":"command_failed",
                "message":command_failure_message(command)
            }),
        )
    };
    json!({
        "capabilityId":route.capability_id,
        "operation":route.operation,
        "providerMode":sentrux_provider_mode(tool_path_prefix),
        "authority":authority.unwrap_or("compatibility"),
        "status":status,
        "verdict":verdict,
        "command":capability_command_evidence(route.operation, command),
        "outputSummary":command.output_summary.to_json(&command.stdout, &command.stderr),
        "failure":failure
    })
}

fn capability_command_evidence(operation: &str, command: &SentruxCommand) -> Value {
    let mut evidence = command_evidence(operation, command);
    evidence["governed"] = json!(command.governed);
    evidence["violations"] = command.violations_json();
    evidence
}

fn not_applicable_observation(
    route: &CapabilityRoute,
    tool_path_prefix: Option<&Path>,
    failure_kind: &str,
    message: &str,
) -> Value {
    json!({
        "capabilityId":route.capability_id,
        "operation":route.operation,
        "providerMode":sentrux_provider_mode(tool_path_prefix),
        "authority":"declared_only",
        "status":"not_applicable",
        "verdict":"unknown",
        "command":Value::Null,
        "failure":{"kind":failure_kind,"message":message}
    })
}

fn unavailable_observation(
    route: &CapabilityRoute,
    tool_path_prefix: Option<&Path>,
    message: &str,
) -> Value {
    json!({
        "capabilityId":route.capability_id,
        "operation":route.operation,
        "providerMode":sentrux_provider_mode(tool_path_prefix),
        "authority":"compatibility",
        "status":"unavailable",
        "verdict":"unknown",
        "command":Value::Null,
        "failure":{"kind":"capability_unavailable","message":message}
    })
}

fn route_error_observation(
    route: &CapabilityRoute,
    tool_path_prefix: Option<&Path>,
    error: &AdapterError,
) -> Value {
    let unavailable = matches!(error, AdapterError::Unavailable(_));
    json!({
        "capabilityId":route.capability_id,
        "operation":route.operation,
        "providerMode":sentrux_provider_mode(tool_path_prefix),
        "authority":if unavailable { "compatibility" } else { "authoritative" },
        "status":if unavailable { "unavailable" } else { "failed" },
        "verdict":"unknown",
        "command":Value::Null,
        "failure":{
            "kind":if unavailable { "provider_unavailable" } else { adapter_error_kind(error) },
            "message":format!("{error:?}")
        }
    })
}

fn artifact_authority(observation: &Value, status: &str) -> &'static str {
    match observation["authority"].as_str() {
        Some("authoritative") => "authoritative",
        Some("fallback") => "fallback",
        Some("compatibility") => "compatibility",
        Some("declared_only") => "declared_only",
        _ => match status {
            "succeeded" | "failed" => "authoritative",
            "not_applicable" => "declared_only",
            _ => "compatibility",
        },
    }
}

fn observation_command(observation: &Value) -> Option<SentruxCommand> {
    let command = observation["command"].as_object()?;
    let stdout = command["stdout"].as_str().unwrap_or_default().to_owned();
    let stderr = command["stderr"].as_str().unwrap_or_default().to_owned();
    let output_summary = command
        .get("outputSummary")
        .unwrap_or(&Value::Null)
        .as_object()
        .and_then(|summary| {
            Some(super::sentrux_command::OutputSummary::from_metadata(
                summary,
            ))
        })
        .unwrap_or_else(|| {
            super::sentrux_command::OutputSummary::from_bytes(stdout.as_bytes(), stderr.as_bytes())
        });
    Some(SentruxCommand {
        argv: command["argv"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        exit_code: command["exitCode"].as_i64().map(|value| value as i32),
        success: command["success"].as_bool().unwrap_or(false),
        stdout,
        stderr,
        violations: SentruxCommand::violations_from_json(command.get("violations")),
        // Capability artifacts intentionally retain an ungoverned gate as a
        // successful, unknown observation. Rehydrate that distinction here so
        // the capability evidence cannot turn an absent baseline into a
        // structural failure when it is fed back into the authoritative rules.
        governed: command
            .get("governed")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| {
                !(command["success"] == false
                    && observation["status"] == "succeeded"
                    && observation["verdict"] == "unknown")
            }),
        output_summary,
    })
}

fn sentrux_provider_mode(tool_path_prefix: Option<&Path>) -> &'static str {
    if tool_path_prefix.is_some() {
        "external"
    } else {
        "builtin_lite"
    }
}

fn adapter_error_kind(error: &AdapterError) -> &'static str {
    match error {
        AdapterError::Unavailable(_) => "provider_unavailable",
        AdapterError::Contract(_) => "contract_error",
        AdapterError::InvalidOptions(_) => "invalid_options",
        AdapterError::Internal(_) => "provider_error",
        AdapterError::Io(_) => "io_error",
    }
}

fn command_failure_message(command: &SentruxCommand) -> String {
    command
        .stderr
        .lines()
        .chain(command.stdout.lines())
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("Sentrux command reported a failing verdict")
        .chars()
        .take(1024)
        .collect()
}
