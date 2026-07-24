use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use crate::artifact_ref;
use crate::capability::{reject_duplicate_json_keys, sha256_hex, validate_artifact_ref_shape};
use crate::stable_artifact;
use crate::staged_artifact::{self, ArtifactWriteContract, StagedArtifactSet, StagedWriter};

const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MARKER_BYTES: u64 = 64 * 1024;
static MARKER_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublicationPhase {
    Prevalidate,
    Rename,
    DirectorySync,
    MarkerTemp,
    MarkerPublish,
    PostMarkerVerify,
    Rollback,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CommitOptions {
    pub(crate) interrupt_before: Option<PublicationPhase>,
    pub(crate) fail_marker_sync: bool,
    pub(crate) fail_marker_read: bool,
}

#[derive(Debug)]
pub(crate) enum CommitError {
    Contract(String),
    Collision(String),
    Interrupted(PublicationPhase),
    HostIo(String),
}

impl fmt::Display for CommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(message) | Self::Collision(message) | Self::HostIo(message) => {
                f.write_str(message)
            }
            Self::Interrupted(phase) => write!(f, "injected interruption before {phase:?}"),
        }
    }
}

impl std::error::Error for CommitError {}

#[derive(Clone, Debug)]
pub(crate) struct CommitResult {
    pub(crate) final_path: PathBuf,
    pub(crate) marker: Value,
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match parse_cli(raw).and_then(execute_cli) {
        Ok(result) => {
            println!("{}", serde_json::to_string(&result.marker).unwrap());
            0
        }
        Err(CommitError::Contract(message) | CommitError::Collision(message)) => {
            eprintln!("{message}");
            65
        }
        Err(CommitError::HostIo(message)) => {
            eprintln!("{message}");
            74
        }
        Err(CommitError::Interrupted(phase)) => {
            eprintln!("injected interruption before {phase:?}");
            75
        }
    }
}

struct CommitCli {
    source_root: PathBuf,
    authority_root: PathBuf,
    manifest_ref: PathBuf,
    final_name: String,
}

fn parse_cli(raw: &[String]) -> Result<CommitCli, CommitError> {
    if raw.first().map(String::as_str) != Some("commit") {
        return Err(CommitError::Contract("usage: run commit --source-root <A09-artifact-root> --authority-root <publication-root> --manifest-ref <artifact-ref.json> --final-name <name>".to_string()));
    }
    let mut source_root = None;
    let mut authority_root = None;
    let mut manifest_ref = None;
    let mut final_name = None;
    let mut index = 1;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--source-root" | "--authority-root" | "--manifest-ref" | "--final-name"
        ) {
            return Err(CommitError::Contract(format!(
                "unknown run commit argument: {flag}"
            )));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| CommitError::Contract(format!("{flag} requires one value")))?;
        let target = match flag {
            "--source-root" => &mut source_root,
            "--authority-root" => &mut authority_root,
            "--manifest-ref" => &mut manifest_ref,
            "--final-name" => {
                if final_name.replace(value.clone()).is_some() {
                    return Err(CommitError::Contract("duplicate --final-name".to_string()));
                }
                index += 2;
                continue;
            }
            _ => unreachable!(),
        };
        if target.replace(PathBuf::from(value)).is_some() {
            return Err(CommitError::Contract(format!("duplicate {flag}")));
        }
        index += 2;
    }
    let cli = CommitCli {
        source_root: source_root
            .ok_or_else(|| CommitError::Contract("--source-root is required".to_string()))?,
        authority_root: authority_root
            .ok_or_else(|| CommitError::Contract("--authority-root is required".to_string()))?,
        manifest_ref: manifest_ref
            .ok_or_else(|| CommitError::Contract("--manifest-ref is required".to_string()))?,
        final_name: final_name
            .ok_or_else(|| CommitError::Contract("--final-name is required".to_string()))?,
    };
    if !cli.source_root.is_dir() || !cli.authority_root.is_dir() {
        return Err(CommitError::Contract(
            "source and authority roots must be existing directories".to_string(),
        ));
    }
    validate_final_name(&cli.final_name)?;
    Ok(cli)
}

