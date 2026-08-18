use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::run_commit;

use super::{absolute_existing_dir, resolve_artifact_root, Result};

#[derive(Debug)]
struct ReportArtifact {
    schema: String,
    artifact_type: String,
    path: PathBuf,
}

impl ReportArtifact {
    fn to_json(&self) -> Value {
        serde_json::json!({
            "schema": self.schema,
            "type": self.artifact_type,
            "path": self.path,
        })
    }
}

pub(super) fn report(repo: &Path, artifact_root: Option<&Path>, json: bool) -> Result<()> {
    let repo = absolute_existing_dir(repo)?;
    let artifact_root = resolve_artifact_root(artifact_root)?;
    let repo_name = repo
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("repo path has no final directory name")?;
    let repo_artifacts = artifact_root.join(repo_name);
    let (run_root, manifest) = latest_committed_run(&repo_artifacts)?;
    let hospital = required_report_artifact(&run_root, &manifest, "diagnosis.hospital")?;
    let hospital_markdown =
        required_report_artifact(&run_root, &manifest, "diagnosis.hospital-view")?;
    let hospital_value = read_json_artifact(&hospital.path, "diagnosis.hospital")?;
    let hospital_text = fs::read_to_string(&hospital_markdown.path)?;
    let agent_code_slice_ranking =
        optional_report_artifact(&run_root, &manifest, "code_evidence.agent_slice")?;
    let sentrux_evidence = project_sentrux_evidence(&run_root, &manifest, &hospital_value);

    let out = serde_json::json!({
        "schema": "code-intel-report.v1",
        "repo": repo,
        "run": run_root.file_name().and_then(|name| name.to_str()),
        "hospital": hospital.to_json(),
        "hospitalMarkdown": hospital_markdown.to_json(),
        "agentCodeSliceRanking": agent_code_slice_ranking.as_ref().map(ReportArtifact::to_json),
        "sentruxEvidence": sentrux_evidence,
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("Code Intel Report");
        println!("repo: {}", repo.display());
        println!("run: {}", out["run"].as_str().unwrap_or(""));
        println!("hospital: {}", hospital.path.display());
        println!("hospitalMarkdown: {}", hospital_markdown.path.display());
        if let Some(ranking) = &agent_code_slice_ranking {
            println!("agentCodeSliceRanking: {}", ranking.path.display());
        }
        println!(
            "sentruxEvidence: {}",
            out["sentruxEvidence"]["status"]
                .as_str()
                .unwrap_or("unknown")
        );
        println!();
        println!("--- hospital.md ---");
        print!("{hospital_text}");
        if !hospital_text.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

fn read_json_artifact(path: &Path, artifact_type: &str) -> Result<Value> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "committed `{artifact_type}` artifact {} is not valid JSON: {error}",
            path.display()
        )
        .into()
    })
}

