use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::stable_artifact::{self, StableReadError};

static NONCE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub(crate) struct ArtifactWriteContract {
    pub(crate) artifact_schema: &'static str,
    pub(crate) artifact_type: &'static str,
    pub(crate) max_bytes: u64,
    pub(crate) validate_payload: fn(&[u8]) -> Result<(), String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InterruptAfter {
    StageCreated,
    TempCreated,
    FileSynced,
    ObjectPublished,
    DirectorySynced,
}

#[derive(Clone, Debug)]
pub(crate) struct WriterOptions {
    pub(crate) nonce: String,
    pub(crate) interrupt_after: Option<InterruptAfter>,
    pub(crate) before_publish: Option<fn(&Path, &str) -> Result<(), String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StagedArtifactRef {
    pub(crate) artifact_schema: String,
    pub(crate) artifact_type: String,
    pub(crate) path: String,
    pub(crate) sha256: String,
    pub(crate) consumed_snapshot_identity: String,
    pub(crate) size: u64,
    pub(crate) owned_by_stage: bool,
}

impl StagedArtifactRef {
    pub(crate) fn to_artifact_ref_value(&self) -> Value {
        json!({
            "schema": "code-intel-artifact-ref.v1",
            "artifactSchema": self.artifact_schema,
            "type": self.artifact_type,
            "path": self.path,
            "sha256": self.sha256,
            "consumedSnapshotIdentity": self.consumed_snapshot_identity,
        })
    }
}

#[derive(Debug)]
pub(crate) enum StageWriteError {
    Contract(String),
    Boundary(String),
    Collision(String),
    Interrupted(InterruptAfter),
    HostIo(String),
}

impl fmt::Display for StageWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(message)
            | Self::Boundary(message)
            | Self::Collision(message)
            | Self::HostIo(message) => formatter.write_str(message),
            Self::Interrupted(phase) => write!(formatter, "injected interruption after {phase:?}"),
        }
    }
}

impl std::error::Error for StageWriteError {}

impl StageWriteError {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Contract(_) => "contract",
            Self::Boundary(_) => "boundary",
            Self::Collision(_) => "collision",
            Self::Interrupted(_) => "interrupted",
            Self::HostIo(_) => "host_io",
        }
    }
}

#[derive(Debug)]
struct Layout {
    _root: HeldDir,
    _staging: HeldDir,
    stage: HeldDir,
    _objects: HeldDir,
    sha256: HeldDir,
}

