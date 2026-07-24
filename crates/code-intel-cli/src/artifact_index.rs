use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::capability::{reject_duplicate_json_keys, validate_artifact_ref_shape};
use crate::run_commit;

const INDEX_SCHEMA: &str = "code-intel-artifact-index.v1";
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum IndexError {
    Contract(String),
    HostIo(String),
}

impl fmt::Display for IndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(message) | Self::HostIo(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for IndexError {}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match parse_cli(raw).and_then(execute_cli) {
        Ok(index) => {
            println!("{}", serde_json::to_string(&index).unwrap());
            0
        }
        Err(IndexError::Contract(message)) => {
            eprintln!("{message}");
            65
        }
        Err(IndexError::HostIo(message)) => {
            eprintln!("{message}");
            74
        }
    }
}

struct Cli {
    artifact_root: PathBuf,
    output: Option<PathBuf>,
    operation: Operation,
    existing: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Rebuild,
    Incremental,
}

fn parse_cli(raw: &[String]) -> Result<Cli, IndexError> {
    if raw.first().map(String::as_str) != Some("index") {
        return Err(IndexError::Contract("usage: artifact index --artifact-root <root> [--output <index.json>] [--operation rebuild|incremental] [--existing <index.json>]".into()));
    }
    let mut artifact_root = None;
    let mut output = None;
    let mut operation = Operation::Rebuild;
    let mut operation_seen = false;
    let mut existing = None;
    let mut index = 1;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--artifact-root" | "--output" | "--operation" | "--existing"
        ) {
            return Err(IndexError::Contract(format!(
                "unknown artifact index argument: {flag}"
            )));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| IndexError::Contract(format!("{flag} requires one value")))?;
        match flag {
            "--artifact-root" => set_once(&mut artifact_root, PathBuf::from(value), flag)?,
            "--output" => set_once(&mut output, PathBuf::from(value), flag)?,
            "--existing" => set_once(&mut existing, PathBuf::from(value), flag)?,
            "--operation" => {
                if operation_seen {
                    return Err(IndexError::Contract("duplicate --operation".into()));
                }
                operation_seen = true;
                operation = match value.as_str() {
                    "rebuild" => Operation::Rebuild,
                    "incremental" => Operation::Incremental,
                    _ => {
                        return Err(IndexError::Contract(
                            "--operation must be rebuild or incremental".into(),
                        ))
                    }
                };
            }
            _ => unreachable!(),
        }
        index += 2;
    }
    let artifact_root =
        artifact_root.ok_or_else(|| IndexError::Contract("--artifact-root is required".into()))?;
    if !artifact_root.is_dir() {
        return Err(IndexError::Contract(
            "artifact root must be an existing directory".into(),
        ));
    }
    if operation == Operation::Incremental && existing.is_none() {
        return Err(IndexError::Contract(
            "incremental operation requires --existing".into(),
        ));
    }
    if operation == Operation::Rebuild && existing.is_some() {
        return Err(IndexError::Contract(
            "--existing is only valid for incremental operation".into(),
        ));
    }
    Ok(Cli {
        artifact_root,
        output,
        operation,
        existing,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), IndexError> {
    if slot.replace(value).is_some() {
        Err(IndexError::Contract(format!("duplicate {flag}")))
    } else {
        Ok(())
    }
}

fn execute_cli(cli: Cli) -> Result<Value, IndexError> {
    let index = match cli.operation {
        Operation::Rebuild => rebuild(&cli.artifact_root)?,
        Operation::Incremental => {
            let path = cli.existing.as_ref().unwrap();
            let existing = read_index(path)?;
            incremental(&cli.artifact_root, &existing)?
        }
    };
    if let Some(path) = cli.output {
        write_index(&path, &index)?;
    }
    Ok(index)
}

pub(crate) fn rebuild(artifact_root: &Path) -> Result<Value, IndexError> {
    scan(artifact_root)
}

pub(crate) fn incremental(artifact_root: &Path, existing: &Value) -> Result<Value, IndexError> {
    validate_index(existing)?;
    // Existing rows are hints only. Every refresh revalidates the A07 authority
    // boundary so a stale or tampered committed run cannot survive incrementally.
    scan(artifact_root)
}

fn scan(artifact_root: &Path) -> Result<Value, IndexError> {
    let mut admitted: BTreeMap<String, Value> = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for repo in child_directories(artifact_root)? {
        let repo_name = file_name(&repo)?;
        for run in child_directories(&repo)? {
            let run_name = file_name(&run)?;
            if is_staging_name(&run_name) {
                diagnostics.push(diagnostic(
                    &repo_name,
                    &run_name,
                    "staging",
                    "run directory is staging and has no publication authority",
                ));
                continue;
            }
            match run_commit::validate_committed_run(&run) {
                Ok((marker, manifest)) => {
                    let outcome = manifest["outcome"]
                        .as_str()
                        .expect("validated run manifest outcome");
                    if outcome != "completed" {
                        diagnostics.push(diagnostic(
                            &repo_name,
                            &run_name,
                            "non_completed",
                            &format!(
                                "committed audit run outcome is {outcome}; only completed runs are query authority"
                            ),
                        ));
                        continue;
                    }
                    let entry = entry(&repo_name, &run_name, &marker, &manifest);
                    let replace = admitted
                        .get(&repo_name)
                        .and_then(|value| value["run"].as_str())
                        .is_none_or(|prior| run_name.as_str() > prior);
                    if replace {
                        admitted.insert(repo_name.clone(), entry);
                    }
                }
                Err(error) => {
                    let marker_exists = run.join("run-complete.json").is_file();
                    let (classification, reason) = if marker_exists {
                        (
                            "forged",
                            "completion marker or its manifest/Artifact Refs failed validation",
                        )
                    } else if run.join("objects").is_dir() {
                        ("incomplete", "A07 completion marker is absent")
                    } else {
                        ("legacy", "legacy run has no A07 completion marker")
                    };
                    let _ = error;
                    diagnostics.push(diagnostic(&repo_name, &run_name, classification, reason));
                }
            }
        }
    }
    diagnostics.sort_by(|left, right| {
        (left["repo"].as_str(), left["run"].as_str())
            .cmp(&(right["repo"].as_str(), right["run"].as_str()))
    });
    Ok(json!({
        "schema": INDEX_SCHEMA,
        "entries": admitted.into_values().collect::<Vec<_>>(),
        "diagnostics": diagnostics,
    }))
}

fn entry(repo: &str, run: &str, marker: &Value, manifest: &Value) -> Value {
    let mut refs = Vec::new();
    for node in manifest["nodes"].as_object().unwrap().values() {
        if let Some(artifacts) = node["artifacts"].as_array() {
            refs.extend(artifacts.iter().cloned());
        }
    }
    refs.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    json!({
        "repo": repo,
        "run": run,
        "runIdentity": marker["runIdentity"],
        "snapshotIdentity": marker["snapshotIdentity"],
        "outcome": manifest["outcome"],
        "manifest": marker["manifest"],
        "artifactRefs": refs,
    })
}

fn diagnostic(repo: &str, run: &str, classification: &str, reason: &str) -> Value {
    json!({"repo":repo,"run":run,"classification":classification,"reason":reason})
}

fn child_directories(root: &Path) -> Result<Vec<PathBuf>, IndexError> {
    let mut paths = Vec::new();
    let entries = fs::read_dir(root)
        .map_err(|error| IndexError::HostIo(format!("read artifact index directory: {error}")))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| IndexError::HostIo(format!("read artifact index entry: {error}")))?;
        let file_type = entry.file_type().map_err(|error| {
            IndexError::HostIo(format!("inspect artifact index entry: {error}"))
        })?;
        if file_type.is_dir() && !file_type.is_symlink() {
            paths.push(entry.path());
        }
    }
    paths.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    Ok(paths)
}

