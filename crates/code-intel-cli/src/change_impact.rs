use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::committed_evidence::{self, EvidenceError};
use crate::impact_graph::{impacted_files, reverse_import_graph, select_tests, test_commands};

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match Cli::parse(raw).and_then(execute) {
        Ok(result) => {
            println!("{}", serde_json::to_string(&result).unwrap());
            0
        }
        Err(ImpactError::Contract(message)) => {
            eprintln!("{message}");
            65
        }
        Err(ImpactError::HostIo(message)) => {
            eprintln!("{message}");
            74
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Staleness {
    Current,
    Advisory,
}

struct Cli {
    artifact_root: PathBuf,
    repo: String,
    repo_path: PathBuf,
    changed: Vec<String>,
    staleness: Staleness,
}

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, ImpactError> {
        if raw.first().map(String::as_str) != Some("impact") {
            return Err(ImpactError::Contract("usage: change impact --artifact-root <root> --repo <name> --repo-path <checkout> --changed <relative-path> [--changed <relative-path>]... [--staleness current|advisory]".into()));
        }
        let mut artifact_root = None;
        let mut repo = None;
        let mut repo_path = None;
        let mut changed = Vec::new();
        let mut staleness = None;
        let mut index = 1;
        while index < raw.len() {
            let flag = raw[index].as_str();
            if !matches!(
                flag,
                "--artifact-root" | "--repo" | "--repo-path" | "--changed" | "--staleness"
            ) {
                return Err(ImpactError::Contract(format!(
                    "unknown change impact argument: {flag}"
                )));
            }
            let value = raw
                .get(index + 1)
                .filter(|value| !value.is_empty() && !value.starts_with("--"))
                .ok_or_else(|| ImpactError::Contract(format!("{flag} requires one value")))?;
            match flag {
                "--artifact-root" => {
                    set_once(&mut artifact_root, PathBuf::from(value), "--artifact-root")?
                }
                "--repo" => set_once(&mut repo, value.clone(), "--repo")?,
                "--repo-path" => set_once(&mut repo_path, PathBuf::from(value), "--repo-path")?,
                "--changed" => changed.push(normalize_relative(value)?),
                "--staleness" => {
                    let mode = match value.as_str() {
                        "current" => Staleness::Current,
                        "advisory" => Staleness::Advisory,
                        other => {
                            return Err(ImpactError::Contract(format!(
                                "--staleness must be current or advisory: {other}"
                            )))
                        }
                    };
                    set_once(&mut staleness, mode, "--staleness")?
                }
                _ => unreachable!(),
            }
            index += 2;
        }
        let artifact_root = artifact_root
            .ok_or_else(|| ImpactError::Contract("--artifact-root is required".into()))?;
        let repo_path =
            repo_path.ok_or_else(|| ImpactError::Contract("--repo-path is required".into()))?;
        if !artifact_root.is_dir() || !repo_path.is_dir() {
            return Err(ImpactError::Contract(
                "artifact root and repository path must be existing directories".into(),
            ));
        }
        changed.sort();
        changed.dedup();
        if changed.is_empty() {
            return Err(ImpactError::Contract(
                "at least one --changed path is required".into(),
            ));
        }
        Ok(Self {
            artifact_root,
            repo: repo.ok_or_else(|| ImpactError::Contract("--repo is required".into()))?,
            repo_path,
            changed,
            staleness: staleness.unwrap_or(Staleness::Current),
        })
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), ImpactError> {
    if slot.replace(value).is_some() {
        Err(ImpactError::Contract(format!("duplicate {flag}")))
    } else {
        Ok(())
    }
}

fn execute(cli: Cli) -> Result<Value, ImpactError> {
    let evidence = committed_evidence::load(&cli.artifact_root, &cli.repo).map_err(map_evidence)?;
    let run_outcome = evidence.entry["outcome"]
        .as_str()
        .expect("A08 entry outcome");
    if run_outcome != "completed" {
        return Err(ImpactError::Contract(format!(
            "change impact requires a completed authoritative run; latest committed run outcome is {run_outcome}"
        )));
    }
    let mut freshness = evidence
        .freshness(Some(&cli.repo_path))
        .map_err(map_evidence)?;
    if freshness["status"] != "current" {
        if cli.staleness == Staleness::Current {
            return Err(ImpactError::Contract(format!(
                "change impact requires the committed snapshot to be current; recorded={} current={}",
                freshness["recordedIdentity"].as_str().unwrap_or("unknown"),
                freshness["currentIdentity"].as_str().unwrap_or("unknown")
            )));
        }
        freshness["status"] = json!("stale-advisory");
    }
    let stale_identities = (freshness["status"] == "stale-advisory").then(|| {
        (
            freshness["recordedIdentity"].clone(),
            freshness["currentIdentity"].clone(),
        )
    });
    let (files_ref, files_artifact) = evidence
        .artifact("code_evidence.files")
        .ok_or_else(|| ImpactError::Contract("committed run lacks code_evidence.files".into()))?;
    let (imports_ref, imports_artifact) = evidence
        .artifact("code_evidence.imports")
        .ok_or_else(|| ImpactError::Contract("committed run lacks code_evidence.imports".into()))?;
    let files_json: Value = serde_json::from_slice(files_artifact.bytes())
        .map_err(|_| ImpactError::Contract("code_evidence.files is invalid JSON".into()))?;
    let imports_json: Value = serde_json::from_slice(imports_artifact.bytes())
        .map_err(|_| ImpactError::Contract("code_evidence.imports is invalid JSON".into()))?;
    let files = files_json["files"]
        .as_array()
        .expect("registered native files artifact");
    let file_paths = files
        .iter()
        .map(|file| {
            file["path"].as_str().map(str::to_string).ok_or_else(|| {
                ImpactError::Contract("code_evidence.files entries must carry a string path".into())
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let imports = imports_json["imports"]
        .as_array()
        .expect("registered native imports artifact");
    let (reverse, resolved_edges, unresolved_edges) =
        reverse_import_graph(imports, &file_paths).map_err(ImpactError::Contract)?;
    let impacted = impacted_files(&cli.changed, &file_paths, &reverse);
    let test_files = select_tests(&impacted, &cli.changed, &file_paths);
    let commands = test_commands(&test_files);
    let changed = cli
        .changed
        .iter()
        .map(|path| json!({"path":path,"inInventory":file_paths.contains(path)}))
        .collect::<Vec<_>>();
    let impact_rows = impacted
        .iter()
        .map(|(path, reason)| {
            json!({
                "path":path,
                "distance":reason.distance,
                "reason":reason.reason,
                "via":reason.via,
                "confidence":reason.confidence,
            })
        })
        .collect::<Vec<_>>();
    let mut result = json!({
        "schema":"code-intel-change-impact.v1",
        "repo":cli.repo,
        "run":evidence.entry["run"],
        "runIdentity":evidence.entry["runIdentity"],
        "runOutcome":run_outcome,
        "snapshotIdentity":evidence.snapshot_identity(),
        "freshness":freshness,
        "changed":changed,
        "evidenceRefs":[files_ref,imports_ref],
        "impact":{
            "files":impact_rows,
            "resolvedImportEdges":resolved_edges,
            "unresolvedImportEdges":unresolved_edges,
        },
        "testSelection":{
            "status":if test_files.is_empty() { "none" } else { "candidates" },
            "files":test_files,
            "commands":commands,
            "advisoryOnly":true,
            "rationale":"Select impacted test files reachable through the verified snapshot's reverse import graph; use same-module test co-location only as a fallback.",
        },
        "limitations":[
            "Native import extraction is heuristic and does not prove runtime call paths.",
            "Dynamic imports, generated code, reflection, build-system edges, and external packages may be unresolved.",
            "Test commands are candidates only and are never executed by this command."
        ]
    });
    if let Some((recorded, current)) = stale_identities {
        result["recordedSnapshotIdentity"] = recorded;
        result["currentSnapshotIdentity"] = current;
        result["limitations"]
            .as_array_mut()
            .expect("constructed limitations array")
            .push(json!("Freshness is stale-advisory: impact derives from the committed snapshot, not the current working tree, and must never gate."));
    }
    Ok(result)
}

fn normalize_relative(path: &str) -> Result<String, ImpactError> {
    let path = path.replace('\\', "/");
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(':')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(ImpactError::Contract(format!(
            "--changed must be a portable repository-relative path: {path}"
        )));
    }
    Ok(path)
}

fn map_evidence(error: EvidenceError) -> ImpactError {
    match error {
        EvidenceError::Contract(message) => ImpactError::Contract(message),
        EvidenceError::HostIo(message) => ImpactError::HostIo(message),
    }
}

enum ImpactError {
    Contract(String),
    HostIo(String),
}
