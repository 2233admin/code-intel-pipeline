use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use super::{AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;
use crate::capability::sha256_hex;

// Included by path rather than imported from the crate root: several
// integration tests pull this adapter into their own crate via `#[path]`, and
// those roots do not declare the binary's module list. Same convention the
// adapter already used for `tool_path`.
//
// `allow(dead_code)` sits here rather than on the individual items because it
// is this inclusion that makes them unreachable: the adapter needs only
// `Options`, `observe` and `BOOTSTRAP_SCHEMA`, so the CLI surface (`run_raw`,
// `render_human`, `pipeline_root`, …) has no caller in this copy. `main.rs`
// declares the same module without the allowance, so genuinely dead code still
// warns where the module is actually the binary's.
#[allow(dead_code)]
#[path = "doctor_bootstrap/mod.rs"]
mod doctor_bootstrap;

// Same reason: included by path so the integration-test crate roots that pull
// this adapter in still resolve it.
#[path = "doctor_provider_rows.rs"]
mod doctor_provider_rows;

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    validate_snapshot_input(request, verified_inputs)?;
    let options = Options::parse(request)?;
    let bootstrap = run_bootstrap(&options)?;
    let manifest = validate_manifest(&options.manifest_path)?;
    let document = adapt(request, &options, &bootstrap, &manifest)?;
    let domain_failure = diagnosis(&document, &bootstrap);
    let domain_verdict = if domain_failure.is_some() {
        AdapterDomainVerdict::Fail
    } else {
        AdapterDomainVerdict::Pass
    };
    let bytes = serde_json::to_vec(&document).map_err(|error| {
        AdapterError::Internal(format!("serialize doctor observation: {error}"))
    })?;
    publish(out, "doctor-observation.json", &bytes)?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-doctor-observation.v1".into(),
            artifact_type: "doctor.observation".into(),
            relative_path: "doctor-observation.json".into(),
            bytes,
        }],
        observed_effects: vec!["repo_read".into(), "local_write".into()],
        domain_verdict,
        domain_failure,
    })
}

struct Options {
    repo_path: PathBuf,
    config_path: Option<PathBuf>,
    manifest_path: PathBuf,
    platform: String,
    require_repowise: bool,
    require_understand: bool,
    tool_path_prefix: Option<PathBuf>,
}

impl Options {
    fn parse(request: &Value) -> Result<Self, AdapterError> {
        let options = request["options"].as_object().ok_or_else(|| {
            AdapterError::InvalidOptions("doctor options must be an object".into())
        })?;
        let allowed = [
            "repoPath",
            "configPath",
            "manifestPath",
            "platform",
            "requireRepowise",
            "requireUnderstand",
            "toolPathPrefix",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        if options.keys().any(|key| !allowed.contains(key.as_str())) {
            return Err(AdapterError::InvalidOptions(
                "doctor accepts only repoPath/configPath/manifestPath/platform/requireRepowise/requireUnderstand/toolPathPrefix"
                    .into(),
            ));
        }
        let repo_path = required_existing_directory(options.get("repoPath"), "options.repoPath")?;
        let config_path = optional_existing_file(options.get("configPath"), "options.configPath")?;
        let manifest_path = match options.get("manifestPath") {
            Some(value) => required_existing_file(Some(value), "options.manifestPath")?,
            None => pipeline_root()
                .join("orchestration")
                .join("integrations.json"),
        };
        if !manifest_path.is_file() {
            return Err(AdapterError::InvalidOptions(format!(
                "options.manifestPath is not a file: {}",
                manifest_path.display()
            )));
        }
        let platform = options
            .get("platform")
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| matches!(*value, "auto" | "windows" | "macos" | "linux"))
                    .map(str::to_string)
                    .ok_or_else(|| {
                        AdapterError::InvalidOptions(
                            "options.platform must be auto/windows/macos/linux".into(),
                        )
                    })
            })
            .transpose()?
            .unwrap_or_else(|| "auto".into());
        Ok(Self {
            repo_path,
            config_path,
            manifest_path,
            platform,
            require_repowise: optional_bool(
                options.get("requireRepowise"),
                true,
                "requireRepowise",
            )?,
            require_understand: optional_bool(
                options.get("requireUnderstand"),
                false,
                "requireUnderstand",
            )?,
            tool_path_prefix: options
                .get("toolPathPrefix")
                .map(|value| required_existing_directory(Some(value), "options.toolPathPrefix"))
                .transpose()?,
        })
    }
}