fn execute_cli(cli: CommitCli) -> Result<CommitResult, CommitError> {
    publish_existing(
        &cli.source_root,
        &cli.authority_root,
        &cli.manifest_ref,
        &cli.final_name,
    )
}

pub(crate) fn publish_existing(
    source_root: &Path,
    authority_root: &Path,
    manifest_ref: &Path,
    final_name: &str,
) -> Result<CommitResult, CommitError> {
    if !source_root.is_dir() || !authority_root.is_dir() {
        return Err(CommitError::Contract(
            "source and authority roots must be existing directories".to_string(),
        ));
    }
    validate_final_name(final_name)?;
    let bytes = fs::read(manifest_ref)
        .map_err(|error| CommitError::HostIo(format!("read manifest Artifact Ref: {error}")))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| CommitError::Contract("manifest Artifact Ref is not UTF-8".to_string()))?;
    reject_duplicate_json_keys(text).map_err(CommitError::Contract)?;
    let source_manifest_ref: Value = serde_json::from_str(text)
        .map_err(|_| CommitError::Contract("manifest Artifact Ref is invalid JSON".to_string()))?;
    let manifest = prevalidate_existing(source_root, &source_manifest_ref)?;
    let snapshot = manifest["snapshotIdentity"].as_str().unwrap();
    let source_refs = manifest_artifact_refs(&manifest)?;
    let verified = artifact_ref::verify_inputs(
        &Value::Array(source_refs.clone()),
        Some(source_root),
        snapshot,
    )
    .map_err(|error| CommitError::Contract(error.message().to_string()))?;
    let mut writer = StagedWriter::begin(authority_root, snapshot).map_err(map_stage_error)?;
    let mut published_refs = Vec::with_capacity(source_refs.len());
    for (source_ref, artifact) in source_refs.iter().zip(verified.iter()) {
        let contract = artifact_ref::registered_contract(source_ref)
            .map_err(|error| CommitError::Contract(error.message().to_string()))?;
        let staged_ref = writer
            .stage(
                artifact.bytes(),
                ArtifactWriteContract {
                    artifact_schema: contract.artifact_schema,
                    artifact_type: contract.artifact_type,
                    max_bytes: contract.max_bytes,
                    validate_payload: contract.validate_payload,
                },
            )
            .map_err(map_stage_error)?;
        published_refs.push(staged_ref.to_artifact_ref_value());
    }
    let mut published_manifest = manifest;
    replace_manifest_artifact_refs(&mut published_manifest, &published_refs)?;
    let manifest_bytes = serde_json::to_vec(&published_manifest)
        .map_err(|error| CommitError::Contract(format!("serialize published manifest: {error}")))?;
    let staged_manifest_ref = writer
        .stage(
            &manifest_bytes,
            ArtifactWriteContract {
                artifact_schema: "code-intel-run-manifest.v1",
                artifact_type: "run.manifest",
                max_bytes: MAX_MANIFEST_BYTES,
                validate_payload: validate_run_manifest_bytes,
            },
        )
        .map_err(map_stage_error)?
        .to_artifact_ref_value();
    let staged = writer.seal().map_err(map_stage_error)?;
    commit(
        staged,
        &staged_manifest_ref,
        final_name,
        CommitOptions::default(),
    )
}

