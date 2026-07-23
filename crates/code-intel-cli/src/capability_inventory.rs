use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifact_ref::VerifiedArtifact;
use crate::snapshot;

#[path = "builtin_provider_evidence.rs"]
mod builtin_provider_evidence;
#[path = "compatibility_retirement_gate.rs"]
mod compatibility_retirement_gate;
#[path = "compatibility_retirement_ticket.rs"]
mod compatibility_retirement_ticket;
#[path = "delivery_light_speed.rs"]
mod delivery_light_speed;
#[path = "doctor_adapter.rs"]
mod doctor_adapter;
#[path = "hospital_diagnosis.rs"]
mod hospital_diagnosis;
#[path = "native_code_evidence.rs"]
mod native_code_evidence;
#[path = "project_orientation.rs"]
mod project_orientation;
#[path = "project_orientation_benchmark.rs"]
mod project_orientation_benchmark;
#[path = "understanding_quadrant.rs"]
mod understanding_quadrant;

const EXCLUDES: [&str; 10] = [
    "!**/.git/**",
    "!**/node_modules/**",
    "!**/.repowise/**",
    "!**/.understand-anything/**",
    "!**/.sentrux/**",
    "!**/target/**",
    "!**/dist/**",
    "!**/build/**",
    "!**/.venv/**",
    "!**/__pycache__/**",
];

static MIRROR_NONCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct AdapterOutput {
    pub(crate) artifacts: Vec<AdapterArtifact>,
    pub(crate) observed_effects: Vec<String>,
    pub(crate) domain_verdict: AdapterDomainVerdict,
    pub(crate) domain_failure: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdapterDomainVerdict {
    Pass,
    Fail,
    Unknown,
    NotApplicable,
}

impl AdapterDomainVerdict {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Unknown => "unknown",
            Self::NotApplicable => "not_applicable",
        }
    }
}
pub(crate) struct AdapterArtifact {
    pub(crate) artifact_schema: String,
    pub(crate) artifact_type: String,
    pub(crate) relative_path: String,
    pub(crate) bytes: Vec<u8>,
}
#[derive(Debug)]
pub(crate) enum AdapterError {
    InvalidOptions(String),
    Contract(String),
    Unavailable(String),
    Internal(String),
    Io(String),
}

pub(crate) fn execute(
    adapter: &str,
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    for input in verified_inputs {
        let _owned_input_contract = (
            input.bytes(),
            input.artifact_schema(),
            input.artifact_type(),
            input.sha256(),
            input.consumed_snapshot_identity(),
        );
    }
    match adapter {
        "repository.snapshot.compat" => repository_snapshot(request, verified_inputs, out),
        "inventory.rg.compat" => inventory(request, out),
        "evidence.native-code.compat" => {
            native_code_evidence::execute(request, verified_inputs, out)
        }
        "project.orientation.compat" => project_orientation::execute(request, verified_inputs, out),
        "understanding.quadrant.compat" => {
            understanding_quadrant::execute(request, verified_inputs, out)
        }
        "compatibility.retirement-gate.compat" => {
            compatibility_retirement_gate::execute(request, verified_inputs, out)
        }
        "compatibility.retirement-ticket-template.compat" => {
            compatibility_retirement_ticket::execute(request, verified_inputs, out)
        }
        "project.orientation-benchmark.compat" => {
            project_orientation_benchmark::execute(request, verified_inputs, out)
        }
        "delivery.light-speed-measure.compat" => {
            delivery_light_speed::execute(request, verified_inputs, out)
        }
        "diagnosis.hospital.compat" => hospital_diagnosis::execute(request, verified_inputs, out),
        "provider.graph-builtin.compat" => {
            builtin_provider_evidence::graph_admission(request, verified_inputs, out)
        }
        "provider.sentrux-builtin.compat" => {
            builtin_provider_evidence::sentrux_admission(request, verified_inputs, out)
        }
        "doctor.envelope.compat" => doctor_adapter::execute(request, verified_inputs, out),
        "advisory.workflow-recommend.compat" => {
            workflow_recommendation(request, verified_inputs, out)
        }
        other => Err(AdapterError::Unavailable(format!(
            "runtime adapter is not installed: {other}"
        ))),
    }
}