fn file_name(path: &Path) -> Result<String, IndexError> {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| IndexError::Contract("artifact index path is not portable UTF-8".into()))
}

fn is_staging_name(name: &str) -> bool {
    name.starts_with('.') || name.starts_with("stage-") || name.contains("staging")
}

fn read_index(path: &Path) -> Result<Value, IndexError> {
    let metadata = fs::metadata(path)
        .map_err(|error| IndexError::HostIo(format!("inspect existing artifact index: {error}")))?;
    if !metadata.is_file() || metadata.len() > MAX_INDEX_BYTES {
        return Err(IndexError::Contract(
            "existing artifact index must be a bounded regular file".into(),
        ));
    }
    let bytes = fs::read(path)
        .map_err(|error| IndexError::HostIo(format!("read existing artifact index: {error}")))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| IndexError::Contract("existing artifact index must be UTF-8 JSON".into()))?;
    reject_duplicate_json_keys(text).map_err(IndexError::Contract)?;
    let value = serde_json::from_str(text)
        .map_err(|_| IndexError::Contract("existing artifact index is invalid JSON".into()))?;
    validate_index(&value)?;
    Ok(value)
}

fn validate_index(value: &Value) -> Result<(), IndexError> {
    let object = value
        .as_object()
        .ok_or_else(|| IndexError::Contract("existing artifact index must be an object".into()))?;
    if object.len() != 3
        || value["schema"] != INDEX_SCHEMA
        || !value["entries"].is_array()
        || !value["diagnostics"].is_array()
    {
        return Err(IndexError::Contract(
            "existing artifact index contract is invalid".into(),
        ));
    }
    let mut repos = BTreeSet::new();
    for entry in value["entries"].as_array().unwrap() {
        validate_index_entry(entry)?;
        let repo = entry["repo"].as_str().unwrap();
        if !repos.insert(repo) {
            return Err(IndexError::Contract(
                "existing artifact index contains duplicate repositories".into(),
            ));
        }
    }
    for diagnostic in value["diagnostics"].as_array().unwrap() {
        validate_index_diagnostic(diagnostic)?;
    }
    Ok(())
}