fn replace_manifest_artifact_refs(
    manifest: &mut Value,
    replacements: &[Value],
) -> Result<(), CommitError> {
    let nodes = manifest["nodes"]
        .as_object_mut()
        .ok_or_else(|| CommitError::Contract("run manifest nodes must be an object".to_string()))?;
    let mut replacement = replacements.iter();
    for node in nodes.values_mut() {
        if node["status"] != "succeeded" && node["status"] != "domain_failed" {
            continue;
        }
        let artifacts = node["artifacts"].as_array_mut().ok_or_else(|| {
            CommitError::Contract(
                "artifact-producing run node artifacts must be an array".to_string(),
            )
        })?;
        for artifact in artifacts {
            *artifact = replacement
                .next()
                .ok_or_else(|| {
                    CommitError::Contract(
                        "published Artifact Ref count is smaller than the run manifest".to_string(),
                    )
                })?
                .clone();
        }
    }
    if replacement.next().is_some() {
        return Err(CommitError::Contract(
            "published Artifact Ref count is larger than the run manifest".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn commit(
    mut staged: StagedArtifactSet,
    manifest_ref: &Value,
    final_name: &str,
    options: CommitOptions,
) -> Result<CommitResult, CommitError> {
    interrupt(&options, PublicationPhase::Prevalidate)?;
    validate_final_name(final_name)?;
    let stage_path = staged.path().to_path_buf();
    let authority_root = staged.authority_root().to_path_buf();
    let manifest = prevalidate(staged.artifacts(), &stage_path, manifest_ref)?;
    staged
        .prepare_for_commit()
        .map_err(|error| CommitError::HostIo(error.to_string()))?;

    interrupt(&options, PublicationPhase::Rename)?;
    let final_path = authority_root.join(final_name);
    rename_directory_no_replace(&stage_path, &final_path)?;

    interrupt_after_promotion(&options, PublicationPhase::DirectorySync, &final_path)?;
    sync_directory(&authority_root)?;

    publish_marker(&final_path, manifest_ref, &manifest, &options)
}

pub(crate) fn recover(
    final_path: &Path,
    manifest_ref: &Value,
    options: CommitOptions,
) -> Result<CommitResult, CommitError> {
    if final_path.join("run-complete.json").exists() {
        return Err(CommitError::Collision(
            "run is already committed or has a competing completion marker".to_string(),
        ));
    }
    interrupt(&options, PublicationPhase::Prevalidate)?;
    let manifest = prevalidate_existing(final_path, manifest_ref)?;
    let authority_root = final_path.parent().ok_or_else(|| {
        CommitError::Contract("recoverable run has no authority root".to_string())
    })?;
    sync_directory(authority_root)?;
    publish_marker(final_path, manifest_ref, &manifest, &options)
}

pub(crate) fn classify(path: &Path) -> &'static str {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with(".staging") || name.starts_with("stage-") || name.contains(".staging-")
        })
    {
        return "staged";
    }
    match validate_published_marker(path) {
        Ok(_) => "committed",
        Err(_) if path.is_dir() => "legacy-uncommitted",
        Err(_) => "invalid",
    }
}

pub(crate) fn validate_run_manifest_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("run manifest exceeds size limit".to_string());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "run manifest is not UTF-8".to_string())?;
    reject_duplicate_json_keys(text)?;
    let value: Value =
        serde_json::from_str(text).map_err(|_| "run manifest is invalid JSON".to_string())?;
    validate_run_manifest(&value)
}

fn prevalidate(
    staged_refs: &[crate::staged_artifact::StagedArtifactRef],
    root: &Path,
    manifest_ref: &Value,
) -> Result<Value, CommitError> {
    let declared = staged_refs
        .iter()
        .map(|item| item.to_artifact_ref_value())
        .collect::<Vec<_>>();
    if !declared.iter().any(|item| item == manifest_ref) {
        return Err(CommitError::Contract(
            "run manifest Artifact Ref is not owned by the staged set".to_string(),
        ));
    }
    prevalidate_refs(root, &declared, manifest_ref)
}

fn prevalidate_existing(root: &Path, manifest_ref: &Value) -> Result<Value, CommitError> {
    let manifest = read_manifest(root, manifest_ref)?;
    let refs = manifest_artifact_refs(&manifest)?;
    let mut all = refs;
    all.push(manifest_ref.clone());
    prevalidate_refs(root, &all, manifest_ref)
}