fn validate_snapshot_input(
    request: &Value,
    inputs: &[VerifiedArtifact],
) -> Result<(), AdapterError> {
    if inputs.len() != 1
        || inputs[0].artifact_schema() != "code-intel-repository-snapshot.v1"
        || inputs[0].artifact_type() != "repository.snapshot"
    {
        return Err(AdapterError::Contract(
            "doctor requires exactly one verified repository.snapshot Artifact Ref".into(),
        ));
    }
    let snapshot: Value = serde_json::from_slice(inputs[0].bytes()).map_err(|error| {
        AdapterError::Contract(format!(
            "verified repository snapshot is invalid JSON: {error}"
        ))
    })?;
    if snapshot.pointer("/snapshot/identity") != request.pointer("/snapshot/identity") {
        return Err(AdapterError::Contract(
            "doctor repository.snapshot payload differs from request snapshot".into(),
        ));
    }
    Ok(())
}

/// Run the bootstrap probe in-process.
///
/// Before T3 this shelled out to `legacy/check-code-intel-tools.ps1` and fell
/// back to a hand-written stub when `pwsh` or the script was absent — a stub
/// that answered `graphProvider` with hardcoded `true`s and so could not
/// report the very drift doctor exists to catch. The probe is native now, so
/// there is one implementation, no `pwsh` dependency on the kernel path, and
/// the same answers on every platform.
fn run_bootstrap(options: &Options) -> Result<Value, AdapterError> {
    let mut probe = doctor_bootstrap::Options::new(pipeline_root());
    probe.repo_path = Some(options.repo_path.to_string_lossy().into_owned());
    probe.config = options.config_path.clone();
    probe.platform = options.platform.clone();
    probe.require_repowise = options.require_repowise;
    probe.require_understand = options.require_understand;
    probe.tool_path_prefix = options.tool_path_prefix.clone();
    let value = doctor_bootstrap::observe(&probe)
        .map_err(|error| AdapterError::InvalidOptions(format!("doctor bootstrap: {error}")))?;
    // Kept as an explicit boundary check rather than an invariant assumed from
    // the call above: the adapter must never publish an observation that does
    // not carry the non-authoritative v1 contract, whoever produced it.
    if value["schema"] != doctor_bootstrap::BOOTSTRAP_SCHEMA
        || value["authority"] != "observation_only"
        || !value["ok"].is_boolean()
    {
        return Err(AdapterError::Contract(
            "doctor bootstrap observation lacks the non-authoritative v1 contract".into(),
        ));
    }
    Ok(value)
}

fn validate_manifest(path: &Path) -> Result<Value, AdapterError> {
    let binary = std::env::current_exe()
        .map_err(|error| AdapterError::Io(format!("locate code-intel executable: {error}")))?;
    let output = Command::new(binary)
        .args(["orchestrate", "--action", "Validate", "--manifest"])
        .arg(path)
        .arg("--json")
        .output()
        .map_err(|error| {
            AdapterError::Unavailable(format!("run manifest reconciliation: {error}"))
        })?;
    serde_json::from_slice(&output.stdout).map_err(|error| {
        AdapterError::Contract(format!(
            "manifest reconciliation stdout is not one JSON document: {error}"
        ))
    })
}