#[derive(Debug)]
struct Ownership {
    files: BTreeMap<PathBuf, OwnedFileIdentity>,
    directories: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OwnedFileIdentity {
    volume: u64,
    file: u128,
}

#[derive(Debug, Default)]
struct CleanupReport {
    residuals: Vec<PathBuf>,
    failures: Vec<String>,
}

impl CleanupReport {
    fn is_clean(&self) -> bool {
        self.residuals.is_empty() && self.failures.is_empty()
    }

    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if !self.residuals.is_empty() {
            parts.push(format!(
                "residual preserved: {}",
                self.residuals
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.failures.is_empty() {
            parts.push(format!("cleanup failures: {}", self.failures.join("; ")));
        }
        parts.join("; ")
    }
}

#[derive(Debug)]
pub(crate) struct StagedWriter {
    authority_root: PathBuf,
    stage_path: PathBuf,
    snapshot_identity: String,
    artifacts: Vec<StagedArtifactRef>,
    interrupt_after: Option<InterruptAfter>,
    before_publish: Option<fn(&Path, &str) -> Result<(), String>>,
    layout: Option<Layout>,
    ownership: Ownership,
}

#[derive(Debug)]
pub(crate) struct StagedArtifactSet {
    authority_root: PathBuf,
    stage_path: PathBuf,
    snapshot_identity: String,
    artifacts: Vec<StagedArtifactRef>,
    layout: Option<Layout>,
    ownership: Ownership,
}

impl StagedWriter {
    pub(crate) fn begin(
        authority_root: &Path,
        snapshot_identity: &str,
    ) -> Result<Self, StageWriteError> {
        let sequence = NONCE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| StageWriteError::HostIo(error.to_string()))?
            .as_nanos();
        Self::begin_with_options(
            authority_root,
            snapshot_identity,
            WriterOptions {
                nonce: format!("{}-{nanos}-{sequence}", std::process::id()),
                interrupt_after: None,
                before_publish: None,
            },
        )
    }

    pub(crate) fn begin_with_options(
        authority_root: &Path,
        snapshot_identity: &str,
        options: WriterOptions,
    ) -> Result<Self, StageWriteError> {
        validate_digest(snapshot_identity, "snapshot identity")?;
        validate_nonce(&options.nonce)?;
        let root = HeldDir::open_root(authority_root)?;
        let authority_volume = root.volume_identity()?;
        let staging = root.ensure_child(".staging")?;
        require_same_volume(authority_volume, &staging)?;
        let stage_name = format!("stage-{}", options.nonce);
        let stage = staging.create_unique_child(&stage_name)?;
        let stage_path = stage.path.clone();
        let mut owned_directories = vec![stage_path.clone()];
        let setup = (|| {
            require_same_volume(authority_volume, &stage)?;
            let objects = stage.create_unique_child("objects")?;
            owned_directories.push(objects.path.clone());
            require_same_volume(authority_volume, &objects)?;
            let sha256 = objects.create_unique_child("sha256")?;
            owned_directories.push(sha256.path.clone());
            require_same_volume(authority_volume, &sha256)?;
            Ok((objects, sha256))
        })();
        let (objects, sha256) = match setup {
            Ok(layout) => layout,
            Err(cause) => {
                drop(stage);
                drop(staging);
                drop(root);
                let report = cleanup_owned_entries(&mut Ownership {
                    files: BTreeMap::new(),
                    directories: owned_directories,
                });
                return Err(append_cleanup_report(cause, &report));
            }
        };
        let mut writer = Self {
            authority_root: authority_root.to_path_buf(),
            stage_path: stage_path.clone(),
            snapshot_identity: snapshot_identity.to_string(),
            artifacts: Vec::new(),
            interrupt_after: options.interrupt_after,
            before_publish: options.before_publish,
            layout: Some(Layout {
                _root: root,
                _staging: staging,
                stage,
                _objects: objects,
                sha256,
            }),
            ownership: Ownership {
                files: BTreeMap::new(),
                directories: owned_directories,
            },
        };
        if let Err(cause) = writer.interrupt(InterruptAfter::StageCreated) {
            let report = writer.cleanup_owned();
            return Err(append_cleanup_report(cause, &report));
        }
        Ok(writer)
    }

    pub(crate) fn stage(
        &mut self,
        bytes: &[u8],
        contract: ArtifactWriteContract,
    ) -> Result<StagedArtifactRef, StageWriteError> {
        let result = self.stage_once(bytes, contract);
        if let Err(cause) = result {
            let report = self.cleanup_owned();
            return Err(append_cleanup_report(cause, &report));
        }
        result
    }

    fn stage_once(
        &mut self,
        bytes: &[u8],
        contract: ArtifactWriteContract,
    ) -> Result<StagedArtifactRef, StageWriteError> {
        validate_contract(contract, bytes)?;
        let digest = sha256_hex(bytes);
        let temp_name = format!(
            ".tmp-{}-{}",
            digest,
            NONCE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let layout = self
            .layout
            .as_ref()
            .ok_or_else(|| StageWriteError::HostIo("staged writer is not active".to_string()))?;

        let mut temporary = layout.sha256.create_new_file(&temp_name)?;
        let temp_path = layout.sha256.path.join(&temp_name);
        let temp_identity = platform::file_identity(&temporary)
            .map_err(|error| io_error("identify owned temporary", &temp_path, error))?;
        self.ownership
            .files
            .insert(temp_path.clone(), temp_identity);
        self.interrupt(InterruptAfter::TempCreated)?;
        let write_result = (|| {
            temporary
                .write_all(bytes)
                .map_err(|error| io_error("write staged artifact", &temp_path, error))?;
            temporary
                .sync_all()
                .map_err(|error| io_error("flush staged artifact", &temp_path, error))?;
            Ok::<_, StageWriteError>(())
        })();
        drop(temporary);
        if let Err(error) = write_result {
            if layout.sha256.remove_owned_file(&temp_name).is_ok() {
                self.ownership.files.remove(&temp_path);
            }
            return Err(error);
        }
        self.interrupt(InterruptAfter::FileSynced)?;

        if let Some(hook) = self.before_publish {
            hook(&layout.sha256.path, &digest).map_err(|error| {
                StageWriteError::HostIo(format!("before-publish proving hook failed: {error}"))
            })?;
        }

        let owned_by_stage = match layout.sha256.publish_no_replace(&temp_name, &digest) {
            Ok(()) => {
                self.ownership.files.remove(&temp_path);
                self.ownership
                    .files
                    .insert(layout.sha256.path.join(&digest), temp_identity);
                true
            }
            Err(PublishError::Exists) => {
                layout.sha256.remove_owned_file(&temp_name)?;
                self.ownership.files.remove(&temp_path);
                self.ownership
                    .files
                    .contains_key(&layout.sha256.path.join(&digest))
            }
            Err(PublishError::Io(error)) => {
                if layout.sha256.remove_owned_file(&temp_name).is_ok() {
                    self.ownership.files.remove(&temp_path);
                }
                return Err(error);
            }
        };
        self.interrupt(InterruptAfter::ObjectPublished)?;
        layout.sha256.sync_metadata()?;
        self.interrupt(InterruptAfter::DirectorySynced)?;

        let verified = stable_artifact::read_beneath(
            &self.stage_path,
            &["objects", "sha256", &digest],
            contract.max_bytes,
        )
        .map_err(map_stable_error)?;
        if verified.bytes != bytes || sha256_hex(&verified.bytes) != digest {
            return Err(StageWriteError::Collision(format!(
                "content-addressed object collision for sha256:{digest}"
            )));
        }
        (contract.validate_payload)(&verified.bytes).map_err(|error| {
            StageWriteError::Contract(format!("staged payload validation failed: {error}"))
        })?;

        let artifact = StagedArtifactRef {
            artifact_schema: contract.artifact_schema.to_string(),
            artifact_type: contract.artifact_type.to_string(),
            path: format!("objects/sha256/{digest}"),
            sha256: digest,
            consumed_snapshot_identity: self.snapshot_identity.clone(),
            size: bytes.len() as u64,
            owned_by_stage,
        };
        validate_ref_coherence(&artifact)?;
        self.artifacts.push(artifact.clone());
        Ok(artifact)
    }

    pub(crate) fn observed_effects(&self) -> &'static [&'static str] {
        &["local_write"]
    }

    pub(crate) fn seal(mut self) -> Result<StagedArtifactSet, StageWriteError> {
        if self
            .artifacts
            .iter()
            .any(|artifact| !artifact.owned_by_stage)
        {
            let report = self.cleanup_owned();
            return Err(append_cleanup_report(
                StageWriteError::Collision(
                    "cannot seal a staging set containing an unowned deduplicated object"
                        .to_string(),
                ),
                &report,
            ));
        }
        let layout = self
            .layout
            .as_ref()
            .ok_or_else(|| StageWriteError::HostIo("staged writer is not active".to_string()))?;
        layout.sha256.sync_metadata()?;
        layout.stage.sync_metadata()?;
        let layout = self.layout.take();
        Ok(StagedArtifactSet {
            authority_root: self.authority_root.clone(),
            stage_path: self.stage_path.clone(),
            snapshot_identity: self.snapshot_identity.clone(),
            artifacts: std::mem::take(&mut self.artifacts),
            layout,
            ownership: std::mem::replace(
                &mut self.ownership,
                Ownership {
                    files: BTreeMap::new(),
                    directories: Vec::new(),
                },
            ),
        })
    }

    fn interrupt(&self, phase: InterruptAfter) -> Result<(), StageWriteError> {
        if self.interrupt_after == Some(phase) {
            Err(StageWriteError::Interrupted(phase))
        } else {
            Ok(())
        }
    }

    fn cleanup_owned(&mut self) -> CleanupReport {
        let mut report = cleanup_owned_files(&mut self.ownership);
        self.layout.take();
        cleanup_owned_directories(&mut self.ownership, &mut report);
        report
    }
}

impl Drop for StagedWriter {
    fn drop(&mut self) {
        let _ = self.cleanup_owned();
    }
}

impl StagedArtifactSet {
    pub(crate) fn path(&self) -> &Path {
        &self.stage_path
    }