fn prevalidate_refs(
    root: &Path,
    refs: &[Value],
    manifest_ref: &Value,
) -> Result<Value, CommitError> {
    validate_artifact_ref_shape(manifest_ref).map_err(CommitError::Contract)?;
    if manifest_ref["artifactSchema"] != "code-intel-run-manifest.v1"
        || manifest_ref["type"] != "run.manifest"
    {
        return Err(CommitError::Contract(
            "run manifest Artifact Ref contract is invalid".to_string(),
        ));
    }
    let manifest = read_manifest(root, manifest_ref)?;
    let snapshot = manifest["snapshotIdentity"].as_str().unwrap();
    if manifest_ref["consumedSnapshotIdentity"] != snapshot {
        return Err(CommitError::Contract(
            "run manifest Artifact Ref snapshot does not match the manifest".to_string(),
        ));
    }
    let manifest_refs = manifest_artifact_refs(&manifest)?;
    let declared_non_manifest = refs
        .iter()
        .filter(|item| *item != manifest_ref)
        .map(ref_identity)
        .collect::<Result<BTreeSet<_>, _>>()?;
    let manifest_identities = manifest_refs
        .iter()
        .map(ref_identity)
        .collect::<Result<BTreeSet<_>, _>>()?;
    if declared_non_manifest.len() != refs.iter().filter(|item| *item != manifest_ref).count()
        || manifest_identities.len() != manifest_refs.len()
    {
        return Err(CommitError::Contract(
            "run publication contains duplicate Artifact Refs".to_string(),
        ));
    }
    if declared_non_manifest != manifest_identities {
        return Err(CommitError::Contract(
            "staged Artifact Refs do not exactly match the complete run manifest".to_string(),
        ));
    }
    artifact_ref::verify_inputs(&Value::Array(manifest_refs), Some(root), snapshot)
        .map_err(|error| CommitError::Contract(error.message().to_string()))?;
    Ok(manifest)
}

fn read_manifest(root: &Path, manifest_ref: &Value) -> Result<Value, CommitError> {
    validate_artifact_ref_shape(manifest_ref).map_err(CommitError::Contract)?;
    let path = manifest_ref["path"]
        .as_str()
        .ok_or_else(|| CommitError::Contract("run manifest path is invalid".to_string()))?;
    let components = path.split('/').collect::<Vec<_>>();
    let stable = stable_artifact::read_beneath(root, &components, MAX_MANIFEST_BYTES)
        .map_err(map_stable_error)?;
    let digest = sha256_hex(&stable.bytes);
    if manifest_ref["sha256"] != digest {
        return Err(CommitError::Contract(
            "run manifest digest mismatch".to_string(),
        ));
    }
    validate_run_manifest_bytes(&stable.bytes).map_err(CommitError::Contract)?;
    serde_json::from_slice(&stable.bytes)
        .map_err(|_| CommitError::Contract("run manifest is invalid JSON".to_string()))
}

