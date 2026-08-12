use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::adapter_contract::{AdapterArtifact, AdapterDomainVerdict, AdapterError, AdapterOutput};
use crate::artifact_ref::VerifiedArtifact;
use crate::capability::sha256_hex;

const FINDING_SCHEMA: &str = "code-intel-dogfood-finding.v1";
const FINDING_TYPE: &str = "verification.dogfood-finding";

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match build(raw) {
        Ok(finding) => {
            let out = raw
                .windows(2)
                .find(|pair| pair[0] == "--out")
                .map(|pair| PathBuf::from(&pair[1]));
            let bytes = serde_json::to_vec_pretty(&finding).unwrap();
            if let Some(out) = out {
                if let Err(message) = write_new_artifact(&out, &bytes) {
                    eprintln!("{message}");
                    return 74;
                }
            } else {
                println!("{}", String::from_utf8_lossy(&bytes));
            }
            0
        }
        Err(message) => {
            eprintln!("{message}");
            64
        }
    }
}

fn build(raw: &[String]) -> Result<Value, String> {
    let (run_path, target, _) = parse_args(raw)?;
    let run = read_json(&run_path, "run commit")?;
    let manifest = read_referenced(&run, "manifest", &run_path)?;
    let hospital_node = manifest
        .pointer("/nodes/diagnosis.hospital")
        .ok_or_else(|| "run manifest lacks diagnosis.hospital".to_string())?;
    let hospital_ref = hospital_node["artifacts"]
        .as_array()
        .and_then(|artifacts| {
            artifacts
                .iter()
                .find(|artifact| artifact["type"] == "diagnosis.hospital")
        })
        .ok_or_else(|| "hospital node lacks diagnosis.hospital artifact".to_string())?;
    let hospital = read_referenced_value(hospital_ref, &run_path)?;
    build_finding(
        &run,
        &hospital,
        &target,
        &sha256_hex(&fs::read(&run_path).map_err(|error| error.to_string())?),
        hospital_ref["sha256"]
            .as_str()
            .ok_or_else(|| "hospital artifact digest is missing".to_string())?,
    )
}

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options.len() != 1 || !options.contains_key("target") {
        return Err(AdapterError::InvalidOptions(
            "dogfood.finding accepts only options.target".into(),
        ));
    }
    let target = options["target"]
        .as_str()
        .ok_or_else(|| AdapterError::InvalidOptions("options.target must be a string".into()))?;
    if !matches!(
        target,
        "code-intel-pipeline" | "reverse-skill-evolver" | "tdxcli-rs"
    ) {
        return Err(AdapterError::InvalidOptions(
            "options.target is not a supported repository".into(),
        ));
    }
    let find = |schema: &str, artifact_type: &str| {
        verified_inputs.iter().find(|artifact| {
            artifact.artifact_schema() == schema && artifact.artifact_type() == artifact_type
        })
    };
    let run_artifact = find("code-intel-run-commit.v1", "run.commit").ok_or_else(|| {
        AdapterError::Contract("dogfood finding requires a run.commit input".into())
    })?;
    let manifest_artifact =
        find("code-intel-run-manifest.v1", "run.manifest").ok_or_else(|| {
            AdapterError::Contract("dogfood finding requires a run.manifest input".into())
        })?;
    let hospital_artifact =
        find("code-intel-hospital.v1", "diagnosis.hospital").ok_or_else(|| {
            AdapterError::Contract("dogfood finding requires a diagnosis.hospital input".into())
        })?;
    let run: Value = serde_json::from_slice(run_artifact.bytes())
        .map_err(|error| AdapterError::Contract(format!("run commit is invalid JSON: {error}")))?;
    let manifest: Value = serde_json::from_slice(manifest_artifact.bytes()).map_err(|error| {
        AdapterError::Contract(format!("run manifest is invalid JSON: {error}"))
    })?;
    let hospital: Value = serde_json::from_slice(hospital_artifact.bytes()).map_err(|error| {
        AdapterError::Contract(format!("hospital report is invalid JSON: {error}"))
    })?;
    if run["manifest"]["sha256"] != manifest_artifact.sha256() {
        return Err(AdapterError::Contract(
            "run commit does not bind the supplied run manifest".into(),
        ));
    }
    let expected_hospital = manifest
        .pointer("/nodes/diagnosis.hospital/artifacts")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["type"] == "diagnosis.hospital")
        })
        .ok_or_else(|| {
            AdapterError::Contract("run manifest lacks diagnosis hospital ref".into())
        })?;
    if expected_hospital["sha256"] != hospital_artifact.sha256() {
        return Err(AdapterError::Contract(
            "run manifest does not bind the supplied hospital report".into(),
        ));
    }
    let finding = build_finding(
        &run,
        &hospital,
        target,
        run_artifact.sha256(),
        hospital_artifact.sha256(),
    )
    .map_err(AdapterError::Contract)?;
    let bytes = serde_json::to_vec_pretty(&finding)
        .map_err(|error| AdapterError::Internal(format!("serialize finding: {error}")))?;
    publish_new_artifact(out, "dogfood-finding.json", &bytes)?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: FINDING_SCHEMA.into(),
            artifact_type: FINDING_TYPE.into(),
            relative_path: "dogfood-finding.json".into(),
            bytes,
        }],
        observed_effects: vec!["local_write".into()],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn build_finding(
    run: &Value,
    hospital: &Value,
    target: &str,
    run_commit_sha256: &str,
    hospital_sha256: &str,
) -> Result<Value, String> {
    let message = hospital
        .pointer("/treatment/failing_rules/0/message")
        .and_then(Value::as_str)
        .ok_or_else(|| "hospital diagnosis lacks a failing rule message".to_string())?;
    if hospital
        .pointer("/treatment/failing_rules/0/id")
        .and_then(Value::as_str)
        != Some("sentrux_gate")
        || !message.contains("missing")
        || !message.contains("baseline.json")
    {
        return Err("unsupported hospital diagnosis for dogfood finding".into());
    }

    Ok(json!({
        "schema": FINDING_SCHEMA,
        "status": "candidate",
        "classification": "tooling_gap",
        "suggested_action": "improve_onboarding",
        "target": {"repository": target},
        "evidence": {
            "runCommitSha256": run_commit_sha256,
            "snapshotIdentity": run["snapshotIdentity"],
            "runIdentity": run["runIdentity"],
            "manifestSha256": run["manifest"]["sha256"],
            "hospitalSha256": hospital_sha256
        },
        "summary": "Normal analysis treats a missing ignored Sentrux baseline as an architecture gate failure; establish or guide baseline setup before classifying repository health.",
        "effects": []
    }))
}

