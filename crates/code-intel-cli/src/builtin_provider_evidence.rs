use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::{AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;
use crate::capability::sha256_hex;
use crate::snapshot;

#[path = "admissibility.rs"]
mod admissibility;
// A local nested copy, not `crate::codenexus_adapter`: this file is compiled
// standalone into several test binaries (via `capability_inventory.rs`'s own
// `#[path]` tree) that never declare a crate-root `codenexus_adapter` module,
// exactly like `graph_adapter`/`sentrux_adapter` below already have to be.
#[path = "codenexus_adapter.rs"]
mod codenexus_adapter;
#[path = "codenexus_lite.rs"]
mod codenexus_lite;
#[path = "codenexus_scratch.rs"]
mod codenexus_scratch;
#[path = "graph/mod.rs"]
mod graph;
#[path = "graph_adapter.rs"]
mod graph_adapter;
#[path = "sentrux_adapter.rs"]
mod sentrux_adapter;
#[path = "sentrux_analysis.rs"]
mod sentrux_analysis;
#[path = "sentrux_capability_artifacts.rs"]
mod sentrux_capability_artifacts;
#[path = "sentrux_command.rs"]
mod sentrux_command;
// Real `what_if` (and `evolution`) engine -- issue #374 wired the DAG
// dispatch's `"what_if"` arm to this instead of `sentrux_lite_capabilities`'s
// simplified fallback. Nested the same way as the other local copies above:
// its own `use crate::sentrux_analysis;` was changed to `use
// super::sentrux_analysis;` so it binds to this module's own nested
// `sentrux_analysis` copy here, and to the crate-root one when this file is
// compiled directly via `main.rs`'s `mod sentrux_evolution;` instead.
#[path = "sentrux_evolution.rs"]
mod sentrux_evolution;
#[path = "sentrux_gate.rs"]
mod sentrux_gate;
#[path = "sentrux_lite_capabilities.rs"]
mod sentrux_lite_capabilities;

use codenexus_scratch::{create_codenexus_scratch_dir, ScratchDir};
pub(super) use sentrux_command::{command_evidence, SentruxCommand};
use sentrux_gate::Violation;
const MAX_AGE_SECONDS: u64 = 300;
// The codenexus-domain effect vocabulary validated by
// `codenexus_adapter::validate_native` -- distinct from the generic
// repo_read/local_write/process_spawn effects `publish_admission` reports at
// the capability-policy layer. Never let one leak into the other's field.
const CODENEXUS_EFFECTS: [&str; 4] = [
    "read_repository",
    "read_git_history",
    "read_sentrux_artifacts",
    "write_compatibility_artifact",
];

pub(super) fn graph_admission(
    request: &Value,
    inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let repo = provider_repo(request, inputs, "provider.graph-adapt")?;
    let lease =
        snapshot::begin_consumption(repo, &request["snapshot"]).map_err(AdapterError::Contract)?;
    let collected_at = now()?;
    let document = graph::generate(repo, "zh", false, false)
        .map_err(|error| AdapterError::Internal(format!("generate built-in graph: {error}")))?;
    lease.verify_after(repo).map_err(AdapterError::Contract)?;
    let observed_at = now()?.max(collected_at);
    let identity = snapshot_identity(request)?;
    let payload = json!({
        "schema":"code-intel-evidence-payload.v1",
        "data":{"architectureGraph":{
            "schema":"code-intel-architecture-graph-evidence.v1",
            "snapshotIdentity":identity,
            "provider":{
                "mode":"internal",
                "implementationId":"architecture-graph.internal-rust",
                "fallbackIdentity":Value::Null
            },
            "provenance":payload_provenance(request),
            "completeness":"complete",
            "graph":document
        }}
    });
    fs::create_dir(out)
        .map_err(|error| AdapterError::Io(format!("create graph provider output: {error}")))?;
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|error| AdapterError::Internal(format!("serialize graph payload: {error}")))?;
    fs::write(out.join("graph-payload.json"), &payload_bytes)
        .map_err(|error| AdapterError::Io(format!("write graph payload: {error}")))?;
    let native = json!({
        "schema":"code-intel-graph-provider-native.v1",
        "providerMode":"internal",
        "status":"current",
        "implementation":{
            "id":"architecture-graph.internal-rust",
            "version":"1.0.0",
            "digest":sha256_hex(include_bytes!("graph/mod.rs"))
        },
        "sourceRevision":source_revision(request),
        "expectedSnapshotIdentity":identity,
        "sourceSnapshotIdentity":identity,
        "collectedAt":collected_at,
        "observedAt":observed_at,
        "payload":payload_ref("graph-payload.json", &payload_bytes, identity),
        "fallback":Value::Null
    });
    let adapter = graph_adapter::translate(&native, observed_at, MAX_AGE_SECONDS)
        .map_err(AdapterError::Contract)?;
    let admission = admissibility::validate_for_consumer(&adapter["evidence"]["request"], out)
        .map_err(AdapterError::Contract)?;
    graph_adapter::validate_admitted_payload(admission.payload(), &adapter)
        .map_err(AdapterError::Contract)?;
    if admission.result()["domainVerdict"] != "observed" {
        return Err(AdapterError::Contract(
            "built-in current graph was not admitted as observed evidence".into(),
        ));
    }
    let mut output = publish_admission(
        out,
        "graph-admission.json",
        admission.result().clone(),
        &["repo_read", "local_write"],
    )?;
    output.artifacts.push(AdapterArtifact {
        artifact_schema: "code-intel-evidence-payload.v1".into(),
        artifact_type: "observed.evidence.payload".into(),
        relative_path: "graph-payload.json".into(),
        bytes: payload_bytes,
    });
    Ok(output)
}