fn validate_run_manifest(value: &Value) -> Result<(), String> {
    exact(
        value,
        &[
            "schema",
            "runIdentity",
            "snapshotIdentity",
            "outcome",
            "nodes",
        ],
        "run manifest",
    )?;
    if value["schema"] != "code-intel-run-manifest.v1"
        || !value["runIdentity"]
            .as_str()
            .is_some_and(valid_run_identity)
        || !value["snapshotIdentity"].as_str().is_some_and(valid_digest)
        || !matches!(
            value["outcome"].as_str(),
            Some("completed" | "domain_failed" | "domain_unknown" | "process_failed")
        )
    {
        return Err("run manifest identity/outcome is incomplete or invalid".to_string());
    }
    let nodes = value["nodes"]
        .as_object()
        .ok_or("run manifest nodes must be an object")?;
    if nodes.is_empty() {
        return Err("run manifest must contain at least one terminal node".to_string());
    }
    for (id, node) in nodes {
        if !id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            || !id.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
            })
        {
            return Err("run manifest node id is invalid".to_string());
        }
        match node["status"].as_str() {
            Some("succeeded") => {
                exact(
                    node,
                    &["status", "verdict", "artifacts"],
                    "succeeded run node",
                )?;
                if !matches!(
                    node["verdict"].as_str(),
                    Some("pass" | "unknown" | "not_applicable")
                ) || !node["artifacts"].is_array()
                {
                    return Err("succeeded run node is invalid".to_string());
                }
            }
            Some("domain_failed") => {
                exact(
                    node,
                    &["status", "verdict", "diagnostic", "artifacts"],
                    "domain-failed run node",
                )?;
                if node["verdict"] != "fail"
                    || !node["diagnostic"]
                        .as_str()
                        .is_some_and(|text| !text.is_empty())
                    || !node["artifacts"].is_array()
                {
                    return Err("domain-failed run node is invalid".to_string());
                }
            }
            Some("process_failed") => {
                exact(
                    node,
                    &["status", "failure", "diagnostic"],
                    "process-failed run node",
                )?;
                if !matches!(
                    node["failure"].as_str(),
                    Some("contract" | "unavailable" | "internal" | "io")
                ) || !node["diagnostic"]
                    .as_str()
                    .is_some_and(|text| !text.is_empty())
                {
                    return Err("process-failed run node is invalid".to_string());
                }
            }
            Some("dependency_blocked") => {
                exact(node, &["status", "blockedBy"], "blocked run node")?;
                let blocked = node["blockedBy"]
                    .as_array()
                    .ok_or("blocked run node is invalid")?;
                let unique = blocked
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>();
                if blocked.is_empty()
                    || blocked
                        .iter()
                        .any(|item| !item.as_str().is_some_and(|text| !text.is_empty()))
                    || unique.len() != blocked.len()
                {
                    return Err("blocked run node is invalid".to_string());
                }
            }
            _ => return Err("run manifest contains a non-terminal node".to_string()),
        }
    }
    Ok(())
}

fn manifest_artifact_refs(manifest: &Value) -> Result<Vec<Value>, CommitError> {
    let mut refs = Vec::new();
    for node in manifest["nodes"].as_object().unwrap().values() {
        if node["status"] == "succeeded" || node["status"] == "domain_failed" {
            for item in node["artifacts"].as_array().unwrap() {
                validate_artifact_ref_shape(item).map_err(CommitError::Contract)?;
                refs.push(item.clone());
            }
        }
    }
    Ok(refs)
}