fn adapt(
    request: &Value,
    options: &Options,
    raw: &Value,
    manifest: &Value,
) -> Result<Value, AdapterError> {
    let tools = raw
        .pointer("/checks/tools")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AdapterError::Contract("doctor bootstrap checks.tools must be an array".into())
        })?;
    let tool_observations = tools
        .iter()
        .map(|tool| {
            json!({
                "name": string(tool, "name"),
                "required": boolean(tool, "required"),
                "presence": if boolean(tool, "found") { "present" } else { "missing" },
                "readiness": if boolean(tool, "found") { "ready" } else { "unavailable" },
                "conformance": "not_evaluated",
                "admissibility": "not_evaluated"
            })
        })
        .collect::<Vec<_>>();
    let policy = json!({
        "platform": options.platform,
        "requireRepowise": options.require_repowise,
        "requireUnderstand": options.require_understand
    });
    let policy_bytes = serde_json::to_vec(&policy)
        .map_err(|error| AdapterError::Internal(format!("serialize doctor policy: {error}")))?;
    let registry_ok = manifest
        .pointer("/registryAudit/ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let manifest_ok = manifest["ok"].as_bool().unwrap_or(false) && registry_ok;
    Ok(json!({
        "schema":"code-intel-doctor-observation.v1",
        "snapshotIdentity":request["snapshot"]["identity"],
        "environmentPolicy":{"policy":policy,"sha256":sha256_hex(&policy_bytes)},
        "bootstrap":{"schema":raw["schema"],"authority":"observation_only","ready":raw["ok"]},
        "repository":{"presence":if raw.pointer("/checks/repo/exists").and_then(Value::as_bool).unwrap_or(false) { "present" } else { "missing" },"readiness":if raw.pointer("/checks/repo/exists").and_then(Value::as_bool).unwrap_or(false) { "ready" } else { "unavailable" },"conformance":"not_evaluated","admissibility":"not_evaluated"},
        "tools":tool_observations,
        "providers":doctor_provider_rows::provider_rows(raw),
        "manifest":{"reconciled":manifest_ok,"registryReconciled":registry_ok,"findingCount":manifest["errors"].as_array().map_or(0, Vec::len)},
        "diagnostics":{"bootstrapReady":raw["ok"],"manifestReady":manifest_ok},
        "engineeringFacts":[]
    }))
}