/// Project Sentrux evidence only when the Hospital's references are also
/// present in the committed run manifest and the referenced capability
/// artifact verifies against that manifest. The report must remain useful on
/// older or partial runs, but it must never turn an absent or untrusted
/// capability reference into a successful result.
fn project_sentrux_evidence(run_root: &Path, manifest: &Value, hospital: &Value) -> Value {
    let manifest_refs = manifest_capability_refs(manifest);
    let hospital_refs = match hospital.pointer("/tools/sentruxCapabilities") {
        Some(Value::Array(refs)) => refs,
        Some(_) => {
            return serde_json::json!({
                "status": if manifest_refs.is_empty() { "unknown" } else { "degraded" },
                "manifestCapabilityRefs": manifest_refs.len(),
                "hospitalCapabilityRefs": 0,
                "capabilities": [],
                "missingReferences": manifest_refs.iter().map(|reference| reference_descriptor(reference)).collect::<Vec<_>>(),
                "unverifiedReferences": [],
                "reason": "Hospital tools.sentruxCapabilities is not an array"
            });
        }
        None => {
            let status = if manifest_refs.is_empty() {
                "unknown"
            } else {
                "degraded"
            };
            return serde_json::json!({
                "status": status,
                "manifestCapabilityRefs": manifest_refs.len(),
                "hospitalCapabilityRefs": 0,
                "capabilities": [],
                "missingReferences": manifest_refs.iter().map(|reference| reference_descriptor(reference)).collect::<Vec<_>>(),
                "unverifiedReferences": [],
                "reason": if manifest_refs.is_empty() {
                    "Hospital did not publish Sentrux capability references and the manifest contains none"
                } else {
                    "Manifest contains Sentrux capability references but Hospital did not project them"
                }
            });
        }
    };

    let manifest_keys = manifest_refs
        .iter()
        .filter_map(|reference| reference_key(reference).map(|key| (key, *reference)))
        .collect::<Vec<_>>();
    let mut projected_keys = BTreeSet::new();
    let mut verified = Vec::new();
    let mut unverified = Vec::new();

    for reference in hospital_refs {
        let Some(key) = reference_key(reference) else {
            unverified.push(serde_json::json!({
                "reference": reference,
                "reason": "Hospital Sentrux capability reference is malformed"
            }));
            continue;
        };
        if !projected_keys.insert(key.clone()) {
            unverified.push(serde_json::json!({
                "reference": reference,
                "reason": "Hospital Sentrux capability reference is duplicated"
            }));
            continue;
        }
        let Some((_, manifest_reference)) = manifest_keys
            .iter()
            .find(|(candidate, _)| *candidate == key)
        else {
            unverified.push(serde_json::json!({
                "reference": reference,
                "reason": "Hospital Sentrux capability reference is not present in the committed manifest"
            }));
            continue;
        };

        let Some(snapshot) = manifest["snapshotIdentity"].as_str() else {
            unverified.push(serde_json::json!({
                "reference": reference,
                "reason": "Committed manifest has no snapshot identity"
            }));
            continue;
        };
        let artifact = match crate::artifact_ref::registered_contract(manifest_reference).and_then(
            |contract| {
                crate::artifact_ref::verify_artifact_ref(
                    run_root,
                    snapshot,
                    contract,
                    manifest_reference,
                )
            },
        ) {
            Ok(artifact) => artifact,
            Err(error) => {
                unverified.push(serde_json::json!({
                    "reference": reference,
                    "reason": format!("Committed Sentrux capability artifact failed verification: {error:?}")
                }));
                continue;
            }
        };
        let payload: Value = match serde_json::from_slice(artifact.bytes()) {
            Ok(payload) => payload,
            Err(error) => {
                unverified.push(serde_json::json!({
                    "reference": reference,
                    "reason": format!("Verified Sentrux capability artifact is not JSON: {error}")
                }));
                continue;
            }
        };
        verified.push(serde_json::json!({
            "capabilityId": payload["capabilityId"],
            "operation": payload["operation"],
            "status": payload["status"],
            "authority": payload["authority"],
            "verdict": payload.pointer("/outputs/verdict").cloned().unwrap_or(Value::String("unknown".into())),
            "provider": payload["provider"],
            "artifact": reference_descriptor(manifest_reference),
            "verification": "committed_manifest_and_payload"
        }));
    }

    let missing = manifest_refs
        .iter()
        .filter(|reference| {
            reference_key(reference).is_some_and(|key| !projected_keys.contains(&key))
        })
        .map(|reference| reference_descriptor(reference))
        .collect::<Vec<_>>();
    let has_degraded_capability = verified.iter().any(|capability| {
        !matches!(capability["status"].as_str(), Some("succeeded"))
            || matches!(capability["verdict"].as_str(), Some("fail" | "unknown"))
    });
    let status = if verified.is_empty() {
        "unknown"
    } else if !unverified.is_empty() || !missing.is_empty() || has_degraded_capability {
        "degraded"
    } else {
        "verified"
    };
    serde_json::json!({
        "status": status,
        "manifestCapabilityRefs": manifest_refs.len(),
        "hospitalCapabilityRefs": hospital_refs.len(),
        "capabilities": verified,
        "missingReferences": missing,
        "unverifiedReferences": unverified,
        "reason": if status == "verified" {
            "All Hospital Sentrux capability references are manifest-bound and payload-verified"
        } else if status == "degraded" {
            "Sentrux evidence is partially verified; missing, unverified, or non-success capabilities are listed"
        } else {
            "No Hospital Sentrux capability reference could be verified"
        }
    })
}