fn publish_marker(
    final_path: &Path,
    manifest_ref: &Value,
    manifest: &Value,
    options: &CommitOptions,
) -> Result<CommitResult, CommitError> {
    interrupt_after_promotion(options, PublicationPhase::MarkerTemp, final_path)?;
    let marker = json!({
        "schema":"code-intel-run-commit.v1",
        "runIdentity":manifest["runIdentity"],
        "snapshotIdentity":manifest["snapshotIdentity"],
        "manifest":{"path":manifest_ref["path"],"sha256":manifest_ref["sha256"]}
    });
    let bytes = serde_json::to_vec(&marker).unwrap();
    let marker_path = final_path.join("run-complete.json");
    let (temp_path, temp_name, mut temp) = create_marker_temp(final_path)?;
    if let Err(error) = temp.write_all(&bytes).and_then(|_| temp.sync_all()) {
        drop(temp);
        return Err(CommitError::HostIo(format!(
            "write completion marker: {error}; owned temp retained for cleanup: {}",
            temp_path.display()
        )));
    }
    drop(temp);
    let temp_identity =
        stable_artifact::read_beneath(final_path, &[temp_name.as_str()], MAX_MARKER_BYTES)
            .map_err(map_stable_error)?
            .id;
    if let Err(error) = interrupt(options, PublicationPhase::MarkerPublish) {
        remove_owned_marker(final_path, &temp_path, &bytes, temp_identity);
        return Err(error);
    }
    if let Err(error) = rename_file_no_replace(&temp_path, &marker_path) {
        remove_owned_marker(final_path, &temp_path, &bytes, temp_identity);
        return Err(error);
    }
    let marker_sync = if options.fail_marker_sync {
        Err(CommitError::HostIo(
            "injected completion marker directory sync failure".to_string(),
        ))
    } else {
        sync_directory(final_path)
    };
    if let Err(error) = marker_sync {
        remove_owned_marker(final_path, &marker_path, &bytes, temp_identity);
        let _ = sync_directory(final_path);
        return Err(error);
    }
    if let Err(error) = interrupt(options, PublicationPhase::PostMarkerVerify) {
        remove_owned_marker(final_path, &marker_path, &bytes, temp_identity);
        let _ = sync_directory(final_path);
        return Err(error);
    }
    let verified = if options.fail_marker_read {
        Err(CommitError::HostIo(
            "injected completion marker read failure".to_string(),
        ))
    } else {
        validate_published_marker(final_path)
    };
    let verified = match verified {
        Ok(marker) => marker,
        Err(error) => {
            remove_owned_marker(final_path, &marker_path, &bytes, temp_identity);
            let _ = sync_directory(final_path);
            return Err(error);
        }
    };
    if verified != marker {
        remove_owned_marker(final_path, &marker_path, &bytes, temp_identity);
        let _ = sync_directory(final_path);
        return Err(CommitError::Contract(
            "published completion marker verification failed".to_string(),
        ));
    }
    if options.interrupt_before == Some(PublicationPhase::Rollback) {
        remove_owned_marker(final_path, &marker_path, &bytes, temp_identity);
        let _ = sync_directory(final_path);
        return Err(CommitError::Interrupted(PublicationPhase::Rollback));
    }
    Ok(CommitResult {
        final_path: final_path.to_path_buf(),
        marker,
    })
}

fn create_marker_temp(final_path: &Path) -> Result<(PathBuf, String, fs::File), CommitError> {
    for _ in 0..1024 {
        let sequence = MARKER_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".run-complete.json.tmp.{}.{}", std::process::id(), sequence);
        let path = final_path.join(&name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, name, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(CommitError::HostIo(format!(
                    "create completion marker temp: {error}"
                )))
            }
        }
    }
    Err(CommitError::Collision(
        "completion marker temp namespace is exhausted".to_string(),
    ))
}

pub(crate) fn validate_committed_run(root: &Path) -> Result<(Value, Value), CommitError> {
    let marker = validate_published_marker(root)?;
    let manifest_ref = json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-run-manifest.v1",
        "type":"run.manifest",
        "path":marker["manifest"]["path"],
        "sha256":marker["manifest"]["sha256"],
        "consumedSnapshotIdentity":marker["snapshotIdentity"]
    });
    let manifest = prevalidate_existing(root, &manifest_ref)?;
    Ok((marker, manifest))
}

