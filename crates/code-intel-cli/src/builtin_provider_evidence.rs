use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::{AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;
use crate::capability::sha256_hex;
use crate::snapshot;

#[path = "admissibility.rs"]
mod admissibility;
#[path = "graph.rs"]
mod graph;
#[path = "graph_adapter.rs"]
mod graph_adapter;
#[path = "sentrux_adapter.rs"]
mod sentrux_adapter;
const MAX_AGE_SECONDS: u64 = 300;
const MAX_COMMAND_EVIDENCE_BYTES: usize = 1024 * 1024;

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
            "provenance":{
                "sourceRevision":source_revision(request),
                "observedAt":observed_at
            },
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
            "digest":sha256_hex(include_bytes!("graph.rs"))
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
    let gate = run_sentrux(repo, tool_path_prefix, "gate")?;
    let check = run_sentrux(repo, tool_path_prefix, "check")?;
    lease.verify_after(repo).map_err(AdapterError::Contract)?;
    let observed_at = now()?.max(collected_at);
    let identity = snapshot_identity(request)?;
    let command_observation = json!({
        "schema":"code-intel-sentrux-command-observation.v1",
        "snapshotIdentity":identity,
        "commands":[command_evidence("gate", &gate), command_evidence("check", &check)]
    });
    let command_observation_bytes = serde_json::to_vec(&command_observation).map_err(|error| {
        AdapterError::Internal(format!("serialize Sentrux command observation: {error}"))
    })?;
    let rules = json!([
        command_rule("sentrux_gate", gate.status.success()),
        command_rule("sentrux_check", check.status.success())
    ]);
    let native = json!({
        "schema":"code-intel-sentrux-provider-native.v1",
        "status":"complete",
        "implementation":{
            "id":"sentrux.command-adapter",
            "version":"1.0.0",
            "digest":sha256_hex(include_bytes!("builtin_provider_evidence.rs"))
        },
        "rollbackIdentity":"sentrux gate/check",
        "sourceRevision":source_revision(request),
        "expectedSnapshotIdentity":identity,
        "sourceSnapshotIdentity":identity,
        "collectedAt":collected_at,
        "observedAt":observed_at,
        "declaredEffects":["local_write","process_spawn","repo_read"],
        "observedEffects":["local_write","process_spawn","repo_read"],
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
    let payload = json!({
        "schema":"code-intel-evidence-payload.v1",
        "data":{"structuralEvidence":{
            "schema":"code-intel-structural-evidence-payload.v1",
            "snapshotIdentity":identity,
            "provider":first["port"]["provider"],
            "provenance":first["port"]["provenance"],
            "effects":first["port"]["effects"],
            "completeness":first["port"]["completeness"],
            "rules":first["port"]["rules"]
        }}
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
        &["repo_read", "local_write", "process_spawn"],
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
) -> Result<Output, AdapterError> {
    let explicit = match tool_path_prefix {
        Some(prefix) => Some(resolve_sentrux(prefix)?),
        None => resolve_sentrux_from_path(),
    };
    let mut command = match explicit.as_deref() {
        #[cfg(windows)]
        Some(path)
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| {
                    matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat")
                }) =>
        {
            let mut command = Command::new("cmd.exe");
            command.args(["/d", "/c"]).arg(path);
            command
        }
        Some(path) => Command::new(path),
        None => Command::new("sentrux"),
    };
    let output = command
        .arg(subcommand)
        .arg(".")
        .current_dir(repo)
        .output()
        .map_err(|error| {
            AdapterError::Unavailable(format!("start Sentrux {subcommand}: {error}"))
        })?;
    if output.stdout.len() > MAX_COMMAND_EVIDENCE_BYTES
        || output.stderr.len() > MAX_COMMAND_EVIDENCE_BYTES
    {
        return Err(AdapterError::Contract(format!(
            "Sentrux {subcommand} output exceeds the bounded evidence limit"
        )));
    }
    Ok(output)
}

fn resolve_sentrux(prefix: &Path) -> Result<PathBuf, AdapterError> {
    sentrux_names()
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

#[cfg(windows)]
fn resolve_sentrux_from_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    resolve_sentrux_in_directories(std::env::split_paths(&path))
}

#[cfg(not(windows))]
fn resolve_sentrux_from_path() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn resolve_sentrux_in_directories(
    directories: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    directories.into_iter().find_map(|directory| {
        sentrux_names()
            .iter()
            .map(|name| directory.join(name))
            .find(|path| path.is_file())
    })
}

fn sentrux_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["sentrux.exe", "sentrux.cmd", "sentrux.bat", "sentrux"]
    } else {
        &["sentrux"]
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::resolve_sentrux_in_directories;
    use std::{fs, process, time::SystemTime};

    #[test]
    fn path_resolution_includes_windows_command_shims() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("test clock is after the Unix epoch")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("code-intel-sentrux-path-{}-{nonce}", process::id()));
        fs::create_dir_all(&root).expect("create Sentrux PATH fixture");
        let shim = root.join("sentrux.cmd");
        fs::write(&shim, b"@echo off\r\nexit /b 0\r\n").expect("write Sentrux command shim");

        let resolved = resolve_sentrux_in_directories([root.clone()]);
        assert_eq!(resolved.as_deref(), Some(shim.as_path()));

        fs::remove_dir_all(root).expect("remove Sentrux PATH fixture");
    }
}

fn command_evidence(subcommand: &str, output: &Output) -> Value {
    json!({
        "id":subcommand,
        "argv":["sentrux",subcommand,"."],
        "exitCode":output.status.code(),
        "success":output.status.success(),
        "stdout":String::from_utf8_lossy(&output.stdout),
        "stderr":String::from_utf8_lossy(&output.stderr)
    })
}

fn command_rule(kind: &str, pass: bool) -> Value {
    json!({
        "kind":kind,
        "status":"evaluated",
        "verdict":if pass { "pass" } else { "fail" },
        "failure":{"kind":"none"}
    })
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