fn publish_new_artifact(out: &Path, name: &str, bytes: &[u8]) -> Result<(), AdapterError> {
    let parent = out;
    if !parent.is_dir() {
        return Err(AdapterError::Io(format!(
            "artifact output directory does not exist: {}",
            parent.display()
        )));
    }
    let path = parent.join(name);
    if path.exists() {
        return Err(AdapterError::Io(format!(
            "artifact output already exists: {}",
            path.display()
        )));
    }
    let temporary = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| AdapterError::Io(format!("create artifact output: {error}")))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| AdapterError::Io(format!("write artifact output: {error}")))?;
        fs::rename(&temporary, &path)
            .map_err(|error| AdapterError::Io(format!("publish artifact output: {error}")))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn parse_args(raw: &[String]) -> Result<(PathBuf, String, Option<PathBuf>), String> {
    let mut run = None;
    let mut target = None;
    let mut out = None;
    let mut index = 0;
    while index < raw.len() {
        let value = raw.get(index + 1).filter(|value| !value.starts_with("--"));
        match raw[index].as_str() {
            "--run" if run.is_none() => run = value.map(PathBuf::from),
            "--target" if target.is_none() => target = value.cloned(),
            "--out" if out.is_none() => out = value.map(PathBuf::from),
            flag => return Err(format!("unknown dogfood finding argument: {flag}")),
        }
        if value.is_none() {
            return Err(format!("{} requires one value", raw[index]));
        }
        index += 2;
    }
    let run = run.ok_or_else(|| "--run is required".to_string())?;
    let target = target.ok_or_else(|| "--target is required".to_string())?;
    if target != "code-intel-pipeline" && target != "reverse-skill-evolver" && target != "tdxcli-rs"
    {
        return Err(
            "--target must be code-intel-pipeline, reverse-skill-evolver, or tdxcli-rs".into(),
        );
    }
    Ok((run, target, out))
}

pub(crate) fn validate_finding(bytes: &[u8]) -> Result<(), String> {
    crate::artifact_ref::validate_dogfood_finding(bytes)
}

fn write_new_artifact(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        return Err(format!("output already exists: {}", path.display()));
    }
    let parent = path
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| "output parent directory does not exist".to_string())?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "output file name is invalid".to_string())?;
    let temporary = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("cannot create staged output: {error}"))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("cannot write staged output: {error}"))?;
        fs::rename(&temporary, path).map_err(|error| format!("cannot publish output: {error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn validator_rejects_unsupported_diagnosis() {
        let mut finding = serde_json::json!({
            "schema": "code-intel-dogfood-finding.v1",
            "status": "candidate",
            "classification": "tooling_gap",
            "suggested_action": "improve_onboarding",
            "target": {"repository": "code-intel-pipeline"},
            "evidence": {
                "runCommitSha256": "a".repeat(64),
                "snapshotIdentity": "b".repeat(64),
                "runIdentity": "dag-v1:test",
                "manifestSha256": "c".repeat(64),
                "hospitalSha256": "d".repeat(64)
            },
            "summary": "candidate",
            "effects": []
        });
        finding["classification"] = serde_json::json!("architecture_defect");
        assert!(super::validate_finding(&serde_json::to_vec(&finding).unwrap()).is_err());
    }
}

fn read_json(path: &Path, label: &str) -> Result<Value, String> {
    serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("cannot read {label}: {error}"))?,
    )
    .map_err(|error| format!("cannot parse {label}: {error}"))
}

fn read_referenced(parent: &Value, field: &str, run_path: &Path) -> Result<Value, String> {
    read_referenced_value(&parent[field], run_path)
}

fn read_referenced_value(reference: &Value, run_path: &Path) -> Result<Value, String> {
    let relative = reference["path"]
        .as_str()
        .ok_or_else(|| "artifact reference path is missing".to_string())?;
    let root = run_path
        .parent()
        .ok_or_else(|| "run path has no parent".to_string())?;
    let path = root.join(relative);
    let bytes = fs::read(&path).map_err(|error| format!("cannot read artifact: {error}"))?;
    let expected = reference["sha256"]
        .as_str()
        .ok_or_else(|| "artifact reference sha256 is missing".to_string())?;
    if sha256_hex(&bytes) != expected {
        return Err("artifact digest mismatch".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("cannot parse artifact: {error}"))
}