fn validate_published_marker(root: &Path) -> Result<Value, CommitError> {
    let stable = stable_artifact::read_beneath(root, &["run-complete.json"], MAX_MARKER_BYTES)
        .map_err(map_stable_error)?;
    let text = std::str::from_utf8(&stable.bytes)
        .map_err(|_| CommitError::Contract("completion marker is not UTF-8".to_string()))?;
    reject_duplicate_json_keys(text).map_err(CommitError::Contract)?;
    let marker: Value = serde_json::from_str(text)
        .map_err(|_| CommitError::Contract("completion marker is invalid JSON".to_string()))?;
    exact(
        &marker,
        &["schema", "runIdentity", "snapshotIdentity", "manifest"],
        "completion marker",
    )
    .map_err(CommitError::Contract)?;
    exact(
        &marker["manifest"],
        &["path", "sha256"],
        "completion marker manifest",
    )
    .map_err(CommitError::Contract)?;
    if marker["schema"] != "code-intel-run-commit.v1"
        || !marker["runIdentity"]
            .as_str()
            .is_some_and(valid_run_identity)
        || !marker["snapshotIdentity"]
            .as_str()
            .is_some_and(valid_digest)
        || !marker["manifest"]["sha256"]
            .as_str()
            .is_some_and(valid_digest)
    {
        return Err(CommitError::Contract(
            "completion marker fields are invalid".to_string(),
        ));
    }
    let manifest_ref = json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":"code-intel-run-manifest.v1","type":"run.manifest","path":marker["manifest"]["path"],"sha256":marker["manifest"]["sha256"],"consumedSnapshotIdentity":marker["snapshotIdentity"]});
    let manifest = read_manifest(root, &manifest_ref)?;
    if manifest["runIdentity"] != marker["runIdentity"]
        || manifest["snapshotIdentity"] != marker["snapshotIdentity"]
    {
        return Err(CommitError::Contract(
            "completion marker identity does not bind the run manifest".to_string(),
        ));
    }
    Ok(marker)
}

fn ref_identity(value: &Value) -> Result<(String, String, String, String), CommitError> {
    validate_artifact_ref_shape(value).map_err(CommitError::Contract)?;
    Ok((
        value["path"].as_str().unwrap().to_string(),
        value["sha256"].as_str().unwrap().to_string(),
        value["artifactSchema"].as_str().unwrap().to_string(),
        value["type"].as_str().unwrap().to_string(),
    ))
}

fn validate_final_name(name: &str) -> Result<(), CommitError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.starts_with('.')
        || name.contains("staging")
    {
        Err(CommitError::Contract(
            "final run name is invalid".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn interrupt(options: &CommitOptions, phase: PublicationPhase) -> Result<(), CommitError> {
    if options.interrupt_before == Some(phase) {
        Err(CommitError::Interrupted(phase))
    } else {
        Ok(())
    }
}

fn interrupt_after_promotion(
    options: &CommitOptions,
    phase: PublicationPhase,
    final_path: &Path,
) -> Result<(), CommitError> {
    interrupt(options, phase).map_err(|error| {
        debug_assert!(final_path.is_dir());
        error
    })
}

fn exact(value: &Value, fields: &[&str], label: &str) -> Result<(), String> {
    let actual = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = fields.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label} fields are invalid"))
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}
fn valid_run_identity(value: &str) -> bool {
    value.strip_prefix("dag-v1:").is_some_and(|tail| {
        !tail.is_empty()
            && tail.len() % 2 == 0
            && tail
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    })
}

fn remove_owned_marker(
    root: &Path,
    path: &Path,
    expected: &[u8],
    expected_identity: stable_artifact::FileId,
) {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    if stable_artifact::read_beneath(root, &[name], MAX_MARKER_BYTES)
        .ok()
        .is_some_and(|stable| stable.id == expected_identity && stable.bytes == expected)
    {
        let _ = fs::remove_file(path);
    }
}

fn map_stable_error(error: stable_artifact::StableReadError) -> CommitError {
    match error {
        stable_artifact::StableReadError::HostIo(message) => CommitError::HostIo(message),
        stable_artifact::StableReadError::TooLarge(message)
        | stable_artifact::StableReadError::Boundary(message)
        | stable_artifact::StableReadError::Identity(message) => CommitError::Contract(message),
    }
}

fn map_stage_error(error: staged_artifact::StageWriteError) -> CommitError {
    match error {
        staged_artifact::StageWriteError::Contract(message)
        | staged_artifact::StageWriteError::Boundary(message) => CommitError::Contract(message),
        staged_artifact::StageWriteError::Collision(message) => CommitError::Collision(message),
        staged_artifact::StageWriteError::Interrupted(phase) => {
            CommitError::HostIo(format!("A06 staging interrupted after {phase:?}"))
        }
        staged_artifact::StageWriteError::HostIo(message) => CommitError::HostIo(message),
    }
}

fn sync_directory(path: &Path) -> Result<(), CommitError> {
    staged_artifact::sync_directory_path(path)
        .map_err(|error| CommitError::HostIo(error.to_string()))
}

#[cfg(windows)]
fn rename_directory_no_replace(source: &Path, destination: &Path) -> Result<(), CommitError> {
    rename_windows_no_replace(source, destination, "promote staged run")
}
#[cfg(windows)]
fn rename_file_no_replace(source: &Path, destination: &Path) -> Result<(), CommitError> {
    rename_windows_no_replace(source, destination, "publish completion marker")
}

#[cfg(windows)]
fn rename_windows_no_replace(
    source: &Path,
    destination: &Path,
    action: &str,
) -> Result<(), CommitError> {
    use std::os::windows::ffi::OsStrExt;
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) };
    if result != 0 {
        Ok(())
    } else {
        Err(CommitError::Collision(format!(
            "{action} without replacement: {}",
            std::io::Error::last_os_error()
        )))
    }
}