pub(super) fn sentrux_admission(
    request: &Value,
    inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let (repo, tool_path_prefix) = sentrux_provider_options(request, inputs)?;
    let lease =
        snapshot::begin_consumption(repo, &request["snapshot"]).map_err(AdapterError::Contract)?;
    let collected_at = now()?;
    let (gate, check, capability_observations) =
        sentrux_capability_artifacts::collect_sentrux_capabilities(repo, tool_path_prefix)?;
    lease.verify_after(repo).map_err(AdapterError::Contract)?;
    let observed_at = now()?.max(collected_at);
    let identity = snapshot_identity(request)?;
    let command_observation = json!({
        "schema":"code-intel-sentrux-command-observation.v1",
        "snapshotIdentity":identity,
        "commands":[
            command_evidence("gate", &gate),
            command_evidence("check", &check)
        ]
    });
    let command_observation_bytes = serde_json::to_vec(&command_observation).map_err(|error| {
        AdapterError::Internal(format!("serialize Sentrux command observation: {error}"))
    })?;
    let rules = json!([
        command_rule("sentrux_gate", &gate),
        command_rule("sentrux_check", &check)
    ]);
    let implementation = if tool_path_prefix.is_some() {
        json!({
            "id":"sentrux.command-adapter",
            "version":"1.0.0",
            "digest":sha256_hex(include_bytes!("builtin_provider_evidence.rs"))
        })
    } else {
        json!({
            "id":sentrux_gate::ENGINE_ID,
            "version":sentrux_gate::ENGINE_VERSION,
            "digest":sha256_hex(include_bytes!("sentrux_gate.rs"))
        })
    };
    // The built-in engine analyzes in-process; only the external path spawns.
    let effects: &[&str] = if tool_path_prefix.is_some() {
        &["local_write", "process_spawn", "repo_read"]
    } else {
        &["local_write", "repo_read"]
    };
    let native = json!({
        "schema":"code-intel-sentrux-provider-native.v1",
        "status":"complete",
        "implementation":implementation,
        "rollbackIdentity":"sentrux gate/check",
        "sourceRevision":source_revision(request),
        "expectedSnapshotIdentity":identity,
        "sourceSnapshotIdentity":identity,
        "collectedAt":collected_at,
        "observedAt":observed_at,
        "declaredEffects":effects,
        "observedEffects":effects,
        "authoritativeRules":rules,
        "nativeFailure":{"kind":"none"},
        "payload":{
            "schema":"code-intel-artifact-ref.v1",
            "artifactSchema":"code-intel-evidence-payload.v1",
            "type":"observed.evidence.payload",
            "path":"sentrux-payload.json",
            "sha256":"0".repeat(64),
            "consumedSnapshotIdentity":identity
        }
    });
    let first = sentrux_adapter::translate(&native, observed_at, MAX_AGE_SECONDS)
        .map_err(AdapterError::Contract)?;
    let run_id = format!("sentrux-{identity}");
    let (capability_artifacts, capability_refs) =
        sentrux_capability_artifacts::build_capability_artifacts(
            &capability_observations,
            identity,
            &run_id,
        )?;
    let payload = json!({
        "schema":"code-intel-evidence-payload.v1",
        "data":{
            "structuralEvidence":{
                "schema":"code-intel-structural-evidence-payload.v1",
                "snapshotIdentity":identity,
                "provider":first["port"]["provider"],
                "provenance":payload_provenance(request),
                "effects":first["port"]["effects"],
                "completeness":first["port"]["completeness"],
                "rules":first["port"]["rules"]
            },
            "sentruxCapabilities":capability_observations,
            "capabilityArtifactRefs":capability_refs
        }
    });
    fs::create_dir(out)
        .map_err(|error| AdapterError::Io(format!("create Sentrux provider output: {error}")))?;
    fs::write(
        out.join("sentrux-command-observation.json"),
        &command_observation_bytes,
    )
    .map_err(|error| AdapterError::Io(format!("write Sentrux command observation: {error}")))?;
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|error| AdapterError::Internal(format!("serialize Sentrux payload: {error}")))?;
    fs::write(out.join("sentrux-payload.json"), &payload_bytes)
        .map_err(|error| AdapterError::Io(format!("write Sentrux payload: {error}")))?;
    for artifact in &capability_artifacts {
        fs::write(out.join(&artifact.relative_path), &artifact.bytes).map_err(|error| {
            AdapterError::Io(format!("write Sentrux capability artifact: {error}"))
        })?;
    }
    let mut native = native;
    native["payload"] = payload_ref("sentrux-payload.json", &payload_bytes, identity);
    let adapter = sentrux_adapter::translate(&native, observed_at, MAX_AGE_SECONDS)
        .map_err(AdapterError::Contract)?;
    let admission = admissibility::validate_for_consumer(&adapter["evidence"]["request"], out)
        .map_err(AdapterError::Contract)?;
    sentrux_adapter::validate_admitted_payload(admission.payload(), &adapter)
        .map_err(AdapterError::Contract)?;
    let mut output = publish_admission(
        out,
        "sentrux-admission.json",
        admission.result().clone(),
        effects,
    )?;
    output.artifacts.extend([
        AdapterArtifact {
            artifact_schema: "code-intel-evidence-payload.v1".into(),
            artifact_type: "observed.evidence.payload".into(),
            relative_path: "sentrux-payload.json".into(),
            bytes: payload_bytes,
        },
        AdapterArtifact {
            artifact_schema: "code-intel-sentrux-command-observation.v1".into(),
            artifact_type: "provider.sentrux.command-observation".into(),
            relative_path: "sentrux-command-observation.json".into(),
            bytes: command_observation_bytes,
        },
    ]);
    output.artifacts.extend(capability_artifacts);
    Ok(output)
}