fn manifest_capability_refs(manifest: &Value) -> Vec<&Value> {
    manifest["nodes"]
        .as_object()
        .into_iter()
        .flat_map(|nodes| nodes.values())
        .flat_map(|node| node["artifacts"].as_array().into_iter().flatten())
        .filter(|reference| {
            reference["type"] == "provider.sentrux.capability-artifact"
                && reference["artifactSchema"] == "code-intel-sentrux-capability-artifact.v1"
        })
        .collect()
}

fn reference_key(reference: &Value) -> Option<String> {
    [
        "schema",
        "artifactSchema",
        "type",
        "sha256",
        "consumedSnapshotIdentity",
    ]
    .into_iter()
    .map(|field| reference[field].as_str())
    .collect::<Option<Vec<_>>>()
    .map(|fields| fields.join("\u{0}"))
}

fn reference_descriptor(reference: &Value) -> Value {
    serde_json::json!({
        "schema": reference["schema"],
        "artifactSchema": reference["artifactSchema"],
        "type": reference["type"],
        "path": reference["path"],
        "sha256": reference["sha256"],
        "consumedSnapshotIdentity": reference["consumedSnapshotIdentity"]
    })
}

fn required_report_artifact(
    run_root: &Path,
    manifest: &Value,
    artifact_type: &str,
) -> Result<ReportArtifact> {
    optional_report_artifact(run_root, manifest, artifact_type)?.ok_or_else(|| {
        format!(
            "committed run {} has no `{artifact_type}` Artifact Ref",
            run_root.display()
        )
        .into()
    })
}