fn workflow_recommendation(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "advisory.workflow-recommend does not accept input artifacts".into(),
        ));
    }
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options
        .keys()
        .any(|key| !matches!(key.as_str(), "repoPath" | "auto"))
    {
        return Err(AdapterError::InvalidOptions(
            "advisory.workflow-recommend accepts only repoPath/auto".into(),
        ));
    }
    let repo = options
        .get("repoPath")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(Path::new)
        .ok_or_else(|| AdapterError::InvalidOptions("options.repoPath must be non-empty".into()))?;
    if !repo.is_dir() {
        return Err(AdapterError::InvalidOptions(format!(
            "repoPath is not a directory: {}",
            repo.display()
        )));
    }
    let auto = match options.get("auto") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| {
            AdapterError::InvalidOptions("options.auto must be boolean when present".into())
        })?,
    };
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("Invoke-WorkflowRecommendation.ps1");
    if !script.is_file() {
        return Err(AdapterError::Unavailable(format!(
            "workflow recommendation facade is unavailable: {}",
            script.display()
        )));
    }
    let mut command = Command::new("pwsh");
    command
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(&script)
        .arg("-RepoPath")
        .arg(repo)
        .args(["-Quiet", "-Json"]);
    if auto {
        command.arg("-Auto");
    }
    let output = command
        .output()
        .map_err(|error| AdapterError::Unavailable(format!("start workflow atom: {error}")))?;
    if !output.status.success() {
        return Err(AdapterError::Internal(format!(
            "workflow atom failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let proposal: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        AdapterError::Contract(format!(
            "workflow atom stdout is not one JSON proposal: {error}"
        ))
    })?;
    validate_workflow_proposal(&proposal)?;
    let bytes = serde_json::to_vec(&proposal)
        .map_err(|error| AdapterError::Internal(format!("serialize workflow proposal: {error}")))?;
    publish_named(out, "workflow-recommendation.json", &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-advisory-workflow-recommendation.v1".into(),
            artifact_type: "advisory.workflow-recommendation".into(),
            relative_path: "workflow-recommendation.json".into(),
            bytes,
        }],
        observed_effects: vec![],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn validate_workflow_proposal(value: &Value) -> Result<(), AdapterError> {
    let object = value.as_object().ok_or_else(|| {
        AdapterError::Contract("workflow recommendation must be an object".into())
    })?;
    let expected = [
        "schema",
        "kind",
        "recommendation",
        "evidence",
        "confidence",
        "alternatives",
        "provenance",
        "effects",
    ];
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(AdapterError::Contract(
            "workflow recommendation top-level contract is not exact".into(),
        ));
    }
    if value["schema"] != "code-intel-advisory-workflow-recommendation.v1"
        || value["kind"] != "proposal"
        || !matches!(
            value["confidence"].as_str(),
            Some("low" | "medium" | "high")
        )
        || value["evidence"].as_array().map_or(true, Vec::is_empty)
        || value["alternatives"]
            .as_array()
            .map_or(true, |items| items.len() < 3)
        || value["effects"]
            .as_array()
            .map_or(true, |items| !items.is_empty())
        || value
            .pointer("/provenance/capabilityId")
            .and_then(Value::as_str)
            != Some("advisory.workflow-recommend")
    {
        return Err(AdapterError::Contract(
            "workflow recommendation violates the advisory proposal boundary".into(),
        ));
    }
    Ok(())
}