    pub(crate) fn authority_root(&self) -> &Path {
        &self.authority_root
    }

    pub(crate) fn artifacts(&self) -> &[StagedArtifactRef] {
        &self.artifacts
    }

    pub(crate) fn to_manifest_value(&self) -> Value {
        json!({
            "schema": "code-intel-staged-artifact-set.v1",
            "snapshotIdentity": self.snapshot_identity,
            "artifacts": self.artifacts.iter().map(StagedArtifactRef::to_artifact_ref_value).collect::<Vec<_>>(),
        })
    }

    pub(crate) fn prepare_for_commit(&mut self) -> Result<(), StageWriteError> {
        if let Some(layout) = self.layout.as_ref() {
            layout.sha256.sync_metadata()?;
            layout.stage.sync_metadata()?;
        }
        self.layout.take();
        Ok(())
    }
}

impl Drop for StagedArtifactSet {
    fn drop(&mut self) {
        let mut report = cleanup_owned_files(&mut self.ownership);
        self.layout.take();
        cleanup_owned_directories(&mut self.ownership, &mut report);
    }
}

pub(crate) fn sync_directory_path(path: &Path) -> Result<(), StageWriteError> {
    let directory = platform::open_directory(path)
        .map_err(|error| classify_open_error("open directory for sync", path, error))?;
    platform::sync_directory(&directory).map_err(|error| io_error("sync directory", path, error))
}

fn cleanup_owned_entries(ownership: &mut Ownership) -> CleanupReport {
    let mut report = cleanup_owned_files(ownership);
    cleanup_owned_directories(ownership, &mut report);
    report
}

fn cleanup_owned_files(ownership: &mut Ownership) -> CleanupReport {
    let mut report = CleanupReport::default();
    let files = ownership
        .files
        .iter()
        .map(|(path, identity)| (path.clone(), *identity))
        .collect::<Vec<_>>();
    for (path, expected) in files {
        match platform::open_file_identity(&path) {
            Ok(actual) if actual == expected => match fs::remove_file(&path) {
                Ok(()) => {
                    ownership.files.remove(&path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    ownership.files.remove(&path);
                }
                Err(error) => report.failures.push(format!(
                    "remove identity-owned file {}: {error}",
                    path.display()
                )),
            },
            Ok(_) => {
                ownership.files.remove(&path);
                report.residuals.push(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ownership.files.remove(&path);
            }
            Err(error) => report.failures.push(format!(
                "verify identity-owned file {}: {error}",
                path.display()
            )),
        }
    }
    report
}

fn cleanup_owned_directories(ownership: &mut Ownership, report: &mut CleanupReport) {
    for path in ownership.directories.iter().rev() {
        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                report.residuals.push(path.clone());
            }
            Err(error) => report.failures.push(format!(
                "remove owned directory {}: {error}",
                path.display()
            )),
        }
    }
    ownership.directories.clear();
}

fn append_cleanup_report(cause: StageWriteError, report: &CleanupReport) -> StageWriteError {
    if report.is_clean() {
        return cause;
    }
    let detail = format!("{cause}; {}", report.describe());
    match cause {
        StageWriteError::Contract(_) => StageWriteError::Contract(detail),
        StageWriteError::Boundary(_) => StageWriteError::Boundary(detail),
        StageWriteError::Collision(_) => StageWriteError::Collision(detail),
        StageWriteError::Interrupted(phase) => StageWriteError::HostIo(format!(
            "interrupted after {phase:?}; {}",
            report.describe()
        )),
        StageWriteError::HostIo(_) => StageWriteError::HostIo(detail),
    }
}

fn validate_contract(contract: ArtifactWriteContract, bytes: &[u8]) -> Result<(), StageWriteError> {
    for (label, value) in [
        ("artifact schema", contract.artifact_schema),
        ("artifact type", contract.artifact_type),
    ] {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(StageWriteError::Contract(format!(
                "{label} is not a portable identifier"
            )));
        }
    }
    if contract.max_bytes == 0 || bytes.len() as u64 > contract.max_bytes {
        return Err(StageWriteError::Contract(format!(
            "artifact payload exceeds the registered {} byte limit",
            contract.max_bytes
        )));
    }
    (contract.validate_payload)(bytes).map_err(|error| {
        StageWriteError::Contract(format!(
            "artifact payload failed schema validation: {error}"
        ))
    })
}

fn validate_ref_coherence(artifact: &StagedArtifactRef) -> Result<(), StageWriteError> {
    validate_digest(&artifact.sha256, "artifact digest")?;
    validate_digest(
        &artifact.consumed_snapshot_identity,
        "consumed snapshot identity",
    )?;
    if artifact.path != format!("objects/sha256/{}", artifact.sha256) {
        return Err(StageWriteError::Contract(
            "artifact path and digest are incoherent".to_string(),
        ));
    }
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> Result<(), StageWriteError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(StageWriteError::Contract(format!(
            "{label} must be a lowercase sha256 digest"
        )))
    }
}