/// Builtin compatibility route for the CodeNexus lite path: shells out to the
/// repository-owned `legacy/Invoke-CodeNexusLite.ps1` facade (the same script
/// and invocation shape `run-code-intel.ps1` already uses), then builds and
/// admits the evidence contract itself -- mirroring `sentrux_admission`'s
/// "collect raw, build the contract in Rust" split rather than delegating
/// contract construction to the script.
///
/// The script's exit/output state maps onto the three CodeNexus native
/// statuses: a clean run with a parseable `codenexus-context.json` object is
/// `current`; a clean run that produced no usable document is the defensive
/// `partial` fallback; anything that failed to spawn or exited non-zero is
/// `unavailable`. Every one of those still flows through the full two-phase
/// translate/admit pipeline -- CodeNexus absence is first-class admitted
/// evidence (domainVerdict "unknown"), not a skipped node, matching the
/// "provider_unavailable_diagnosis" contract the registry already declares.
pub(super) fn codenexus_admission(
    request: &Value,
    inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let repo = provider_repo(request, inputs, "provider.codenexus-adapt")?;
    let lease =
        snapshot::begin_consumption(repo, &request["snapshot"]).map_err(AdapterError::Contract)?;
    let collected_at = now()?;
    // Issue #275: the CodeNexus-lite context is now generated in-process by
    // the Rust implementation (`codenexus_lite`), replacing the PowerShell
    // facade `legacy/Invoke-CodeNexusLite.ps1`. The facade previously ran
    // without -DsmPath/-HotspotsPath, so it always took the largest-code-file
    // fallback path; the Rust implementation preserves that exact behavior
    // (no dsm/hotspots inputs are read).
    let implementation_digest = codenexus_lite::implementation_digest();
    let scratch_guard = ScratchDir(create_codenexus_scratch_dir()?);
    let scratch = scratch_guard.path();
    let context_path = scratch.join("codenexus-context.json");
    let document = codenexus_lite::build_context(
        repo, repo, None, None,
        // Facade defaults: -MaxFiles 8, -MaxReferencesPerFile 12,
        // -MaxCommitsPerFile 0 (as passed by this admission route).
        8, 12, 0,
    );
    let document_bytes = serde_json::to_vec(&document)
        .map_err(|error| AdapterError::Internal(format!("serialize CodeNexus context: {error}")))?;
    fs::write(&context_path, &document_bytes)
        .map_err(|error| AdapterError::Io(format!("write CodeNexus context document: {error}")))?;
    lease.verify_after(repo).map_err(AdapterError::Contract)?;
    let observed_at = now()?.max(collected_at);
    let identity = snapshot_identity(request)?;
    let (status, provider_data): (&str, Value) =
        match serde_json::from_slice::<Value>(&document_bytes) {
            Ok(document) if document.is_object() => ("current", document),
            _ => ("partial", Value::Null),
        };
    drop(scratch_guard);
    let placeholder_native = json!({
        "schema":"code-intel-codenexus-native-result.v1",
        "providerMode":"lite",
        "status":status,
        "providerId":"codenexus.lite-compat",
        "implementation":{
            "id":codenexus_lite::IMPLEMENTATION_ID,
            "version":"1.0.0",
            "digest":implementation_digest
        },
        "sourceRevision":source_revision(request),
        "expectedSnapshotIdentity":identity,
        "sourceSnapshotIdentity":identity,
        "collectedAt":collected_at,
        "observedAt":observed_at,
        "payload":{
            "schema":"code-intel-artifact-ref.v1",
            "artifactSchema":"code-intel-evidence-payload.v1",
            "type":"observed.evidence.payload",
            "path":"codenexus-payload.json",
            "sha256":"0".repeat(64),
            "consumedSnapshotIdentity":identity
        },
        "activation":"legacy_rollback",
        "effects":CODENEXUS_EFFECTS
    });
    let first = codenexus_adapter::translate(&placeholder_native, observed_at, MAX_AGE_SECONDS)
        .map_err(AdapterError::Contract)?;
    let availability = if status == "unavailable" {
        "provider_unavailable"
    } else {
        "available"
    };
    let payload = json!({
        "schema":"code-intel-evidence-payload.v1",
        "data":{"codenexus":{
            "schema":"code-intel-codenexus-evidence.v1",
            "snapshotIdentity":identity,
            "provider":first["port"]["provider"],
            "provenance":first["port"]["provenance"],
            "completeness":first["port"]["completeness"],
            "availability":availability,
            "providerData":provider_data
        }}
    });
    fs::create_dir(out)
        .map_err(|error| AdapterError::Io(format!("create CodeNexus provider output: {error}")))?;
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|error| AdapterError::Internal(format!("serialize CodeNexus payload: {error}")))?;
    fs::write(out.join("codenexus-payload.json"), &payload_bytes)
        .map_err(|error| AdapterError::Io(format!("write CodeNexus payload: {error}")))?;
    let mut native = placeholder_native;
    native["payload"] = payload_ref("codenexus-payload.json", &payload_bytes, identity);
    let adapter = codenexus_adapter::translate(&native, observed_at, MAX_AGE_SECONDS)
        .map_err(AdapterError::Contract)?;
    let admission = admissibility::validate_for_consumer(&adapter["evidence"]["request"], out)
        .map_err(AdapterError::Contract)?;
    codenexus_adapter::validate_admitted_payload(admission.payload(), &adapter)
        .map_err(AdapterError::Contract)?;
    let mut output = publish_admission(
        out,
        "codenexus-admission.json",
        admission.result().clone(),
        &["repo_read", "local_write", "process_spawn"],
    )?;
    output.artifacts.push(AdapterArtifact {
        artifact_schema: "code-intel-evidence-payload.v1".into(),
        artifact_type: "observed.evidence.payload".into(),
        relative_path: "codenexus-payload.json".into(),
        bytes: payload_bytes,
    });
    Ok(output)
}