fn diagnosis(document: &Value, raw: &Value) -> Option<String> {
    let bootstrap_ready = document
        .pointer("/diagnostics/bootstrapReady")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let manifest_ready = document
        .pointer("/diagnostics/manifestReady")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let nonconforming = document["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|provider| provider["conformance"] == "nonconforming")
        .filter_map(|provider| provider["id"].as_str())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut causes = Vec::<String>::new();
    if !bootstrap_ready {
        causes.push("bootstrap readiness failed".into());
    }
    if !nonconforming.is_empty() {
        let details = nonconforming
            .iter()
            .map(|provider| format!("{provider}: {}", provider_probe_output(raw, provider)))
            .collect::<Vec<_>>()
            .join("; ");
        causes.push(format!("provider conformance failed: {details}"));
    }
    if !manifest_ready {
        causes.push("manifest reconciliation failed".into());
    }
    (!causes.is_empty()).then(|| format!("doctor diagnosis: {}", causes.join("; ")))
}

fn provider_probe_output(raw: &Value, provider: &str) -> String {
    let outputs = match provider {
        "sentrux" => [
            raw.pointer("/checks/sentrux/core/output")
                .and_then(Value::as_str),
            raw.pointer("/checks/sentrux/pro/output")
                .and_then(Value::as_str),
        ],
        _ => [None, None],
    };
    let output = outputs
        .into_iter()
        .flatten()
        .filter(|output| !output.trim().is_empty())
        .map(|output| redact_probe_output(output.trim()))
        .collect::<Vec<_>>()
        .join(" | ");
    if output.is_empty() {
        "probe emitted no output".into()
    } else {
        output
    }
}

fn redact_probe_output(output: &str) -> String {
    let mut redacted = Vec::new();
    let mut authorization_scheme = false;
    let mut authorization_value = false;
    for token in output.split_whitespace() {
        if authorization_value {
            redacted.push("<redacted>".to_string());
            authorization_value = false;
            continue;
        }
        if authorization_scheme {
            redacted.push(token.to_string());
            authorization_scheme = false;
            authorization_value = token.eq_ignore_ascii_case("bearer");
            continue;
        }
        if token.eq_ignore_ascii_case("authorization:") {
            redacted.push(token.to_string());
            authorization_scheme = true;
            continue;
        }
        if let Some((key, _)) = token.split_once('=') {
            if matches!(
                key.to_ascii_lowercase().as_str(),
                "password" | "token" | "secret"
            ) {
                redacted.push(format!("{key}=<redacted>"));
                continue;
            }
        }
        redacted.push(token.to_string());
    }
    redacted.join(" ")
}

fn publish(out: &Path, relative: &str, bytes: &[u8]) -> Result<(), AdapterError> {
    fs::create_dir(out).map_err(|error| {
        AdapterError::Io(format!(
            "exclusive doctor staging create {}: {error}",
            out.display()
        ))
    })?;
    let path = out.join(relative);
    fs::write(&path, bytes).map_err(|error| {
        AdapterError::Io(format!("write doctor artifact {}: {error}", path.display()))
    })
}

fn pipeline_root() -> PathBuf {
    crate::capability::discover_manifest(None)
        .and_then(|manifest| manifest.parent()?.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."))
}

fn required_existing_directory(value: Option<&Value>, name: &str) -> Result<PathBuf, AdapterError> {
    let path = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| AdapterError::InvalidOptions(format!("{name} must be a non-empty path")))?;
    if !path.is_dir() {
        return Err(AdapterError::InvalidOptions(format!(
            "{name} is not a directory: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn required_existing_file(value: Option<&Value>, name: &str) -> Result<PathBuf, AdapterError> {
    let path = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| AdapterError::InvalidOptions(format!("{name} must be a non-empty path")))?;
    if !path.is_file() {
        return Err(AdapterError::InvalidOptions(format!(
            "{name} is not a file: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn optional_existing_file(
    value: Option<&Value>,
    name: &str,
) -> Result<Option<PathBuf>, AdapterError> {
    value
        .map(|value| required_existing_file(Some(value), name))
        .transpose()
}

fn optional_bool(value: Option<&Value>, default: bool, name: &str) -> Result<bool, AdapterError> {
    value
        .map(|value| {
            value.as_bool().ok_or_else(|| {
                AdapterError::InvalidOptions(format!("options.{name} must be boolean"))
            })
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn string<'a>(value: &'a Value, field: &str) -> &'a str {
    value.get(field).and_then(Value::as_str).unwrap_or("")
}

fn boolean(value: &Value, field: &str) -> bool {
    value.get(field).and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Options {
        Options {
            repo_path: PathBuf::from("fixture"),
            config_path: None,
            manifest_path: PathBuf::from("integrations.json"),
            platform: "windows".into(),
            require_repowise: true,
            require_understand: false,
            tool_path_prefix: None,
        }
    }

    fn request() -> Value {
        json!({"snapshot":{"identity":"a".repeat(64)}})
    }

    fn bootstrap() -> Value {
        json!({
            "schema":"code-intel-doctor-bootstrap-observation.v1",
            "authority":"observation_only",
            "ok":false,
            "checks":{
                "repo":{"exists":true,"path":"C:/secret/repo"},
                "tools":[
                    {"name":"rg","required":true,"found":true,"source":"C:/secret/bin/rg.exe"},
                    {"name":"repowise","required":true,"found":true,"source":"C:/secret/bin/repowise.exe"},
                    {"name":"sentrux","required":true,"found":true,"source":"C:/secret/bin/sentrux.exe"}
                ],
                "sentrux":{
                    "core":{"found":false,"output":"Authorization: Bearer super-secret-token"},
                    "pro":{"found":true,"output":"password=hunter2"}
                },
                "graphProvider":{"sourceFound":true,"cargoFound":true,"binaryFound":true,"command":"secret"}
            }
        })
    }

    #[test]
    fn adaptation_separates_health_boundaries_redacts_and_fails_domain() {
        let manifest = json!({"ok":false,"errors":["drift"],"registryAudit":{"ok":false}});
        let document = adapt(&request(), &options(), &bootstrap(), &manifest).unwrap();
        let text = serde_json::to_string(&document).unwrap();

        assert_eq!(document["providers"][0]["presence"], "present");
        assert_eq!(document["providers"][0]["conformance"], "not_evaluated");
        assert_eq!(document["providers"][0]["admissibility"], "not_evaluated");
        assert_eq!(document["providers"][1]["presence"], "present");
        assert_eq!(document["providers"][1]["conformance"], "nonconforming");
        assert_eq!(document["manifest"]["reconciled"], false);
        assert_eq!(document["engineeringFacts"], json!([]));
        assert!(!text.contains("super-secret-token"));
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("C:/secret"));
        let diagnosis = diagnosis(&document, &bootstrap()).expect("diagnosis");
        assert!(diagnosis.contains("sentrux"));
        assert!(diagnosis.contains("Authorization: Bearer <redacted>"));
        assert!(!diagnosis.contains("super-secret-token"));
    }

    #[test]
    fn adaptation_is_byte_replayable_for_the_same_observations_and_policy() {
        let manifest = json!({"ok":true,"errors":[],"registryAudit":{"ok":true}});
        let left =
            serde_json::to_vec(&adapt(&request(), &options(), &bootstrap(), &manifest).unwrap())
                .unwrap();
        let right =
            serde_json::to_vec(&adapt(&request(), &options(), &bootstrap(), &manifest).unwrap())
                .unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn registry_toolchain_digests_bind_the_adapter_and_dispatch_sources() {
        let root = pipeline_root();
        let registry: Value = serde_json::from_slice(
            &fs::read(root.join("orchestration/integrations.json")).unwrap(),
        )
        .unwrap();
        let declaration = registry["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|integration| {
                integration.pointer("/capabilityDeclaration/id") == Some(&json!("doctor"))
            })
            .unwrap();
        let declared = declaration["capabilityDeclaration"]["implementation"]["toolchainDigests"]
            .as_array()
            .unwrap();
        let inputs: Vec<&str> = declaration["toolchainDigestEvidence"]["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| input.as_str().unwrap())
            .collect();
        // The CI atomic-capability contract checks inputs[i] <-> digests[i]
        // pairing; assert the same here so a membership-only local run can no
        // longer pass a misaligned declaration (PR #200 caught this only in
        // CI — the local face of #176's checklist blind spot).
        assert_eq!(
            inputs.len(),
            declared.len(),
            "toolchainDigestEvidence.inputs and toolchainDigests must pair by index"
        );
        for relative in [
            "crates/code-intel-cli/src/doctor_adapter.rs",
            "crates/code-intel-cli/src/doctor_bootstrap/mod.rs",
            "crates/code-intel-cli/src/doctor_bootstrap/config.rs",
            "crates/code-intel-cli/src/doctor_bootstrap/identity.rs",
            "crates/code-intel-cli/src/doctor_bootstrap/paths.rs",
            "crates/code-intel-cli/src/doctor_bootstrap/probe.rs",
            "crates/code-intel-cli/src/capability_inventory.rs",
            "crates/code-intel-cli/src/doctor_provider_rows.rs",
        ] {
            assert!(
                inputs.contains(&relative),
                "digest-bound source missing from toolchainDigestEvidence.inputs: {relative}"
            );
        }
        for (index, relative) in inputs.iter().enumerate() {
            let actual = sha256_hex(&fs::read(root.join(relative)).unwrap());
            assert_eq!(
                declared[index],
                json!(actual),
                "stale doctor toolchain digest for {relative}"
            );
        }
    }
}