fn validate_nonce(nonce: &str) -> Result<(), StageWriteError> {
    if !nonce.is_empty()
        && nonce.len() <= 96
        && nonce
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        Ok(())
    } else {
        Err(StageWriteError::Boundary(
            "staging nonce is not a portable path component".to_string(),
        ))
    }
}

fn map_stable_error(error: StableReadError) -> StageWriteError {
    match error {
        StableReadError::TooLarge(message) => StageWriteError::Contract(message),
        StableReadError::Boundary(message) | StableReadError::Identity(message) => {
            StageWriteError::Boundary(message)
        }
        StableReadError::HostIo(message) => StageWriteError::HostIo(message),
    }
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> StageWriteError {
    StageWriteError::HostIo(format!("{action} {}: {error}", path.display()))
}

#[derive(Debug)]
struct HeldDir {
    path: PathBuf,
    handle: File,
}

enum PublishError {
    Exists,
    Io(StageWriteError),
}

impl HeldDir {
    fn open_root(path: &Path) -> Result<Self, StageWriteError> {
        let handle = platform::open_directory(path)
            .map_err(|error| classify_open_error("open staging authority", path, error))?;
        Ok(Self {
            path: path.to_path_buf(),
            handle,
        })
    }

    fn ensure_child(&self, name: &str) -> Result<Self, StageWriteError> {
        match platform::create_directory(&self.handle, &self.path, name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(io_error(
                    "create staging directory",
                    &self.path.join(name),
                    error,
                ))
            }
        }
        self.open_child(name)
    }

    fn create_unique_child(&self, name: &str) -> Result<Self, StageWriteError> {
        match platform::create_directory(&self.handle, &self.path, name) {
            Ok(()) => self.open_child(name),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(StageWriteError::Collision(format!(
                    "staging nonce collision: {}",
                    self.path.join(name).display()
                )))
            }
            Err(error) => Err(io_error(
                "create owned staging directory",
                &self.path.join(name),
                error,
            )),
        }
    }

    fn open_child(&self, name: &str) -> Result<Self, StageWriteError> {
        let path = self.path.join(name);
        let handle = platform::open_child_directory(&self.handle, &path, name)
            .map_err(|error| classify_open_error("open staging directory", &path, error))?;
        Ok(Self { path, handle })
    }

    fn create_new_file(&self, name: &str) -> Result<File, StageWriteError> {
        platform::create_new_file(&self.handle, &self.path, name).map_err(|error| {
            io_error(
                "create staged artifact temporary",
                &self.path.join(name),
                error,
            )
        })
    }

    fn publish_no_replace(&self, source: &str, target: &str) -> Result<(), PublishError> {
        platform::publish_no_replace(&self.handle, &self.path, source, target).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                PublishError::Exists
            } else {
                PublishError::Io(io_error(
                    "publish staged content-addressed object",
                    &self.path.join(target),
                    error,
                ))
            }
        })
    }

    fn remove_owned_file(&self, name: &str) -> Result<(), StageWriteError> {
        platform::remove_file(&self.handle, &self.path, name)
            .map_err(|error| io_error("remove owned temporary", &self.path.join(name), error))
    }

    fn sync_metadata(&self) -> Result<(), StageWriteError> {
        platform::sync_directory(&self.handle)
            .map_err(|error| io_error("sync staging directory", &self.path, error))
    }

    fn volume_identity(&self) -> Result<u64, StageWriteError> {
        platform::volume_identity(&self.handle)
            .map_err(|error| io_error("read staging volume identity", &self.path, error))
    }
}

