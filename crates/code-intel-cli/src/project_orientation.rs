use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::{AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if request["options"]
        .as_object()
        .map_or(true, |options| !options.is_empty())
    {
        return Err(AdapterError::InvalidOptions(
            "project.orientation accepts no options".into(),
        ));
    }
    let inputs = Inputs::parse(verified_inputs)?;
    let orientation = compose(request, &inputs)?;
    let json_bytes = serde_json::to_vec(&orientation).map_err(|error| {
        AdapterError::Internal(format!("serialize project orientation: {error}"))
    })?;
    let markdown_bytes = render_summary(&orientation).into_bytes();
    publish(out, "project-orientation.json", &json_bytes)?;
    publish(out, "project-orientation.md", &markdown_bytes)?;
    Ok(AdapterOutput {
        artifacts: vec![
            AdapterArtifact {
                artifact_schema: "code-intel-project-orientation.v1".into(),
                artifact_type: "project.orientation".into(),
                relative_path: "project-orientation.json".into(),
                bytes: json_bytes,
            },
            AdapterArtifact {
                artifact_schema: "code-intel-project-orientation-summary.v1".into(),
                artifact_type: "materialized_view.project_orientation_summary".into(),
                relative_path: "project-orientation.md".into(),
                bytes: markdown_bytes,
            },
        ],
        observed_effects: vec!["local_write".into()],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

struct Inputs<'a> {
    snapshot: (&'a VerifiedArtifact, Value),
    inventory: (&'a VerifiedArtifact, Vec<String>),
    survival: (&'a VerifiedArtifact, Value),
    native_files: (&'a VerifiedArtifact, Value),
    coverage: (&'a VerifiedArtifact, Value),
    ranking: (&'a VerifiedArtifact, Value),
}

impl<'a> Inputs<'a> {
    fn parse(inputs: &'a [VerifiedArtifact]) -> Result<Self, AdapterError> {
        if inputs.len() != 6 {
            return Err(contract(
                "project.orientation requires exactly six A03-verified inputs",
            ));
        }
        let find = |kind: &str| {
            let matches = inputs
                .iter()
                .filter(|input| input.artifact_type() == kind)
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [input] => Ok(*input),
                [] => Err(contract(format!(
                    "project.orientation input is missing: {kind}"
                ))),
                _ => Err(contract(format!(
                    "project.orientation input is duplicated: {kind}"
                ))),
            }
        };
        let snapshot = find("repository.snapshot")?;
        let inventory = find("inventory.files")?;
        let survival = find("repository.survival-scan")?;
        let native_files = find("code_evidence.files")?;
        let coverage = find("code_evidence.coverage")?;
        let ranking = find("code_evidence.agent_slice")?;
        require_schema(snapshot, "code-intel-repository-snapshot.v1")?;
        require_schema(inventory, "code-intel-file-inventory.v1")?;
        require_schema(survival, "code-intel-repository-survival-scan-result.v1")?;
        require_schema(native_files, "code-evidence-files.v1")?;
        require_schema(coverage, "code-evidence-coverage.v1")?;
        require_schema(ranking, "agent-code-slice-ranking.v1")?;
        Ok(Self {
            snapshot: (snapshot, parse_json(snapshot, "repository snapshot")?),
            inventory: (inventory, inventory_paths(inventory.bytes())?),
            survival: (survival, parse_json(survival, "survival scan")?),
            native_files: (native_files, parse_json(native_files, "native files")?),
            coverage: (coverage, parse_json(coverage, "native coverage")?),
            ranking: (ranking, parse_json(ranking, "native ranking")?),
        })
    }
}

fn compose(request: &Value, inputs: &Inputs<'_>) -> Result<Value, AdapterError> {
    let snapshot = &inputs.snapshot.1;
    let survival = &inputs.survival.1;
    let native_files = &inputs.native_files.1;
    let coverage = &inputs.coverage.1;
    let ranking = &inputs.ranking.1;
    let snapshot_identity = request["snapshot"]["identity"]
        .as_str()
        .ok_or_else(|| contract("project.orientation request snapshot identity is invalid"))?;
    if snapshot["schema"] != "code-intel-repository-snapshot.v1"
        || snapshot["snapshot"] != request["snapshot"]
        || survival["schema"] != "code-intel-repository-survival-scan-result.v1"
        || survival["snapshotIdentity"] != snapshot_identity
        || survival["repository"]["sourceSha256"] != inputs.snapshot.0.sha256()
        || survival["inventory"]["sourceSha256"] != inputs.inventory.0.sha256()
    {
        return Err(contract(
            "project.orientation snapshot or survival evidence is incoherent",
        ));
    }
    let native = native_files["files"]
        .as_array()
        .ok_or_else(|| contract("native files evidence is invalid"))?;
    let native_paths = native
        .iter()
        .map(|file| {
            file["path"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| contract("native file path is invalid"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let inventory_paths = inputs.inventory.1.iter().cloned().collect::<BTreeSet<_>>();
    if native_paths != inventory_paths
        || survival["inventory"]["fileCount"].as_u64() != Some(inventory_paths.len() as u64)
    {
        return Err(contract(
            "inventory, survival, and native file evidence disagree",
        ));
    }
    if coverage["schema"] != "code-evidence-coverage.v1"
        || ranking["schema"] != "agent-code-slice-ranking.v1"
    {
        return Err(contract("native evidence schema is invalid"));
    }

    let snapshot_provenance = provenance(inputs.snapshot.0, "/snapshot");
    let dirty_provenance = provenance(inputs.snapshot.0, "/dirtyOverlay");
    let inventory_provenance = provenance(inputs.inventory.0, "/");
    let survival_provenance = provenance(inputs.survival.0, "/completeness");
    let coverage_provenance = provenance(inputs.coverage.0, "/");
    let native_files_provenance = provenance(inputs.native_files.0, "/files");
    let ranking_provenance = provenance(inputs.ranking.0, "/files");

    let mut language_counts = BTreeMap::<String, u64>::new();
    for file in native {
        let language = file["language"]
            .as_str()
            .ok_or_else(|| contract("native file language is invalid"))?;
        if language != "text" {
            *language_counts.entry(language.to_string()).or_default() += 1;
        }
    }
    let mut languages = language_counts
        .into_iter()
        .map(|(name, file_count)| json!({"name":name,"fileCount":file_count,"provenance":[native_files_provenance.clone()]}))
        .collect::<Vec<_>>();
    languages.sort_by(|left, right| {
        right["fileCount"]
            .as_u64()
            .cmp(&left["fileCount"].as_u64())
            .then_with(|| left["name"].as_str().cmp(&right["name"].as_str()))
    });

    let boundaries = inventory_paths
        .iter()
        .filter_map(|path| path.split_once('/').map(|(root, _)| root.to_string()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|path| json!({"path":path,"kind":"top_level_directory","provenance":[inventory_provenance.clone()]}))
        .collect::<Vec<_>>();
    let ranked = ranking["files"]
        .as_array()
        .ok_or_else(|| contract("native ranking files are invalid"))?;
    let entry_points = ranked
        .iter()
        .filter(|item| {
            item["reasons"]
                .as_array()
                .is_some_and(|reasons| reasons.iter().any(|reason| reason == "entrypoint"))
        })
        .map(|item| {
            let path = item["path"]
                .as_str()
                .filter(|path| native_paths.contains(*path))
                .ok_or_else(|| contract("native ranking references an unknown entry point"))?;
            Ok(json!({"path":path,"classification":"heuristic","provenance":[ranking_provenance.clone()]}))
        })
        .collect::<Result<Vec<_>, AdapterError>>()?;
    let commands = inventory_paths
        .iter()
        .filter(|path| !path.contains('/') && command_script(path))
        .map(|path| json!({"path":path,"kind":"script_path","provenance":[inventory_provenance.clone()]}))
        .collect::<Vec<_>>();

    let dirty = snapshot["dirtyOverlay"]["present"]
        .as_bool()
        .ok_or_else(|| contract("snapshot dirty overlay is invalid"))?;
    let active_paths = snapshot["dirtyOverlay"]["paths"]
        .as_array()
        .ok_or_else(|| contract("snapshot dirty paths are invalid"))?
        .clone();
    let mut unknowns = vec![json!({
        "field":"purpose",
        "reason":"no admitted purpose evidence is present in the composed inputs",
        "provenance":[inventory_provenance.clone(), native_files_provenance.clone()]
    })];
    if commands.is_empty() {
        unknowns.push(json!({
            "field":"commands",
            "reason":"no root command script is present in the verified inventory",
            "provenance":[inventory_provenance.clone()]
        }));
    }
    unknowns.extend([
        json!({"field":"structural_relationships","reason":"survival evidence reports structural verdict unknown","provenance":[survival_provenance.clone()]}),
        json!({"field":"call_graph","reason":"native coverage reports call graph precision unknown","provenance":[coverage_provenance.clone()]}),
    ]);

    let provider_ready = coverage["producer"] == "benchmark-provider-ready";
    let mut risks = vec![
        json!({"code":"heuristic_native_precision","statement":"native symbols and imports are heuristic and call graph precision is unknown","provenance":[coverage_provenance.clone()]}),
    ];
    if !provider_ready {
        risks.insert(0, json!({"code":"structural_evidence_unavailable","statement":"deeper structural perception is unavailable","provenance":[survival_provenance.clone()]}));
    }

    Ok(json!({
        "schema":"code-intel-project-orientation.v1",
        "snapshotIdentity":snapshot_identity,
        "identity":{
            "status":"known",
            "repositoryIdentity":snapshot["snapshot"]["repoIdentity"],
            "repositoryKind":snapshot["repository"]["kind"],
            "revision":snapshot["snapshot"]["head"],
            "provenance":[snapshot_provenance]
        },
        "purpose":{
            "status":"unknown",
            "evidence":[],
            "reason":"no admitted purpose evidence is present in the composed inputs",
            "provenance":[inventory_provenance.clone(), native_files_provenance.clone()]
        },
        "languages":languages,
        "boundaries":boundaries,
        "entryPoints":entry_points,
        "commands":commands,
        "activeChange":{
            "status":if dirty {"dirty"} else {"clean"},
            "paths":active_paths,
            "provenance":[dirty_provenance]
        },
        "evidenceAvailability":[
            {"evidence":"survival_scan","status":"reduced","provenance":[survival_provenance.clone()]},
            {"evidence":"benchmark_provider","status":if provider_ready {"available"} else {"unavailable"},"provenance":[coverage_provenance.clone()]},
            {"evidence":"native_files","status":"available","provenance":[native_files_provenance.clone()]},
            {"evidence":"native_structure","status":"heuristic","provenance":[coverage_provenance.clone()]}
        ],
        "risks":risks,
        "unknowns":unknowns,
        "confidence":{
            "level":"low",
            "basis":["survival completeness is reduced","structural verdict and call graph remain unknown"],
            "provenance":[survival_provenance, coverage_provenance]
        }
    }))
}

fn provenance(artifact: &VerifiedArtifact, pointer: &str) -> Value {
    json!({
        "artifactType":artifact.artifact_type(),
        "artifactSha256":artifact.sha256(),
        "jsonPointer":pointer
    })
}

fn parse_json(artifact: &VerifiedArtifact, label: &str) -> Result<Value, AdapterError> {
    serde_json::from_slice(artifact.bytes())
        .map_err(|_| contract(format!("verified {label} is invalid JSON")))
}

fn require_schema(artifact: &VerifiedArtifact, schema: &str) -> Result<(), AdapterError> {
    if artifact.artifact_schema() == schema {
        Ok(())
    } else {
        Err(contract(format!(
            "{} input must declare artifact schema {schema}",
            artifact.artifact_type()
        )))
    }
}

fn inventory_paths(bytes: &[u8]) -> Result<Vec<String>, AdapterError> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| contract("verified inventory is not UTF-8"))?;
    let mut paths = text
        .split(['\0', '\n'])
        .map(|path| path.trim_end_matches('\r').replace('\\', "/"))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    if paths
        .iter()
        .any(|path| path.starts_with('/') || path.split('/').any(|part| part == ".."))
    {
        return Err(contract("verified inventory contains a non-portable path"));
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn command_script(path: &str) -> bool {
    [".ps1", ".sh", ".cmd", ".bat"]
        .iter()
        .any(|suffix| path.to_ascii_lowercase().ends_with(suffix))
}

fn render_summary(orientation: &Value) -> String {
    let item_lines = |field: &str, key: &str| {
        orientation[field]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item[key].as_str())
            .map(|value| format!("- {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "# Project Orientation\n\n## Identity\n- Repository: {}\n- Revision: {}\n\n## Purpose\n- Status: {}\n\n## Languages\n{}\n\n## Boundaries\n{}\n\n## Entry Points\n{}\n\n## Commands\n{}\n\n## Active Change\n- Status: {}\n\n## Risks\n{}\n\n## Unknowns\n{}\n\n## Confidence\n- Level: {}\n",
        orientation["identity"]["repositoryIdentity"].as_str().unwrap_or("unknown"),
        orientation["identity"]["revision"].as_str().unwrap_or("unknown"),
        orientation["purpose"]["status"].as_str().unwrap_or("unknown"),
        item_lines("languages", "name"),
        item_lines("boundaries", "path"),
        item_lines("entryPoints", "path"),
        item_lines("commands", "path"),
        orientation["activeChange"]["status"].as_str().unwrap_or("unknown"),
        item_lines("risks", "statement"),
        item_lines("unknowns", "field"),
        orientation["confidence"]["level"].as_str().unwrap_or("unknown")
    )
}

fn publish(out: &Path, relative: &str, bytes: &[u8]) -> Result<(), AdapterError> {
    fs::create_dir_all(out)
        .map_err(|error| AdapterError::Io(format!("create orientation output: {error}")))?;
    let path = out.join(relative);
    if path.exists() {
        return Err(AdapterError::Io(format!(
            "refusing to overwrite orientation artifact: {relative}"
        )));
    }
    fs::write(&path, bytes).map_err(|error| AdapterError::Io(format!("write {relative}: {error}")))
}

fn contract(message: impl Into<String>) -> AdapterError {
    AdapterError::Contract(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::sha256_hex;

    #[test]
    fn registry_toolchain_digest_is_the_orientation_source_sha256() {
        let registry_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("orchestration/integrations.json");
        let registry: Value = serde_json::from_slice(&fs::read(registry_path).unwrap()).unwrap();
        let integration = registry["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "project.orientation")
            .unwrap();
        assert_eq!(
            integration["toolchainDigestEvidence"],
            json!({
                "algorithm":"sha256",
                "inputs":["crates/code-intel-cli/src/project_orientation.rs"]
            })
        );
        assert_eq!(
            integration["capabilityDeclaration"]["implementation"]["toolchainDigests"][0],
            sha256_hex(include_bytes!("project_orientation.rs"))
        );
    }
}