fn optional_report_artifact(
    run_root: &Path,
    manifest: &Value,
    artifact_type: &str,
) -> Result<Option<ReportArtifact>> {
    let reference = manifest["nodes"]
        .as_object()
        .into_iter()
        .flat_map(|nodes| nodes.values())
        .flat_map(|node| node["artifacts"].as_array().into_iter().flatten())
        .find(|reference| reference["type"].as_str() == Some(artifact_type));
    let Some(reference) = reference else {
        return Ok(None);
    };
    let path = reference["path"].as_str().ok_or_else(|| {
        format!(
            "Artifact Ref `{artifact_type}` has no path in {}",
            run_root.display()
        )
    })?;
    let path = run_root.join(path);
    if !path.is_file() {
        return Err(format!(
            "Artifact Ref `{artifact_type}` points to missing file {}",
            path.display()
        )
        .into());
    }
    Ok(Some(ReportArtifact {
        schema: reference["artifactSchema"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        artifact_type: artifact_type.to_string(),
        path,
    }))
}

fn latest_committed_run(repo_artifacts: &Path) -> Result<(PathBuf, Value)> {
    if !repo_artifacts.is_dir() {
        return Err(format!(
            "no artifact directory for repository under {}",
            repo_artifacts.display()
        )
        .into());
    }
    let mut dirs = Vec::new();
    for entry in fs::read_dir(repo_artifacts)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    let mut rejection = None;
    for path in dirs.iter().rev() {
        match run_commit::validate_committed_run(path) {
            Ok((_marker, manifest)) => return Ok((path.clone(), manifest)),
            Err(error) => rejection = Some(format!("{}: {error}", path.display())),
        }
    }
    Err(format!(
        "no committed A07 run under {}; latest candidate rejected: {}",
        repo_artifacts.display(),
        rejection.unwrap_or_else(|| "no run directories found".to_string())
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("code-intel-report-sentrux-{stamp}"))
    }

    fn capability_fixture(root: &Path) -> (String, Value) {
        let snapshot = "a".repeat(64);
        let payload = json!({
            "schema":"code-intel-sentrux-capability-artifact.v1",
            "contractVersion":1,
            "capabilityId":"sentrux.scan",
            "operation":"scan",
            "runId":"run-1",
            "snapshotIdentity":snapshot,
            "provider":{
                "mode":"builtin",
                "id":"sentrux.builtin",
                "version":"1.0.0",
                "digest":"b".repeat(64)
            },
            "status":"succeeded",
            "authority":"authoritative",
            "inputs":{"snapshotIdentity":snapshot},
            "outputs":{"verdict":"pass"},
            "failure":null,
            "freshness":{
                "status":"current",
                "evaluatedAt":"2026-08-19T00:00:00Z",
                "consumedSnapshotIdentity":snapshot
            },
            "decisionConsumers":["diagnosis.hospital"]
        });
        let bytes = serde_json::to_vec(&payload).expect("fixture should serialize");
        let relative_path = "objects/sha256/sentrux-scan";
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("fixture has a parent"))
            .expect("fixture directory should be created");
        fs::write(path, bytes.clone()).expect("fixture artifact should be written");
        let reference = json!({
            "schema":"code-intel-artifact-ref.v1",
            "artifactSchema":"code-intel-sentrux-capability-artifact.v1",
            "type":"provider.sentrux.capability-artifact",
            "path":relative_path,
            "sha256":crate::capability::sha256_hex(&bytes),
            "consumedSnapshotIdentity":snapshot
        });
        (snapshot, reference)
    }

    #[test]
    fn report_projects_verified_refs_and_marks_untrusted_refs_degraded() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("fixture root should be created");
        let (snapshot, reference) = capability_fixture(&root);
        let mut untrusted = reference.clone();
        untrusted["sha256"] = json!("c".repeat(64));
        let manifest = json!({
            "snapshotIdentity":snapshot,
            "nodes":{"evidence.sentrux":{"artifacts":[reference]}}
        });
        let mut hospital_reference = reference.clone();
        hospital_reference["path"] = json!("sentrux-capability-scan.json");
        let hospital = json!({
            "tools":{"sentruxCapabilities":[hospital_reference, untrusted]}
        });

        let evidence = project_sentrux_evidence(&root, &manifest, &hospital);

        assert_eq!(evidence["status"], "degraded");
        assert_eq!(evidence["manifestCapabilityRefs"], 1);
        assert_eq!(evidence["hospitalCapabilityRefs"], 2);
        assert_eq!(evidence["capabilities"].as_array().unwrap().len(), 1);
        assert_eq!(evidence["capabilities"][0]["capabilityId"], "sentrux.scan");
        assert_eq!(
            evidence["capabilities"][0]["verification"],
            "committed_manifest_and_payload"
        );
        assert_eq!(
            evidence["unverifiedReferences"].as_array().unwrap().len(),
            1
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn report_marks_missing_hospital_projection_unknown_or_degraded() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("fixture root should be created");
        let (snapshot, reference) = capability_fixture(&root);
        let manifest = json!({
            "snapshotIdentity":snapshot,
            "nodes":{"evidence.sentrux":{"artifacts":[reference]}}
        });

        let degraded = project_sentrux_evidence(&root, &manifest, &json!({"tools":{}}));
        assert_eq!(degraded["status"], "degraded");
        assert_eq!(degraded["missingReferences"].as_array().unwrap().len(), 1);

        let unknown = project_sentrux_evidence(&root, &json!({"nodes":{}}), &json!({}));
        assert_eq!(unknown["status"], "unknown");
        assert!(unknown["capabilities"].as_array().unwrap().is_empty());

        let _ = fs::remove_dir_all(root);
    }
}