#[cfg(target_os = "linux")]
fn rename_directory_no_replace(source: &Path, destination: &Path) -> Result<(), CommitError> {
    rename_linux_no_replace(source, destination, "promote staged run")
}
#[cfg(target_os = "linux")]
fn rename_file_no_replace(source: &Path, destination: &Path) -> Result<(), CommitError> {
    rename_linux_no_replace(source, destination, "publish completion marker")
}

#[cfg(target_os = "linux")]
fn rename_linux_no_replace(
    source: &Path,
    destination: &Path,
    action: &str,
) -> Result<(), CommitError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn renameat2(
            olddirfd: i32,
            oldpath: *const i8,
            newdirfd: i32,
            newpath: *const i8,
            flags: u32,
        ) -> i32;
    }
    const AT_FDCWD: i32 = -100;
    const RENAME_NOREPLACE: u32 = 1;
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| CommitError::Contract("source path contains NUL".to_string()))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| CommitError::Contract("destination path contains NUL".to_string()))?;
    let result = unsafe {
        renameat2(
            AT_FDCWD,
            source.as_ptr(),
            AT_FDCWD,
            destination.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(CommitError::Collision(format!(
            "{action} without replacement: {}",
            std::io::Error::last_os_error()
        )))
    }
}

#[cfg(target_os = "macos")]
fn rename_directory_no_replace(source: &Path, destination: &Path) -> Result<(), CommitError> {
    rename_macos_no_replace(source, destination, "promote staged run")
}
#[cfg(target_os = "macos")]
fn rename_file_no_replace(source: &Path, destination: &Path) -> Result<(), CommitError> {
    rename_macos_no_replace(source, destination, "publish completion marker")
}

#[cfg(target_os = "macos")]
fn rename_macos_no_replace(
    source: &Path,
    destination: &Path,
    action: &str,
) -> Result<(), CommitError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn renamex_np(old: *const i8, new: *const i8, flags: u32) -> i32;
    }
    const RENAME_EXCL: u32 = 0x0000_0004;
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| CommitError::Contract("source path contains NUL".to_string()))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| CommitError::Contract("destination path contains NUL".to_string()))?;
    let result = unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(CommitError::Collision(format!(
            "{action} without replacement: {}",
            std::io::Error::last_os_error()
        )))
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn rename_directory_no_replace(_: &Path, _: &Path) -> Result<(), CommitError> {
    Err(CommitError::HostIo(
        "atomic no-replace directory promotion is unsupported on this platform".to_string(),
    ))
}
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn rename_file_no_replace(_: &Path, _: &Path) -> Result<(), CommitError> {
    Err(CommitError::HostIo(
        "atomic no-replace marker publication is unsupported on this platform".to_string(),
    ))
}
