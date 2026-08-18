use std::path::Path;

use serde_json::{json, Value};

use crate::adapter_contract::{AdapterArtifact, AdapterError};
use crate::capability::{rfc3339_now, sha256_hex};

use super::{command_evidence, run_sentrux, SentruxCommand};

const SENTRUX_CAPABILITY_COMMANDS: [(&str, &str); 7] = [
    ("sentrux.gate", "gate"),
    ("sentrux.check", "check"),
    ("sentrux.scan", "scan"),
    ("sentrux.health", "health"),
    ("sentrux.dsm", "dsm"),
    ("sentrux.check_rules", "check_rules"),
    ("sentrux.gate_save", "gate_save"),
];

pub(super) fn collect_sentrux_capabilities(
    repo: &Path,
    tool_path_prefix: Option<&Path>,
) -> Result<(SentruxCommand, SentruxCommand, Vec<Value>), AdapterError> {
    let mut gate = None;
    let mut check = None;
    let mut observations = Vec::with_capacity(SENTRUX_CAPABILITY_COMMANDS.len());
    for &(capability_id, subcommand) in &SENTRUX_CAPABILITY_COMMANDS {
        if subcommand == "gate_save" {
            observations.push(json!({
                "capabilityId":capability_id,
                "operation":subcommand,
                "providerMode":sentrux_provider_mode(tool_path_prefix),
                "status":"not_run",
                "verdict":"unknown",
                "command":Value::Null,
                "failure":{
                    "kind":"explicit_mutation_required",
                    "message":"gate_save writes the repository baseline and requires explicit authority"
                }
            }));
            continue;
        }
        let command = match run_sentrux(repo, tool_path_prefix, subcommand) {
            Ok(command) => command,
            Err(error) if matches!(subcommand, "gate" | "check") => return Err(error),
            Err(error) => {
                observations.push(json!({
                    "capabilityId":capability_id,
                    "operation":subcommand,
                    "providerMode":sentrux_provider_mode(tool_path_prefix),
                    "status":"failed",
                    "verdict":"unknown",
                    "command":Value::Null,
                    "failure":{
                        "kind":adapter_error_kind(&error),
                        "message":format!("{error:?}")
                    }
                }));
                continue;
            }
        };
        observations.push(capability_observation(
            capability_id,
            subcommand,
            tool_path_prefix,
            &command,
        ));
        match subcommand {
            "gate" => gate = Some(command),
            "check" => check = Some(command),
            _ => {}
        }
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
            "not_run" => "not_applicable",
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
            "authority":if status == "not_applicable" { "declared_only" } else { "authoritative" },
            "inputs":{"snapshotIdentity":snapshot_identity},
            "outputs":{
                "command":observation["command"],
                "verdict":observation["verdict"]
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
        "explicit_mutation_required" | "not_applicable" => "not_applicable",
        "provider_unavailable" => "provider_unavailable",
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
        "sentrux.gate_save" => json!(["sentrux.gate"]),
        _ => json!(["evidence.sentrux", "diagnosis.hospital"]),
    }
}

fn capability_observation(
    capability_id: &str,
    subcommand: &str,
    tool_path_prefix: Option<&Path>,
    command: &SentruxCommand,
) -> Value {
    let (status, verdict, failure) = if command.success {
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
        "capabilityId":capability_id,
        "operation":subcommand,
        "providerMode":sentrux_provider_mode(tool_path_prefix),
        "status":status,
        "verdict":verdict,
        "command":command_evidence(subcommand, command),
        "failure":failure
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