fn sentrux_provider_options<'a>(
    request: &'a Value,
    inputs: &[VerifiedArtifact],
) -> Result<(&'a Path, Option<&'a Path>), AdapterError> {
    let [snapshot_input] = inputs else {
        return Err(AdapterError::Contract(
            "provider.sentrux-adapt requires exactly one repository.snapshot input".into(),
        ));
    };
    if snapshot_input.artifact_schema() != "code-intel-repository-snapshot.v1"
        || snapshot_input.artifact_type() != "repository.snapshot"
    {
        return Err(AdapterError::Contract(
            "provider.sentrux-adapt consumes only repository.snapshot".into(),
        ));
    }
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options
        .keys()
        .any(|key| !matches!(key.as_str(), "repoPath" | "toolPathPrefix"))
    {
        return Err(AdapterError::InvalidOptions(
            "provider.sentrux-adapt accepts only options.repoPath/toolPathPrefix".into(),
        ));
    }
    let repo = options
        .get("repoPath")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(Path::new)
        .filter(|path| path.is_dir())
        .ok_or_else(|| {
            AdapterError::InvalidOptions("options.repoPath must be a directory".into())
        })?;
    let tool_path_prefix = options
        .get("toolPathPrefix")
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty())
                .map(Path::new)
                .filter(|path| path.is_dir())
                .ok_or_else(|| {
                    AdapterError::InvalidOptions(
                        "options.toolPathPrefix must be a directory".into(),
                    )
                })
        })
        .transpose()?;
    Ok((repo, tool_path_prefix))
}