fn require_same_volume(expected: u64, directory: &HeldDir) -> Result<(), StageWriteError> {
    if directory.volume_identity()? == expected {
        Ok(())
    } else {
        Err(StageWriteError::Boundary(format!(
            "staging directory crosses the authority volume: {}",
            directory.path.display()
        )))
    }
}

fn classify_open_error(action: &str, path: &Path, error: std::io::Error) -> StageWriteError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::NotFound
            | std::io::ErrorKind::NotADirectory
            | std::io::ErrorKind::InvalidInput
    ) {
        StageWriteError::Boundary(format!("{action} {}: {error}", path.display()))
    } else {
        StageWriteError::HostIo(format!("{action} {}: {error}", path.display()))
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;

    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const WRITE_THROUGH: u32 = 0x0000_0008;

    pub(super) fn open_directory(path: &Path) -> std::io::Result<File> {
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(1 | 2)
            .custom_flags(OPEN_REPARSE_POINT | BACKUP_SEMANTICS)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "directory authority is a reparse point or not a directory",
            ));
        }
        Ok(file)
    }

    pub(super) fn open_child_directory(
        _parent: &File,
        path: &Path,
        _name: &str,
    ) -> std::io::Result<File> {
        open_directory(path)
    }

    pub(super) fn create_directory(
        _parent: &File,
        parent_path: &Path,
        name: &str,
    ) -> std::io::Result<()> {
        fs::create_dir(parent_path.join(name))
    }

    pub(super) fn create_new_file(
        _parent: &File,
        parent_path: &Path,
        name: &str,
    ) -> std::io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(1 | 2)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(parent_path.join(name))
    }

    pub(super) fn publish_no_replace(
        _parent: &File,
        parent_path: &Path,
        source: &str,
        target: &str,
    ) -> std::io::Result<()> {
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
            fn GetLastError() -> u32;
        }
        let wide = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>()
        };
        // Rust's standard filesystem operations opt into extended-length paths, but this
        // direct Win32 call does not. Canonicalizing the existing authority directory gives
        // MoveFileExW a verbatim (\\?\-prefixed) absolute path and keeps publication working
        // when the content-addressed target crosses the legacy MAX_PATH boundary.
        let authority = fs::canonicalize(parent_path)?;
        let source = wide(&authority.join(source));
        let target = wide(&authority.join(target));
        let ok = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), WRITE_THROUGH) };
        if ok != 0 {
            return Ok(());
        }
        let code = unsafe { GetLastError() };
        if matches!(code, 80 | 183) {
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "content-addressed target already exists",
            ))
        } else {
            Err(std::io::Error::from_raw_os_error(code as i32))
        }
    }

    pub(super) fn remove_file(
        _parent: &File,
        parent_path: &Path,
        name: &str,
    ) -> std::io::Result<()> {
        fs::remove_file(parent_path.join(name))
    }

    pub(super) fn sync_directory(_directory: &File) -> std::io::Result<()> {
        // MoveFileExW(MOVEFILE_WRITE_THROUGH) is the Windows metadata durability barrier.
        Ok(())
    }

    pub(super) fn volume_identity(directory: &File) -> std::io::Result<u64> {
        Ok(file_identity(directory)?.volume)
    }

    pub(super) fn file_identity(file: &File) -> std::io::Result<OwnedFileIdentity> {
        #[repr(C)]
        struct FileIdInfo {
            volume_serial_number: u64,
            file_id: [u8; 16],
        }
        unsafe extern "system" {
            fn GetFileInformationByHandleEx(
                file: *mut c_void,
                class: i32,
                information: *mut c_void,
                size: u32,
            ) -> i32;
        }
        let mut info = std::mem::MaybeUninit::<FileIdInfo>::uninit();
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                18,
                info.as_mut_ptr().cast(),
                std::mem::size_of::<FileIdInfo>() as u32,
            )
        };
        if ok == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            let info = unsafe { info.assume_init() };
            Ok(OwnedFileIdentity {
                volume: info.volume_serial_number,
                file: u128::from_le_bytes(info.file_id),
            })
        }
    }

    pub(super) fn open_file_identity(path: &Path) -> std::io::Result<OwnedFileIdentity> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "owned file path is a reparse point or not a regular file",
            ));
        }
        file_identity(&file)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    const O_WRONLY: i32 = 1;
    const O_CREAT: i32 = 0x40;
    const O_EXCL: i32 = 0x80;
    const O_CLOEXEC: i32 = 0x80000;
    const O_DIRECTORY: i32 = 0x10000;
    const O_NOFOLLOW: i32 = 0x20000;

    unsafe extern "C" {
        fn open(pathname: *const i8, flags: i32, ...) -> i32;
        fn openat(dirfd: i32, pathname: *const i8, flags: i32, ...) -> i32;
        fn mkdirat(dirfd: i32, pathname: *const i8, mode: u32) -> i32;
        fn linkat(
            olddirfd: i32,
            oldpath: *const i8,
            newdirfd: i32,
            newpath: *const i8,
            flags: i32,
        ) -> i32;
        fn unlinkat(dirfd: i32, pathname: *const i8, flags: i32) -> i32;
    }

    fn name(value: &str) -> std::io::Result<CString> {
        CString::new(value)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in name"))
    }

    pub(super) fn open_directory(path: &Path) -> std::io::Result<File> {
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in path"))?;
        let fd = unsafe { open(path.as_ptr(), O_CLOEXEC | O_DIRECTORY | O_NOFOLLOW) };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    pub(super) fn open_child_directory(
        parent: &File,
        _path: &Path,
        child: &str,
    ) -> std::io::Result<File> {
        let child = name(child)?;
        let fd = unsafe {
            openat(
                parent.as_raw_fd(),
                child.as_ptr(),
                O_CLOEXEC | O_DIRECTORY | O_NOFOLLOW,
            )
        };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    pub(super) fn create_directory(
        parent: &File,
        _parent_path: &Path,
        child: &str,
    ) -> std::io::Result<()> {
        let child = name(child)?;
        if unsafe { mkdirat(parent.as_raw_fd(), child.as_ptr(), 0o700) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    pub(super) fn create_new_file(
        parent: &File,
        _parent_path: &Path,
        child: &str,
    ) -> std::io::Result<File> {
        let child = name(child)?;
        let fd = unsafe {
            openat(
                parent.as_raw_fd(),
                child.as_ptr(),
                O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
                0o600u32,
            )
        };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    pub(super) fn publish_no_replace(
        parent: &File,
        _parent_path: &Path,
        source: &str,
        target: &str,
    ) -> std::io::Result<()> {
        let source = name(source)?;
        let target = name(target)?;
        if unsafe {
            linkat(
                parent.as_raw_fd(),
                source.as_ptr(),
                parent.as_raw_fd(),
                target.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { unlinkat(parent.as_raw_fd(), source.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn remove_file(
        parent: &File,
        _parent_path: &Path,
        child: &str,
    ) -> std::io::Result<()> {
        let child = name(child)?;
        if unsafe { unlinkat(parent.as_raw_fd(), child.as_ptr(), 0) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    pub(super) fn sync_directory(directory: &File) -> std::io::Result<()> {
        directory.sync_all()
    }

    pub(super) fn volume_identity(directory: &File) -> std::io::Result<u64> {
        Ok(directory.metadata()?.dev())
    }

    pub(super) fn file_identity(file: &File) -> std::io::Result<OwnedFileIdentity> {
        let metadata = file.metadata()?;
        Ok(OwnedFileIdentity {
            volume: metadata.dev(),
            file: metadata.ino() as u128,
        })
    }

    pub(super) fn open_file_identity(path: &Path) -> std::io::Result<OwnedFileIdentity> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(O_CLOEXEC | O_NOFOLLOW)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "owned file path is not a regular file",
            ));
        }
        file_identity(&file)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::*;

    fn unsupported() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "secure staged writes are supported only on Windows and Linux",
        )
    }

    pub(super) fn open_directory(_path: &Path) -> std::io::Result<File> {
        Err(unsupported())
    }
    pub(super) fn open_child_directory(
        _parent: &File,
        _path: &Path,
        _child: &str,
    ) -> std::io::Result<File> {
        Err(unsupported())
    }
    pub(super) fn create_directory(
        _parent: &File,
        _path: &Path,
        _child: &str,
    ) -> std::io::Result<()> {
        Err(unsupported())
    }
    pub(super) fn create_new_file(
        _parent: &File,
        _path: &Path,
        _child: &str,
    ) -> std::io::Result<File> {
        Err(unsupported())
    }
    pub(super) fn publish_no_replace(
        _parent: &File,
        _path: &Path,
        _source: &str,
        _target: &str,
    ) -> std::io::Result<()> {
        Err(unsupported())
    }
    pub(super) fn remove_file(_parent: &File, _path: &Path, _child: &str) -> std::io::Result<()> {
        Err(unsupported())
    }
    pub(super) fn sync_directory(_directory: &File) -> std::io::Result<()> {
        Err(unsupported())
    }
    pub(super) fn volume_identity(_directory: &File) -> std::io::Result<u64> {
        Err(unsupported())
    }
    pub(super) fn file_identity(_file: &File) -> std::io::Result<OwnedFileIdentity> {
        Err(unsupported())
    }
    pub(super) fn open_file_identity(_path: &Path) -> std::io::Result<OwnedFileIdentity> {
        Err(unsupported())
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut data = bytes.to_vec();
    let bits = (data.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bits.to_be_bytes());
    let mut hash = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in data.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = hash;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let first = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let second = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(first);
            d = c;
            c = b;
            b = a;
            a = first.wrapping_add(second);
        }
        for (state, value) in hash.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *state = state.wrapping_add(value);
        }
    }
    hash.iter().map(|value| format!("{value:08x}")).collect()
}
