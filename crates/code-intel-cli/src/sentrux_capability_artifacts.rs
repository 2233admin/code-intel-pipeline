use std::path::Path;

use serde_json::{json, Value};

use crate::adapter_contract::{AdapterArtifact, AdapterError};
use crate::capability::sha256_hex;

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
    route("sentrux.what_if", "what_if", "what_if", RouteKind::Command),
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
        // `what_if`'s builtin engine (`sentrux_evolution::what_if`) has no
        // parameter for an external tool prefix, structurally the same
        // constraint the still-lite fallbacks have -- but issue #374 moved
        // it out of `uses_lite_fallback` (it's correctly attributed
        // `"builtin"` now, not `"lite_fallback"`), so forcing off the
        // external branch needs its own check here rather than piggybacking
        // on `uses_lite_fallback`. Without this, `what_if` would newly start
        // reaching `Some(prefix)` in `run_sentrux` whenever a
        // `toolPathPrefix` is configured -- resolving #374's decision 3 (the
        // external-Sentrux branch's fate) as a side effect, which the issue
        // explicitly scoped out ("do not resolve it, just don't break it").
        //
        // `route_tool_path_prefix` must be resolved before `provider_mode`,
        // not the other way around: `provider_mode` labels which engine
        // actually ran, so it has to read the post-override prefix. Deriving
        // it from the raw `tool_path_prefix` mislabelled `what_if` as
        // `"external"` whenever a tool prefix was configured, even though
        // this override was about to force it onto the builtin engine
        // regardless (caught in review on #374).
        let route_tool_path_prefix = if (matches!(route.kind, RouteKind::Command)
            && uses_lite_fallback(route.command))
            || route.command == "what_if"
        {
            None
        } else {
            tool_path_prefix
        };
        let provider_mode = route_provider_mode(&route, route_tool_path_prefix);
        let observation = match route.kind {
            RouteKind::NotApplicable {
                failure_kind,
                message,
            } => not_applicable_observation(&route, provider_mode, failure_kind, message),
            RouteKind::ReuseScan => {
                match run_sentrux(
                    repo,
                    route_tool_path_prefix,
                    if route_tool_path_prefix.is_some() {
                        route.command
                    } else {
                        "scan"
                    },
                ) {
                    Ok(command) => capability_observation(
                        &route,
                        provider_mode,
                        &command,
                        Some("authoritative"),
                    ),
                    Err(error) => route_error_observation(&route, provider_mode, &error),
                }
            }
            RouteKind::Command => {
                if route_tool_path_prefix.is_none()
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
                            | "what_if"
                            | "provider_discovery"
                    )
                {
                    unavailable_observation(
                        &route,
                        provider_mode,
                        "the built-in Rust provider has no route for this capability",
                    )
                } else {
                    match run_sentrux(repo, route_tool_path_prefix, route.command) {
                        Ok(command) => capability_observation(
                            &route,
                            provider_mode,
                            &command,
                            Some("authoritative"),
                        ),
                        Err(error) if matches!(route.command, "gate" | "check") => {
                            return Err(error)
                        }
                        Err(error) => route_error_observation(&route, provider_mode, &error),
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
) -> Result<(Vec<AdapterArtifact>, Vec<Value>), AdapterError> {
    let mut artifacts = Vec::with_capacity(observations.len());
    let mut refs = Vec::with_capacity(observations.len());
    for observation in observations {
        let capability_id = observation["capabilityId"].as_str().ok_or_else(|| {
            AdapterError::Contract("Sentrux capability observation has no capabilityId".into())
        })?;
        let operation = observation["operation"].as_str().ok_or_else(|| {
            AdapterError::Contract("Sentrux capability observation has no operation".into())
        })?;
        let provider =
            sentrux_capability_provider(observation["providerMode"].as_str().unwrap_or("builtin"));
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
                "outputSummary":observation["outputSummary"],
                "structuredData":capability_structured_data(observation)
            },
            "failure":capability_failure(observation, status),
            "freshness":{
                "status":"current",
                // The parent Sentrux observation carries the wall-clock
                // freshness authority. Capability artifacts are themselves
                // content-addressed, so their snapshot-bound projection
                // must not embed a per-run timestamp.
                "evaluatedAt":null,
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

// Issue #383: this used to re-parse `observation["command"]["stdout"]` --
// the same 8KB-`bounded_text` preview built for human diagnostics -- as
// JSON. Any real output over 8KB was truncated mid-document, so
// `serde_json::from_str` failed and this silently returned `Value::Null`
// even though the capability's `status` correctly reported
// `"succeeded"`/`"complete"`. `command_evidence` (`sentrux_command.rs`) now
// carries the full, unbounded payload directly as `command.structuredData`,
// parsed once at `SentruxCommand` construction time and never round-tripped
// through the bounded preview text; read it verbatim here instead of
// reparsing anything.
fn capability_structured_data(observation: &Value) -> Value {
    let value = &observation["command"]["structuredData"];
    if value.is_object() || value.is_array() {
        value.clone()
    } else {
        Value::Null
    }
}

fn sentrux_capability_provider(provider_mode: &str) -> Value {
    match provider_mode {
        "external" => json!({
            "mode":"external",
            "id":"sentrux.command-adapter",
            "version":"1.0.0",
            "digest":sha256_hex(include_bytes!("builtin_provider_evidence.rs"))
        }),
        "lite_fallback" => json!({
            "mode":"lite_fallback",
            "id":"sentrux.lite-capabilities",
            "version":"1.0.0",
            "digest":sha256_hex(include_bytes!("sentrux_lite_capabilities.rs"))
        }),
        _ => json!({
            "mode":"builtin",
            "id":super::sentrux_gate::ENGINE_ID,
            "version":super::sentrux_gate::ENGINE_VERSION,
            "digest":sha256_hex(include_bytes!("sentrux_gate.rs"))
        }),
    }
}

fn capability_failure(observation: &Value, status: &str) -> Value {
    if status == "succeeded" {
        return Value::Null;
    }
    let raw_kind = observation["failure"]["kind"].as_str().unwrap_or("unknown");
    let kind = match raw_kind {
        "degraded" => "degraded",
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
    provider_mode: &str,
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
        "providerMode":provider_mode,
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
    provider_mode: &str,
    failure_kind: &str,
    message: &str,
) -> Value {
    json!({
        "capabilityId":route.capability_id,
        "operation":route.operation,
        "providerMode":provider_mode,
        "authority":"declared_only",
        "status":"not_applicable",
        "verdict":"unknown",
        "command":Value::Null,
        "failure":{"kind":failure_kind,"message":message}
    })
}

fn unavailable_observation(route: &CapabilityRoute, provider_mode: &str, message: &str) -> Value {
    json!({
        "capabilityId":route.capability_id,
        "operation":route.operation,
        "providerMode":provider_mode,
        "authority":"compatibility",
        "status":"unavailable",
        "verdict":"unknown",
        "command":Value::Null,
        "failure":{"kind":"capability_unavailable","message":message}
    })
}

fn route_error_observation(
    route: &CapabilityRoute,
    provider_mode: &str,
    error: &AdapterError,
) -> Value {
    let unavailable = matches!(error, AdapterError::Unavailable(_));
    json!({
        "capabilityId":route.capability_id,
        "operation":route.operation,
        "providerMode":provider_mode,
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
        // Issue #383: rehydrate the full structured payload from the same
        // `command.structuredData` field `command_evidence` wrote, not by
        // reparsing `stdout` (which is only ever the bounded preview here).
        structured_stdout: command
            .get("structuredData")
            .and_then(|value| (value.is_object() || value.is_array()).then(|| value.clone())),
    })
}

fn route_provider_mode(
    route: &CapabilityRoute,
    route_tool_path_prefix: Option<&Path>,
) -> &'static str {
    if matches!(route.kind, RouteKind::Command) && uses_lite_fallback(route.command) {
        "lite_fallback"
    } else if route_tool_path_prefix.is_some() {
        "external"
    } else {
        "builtin"
    }
}

// Issue #374: `what_if` no longer routes through
// `sentrux_lite_capabilities.rs` -- `builtin_provider_evidence.rs::run_sentrux`
// calls the real `sentrux_evolution::what_if` engine directly, so it must
// not be classified `lite_fallback` here (that would misattribute its
// capability artifact's `provider` identity/digest to
// `sentrux_lite_capabilities.rs`, a file it no longer touches at all).
// `evolution` stays listed: its own DAG dispatch arm is unchanged, tracked
// separately in #377 (see DR-0008).
fn uses_lite_fallback(command: &str) -> bool {
    matches!(
        command,
        "git_stats" | "evolution" | "test_gaps" | "provider_discovery"
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "code-intel-374-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn write_fake_sentrux_tool(prefix: &std::path::Path) {
        fs::create_dir_all(prefix).expect("tool prefix directory");
        #[cfg(windows)]
        fs::write(
            prefix.join("sentrux.cmd"),
            "@echo off\r\necho {}\r\nexit /b 0\r\n",
        )
        .expect("fake sentrux.cmd");
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = prefix.join("sentrux");
            fs::write(&path, "#!/bin/sh\necho '{}'\nexit 0\n").expect("fake sentrux script");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .expect("fake sentrux script permissions");
        }
    }

    /// Regression for a review finding on #374: `route_provider_mode` used
    /// to be derived from the *pre-override* `tool_path_prefix`, so
    /// `what_if` was mislabelled `providerMode: "external"` whenever a tool
    /// prefix was configured, even though `route_tool_path_prefix` forces
    /// `what_if` onto the builtin engine regardless (it has no parameter for
    /// an external prefix at all). `dsm` is asserted alongside it as a
    /// control: a capability that genuinely does honor the configured
    /// prefix must still show `"external"`, proving the fix didn't just
    /// make everything default to `"builtin"`.
    #[test]
    fn what_if_stays_builtin_even_with_a_configured_external_tool_prefix() {
        let repo = temp_dir("repo");
        fs::create_dir_all(repo.join("src")).expect("repo src directory");
        fs::write(repo.join("src/lib.rs"), "pub fn f() {}\n").expect("repo source file");

        let tool_prefix = temp_dir("tool");
        write_fake_sentrux_tool(&tool_prefix);

        let (_, _, observations) = collect_sentrux_capabilities(&repo, Some(tool_prefix.as_path()))
            .expect("capability collection succeeds with a resolvable external tool");

        let what_if = observations
            .iter()
            .find(|observation| observation["capabilityId"] == "sentrux.what_if")
            .expect("sentrux.what_if observation is present");
        assert_eq!(what_if["providerMode"], "builtin");

        let dsm = observations
            .iter()
            .find(|observation| observation["capabilityId"] == "sentrux.dsm")
            .expect("sentrux.dsm observation is present");
        assert_eq!(dsm["providerMode"], "external");

        let _ = fs::remove_dir_all(&repo);
        let _ = fs::remove_dir_all(&tool_prefix);
    }

    /// Issue #383: a real capability's structured payload over the old 8KB
    /// preview cap must survive `capability_structured_data` (used to build
    /// the persisted `code-intel-sentrux-capability-artifact.v1`) and
    /// `observation_command` (used to rehydrate a `SentruxCommand` back out
    /// of a persisted observation for `sentrux.gate`/`sentrux.check`) intact
    /// -- not silently collapsed to `Value::Null` because the real document
    /// no longer fits in the bounded human-preview text.
    #[test]
    fn capability_structured_data_and_observation_command_round_trip_a_payload_over_8kb() {
        let value = serde_json::json!({
            "marker": "issue-383-capability-artifact-fixture",
            "filler": "y".repeat(8 * 1024 + 4096),
            "coupling_score": 12.5,
        });
        let bytes = serde_json::to_vec(&value).expect("serialize fixture");
        assert!(
            bytes.len() > 8 * 1024,
            "fixture must exceed the 8KB preview cap"
        );

        let command = super::SentruxCommand::from_json(bytes, "scan");
        let command_evidence = capability_command_evidence("scan", &command);
        let observation = json!({
            "capabilityId": "sentrux.scan",
            "operation": "scan",
            "providerMode": "builtin",
            "status": "succeeded",
            "verdict": "pass",
            "command": command_evidence,
        });

        let structured = capability_structured_data(&observation);
        assert_eq!(
            structured, value,
            "capability_structured_data must reflect the full parsed output beyond 8KB, not Value::Null"
        );

        let rehydrated =
            observation_command(&observation).expect("observation_command rehydrates a command");
        assert_eq!(rehydrated.structured_stdout.as_ref(), Some(&value));
    }
}