fn validate_index_entry(value: &Value) -> Result<(), IndexError> {
    exact_keys(
        value,
        &[
            "repo",
            "run",
            "runIdentity",
            "snapshotIdentity",
            "outcome",
            "manifest",
            "artifactRefs",
        ],
        "artifact index entry",
    )?;
    let repo = portable_name(&value["repo"], "entry repo")?;
    let run = portable_name(&value["run"], "entry run")?;
    let _ = (repo, run);
    let run_identity = value["runIdentity"]
        .as_str()
        .filter(|identity| {
            identity.strip_prefix("dag-v1:").is_some_and(|digest| {
                digest.len() >= 2 && digest.len() % 2 == 0 && is_lower_hex(digest)
            })
        })
        .ok_or_else(|| IndexError::Contract("entry runIdentity is invalid".into()))?;
    let _ = run_identity;
    let snapshot = digest(&value["snapshotIdentity"], "entry snapshotIdentity")?;
    if value["outcome"] != "completed" {
        return Err(IndexError::Contract("entry outcome is invalid".into()));
    }
    exact_keys(&value["manifest"], &["path", "sha256"], "entry manifest")?;
    let manifest_digest = digest(&value["manifest"]["sha256"], "entry manifest sha256")?;
    if value["manifest"]["path"].as_str() != Some(&format!("objects/sha256/{manifest_digest}")) {
        return Err(IndexError::Contract(
            "entry manifest path is not content-addressed".into(),
        ));
    }
    let refs = value["artifactRefs"]
        .as_array()
        .ok_or_else(|| IndexError::Contract("entry artifactRefs must be an array".into()))?;
    let mut paths = BTreeSet::new();
    for artifact in refs {
        validate_artifact_ref_shape(artifact).map_err(IndexError::Contract)?;
        let path = artifact["path"].as_str().unwrap();
        if !portable_relative_path(path) || !paths.insert(path) {
            return Err(IndexError::Contract(
                "entry Artifact Ref path is invalid or duplicated".into(),
            ));
        }
        if artifact["consumedSnapshotIdentity"].as_str() != Some(snapshot) {
            return Err(IndexError::Contract(
                "entry Artifact Ref snapshot binding differs from the entry".into(),
            ));
        }
    }
    Ok(())
}

fn validate_index_diagnostic(value: &Value) -> Result<(), IndexError> {
    exact_keys(
        value,
        &["repo", "run", "classification", "reason"],
        "artifact index diagnostic",
    )?;
    portable_name(&value["repo"], "diagnostic repo")?;
    portable_name(&value["run"], "diagnostic run")?;
    if !matches!(
        value["classification"].as_str(),
        Some("staging" | "incomplete" | "forged" | "legacy" | "non_completed")
    ) {
        return Err(IndexError::Contract(
            "diagnostic classification is invalid".into(),
        ));
    }
    if !value["reason"]
        .as_str()
        .is_some_and(|reason| !reason.trim().is_empty())
    {
        return Err(IndexError::Contract("diagnostic reason is invalid".into()));
    }
    Ok(())
}

fn exact_keys(value: &Value, keys: &[&str], context: &str) -> Result<(), IndexError> {
    let object = value
        .as_object()
        .ok_or_else(|| IndexError::Contract(format!("{context} must be an object")))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = keys.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(IndexError::Contract(format!(
            "{context} fields are invalid"
        )))
    }
}

fn portable_name<'a>(value: &'a Value, context: &str) -> Result<&'a str, IndexError> {
    value
        .as_str()
        .filter(|name| {
            !name.is_empty()
                && *name != "."
                && *name != ".."
                && !name.ends_with(['.', ' '])
                && !name.chars().any(|character| {
                    character.is_control()
                        || matches!(
                            character,
                            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                        )
                })
        })
        .ok_or_else(|| IndexError::Contract(format!("{context} is not a portable directory name")))
}

fn portable_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path.split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && !component.ends_with(['.', ' '])
                && !component.chars().any(char::is_control)
        })
}

fn digest<'a>(value: &'a Value, context: &str) -> Result<&'a str, IndexError> {
    value
        .as_str()
        .filter(|digest| digest.len() == 64 && is_lower_hex(digest))
        .ok_or_else(|| IndexError::Contract(format!("{context} is invalid")))
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn write_index(path: &Path, value: &Value) -> Result<(), IndexError> {
    let parent = path
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| IndexError::Contract("index output parent must exist".into()))?;
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    let temp = parent.join(format!(".artifact-index.tmp.{}", std::process::id()));
    fs::write(&temp, &bytes)
        .map_err(|error| IndexError::HostIo(format!("write artifact index temp: {error}")))?;
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| IndexError::HostIo(format!("replace artifact index: {error}")))?;
    }
    fs::rename(&temp, path)
        .map_err(|error| IndexError::HostIo(format!("publish artifact index: {error}")))
}