fn run_sentrux(
    repo: &Path,
    tool_path_prefix: Option<&Path>,
    subcommand: &str,
) -> Result<SentruxCommand, AdapterError> {
    let command = match tool_path_prefix {
        Some(prefix) => {
            let resolved = resolve_sentrux(prefix)?;
            let mut command = external_command(&resolved);
            let provider_subcommand = if subcommand == "provider_discovery" {
                "pro_status"
            } else {
                subcommand
            };
            let output = command
                .arg(provider_subcommand)
                .arg(".")
                .current_dir(repo)
                .output()
                .map_err(|error| {
                    AdapterError::Unavailable(format!("start Sentrux {subcommand}: {error}"))
                })?;
            SentruxCommand::from_external(output, subcommand)
        }
        None => {
            let run = match subcommand {
                "dsm" => return json_command(sentrux_analysis::analyze(repo), subcommand),
                "scan" => return json_command(sentrux_gate::scan_json(repo), subcommand),
                "rescan" => return json_command(sentrux_gate::scan_json(repo), subcommand),
                "health" => return json_command(sentrux_health_json(repo), subcommand),
                "git_stats" => {
                    return json_command(
                        sentrux_lite_capabilities::git_stats_json(repo),
                        subcommand,
                    )
                }
                "evolution" => {
                    return json_command(
                        sentrux_lite_capabilities::evolution_json(repo),
                        subcommand,
                    )
                }
                "test_gaps" => {
                    return json_command(
                        sentrux_lite_capabilities::test_gaps_json(repo),
                        subcommand,
                    )
                }
                "what_if" => {
                    // Issue #374: the DAG capability path used to call
                    // `sentrux_lite_capabilities::what_if_json`, an
                    // intentionally simplified fallback with a different
                    // shape. `sentrux_evolution::what_if` is the same real
                    // engine `legacy/run-code-intel.ps1` already calls via
                    // `code-intel sentrux what_if <path>`; its output is
                    // additionally the one `sentrux.what_if`'s only real
                    // structured-data consumer (`change_impact.rs`'s
                    // `summary.failing` read) was updated to match.
                    //
                    // The engine mirrors the PS1 tool's own wall-clock
                    // `generated_at` stamp
                    // (`legacy/Invoke-SentruxAgentTool.ps1:3090`) for
                    // CLI/PS1 consumers, but this DAG path's
                    // `evidence.sentrux` payload is content-addressed and
                    // must be a pure function of the snapshot
                    // (`evidence_payload_determinism.rs`) -- so this path
                    // nulls the timestamp out of its own copy before it
                    // reaches `json_command`, mirroring how
                    // `build_capability_artifacts` already nulls
                    // `freshness.evaluatedAt` for the same reason.
                    // `code-intel sentrux what_if <path>` (`sentrux.rs`)
                    // still returns the real, timestamped value.
                    return json_command(
                        sentrux_evolution::what_if(repo).map(|mut document| {
                            document["generated_at"] = Value::Null;
                            document
                        }),
                        subcommand,
                    );
                }
                "provider_discovery" => {
                    return json_command(
                        sentrux_lite_capabilities::provider_discovery_json(),
                        subcommand,
                    )
                }
                "check_rules" => sentrux_gate::run_check(repo),
                "check" => sentrux_gate::run_check_aligned(repo, true),
                "gate" => sentrux_gate::run_gate(repo, false),
                other => {
                    return Err(AdapterError::Internal(format!(
                        "unsupported built-in Sentrux subcommand: {other}"
                    )))
                }
            }
            .map_err(AdapterError::Internal)?;
            SentruxCommand::from_native(run, subcommand)
        }
    };
    Ok(command)
}