fn repository_snapshot(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "repository.snapshot does not accept input artifacts".into(),
        ));
    }
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options.len() != 1 || !options.contains_key("repoPath") {
        return Err(AdapterError::InvalidOptions(
            "repository.snapshot accepts only options.repoPath".into(),
        ));
    }
    let repo = options["repoPath"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(Path::new)
        .ok_or_else(|| AdapterError::InvalidOptions("options.repoPath must be non-empty".into()))?;
    let document = snapshot::build_for_capability(repo, &request["snapshot"])
        .map_err(snapshot_adapter_error)?;
    let bytes = serde_json::to_vec(&document)
        .map_err(|error| AdapterError::Internal(format!("serialize snapshot: {error}")))?;
    publish_named(out, "snapshot.json", &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-repository-snapshot.v1".into(),
            artifact_type: "repository.snapshot".into(),
            relative_path: "snapshot.json".into(),
            bytes,
        }],
        observed_effects: vec!["repo_read".into(), "local_write".into()],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn inventory(request: &Value, out: &Path) -> Result<AdapterOutput, AdapterError> {
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options
        .keys()
        .any(|k| !matches!(k.as_str(), "repoPath" | "inventoryExclude"))
    {
        return Err(AdapterError::InvalidOptions(
            "inventory.rg accepts only repoPath/inventoryExclude; --out is the only write boundary"
                .into(),
        ));
    }
    let repo = options
        .get("repoPath")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(Path::new)
        .ok_or_else(|| AdapterError::InvalidOptions("options.repoPath must be non-empty".into()))?;
    if !repo.is_dir() {
        return Err(AdapterError::InvalidOptions(format!(
            "repoPath is not a directory: {}",
            repo.display()
        )));
    }
    let lease =
        snapshot::begin_consumption(repo, &request["snapshot"]).map_err(snapshot_adapter_error)?;
    let rg = if cfg!(windows) { "rg.exe" } else { "rg" };
    let mut baseline_globs = EXCLUDES
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    baseline_globs.extend(
        lease
            .inventory_gitlink_paths()
            .iter()
            .map(|path| gitlink_exclude_glob(path)),
    );
    let mut glob_patterns = baseline_globs.clone();
    if let Some(extra) = options.get("inventoryExclude") {
        let list = extra.as_array().ok_or_else(|| {
            AdapterError::InvalidOptions(
                "inventoryExclude must be unique non-empty glob strings".into(),
            )
        })?;
        let mut seen = std::collections::BTreeSet::new();
        for p in list {
            let p = p.as_str().filter(|v| !v.is_empty()).ok_or_else(|| {
                AdapterError::InvalidOptions(
                    "inventoryExclude must be unique non-empty glob strings".into(),
                )
            })?;
            if !seen.insert(p) {
                return Err(AdapterError::InvalidOptions(
                    "duplicate inventoryExclude".into(),
                ));
            }
            glob_patterns.push(p.to_string());
        }
    }
    let mut actual_baseline = run_rg_files(
        rg,
        repo,
        lease.scopes(),
        &baseline_globs,
        RgIgnoreMode::SnapshotControls,
    )?;
    #[cfg(debug_assertions)]
    if let Ok(extra) = std::env::var("CODE_INTEL_TEST_RG_EXTRA_PATH") {
        actual_baseline.insert(normalize_inventory_path(&extra));
    }
    let (expected_baseline, filtered) = mirror_path_sets(
        rg,
        lease.scopes(),
        &baseline_globs,
        &glob_patterns,
        &lease.inventory_mirror_files(),
    )?;
    verify_inventory_path_sets(&actual_baseline, &expected_baseline)?;
    lease.verify_after(repo).map_err(snapshot_adapter_error)?;
    let records = filtered
        .into_iter()
        .map(String::into_bytes)
        .collect::<Vec<_>>();
    let bytes = join_records(&records, if cfg!(windows) { b'\n' } else { 0 });
    publish(out, &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-file-inventory.v1".into(),
            artifact_type: "inventory.files".into(),
            relative_path: "files.txt".into(),
            bytes,
        }],
        observed_effects: vec!["repo_read".into(), "local_write".into()],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn verify_inventory_path_sets(
    actual: &BTreeSet<String>,
    expected: &BTreeSet<String>,
) -> Result<(), AdapterError> {
    let extra_count = actual.difference(expected).count();
    // The snapshot mirror owns frozen ignore-control semantics. The live view may
    // omit snapshot-bound paths when current ignore bytes differ, but it must
    // never introduce a path that the snapshot did not bind.
    if extra_count == 0 {
        return Ok(());
    }
    const DIAGNOSTIC_SAMPLE_LIMIT: usize = 32;
    let extra = actual
        .difference(expected)
        .take(DIAGNOSTIC_SAMPLE_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    let missing_count = expected.difference(actual).count();
    let missing = expected
        .difference(actual)
        .take(DIAGNOSTIC_SAMPLE_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    Err(AdapterError::Contract(format!(
        "inventory baseline path set differs from snapshot manifest; extra_count={extra_count}; extra_samples={extra:?}; missing_count={missing_count}; missing_samples={missing:?}"
    )))
}

fn gitlink_exclude_glob(path: &str) -> String {
    let mut pattern = String::with_capacity(path.len() + 5);
    pattern.push('!');
    for character in path.chars() {
        if matches!(character, '\\' | '*' | '?' | '[' | ']' | '{' | '}' | '!') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push_str("/**");
    pattern
}

#[derive(Clone, Copy)]
enum RgIgnoreMode {
    Disabled,
    SnapshotControls,
}

fn run_rg_files(
    rg: &str,
    cwd: &Path,
    scopes: &[String],
    glob_patterns: &[String],
    ignore_mode: RgIgnoreMode,
) -> Result<BTreeSet<String>, AdapterError> {
    let mut command = Command::new(rg);
    command.args(["--files", "--hidden", "--null", "--no-require-git"]);
    match ignore_mode {
        RgIgnoreMode::Disabled => {
            command.arg("--no-ignore");
        }
        RgIgnoreMode::SnapshotControls => {
            command.args([
                "--no-ignore-parent",
                "--no-ignore-global",
                "--no-ignore-exclude",
            ]);
        }
    }
    for pattern in glob_patterns {
        command.args(["-g", pattern]);
    }
    let output = command
        .env_remove("RIPGREP_CONFIG_PATH")
        .current_dir(cwd)
        .args(scopes)
        .output()
        .map_err(|error| AdapterError::Unavailable(format!("cannot launch {rg}: {error}")))?;
    let empty =
        output.status.code() == Some(1) && output.stdout.is_empty() && output.stderr.is_empty();
    if !output.status.success() && !empty {
        return Err(AdapterError::Internal(format!(
            "rg failed {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|value| {
            String::from_utf8(value.to_vec())
                .map(|path| normalize_inventory_path(&path))
                .map_err(|error| AdapterError::Contract(format!("rg path is not UTF-8: {error}")))
        })
        .collect()
}

fn mirror_path_sets(
    rg: &str,
    scopes: &[String],
    baseline_globs: &[String],
    glob_patterns: &[String],
    manifest_paths: &BTreeMap<String, Option<Vec<u8>>>,
) -> Result<(BTreeSet<String>, BTreeSet<String>), AdapterError> {
    let mut mirror = InventoryMirror::create(manifest_paths)?;
    let result = run_rg_files(
        rg,
        mirror.root(),
        scopes,
        baseline_globs,
        RgIgnoreMode::Disabled,
    )
    .and_then(|baseline| {
        run_rg_files(
            rg,
            mirror.root(),
            scopes,
            glob_patterns,
            RgIgnoreMode::SnapshotControls,
        )
        .map(|filtered| (baseline, filtered))
    });
    let cleanup = mirror.cleanup();
    match (result, cleanup) {
        (Ok(paths), Ok(())) => Ok(paths),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(AdapterError::Io(format!(
            "inventory manifest mirror cleanup failed: {cleanup}"
        ))),
        (Err(_), Err(cleanup)) => Err(AdapterError::Io(format!(
            "inventory manifest mirror failed and cleanup failed: {cleanup}"
        ))),
    }
}

struct MirrorNode {
    path: PathBuf,
    dir: bool,
    id: StableId,
}

struct InventoryMirror {
    root: PathBuf,
    nodes: Vec<MirrorNode>,
}

impl InventoryMirror {
    fn create(paths: &BTreeMap<String, Option<Vec<u8>>>) -> Result<Self, AdapterError> {
        let mut mirror = Self::create_root()?;
        let mut directories = BTreeSet::new();
        for path in paths.keys() {
            let mut parent = Path::new(path).parent();
            while let Some(value) = parent {
                if value.as_os_str().is_empty() {
                    break;
                }
                directories.insert(value.to_path_buf());
                parent = value.parent();
            }
        }
        let mut directories = directories.into_iter().collect::<Vec<_>>();
        directories.sort_by(|left, right| {
            left.components()
                .count()
                .cmp(&right.components().count())
                .then_with(|| left.cmp(right))
        });
        for relative in directories {
            mirror.create_directory(&relative)?;
        }
        for (relative, bytes) in paths {
            mirror.create_file(Path::new(relative), bytes.as_deref().unwrap_or_default())?;
        }
        Ok(mirror)
    }

    fn create_root() -> Result<Self, AdapterError> {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| AdapterError::Io(format!("read clock for mirror name: {error}")))?
            .as_nanos();
        for _ in 0..64 {
            let nonce = MIRROR_NONCE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "code-intel-inventory-mirror-{}-{epoch}-{nonce}",
                std::process::id()
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    let opened = open_plain(&root, true).map_err(AdapterError::Io)?;
                    let id = opened.id;
                    drop(opened);
                    return Ok(Self {
                        root: root.clone(),
                        nodes: vec![MirrorNode {
                            path: root,
                            dir: true,
                            id,
                        }],
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(AdapterError::Io(format!(
                        "exclusive inventory mirror create failed: {error}"
                    )))
                }
            }
        }
        Err(AdapterError::Io(
            "exclusive inventory mirror name space exhausted".into(),
        ))
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn create_directory(&mut self, relative: &Path) -> Result<(), AdapterError> {
        let path = self.root.join(relative);
        fs::create_dir(&path).map_err(|error| {
            AdapterError::Io(format!("create inventory mirror directory: {error}"))
        })?;
        let opened = open_plain(&path, true).map_err(AdapterError::Io)?;
        self.nodes.push(MirrorNode {
            path,
            dir: true,
            id: opened.id,
        });
        Ok(())
    }

    fn create_file(&mut self, relative: &Path, bytes: &[u8]) -> Result<(), AdapterError> {
        let path = self.root.join(relative);
        let mut file = create_temp(&path)
            .map_err(|error| AdapterError::Io(format!("create inventory mirror file: {error}")))?;
        file.write_all(bytes).map_err(|error| {
            AdapterError::Io(format!("write inventory mirror control file: {error}"))
        })?;
        let id = stable_id(&file).map_err(|error| {
            AdapterError::Io(format!("read inventory mirror file identity: {error}"))
        })?;
        drop(file);
        self.nodes.push(MirrorNode {
            path,
            dir: false,
            id,
        });
        Ok(())
    }

    fn cleanup(&mut self) -> Result<(), String> {
        let mut failures = Vec::new();
        while let Some(node) = self.nodes.pop() {
            match open_if_stable_id(&node.path, node.dir, node.id) {
                Some(opened) => {
                    if let Err(error) = remove_owned(&node.path, &opened, node.dir) {
                        failures.push(error);
                    }
                    drop(opened);
                }
                None => failures.push(format!(
                    "mirror identity changed; preserved {}",
                    node.path.display()
                )),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
}

impl Drop for InventoryMirror {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn normalize_inventory_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized
        .strip_prefix("./")
        .unwrap_or(&normalized)
        .to_string()
}

fn snapshot_adapter_error(message: String) -> AdapterError {
    if message.contains("cannot launch Git") || message.contains("cannot launch rg") {
        AdapterError::Unavailable(message)
    } else {
        AdapterError::Contract(message)
    }
}

fn join_records(records: &[Vec<u8>], sep: u8) -> Vec<u8> {
    let mut out = Vec::new();
    for r in records {
        out.extend_from_slice(r);
        out.push(sep)
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StableId {
    volume: u64,
    file: u64,
}

struct Opened {
    handle: File,
    id: StableId,
}

fn open_plain(path: &Path, dir: bool) -> Result<Opened, String> {
    let before = fs::symlink_metadata(path)
        .map_err(|e| format!("inspect {} before open: {e}", path.display()))?;
    if (dir && !before.is_dir()) || (!dir && !before.is_file()) || reparse(&before) {
        return Err(format!(
            "not a plain no-follow filesystem object: {}",
            path.display()
        ));
    }
    let handle = open_handle(path, dir, true)
        .map_err(|e| format!("open stable handle {}: {e}", path.display()))?;
    let metadata = handle
        .metadata()
        .map_err(|e| format!("inspect opened handle {}: {e}", path.display()))?;
    if (dir && !metadata.is_dir()) || (!dir && !metadata.is_file()) || reparse(&metadata) {
        return Err(format!(
            "opened object is not a plain no-follow filesystem object: {}",
            path.display()
        ));
    }
    let id =
        stable_id(&handle).map_err(|e| format!("read stable identity {}: {e}", path.display()))?;
    let after = fs::symlink_metadata(path)
        .map_err(|e| format!("inspect {} after open: {e}", path.display()))?;
    if reparse(&after) || !path_metadata_matches(&after, id) {
        return Err(format!(
            "path identity changed while opening: {}",
            path.display()
        ));
    }
    #[cfg(windows)]
    {
        let confirmation = open_handle(path, dir, false)
            .and_then(|file| stable_id(&file))
            .map_err(|e| format!("confirm stable identity {}: {e}", path.display()))?;
        if confirmation != id {
            return Err(format!(
                "path identity changed while opening: {}",
                path.display()
            ));
        }
    }
    Ok(Opened { handle, id })
}

fn open_if_stable_id(path: &Path, dir: bool, expected: StableId) -> Option<Opened> {
    let handle = open_handle(path, dir, true).ok()?;
    let metadata = handle.metadata().ok()?;
    if (dir && !metadata.is_dir()) || (!dir && !metadata.is_file()) || reparse(&metadata) {
        return None;
    }
    let id = stable_id(&handle).ok()?;
    (id == expected).then_some(Opened { handle, id })
}

#[cfg(unix)]
fn open_handle(path: &Path, _dir: bool, _delete_access: bool) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(windows)]
fn open_handle(path: &Path, dir: bool, delete_access: bool) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const DELETE: u32 = 0x0001_0000;
    const SHARE_READ: u32 = 1;
    const SHARE_WRITE: u32 = 2;
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let mut options = OpenOptions::new();
    options
        .access_mode(FILE_READ_ATTRIBUTES | if delete_access { DELETE } else { 0 })
        .share_mode(SHARE_READ | SHARE_WRITE | 4)
        .custom_flags(OPEN_REPARSE_POINT | if dir { BACKUP_SEMANTICS } else { 0 });
    options.open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_handle(path: &Path, _dir: bool, _delete_access: bool) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(unix)]
fn stable_id(file: &File) -> std::io::Result<StableId> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok(StableId {
        volume: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(unix)]
fn path_metadata_matches(metadata: &fs::Metadata, expected: StableId) -> bool {
    use std::os::unix::fs::MetadataExt;
    StableId {
        volume: metadata.dev(),
        file: metadata.ino(),
    } == expected
}

#[cfg(windows)]
fn stable_id(file: &File) -> std::io::Result<StableId> {
    use std::ffi::c_void;
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct FileTime {
        dwLowDateTime: u32,
        dwHighDateTime: u32,
    }
    #[repr(C)]
    #[allow(non_snake_case)]
    struct ByHandleFileInformation {
        dwFileAttributes: u32,
        ftCreationTime: FileTime,
        ftLastAccessTime: FileTime,
        ftLastWriteTime: FileTime,
        dwVolumeSerialNumber: u32,
        nFileSizeHigh: u32,
        nFileSizeLow: u32,
        nNumberOfLinks: u32,
        nFileIndexHigh: u32,
        nFileIndexLow: u32,
    }
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            handle: *mut c_void,
            information: *mut ByHandleFileInformation,
        ) -> i32;
    }
    let mut information = MaybeUninit::<ByHandleFileInformation>::uninit();
    let ok = unsafe {
        GetFileInformationByHandle(file.as_raw_handle().cast(), information.as_mut_ptr())
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let information = unsafe { information.assume_init() };
    Ok(StableId {
        volume: information.dwVolumeSerialNumber as u64,
        file: ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
    })
}

#[cfg(windows)]
fn path_metadata_matches(_metadata: &fs::Metadata, _expected: StableId) -> bool {
    true
}

#[cfg(not(any(unix, windows)))]
fn stable_id(file: &File) -> std::io::Result<StableId> {
    Ok(StableId {
        volume: 0,
        file: file.metadata()?.len(),
    })
}

#[cfg(not(any(unix, windows)))]
fn path_metadata_matches(metadata: &fs::Metadata, expected: StableId) -> bool {
    StableId {
        volume: 0,
        file: metadata.len(),
    } == expected
}
fn reparse(m: &fs::Metadata) -> bool {
    if m.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}
fn remove_owned(path: &Path, opened: &Opened, dir: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = dir;
        use std::ffi::c_void;
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        struct FileDispositionInformation {
            delete_file: u8,
        }
        unsafe extern "system" {
            fn SetFileInformationByHandle(
                handle: *mut c_void,
                class: i32,
                information: *const FileDispositionInformation,
                size: u32,
            ) -> i32;
        }
        const FILE_DISPOSITION_INFO: i32 = 4;
        let information = FileDispositionInformation { delete_file: 1 };
        let ok = unsafe {
            SetFileInformationByHandle(
                opened.handle.as_raw_handle().cast(),
                FILE_DISPOSITION_INFO,
                &information,
                std::mem::size_of::<FileDispositionInformation>() as u32,
            )
        };
        if ok == 0 {
            return Err(format!(
                "remove owned handle {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let current = open_plain(path, dir)?;
        if current.id != opened.id {
            return Err(format!("identity changed; preserved {}", path.display()));
        }
        if dir {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        }
        .map_err(|e| format!("remove owned object {}: {e}", path.display()))
    }
}

fn cleanup(
    out: &Path,
    dir: Opened,
    temp: Option<(&Path, Opened)>,
    final_file: Option<(&Path, Opened)>,
) -> String {
    let mut notes = Vec::new();
    if let Some((path, opened)) = final_file {
        if let Err(e) = remove_owned(path, &opened, false) {
            notes.push(e);
        }
        drop(opened);
    }
    if let Some((path, opened)) = temp {
        if let Err(e) = remove_owned(path, &opened, false) {
            notes.push(e);
        }
        drop(opened);
    }
    if let Err(e) = remove_owned(out, &dir, true) {
        notes.push(e);
    }
    drop(dir);
    if notes.is_empty() {
        "; cleanup: completed".into()
    } else {
        format!("; cleanup: {}", notes.join("; "))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublishPoint {
    BeforeLink,
    AfterLinkValidated,
}

fn create_temp(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const GENERIC_WRITE: u32 = 0x4000_0000;
        const FILE_READ_ATTRIBUTES: u32 = 0x80;
        const DELETE: u32 = 0x0001_0000;
        options
            .access_mode(GENERIC_WRITE | FILE_READ_ATTRIBUTES | DELETE)
            .share_mode(1 | 2 | 4);
    }
    options.open(path)
}

fn publish<F>(out: &Path, bytes: &[u8], hook: F) -> Result<(), AdapterError>
where
    F: FnMut(PublishPoint) -> Result<(), String>,
{
    publish_named(out, "files.txt", bytes, hook)
}

fn publish_named<F>(
    out: &Path,
    file_name: &str,
    bytes: &[u8],
    mut hook: F,
) -> Result<(), AdapterError>
where
    F: FnMut(PublishPoint) -> Result<(), String>,
{
    fs::create_dir(out)
        .map_err(|e| AdapterError::Io(format!("exclusive output create {}: {e}", out.display())))?;
    let dir = open_plain(out, true).map_err(|e| {
        AdapterError::Io(format!(
            "output identity unavailable: {e}; cleanup: preserved object because ownership could not be verified"
        ))
    })?;
    let temp = out.join(format!("{file_name}.partial"));
    let final_path = out.join(file_name);
    let mut file = match create_temp(&temp) {
        Ok(file) => file,
        Err(e) => {
            let detail = cleanup(out, dir, None, None);
            return Err(AdapterError::Io(format!(
                "exclusive temp create: {e}{detail}"
            )));
        }
    };
    if let Err(e) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let temp_opened = stable_id(&file).ok().map(|id| Opened { handle: file, id });
        let detail = cleanup(
            out,
            dir,
            temp_opened.map(|opened| (temp.as_path(), opened)),
            None,
        );
        return Err(AdapterError::Io(format!("temp write: {e}{detail}")));
    }
    let temp_id = match stable_id(&file) {
        Ok(identity) => identity,
        Err(e) => {
            drop(file);
            let detail = cleanup(out, dir, None, None);
            return Err(AdapterError::Io(format!(
                "temp identity unavailable: {e}{detail}"
            )));
        }
    };
    let temp_opened = Opened {
        handle: file,
        id: temp_id,
    };
    if let Err(e) = hook(PublishPoint::BeforeLink) {
        let detail = cleanup(out, dir, Some((temp.as_path(), temp_opened)), None);
        return Err(AdapterError::Io(format!("pre-link fault: {e}{detail}")));
    }
    if !open_plain(out, true).is_ok_and(|actual| actual.id == dir.id)
        || !open_plain(&temp, false).is_ok_and(|actual| actual.id == temp_opened.id)
    {
        let detail = cleanup(out, dir, Some((temp.as_path(), temp_opened)), None);
        return Err(AdapterError::Io(format!(
            "directory identity changed{detail}"
        )));
    }
    if let Err(e) = fs::hard_link(&temp, &final_path) {
        let detail = cleanup(out, dir, Some((temp.as_path(), temp_opened)), None);
        return Err(AdapterError::Io(format!("exclusive publish: {e}{detail}")));
    }
    let final_opened = match open_plain(&final_path, false) {
        Ok(opened) => opened,
        Err(e) => {
            let owned_final = open_if_stable_id(&final_path, false, temp_opened.id)
                .map(|opened| (final_path.as_path(), opened));
            let final_note = if owned_final.is_some() {
                "stable final recovered for owned rollback"
            } else {
                "final path is absent or no longer has the owned stable identity; preserved"
            };
            let detail = cleanup(out, dir, Some((temp.as_path(), temp_opened)), owned_final);
            return Err(AdapterError::Io(format!(
                "published identity unavailable: {e}; {final_note}{detail}"
            )));
        }
    };
    if final_opened.id != temp_opened.id
        || !open_plain(out, true).is_ok_and(|actual| actual.id == dir.id)
    {
        let detail = cleanup(
            out,
            dir,
            Some((temp.as_path(), temp_opened)),
            Some((final_path.as_path(), final_opened)),
        );
        return Err(AdapterError::Io(format!(
            "published identity mismatch{detail}"
        )));
    }
    if let Err(e) = hook(PublishPoint::AfterLinkValidated) {
        let detail = cleanup(
            out,
            dir,
            Some((temp.as_path(), temp_opened)),
            Some((final_path.as_path(), final_opened)),
        );
        return Err(AdapterError::Io(format!("post-link fault: {e}{detail}")));
    }
    if let Err(e) = remove_owned(&temp, &temp_opened, false) {
        let detail = cleanup(
            out,
            dir,
            Some((temp.as_path(), temp_opened)),
            Some((final_path.as_path(), final_opened)),
        );
        return Err(AdapterError::Io(format!(
            "published temp cleanup failed: {e}{detail}"
        )));
    }
    drop(temp_opened);
    drop(final_opened);
    drop(dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn nul_serialization_preserves_embedded_newlines() {
        assert_eq!(join_records(&[b"a\nb".to_vec()], 0), b"a\nb\0");
    }

    #[test]
    fn gitlink_exclusion_escapes_every_ripgrep_glob_metacharacter() {
        assert_eq!(
            gitlink_exclude_glob(r"vendor/sub[abc]*?{x}!\tail"),
            r"!vendor/sub\[abc\]\*\?\{x\}\!\\tail/**"
        );
    }

    #[test]
    fn mirror_cleanup_removes_owned_tree_after_rg_failure() {
        let paths = BTreeMap::from([
            ("README.md".to_string(), None),
            ("nested/子/file.rs".to_string(), None),
        ]);
        let mut mirror = match InventoryMirror::create(&paths) {
            Ok(mirror) => mirror,
            Err(_) => panic!("create inventory mirror"),
        };
        let root = mirror.root().to_path_buf();
        let failed = run_rg_files(
            "__code_intel_missing_rg_binary__",
            mirror.root(),
            &[".".to_string()],
            &[],
            RgIgnoreMode::Disabled,
        );
        assert!(matches!(failed, Err(AdapterError::Unavailable(_))));
        mirror.cleanup().unwrap();
        assert!(!root.exists(), "failed mirror rg must leave no temp tree");
    }

    #[test]
    fn publish_collision_preserves_competitor_and_removes_owned_temp() {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = std::env::temp_dir().join(format!("code-intel-publish-{n}"));
        let final_path = out.join("files.txt");
        let result = publish(&out, b"x", |point| {
            if point == PublishPoint::BeforeLink {
                fs::create_dir(&final_path).unwrap();
            }
            Ok(())
        });
        let msg = match result {
            Err(AdapterError::Io(v)) => v,
            _ => panic!(),
        };
        assert!(msg.contains("cleanup:"));
        assert!(!out.join("files.txt.partial").exists());
        assert!(final_path.is_dir());
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn post_link_failure_rolls_back_final_temp_and_directory() {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = std::env::temp_dir().join(format!("code-intel-post-link-{n}"));
        let result = publish(&out, b"x", |point| {
            if point == PublishPoint::AfterLinkValidated {
                Err("injected".into())
            } else {
                Ok(())
            }
        });
        let message = match result {
            Err(AdapterError::Io(message)) => message,
            _ => panic!(),
        };
        assert!(message.contains("cleanup: completed"), "{message}");
        assert!(
            !out.exists(),
            "failed publication must leave no artifact tree"
        );
    }

    #[cfg(windows)]
    #[test]
    fn stable_file_id_rejects_same_size_and_mtime_path_replacement() {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = std::env::temp_dir().join(format!("code-intel-replace-{n}"));
        let temp = out.join("files.txt.partial");
        let result = publish(&out, b"owned", |point| {
            if point == PublishPoint::BeforeLink {
                let modified = fs::metadata(&temp).unwrap().modified().unwrap();
                fs::remove_file(&temp).unwrap();
                fs::write(&temp, b"forge").unwrap();
                let replacement = OpenOptions::new().write(true).open(&temp).unwrap();
                replacement
                    .set_times(std::fs::FileTimes::new().set_modified(modified))
                    .unwrap();
            }
            Ok(())
        });
        assert!(matches!(result, Err(AdapterError::Io(_))));
        assert!(!out.join("files.txt").exists());
        assert_eq!(fs::read(&temp).unwrap(), b"forge");
        fs::remove_dir_all(out).unwrap();
    }
}