fn json_command(
    value: Result<Value, String>,
    subcommand: &str,
) -> Result<SentruxCommand, AdapterError> {
    let value =
        value.map_err(|error| AdapterError::Internal(format!("Sentrux {subcommand}: {error}")))?;
    let stdout = serde_json::to_string_pretty(&value).map_err(|error| {
        AdapterError::Internal(format!("serialize Sentrux {subcommand}: {error}"))
    })?;
    Ok(SentruxCommand::from_json(stdout.into_bytes(), subcommand))
}

fn sentrux_health_json(repo: &Path) -> Result<Value, String> {
    let metrics = sentrux_gate::scan_json(repo)?;
    let bottleneck = metrics["quality_signal_detail"]["bottleneck"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    Ok(json!({
        "status":"ok",
        "tool":sentrux_gate::ENGINE_ID,
        "quality_signal":metrics["quality_signal"],
        "files":metrics["files"],
        "bottleneck":bottleneck,
        "root_causes":metrics["quality_signal_detail"]["root_causes"],
    }))
}

fn external_command(path: &Path) -> Command {
    #[cfg(windows)]
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat"))
    {
        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c"]).arg(path);
        return command;
    }
    Command::new(path)
}

fn resolve_sentrux(prefix: &Path) -> Result<PathBuf, AdapterError> {
    ["sentrux.exe", "sentrux.cmd", "sentrux.bat", "sentrux"]
        .iter()
        .map(|name| prefix.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            AdapterError::Unavailable(format!(
                "Sentrux executable is absent from options.toolPathPrefix: {}",
                prefix.display()
            ))
        })
}

fn command_rule(kind: &str, command: &SentruxCommand) -> Value {
    let verdict = if command.success || !command.governed {
        "pass"
    } else {
        "fail"
    };
    let mut rule =
        json!({"kind":kind,"status":"evaluated","verdict":verdict,"failure":{"kind":"none"}});
    if verdict == "fail" && !command.violations.is_empty() {
        rule["details"] = json!({"violations":command.violations.iter().map(Violation::to_json).collect::<Vec<_>>()});
    }
    rule
}

fn provider_repo<'a>(
    request: &'a Value,
    inputs: &[VerifiedArtifact],
    capability: &str,
) -> Result<&'a Path, AdapterError> {
    let [snapshot_input] = inputs else {
        return Err(AdapterError::Contract(format!(
            "{capability} requires exactly one repository.snapshot input"
        )));
    };
    if snapshot_input.artifact_schema() != "code-intel-repository-snapshot.v1"
        || snapshot_input.artifact_type() != "repository.snapshot"
    {
        return Err(AdapterError::Contract(format!(
            "{capability} consumes only repository.snapshot"
        )));
    }
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options.len() != 1 || !options.contains_key("repoPath") {
        return Err(AdapterError::InvalidOptions(format!(
            "{capability} accepts only options.repoPath"
        )));
    }
    options["repoPath"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(Path::new)
        .filter(|path| path.is_dir())
        .ok_or_else(|| AdapterError::InvalidOptions("options.repoPath must be a directory".into()))
}

fn snapshot_identity(request: &Value) -> Result<&str, AdapterError> {
    request["snapshot"]["identity"]
        .as_str()
        .ok_or_else(|| AdapterError::Contract("request snapshot identity is missing".into()))
}

fn source_revision(request: &Value) -> &str {
    request["snapshot"]["head"].as_str().unwrap_or("unknown")
}

/// Provenance for the *content-addressed evidence payload*, which is not the
/// same thing as the port/admission provenance.
///
/// The payload's bytes are its identity: `payload.sha256` is what the
/// admission envelope references and what the publication layer dedupes on.
/// Stamping the collection wall-clock into those bytes made every payload a
/// fresh object on every run -- a 1.9 MB architecture-graph payload was
/// re-published unchanged once per second-of-difference, and its digest churn
/// propagated into `admissionIdentity`. So the payload carries only what the
/// snapshot determines, and `observedAt` stays in the native result, the
/// provider port and the A04 observation, which is where the freshness policy
/// actually reads it (`admissibility::validate_sealed`).
fn payload_provenance(request: &Value) -> Value {
    json!({"sourceRevision": source_revision(request)})
}

fn now() -> Result<u64, AdapterError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| AdapterError::Internal(format!("read provider clock: {error}")))
}

fn payload_ref(path: &str, bytes: &[u8], identity: &str) -> Value {
    json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-evidence-payload.v1",
        "type":"observed.evidence.payload",
        "path":path,
        "sha256":sha256_hex(bytes),
        "consumedSnapshotIdentity":identity
    })
}

fn publish_admission(
    out: &Path,
    file_name: &str,
    admission: Value,
    effects: &[&str],
) -> Result<AdapterOutput, AdapterError> {
    let domain_verdict = match admission["domainVerdict"].as_str() {
        Some("observed") => AdapterDomainVerdict::Pass,
        Some("unknown") => AdapterDomainVerdict::Unknown,
        Some("not_applicable") => AdapterDomainVerdict::NotApplicable,
        Some("fail") => AdapterDomainVerdict::Fail,
        other => {
            return Err(AdapterError::Contract(format!(
                "evidence admission has unsupported domain verdict: {other:?}"
            )))
        }
    };
    let bytes = serde_json::to_vec(&admission).map_err(|error| {
        AdapterError::Internal(format!("serialize evidence admission: {error}"))
    })?;
    fs::write(out.join(file_name), &bytes)
        .map_err(|error| AdapterError::Io(format!("write evidence admission: {error}")))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-evidence-admissibility-result.v1".into(),
            artifact_type: "evidence.admission".into(),
            relative_path: file_name.into(),
            bytes,
        }],
        observed_effects: effects.iter().map(|effect| (*effect).to_string()).collect(),
        domain_verdict,
        domain_failure: None,
    })
}
