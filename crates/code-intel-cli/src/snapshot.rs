use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::capability::sha256_hex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Policy {
    HeadOnly,
    ExplicitOverlay,
}

impl Policy {
    fn parse(value: &str) -> Result<Self, SnapshotError> {
        match value {
            "head_only" => Ok(Self::HeadOnly),
            "explicit_overlay" => Ok(Self::ExplicitOverlay),
            _ => Err(SnapshotError::Usage(
                "working-tree policy must be head_only or explicit_overlay".into(),
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::HeadOnly => "head_only",
            Self::ExplicitOverlay => "explicit_overlay",
        }
    }
}

#[derive(Debug)]
enum SnapshotError {
    Usage(String),
    Contract(String),
    Unavailable(String),
    Io(String),
}

impl SnapshotError {
    fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 64,
            Self::Contract(_) => 65,
            Self::Unavailable(_) => 69,
            Self::Io(_) => 74,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Usage(message)
            | Self::Contract(message)
            | Self::Unavailable(message)
            | Self::Io(message) => message,
        }
    }
}

struct Cli {
    repo: PathBuf,
    policy: Policy,
    scopes: Vec<String>,
    alternate_vcs_command: Option<String>,
    alternate_vcs_args: Vec<String>,
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match parse_cli(raw) {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("{}", error.message());
            return error.exit_code();
        }
    };
    match build(&cli.repo, cli.policy, &cli.scopes)
        .and_then(|value| verify_alternate_vcs(&cli, value))
    {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string(&value).expect("snapshot serializes")
            );
            0
        }
        Err(error) => {
            eprintln!("{}", error.message());
            error.exit_code()
        }
    }
}

pub(crate) fn build_for_capability(repo: &Path, expected: &Value) -> Result<Value, String> {
    let policy = expected
        .get("workingTreePolicy")
        .and_then(Value::as_str)
        .ok_or_else(|| "expected snapshot omits workingTreePolicy".to_string())
        .and_then(|value| Policy::parse(value).map_err(|error| error.message().to_string()))?;
    let scopes = expected
        .get("scope")
        .and_then(Value::as_array)
        .ok_or("expected snapshot omits scope")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "expected snapshot scope contains a non-string".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let scopes = canonical_scopes(&scopes).map_err(|error| error.message().to_string())?;
    let actual = build(repo, policy, &scopes).map_err(|error| error.message().to_string())?;
    if actual["snapshot"] != *expected {
        return Err("repository inputs do not match the expected snapshot identity".into());
    }
    Ok(actual)
}

pub(crate) fn build_for_dag(
    repo: &Path,
    working_tree_policy: &str,
    scopes: &[String],
) -> Result<Value, String> {
    let policy = Policy::parse(working_tree_policy).map_err(|error| error.message().to_string())?;
    build(repo, policy, scopes).map_err(|error| error.message().to_string())
}

fn parse_cli(raw: &[String]) -> Result<Cli, SnapshotError> {
    if raw.first().map(String::as_str) != Some("identity") {
        return Err(SnapshotError::Usage(
            "usage: snapshot identity --repo <path> --working-tree-policy <head_only|explicit_overlay> [--scope <relative-path>]...".into(),
        ));
    }
    let mut repo = None;
    let mut policy = None;
    let mut scopes = Vec::new();
    let mut alternate_vcs_command = None;
    let mut alternate_vcs_args = Vec::new();
    let mut index = 1;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--repo"
                | "--working-tree-policy"
                | "--scope"
                | "--alternate-vcs-command"
                | "--alternate-vcs-arg"
        ) {
            return Err(SnapshotError::Usage(format!(
                "unknown snapshot argument: {flag}"
            )));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| SnapshotError::Usage(format!("{flag} requires one value")))?;
        match flag {
            "--repo" if repo.replace(PathBuf::from(value)).is_some() => {
                return Err(SnapshotError::Usage("duplicate --repo".into()))
            }
            "--repo" => {}
            "--working-tree-policy" if policy.replace(Policy::parse(value)?).is_some() => {
                return Err(SnapshotError::Usage(
                    "duplicate --working-tree-policy".into(),
                ))
            }
            "--working-tree-policy" => {}
            "--scope" => scopes.push(value.clone()),
            "--alternate-vcs-command" if alternate_vcs_command.replace(value.clone()).is_some() => {
                return Err(SnapshotError::Usage(
                    "duplicate --alternate-vcs-command".into(),
                ))
            }
            "--alternate-vcs-command" => {}
            "--alternate-vcs-arg" => alternate_vcs_args.push(value.clone()),
            _ => unreachable!(),
        }
        index += 2;
    }
    let repo = repo.ok_or_else(|| SnapshotError::Usage("--repo is required".into()))?;
    if !repo.is_dir() {
        return Err(SnapshotError::Usage(format!(
            "repository path is not a directory: {}",
            repo.display()
        )));
    }
    let policy =
        policy.ok_or_else(|| SnapshotError::Usage("--working-tree-policy is required".into()))?;
    let scopes = canonical_scopes(&scopes)?;
    if alternate_vcs_command.is_none() && !alternate_vcs_args.is_empty() {
        return Err(SnapshotError::Usage(
            "--alternate-vcs-arg requires --alternate-vcs-command".into(),
        ));
    }
    Ok(Cli {
        repo,
        policy,
        scopes,
        alternate_vcs_command,
        alternate_vcs_args,
    })
}

fn verify_alternate_vcs(cli: &Cli, authoritative: Value) -> Result<Value, SnapshotError> {
    let Some(command) = cli.alternate_vcs_command.as_ref() else {
        return Ok(authoritative);
    };
    let request = json!({
        "schema": "code-intel-alternate-vcs-snapshot-request.v1",
        "repo": cli.repo,
        "workingTreePolicy": cli.policy.as_str(),
        "scope": cli.scopes,
        "authoritativeSnapshot": authoritative["snapshot"]
    });
    let mut child = Command::new(command)
        .args(&cli.alternate_vcs_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            SnapshotError::Contract(format!("alternate VCS adapter could not start: {error}"))
        })?;
    child
        .stdin
        .take()
        .ok_or_else(|| {
            SnapshotError::Contract("alternate VCS adapter stdin is unavailable".into())
        })?
        .write_all(
            serde_json::to_string(&request)
                .expect("alternate VCS request serializes")
                .as_bytes(),
        )
        .map_err(|error| {
            SnapshotError::Contract(format!("alternate VCS adapter request failed: {error}"))
        })?;
    let output = child.wait_with_output().map_err(|error| {
        SnapshotError::Contract(format!("alternate VCS adapter did not complete: {error}"))
    })?;
    if !output.status.success() {
        return Err(SnapshotError::Contract(format!(
            "alternate VCS adapter exited with {}: {}",
            output.status.code().unwrap_or(65),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let alternate: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        SnapshotError::Contract(format!(
            "alternate VCS adapter returned invalid JSON: {error}"
        ))
    })?;
    if alternate.get("snapshot") != authoritative.get("snapshot") {
        return Err(SnapshotError::Contract(
            "alternate VCS adapter snapshot does not match the authoritative snapshot".into(),
        ));
    }
    Ok(authoritative)
}

fn canonical_scopes(values: &[String]) -> Result<Vec<String>, SnapshotError> {
    let values = if values.is_empty() {
        vec![".".to_string()]
    } else {
        values.to_vec()
    };
    let mut result = BTreeSet::new();
    for value in values {
        let path = Path::new(&value);
        if value.contains('\0')
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(SnapshotError::Usage(format!(
                "scope must be a repository-relative path without '..': {value}"
            )));
        }
        let normalized = path
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
                Component::CurDir => None,
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        result.insert(if normalized.is_empty() {
            ".".to_string()
        } else {
            normalized
        });
    }
    let sorted = result.into_iter().collect::<Vec<_>>();
    #[cfg(windows)]
    for (index, left) in sorted.iter().enumerate() {
        for right in &sorted[index + 1..] {
            let left_folded = left.to_ascii_lowercase();
            let right_folded = right.to_ascii_lowercase();
            let folded_overlap = left_folded == right_folded
                || right_folded
                    .strip_prefix(&left_folded)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                || left_folded
                    .strip_prefix(&right_folded)
                    .is_some_and(|suffix| suffix.starts_with('/'));
            let exact_overlap = left == right
                || right
                    .strip_prefix(left)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                || left
                    .strip_prefix(right)
                    .is_some_and(|suffix| suffix.starts_with('/'));
            if folded_overlap && !exact_overlap {
                return Err(SnapshotError::Usage(format!(
                    "scope case collision is ambiguous on Windows: {left} vs {right}"
                )));
            }
        }
    }
    let mut minimal = Vec::<String>::new();
    for scope in sorted {
        if minimal.iter().any(|parent| {
            parent == "."
                || scope == *parent
                || scope
                    .strip_prefix(parent)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        }) {
            continue;
        }
        minimal.push(scope);
    }
    Ok(minimal)
}

pub(crate) struct SnapshotLease {
    expected: Value,
    policy: Policy,
    scopes: Vec<String>,
    manifest: InputManifest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManifestEntry {
    path: String,
    kind: String,
    mode: String,
    digest: String,
    control_bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InputManifest {
    policy: Policy,
    scopes: Vec<String>,
    entries: Vec<ManifestEntry>,
}

fn is_ignore_control_path(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".gitignore" | ".ignore" | ".rgignore"))
}

fn path_in_scopes(path: &str, scopes: &[String]) -> bool {
    scopes.iter().any(|scope| {
        scope == "."
            || path == scope
            || path
                .strip_prefix(scope)
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

fn ignore_control_relevant(path: &str, scopes: &[String]) -> bool {
    if !is_ignore_control_path(path) {
        return false;
    }
    let parent = path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or(".");
    scopes.iter().any(|scope| {
        scope == "."
            || parent == scope
            || parent
                .strip_prefix(scope)
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

fn filesystem_ignore_controls(
    repo: &Path,
    scopes: &[String],
) -> Result<Vec<String>, SnapshotError> {
    let rg = if cfg!(windows) { "rg.exe" } else { "rg" };
    let output = Command::new(rg)
        .args([
            "--files",
            "--hidden",
            "--null",
            "--no-ignore-parent",
            "--no-ignore-global",
            "--no-ignore-exclude",
            "-g",
            "**/.gitignore",
            "-g",
            "**/.ignore",
            "-g",
            "**/.rgignore",
        ])
        .env_remove("RIPGREP_CONFIG_PATH")
        .current_dir(repo)
        .output()
        .map_err(|error| SnapshotError::Unavailable(format!("cannot launch {rg}: {error}")))?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(SnapshotError::Unavailable(format!(
            "ignore-control inventory failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let mut controls = decode_paths(&output.stdout)?
        .into_iter()
        .filter(|path| ignore_control_relevant(path, scopes))
        .collect::<Vec<_>>();
    controls.sort();
    controls.dedup();
    Ok(controls)
}

pub(crate) fn begin_consumption(repo: &Path, expected: &Value) -> Result<SnapshotLease, String> {
    let policy = expected
        .get("workingTreePolicy")
        .and_then(Value::as_str)
        .ok_or_else(|| "expected snapshot omits workingTreePolicy".to_string())
        .and_then(|value| Policy::parse(value).map_err(|error| error.message().to_string()))?;
    let scopes = expected
        .get("scope")
        .and_then(Value::as_array)
        .ok_or("expected snapshot omits scope")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "expected snapshot scope contains a non-string".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let scopes = canonical_scopes(&scopes).map_err(|error| error.message().to_string())?;
    let actual = build(repo, policy, &scopes).map_err(|error| error.message().to_string())?;
    if actual["snapshot"] != *expected {
        return Err("repository inputs do not match the expected snapshot identity".into());
    }
    let manifest =
        input_manifest(repo, policy, &scopes).map_err(|error| error.message().to_string())?;
    Ok(SnapshotLease {
        expected: expected.clone(),
        policy,
        scopes,
        manifest,
    })
}

impl SnapshotLease {
    pub(crate) fn scopes(&self) -> &[String] {
        &self.scopes
    }

    pub(crate) fn inventory_mirror_files(&self) -> BTreeMap<String, Option<Vec<u8>>> {
        let mut paths = BTreeMap::new();
        for entry in &self.manifest.entries {
            if matches!(entry.kind.as_str(), "tombstone" | "gitlink" | "symlink") {
                continue;
            }
            paths.insert(entry.path.clone(), entry.control_bytes.clone());
        }
        paths
    }

    pub(crate) fn inventory_gitlink_paths(&self) -> Vec<String> {
        self.manifest
            .entries
            .iter()
            .filter(|entry| entry.kind == "gitlink")
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub(crate) fn verify_after(&self, repo: &Path) -> Result<(), String> {
        let actual =
            build(repo, self.policy, &self.scopes).map_err(|error| error.message().to_string())?;
        if actual["snapshot"] != self.expected {
            return Err("repository inputs changed while the capability consumed them".into());
        }
        let manifest = input_manifest(repo, self.policy, &self.scopes)
            .map_err(|error| error.message().to_string())?;
        if manifest != self.manifest {
            return Err(
                "repository input manifest changed while the capability consumed it".into(),
            );
        }
        Ok(())
    }
}

fn build(repo: &Path, policy: Policy, scopes: &[String]) -> Result<Value, SnapshotError> {
    let git = git_context(repo)?;
    if git.is_none() && policy == Policy::HeadOnly {
        return Err(SnapshotError::Unavailable(
            "head_only snapshot identity requires Git with a resolvable HEAD".into(),
        ));
    }
    if git.is_none()
        && scopes
            .iter()
            .filter(|scope| scope.as_str() != ".")
            .any(|scope| !repo.join(scope).exists())
    {
        return Err(SnapshotError::Usage(
            "non-root scope has no manifest entries and does not exist".into(),
        ));
    }

    let (repo_identity, head, input_digest, overlay, repository_kind) = match git {
        Some(git) => {
            let (input_digest, overlay) = match policy {
                Policy::HeadOnly => (digest_head(repo, &git.head, scopes)?, Overlay::default()),
                Policy::ExplicitOverlay => stable_overlay_snapshot(repo, scopes)?,
            };
            (git.repo_identity, git.head, input_digest, overlay, "git")
        }
        None => {
            let (input_digest, paths) = stable_unversioned_snapshot(repo, scopes)?;
            let overlay = Overlay::unversioned(paths);
            let unborn =
                git_output(repo, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|output| {
                    output.status.success() && trim_ascii(&output.stdout) == b"true"
                });
            (
                format!("content-v1:{input_digest}"),
                if unborn { "unborn" } else { "unversioned" }.to_string(),
                input_digest,
                overlay,
                if unborn { "git_unborn" } else { "unversioned" },
            )
        }
    };

    let dirty_paths = overlay.paths();
    let overlay_digest = if dirty_paths.is_empty() {
        None
    } else {
        Some(hash_records(
            dirty_paths
                .iter()
                .map(|path| path.as_bytes().to_vec())
                .collect::<Vec<_>>()
                .as_slice(),
        ))
    };
    let identity = hash_records(&[
        b"code-intel-snapshot.v1".to_vec(),
        repo_identity.as_bytes().to_vec(),
        head.as_bytes().to_vec(),
        policy.as_str().as_bytes().to_vec(),
        scopes.join("\0").into_bytes(),
        input_digest.as_bytes().to_vec(),
    ]);
    let document = json!({
        "schema": "code-intel-repository-snapshot.v1",
        "snapshot": {
            "identity": identity,
            "repoIdentity": repo_identity,
            "head": head,
            "workingTreePolicy": policy.as_str(),
            "scope": scopes,
            "inputDigest": input_digest
        },
        "dirtyOverlay": {
            "present": !dirty_paths.is_empty(),
            "digest": overlay_digest,
            "paths": dirty_paths,
            "members": overlay.members_json(),
            "ignoredPolicy": "excluded_by_git_ignore"
        },
        "repository": { "kind": repository_kind }
    });
    let _ = input_manifest(repo, policy, scopes)?;
    Ok(document)
}

fn input_manifest(
    repo: &Path,
    policy: Policy,
    scopes: &[String],
) -> Result<InputManifest, SnapshotError> {
    let git = git_context(repo)?;
    let mut entries = match (git, policy) {
        (Some(git), Policy::HeadOnly) => head_manifest(repo, &git.head, scopes)?,
        (Some(_), Policy::ExplicitOverlay) => worktree_manifest(repo, scopes)?,
        (None, Policy::ExplicitOverlay) => {
            if scopes
                .iter()
                .filter(|scope| scope.as_str() != ".")
                .any(|scope| !repo.join(scope).exists())
            {
                return Err(SnapshotError::Usage(
                    "non-root scope has no manifest entries and does not exist".into(),
                ));
            }
            unversioned_manifest(repo, scopes)?
        }
        (None, Policy::HeadOnly) => {
            return Err(SnapshotError::Unavailable(
                "head_only snapshot identity requires Git with a resolvable HEAD".into(),
            ))
        }
    };
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    if entries.is_empty()
        && scopes
            .iter()
            .filter(|scope| scope.as_str() != ".")
            .any(|scope| !repo.join(scope).exists())
    {
        return Err(SnapshotError::Usage(
            "non-root scope has no manifest entries and does not exist".into(),
        ));
    }
    Ok(InputManifest {
        policy,
        scopes: scopes.to_vec(),
        entries,
    })
}

fn head_manifest(
    repo: &Path,
    head: &str,
    scopes: &[String],
) -> Result<Vec<ManifestEntry>, SnapshotError> {
    let args = ["ls-tree", "-r", "-z", "--full-tree", head, "--"];
    let bytes = git_required(repo, &args, "build HEAD input manifest")?;
    let mut entries = Vec::new();
    for raw in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let tab = raw.iter().position(|byte| *byte == b'\t').ok_or_else(|| {
            SnapshotError::Unavailable("Git tree entry omitted path separator".into())
        })?;
        let header = String::from_utf8(raw[..tab].to_vec()).map_err(|error| {
            SnapshotError::Unavailable(format!("Git tree header is not UTF-8: {error}"))
        })?;
        let fields = header.split(' ').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(SnapshotError::Unavailable(
                "malformed Git tree entry".into(),
            ));
        }
        let path = String::from_utf8(raw[tab + 1..].to_vec())
            .map_err(|error| SnapshotError::Unavailable(format!("Git path is not UTF-8: {error}")))?
            .replace('\\', "/");
        if !path_in_scopes(&path, scopes) && !ignore_control_relevant(&path, scopes) {
            continue;
        }
        let content = if fields[1] == "blob" {
            git_required(repo, &["cat-file", "blob", fields[2]], "read Git tree blob")?
        } else {
            fields[2].as_bytes().to_vec()
        };
        entries.push(ManifestEntry {
            path,
            kind: if fields[1] == "commit" {
                "gitlink".into()
            } else if fields[0] == "120000" {
                "symlink".into()
            } else {
                "file".into()
            },
            mode: fields[0].into(),
            digest: sha256_hex(&content),
            control_bytes: is_ignore_control_path(
                std::str::from_utf8(&raw[tab + 1..]).unwrap_or_default(),
            )
            .then_some(content),
        });
    }
    Ok(entries)
}

fn worktree_manifest(repo: &Path, scopes: &[String]) -> Result<Vec<ManifestEntry>, SnapshotError> {
    let mut index = index_entries(repo, scopes)?;
    for (path, entry) in index_entries(repo, &[".".to_string()])? {
        if ignore_control_relevant(&path, scopes) {
            index.insert(path, entry);
        }
    }
    let mut paths = index.keys().cloned().collect::<BTreeSet<_>>();
    paths.extend(untracked_paths(repo, scopes)?);
    paths.extend(filesystem_ignore_controls(repo, scopes)?);
    paths
        .into_iter()
        .map(|path| manifest_entry_from_worktree(repo, &path, index.get(&path)))
        .collect()
}

fn unversioned_manifest(
    repo: &Path,
    scopes: &[String],
) -> Result<Vec<ManifestEntry>, SnapshotError> {
    inventory_unversioned(repo, scopes)?
        .into_iter()
        .map(|path| manifest_entry_from_worktree(repo, &path, None))
        .collect()
}

fn manifest_entry_from_worktree(
    repo: &Path,
    path: &str,
    indexed: Option<&IndexEntry>,
) -> Result<ManifestEntry, SnapshotError> {
    let mut mode = indexed
        .map(|entry| entry.mode.clone())
        .unwrap_or_else(|| "100644".into());
    if mode == "160000" {
        let content = indexed.expect("gitlink is indexed").oid.as_bytes();
        return Ok(ManifestEntry {
            path: path.into(),
            kind: "gitlink".into(),
            mode,
            digest: sha256_hex(content),
            control_bytes: None,
        });
    }
    let full_path = repo.join(path);
    match fs::symlink_metadata(&full_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            mode = "120000".into();
            let target = fs::read_link(&full_path)
                .map_err(|error| SnapshotError::Io(format!("read symlink {path}: {error}")))?;
            let bytes = target
                .to_str()
                .ok_or_else(|| SnapshotError::Io(format!("non-UTF-8 symlink: {path}")))?
                .replace('\\', "/")
                .into_bytes();
            Ok(ManifestEntry {
                path: path.into(),
                kind: "symlink".into(),
                mode,
                digest: sha256_hex(&bytes),
                control_bytes: is_ignore_control_path(path).then_some(bytes),
            })
        }
        Ok(metadata) if metadata.is_file() => {
            mode = effective_file_mode(&mode, &metadata);
            let bytes = fs::read(&full_path)
                .map_err(|error| SnapshotError::Io(format!("read {path}: {error}")))?;
            Ok(ManifestEntry {
                path: path.into(),
                kind: "file".into(),
                mode,
                digest: sha256_hex(&bytes),
                control_bytes: is_ignore_control_path(path).then_some(bytes),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ManifestEntry {
            path: path.into(),
            kind: "tombstone".into(),
            mode,
            digest: sha256_hex(b""),
            control_bytes: None,
        }),
        Ok(_) => Err(SnapshotError::Io(format!(
            "manifest input has unsupported filesystem kind: {path}"
        ))),
        Err(error) => Err(SnapshotError::Io(format!("inspect {path}: {error}"))),
    }
}

struct GitContext {
    repo_identity: String,
    head: String,
}

fn git_context(repo: &Path) -> Result<Option<GitContext>, SnapshotError> {
    let inside = git_output(repo, &["rev-parse", "--is-inside-work-tree"])?;
    if !inside.status.success() || trim_ascii(&inside.stdout) != b"true" {
        return Ok(None);
    }
    let top = git_required(repo, &["rev-parse", "--show-toplevel"], "resolve Git root")?;
    let top = String::from_utf8(top)
        .map_err(|error| SnapshotError::Unavailable(format!("Git root is not UTF-8: {error}")))?;
    let actual = fs::canonicalize(repo)
        .map_err(|error| SnapshotError::Io(format!("canonicalize repository: {error}")))?;
    let expected = fs::canonicalize(top.trim())
        .map_err(|error| SnapshotError::Io(format!("canonicalize Git root: {error}")))?;
    if actual != expected {
        return Err(SnapshotError::Usage(
            "--repo must name the Git worktree root; express subdirectories with --scope".into(),
        ));
    }
    let shallow = git_required(
        repo,
        &["rev-parse", "--is-shallow-repository"],
        "inspect shallow repository state",
    )?;
    if trim_ascii(&shallow) == b"true" {
        return Err(SnapshotError::Unavailable(
            "shallow Git repositories are unsupported because lineage identity is incomplete"
                .into(),
        ));
    }
    let head_output = git_output(repo, &["rev-parse", "--verify", "HEAD"])?;
    if !head_output.status.success() {
        return Ok(None);
    }
    let head = String::from_utf8(head_output.stdout)
        .map_err(|error| SnapshotError::Unavailable(format!("Git HEAD is not UTF-8: {error}")))?
        .trim()
        .to_string();
    let roots = git_required(
        repo,
        &["rev-list", "--max-parents=0", &head],
        "resolve Git lineage",
    )?;
    let mut roots = String::from_utf8(roots)
        .map_err(|error| SnapshotError::Unavailable(format!("Git lineage is not UTF-8: {error}")))?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        return Err(SnapshotError::Unavailable(
            "Git repository has no resolvable root commit".into(),
        ));
    }
    let repo_identity = format!(
        "git-lineage-v1:{}",
        hash_records(
            &roots
                .iter()
                .map(|root| root.as_bytes().to_vec())
                .collect::<Vec<_>>()
        )
    );
    Ok(Some(GitContext {
        repo_identity,
        head,
    }))
}

fn digest_head(repo: &Path, head: &str, scopes: &[String]) -> Result<String, SnapshotError> {
    let args = ["ls-tree", "-r", "-z", "--full-tree", head, "--"];
    let output = git_required(repo, &args, "enumerate HEAD snapshot")?;
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .filter(|record| {
            let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                return true;
            };
            let Ok(path) = std::str::from_utf8(&record[tab + 1..]) else {
                return true;
            };
            path_in_scopes(path, scopes) || ignore_control_relevant(path, scopes)
        })
        .map(Vec::from)
        .collect::<Vec<_>>();
    records.sort();
    let mut framed = vec![b"code-intel-head-input.v1".to_vec()];
    framed.append(&mut records);
    Ok(hash_records(&framed))
}

fn stable_overlay_snapshot(
    repo: &Path,
    scopes: &[String],
) -> Result<(String, Overlay), SnapshotError> {
    stable_overlay_snapshot_with(repo, scopes, || {}, filesystem_ignore_controls)
}

fn stable_overlay_snapshot_with<F, I>(
    repo: &Path,
    scopes: &[String],
    between_reads: F,
    ignore_controls: I,
) -> Result<(String, Overlay), SnapshotError>
where
    F: FnOnce(),
    I: Fn(&Path, &[String]) -> Result<Vec<String>, SnapshotError>,
{
    let before = overlay_status_with(repo, scopes, &ignore_controls)?;
    let first = digest_worktree(repo, scopes, &ignore_controls)?;
    between_reads();
    let second = digest_worktree(repo, scopes, &ignore_controls)?;
    let after = overlay_status_with(repo, scopes, &ignore_controls)?;
    if first != second || before != after {
        return Err(SnapshotError::Io(
            "working tree changed while snapshot identity was being computed; retry".into(),
        ));
    }
    Ok((first, after))
}

#[derive(Clone)]
struct IndexEntry {
    mode: String,
    oid: String,
}

fn digest_worktree<I>(
    repo: &Path,
    scopes: &[String],
    ignore_controls: &I,
) -> Result<String, SnapshotError>
where
    I: Fn(&Path, &[String]) -> Result<Vec<String>, SnapshotError>,
{
    let mut index = index_entries(repo, scopes)?;
    for (path, entry) in index_entries(repo, &[".".to_string()])? {
        if ignore_control_relevant(&path, scopes) {
            index.insert(path, entry);
        }
    }
    let mut paths = index.keys().cloned().collect::<BTreeSet<_>>();
    paths.extend(untracked_paths(repo, scopes)?);
    paths.extend(ignore_controls(repo, scopes)?);
    let mut records = vec![b"code-intel-overlay-input.v1".to_vec()];
    for relative in paths {
        let indexed = index.get(&relative);
        let mut mode = indexed
            .map(|entry| entry.mode.clone())
            .unwrap_or_else(|| "100644".into());
        let mut kind = match mode.as_str() {
            "160000" => "gitlink",
            "120000" => "symlink",
            _ => "file",
        };
        let path = repo.join(Path::new(&relative));
        let content = if kind == "gitlink" {
            indexed.expect("gitlink is tracked").oid.as_bytes().to_vec()
        } else {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    kind = "symlink";
                    mode = "120000".into();
                    fs::read_link(&path)
                        .map_err(|error| {
                            SnapshotError::Io(format!("read symlink target {relative}: {error}"))
                        })?
                        .to_str()
                        .ok_or_else(|| {
                            SnapshotError::Io(format!(
                                "symlink target is not portable UTF-8: {relative}"
                            ))
                        })?
                        .replace('\\', "/")
                        .into_bytes()
                }
                Ok(metadata) if metadata.is_file() => {
                    mode = effective_file_mode(&mode, &metadata);
                    fs::read(&path).map_err(|error| {
                        SnapshotError::Io(format!("read scoped input {relative}: {error}"))
                    })?
                }
                Ok(_) => {
                    return Err(SnapshotError::Io(format!(
                        "scoped input has unsupported filesystem kind: {relative}"
                    )))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let mut record = b"tombstone".to_vec();
                    record.push(0);
                    record.extend_from_slice(mode.as_bytes());
                    record.push(0);
                    record.extend_from_slice(relative.as_bytes());
                    records.push(record);
                    continue;
                }
                Err(error) => {
                    return Err(SnapshotError::Io(format!(
                        "inspect scoped input {relative}: {error}"
                    )))
                }
            }
        };
        let mut record = kind.as_bytes().to_vec();
        record.push(0);
        record.extend_from_slice(mode.as_bytes());
        record.push(0);
        record.extend_from_slice(relative.as_bytes());
        record.push(0);
        record.extend_from_slice(&(content.len() as u64).to_be_bytes());
        record.extend_from_slice(&content);
        records.push(record);
    }
    Ok(hash_records(&records))
}

fn index_entries(
    repo: &Path,
    scopes: &[String],
) -> Result<BTreeMap<String, IndexEntry>, SnapshotError> {
    let mut args = vec!["ls-files", "-s", "-z", "--"];
    args.extend(scopes.iter().map(String::as_str));
    let bytes = git_required(repo, &args, "read Git index")?;
    let mut entries = BTreeMap::new();
    for raw in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let tab = raw.iter().position(|byte| *byte == b'\t').ok_or_else(|| {
            SnapshotError::Unavailable("Git index entry omitted path separator".into())
        })?;
        let header = String::from_utf8(raw[..tab].to_vec()).map_err(|error| {
            SnapshotError::Unavailable(format!("Git index header is not UTF-8: {error}"))
        })?;
        let fields = header.split(' ').collect::<Vec<_>>();
        if fields.len() != 3 || fields[2] != "0" {
            return Err(SnapshotError::Unavailable(
                "unmerged or malformed Git index cannot produce a snapshot identity".into(),
            ));
        }
        let path = String::from_utf8(raw[tab + 1..].to_vec())
            .map_err(|error| SnapshotError::Unavailable(format!("Git path is not UTF-8: {error}")))?
            .replace('\\', "/");
        entries.insert(
            path,
            IndexEntry {
                mode: fields[0].to_string(),
                oid: fields[1].to_string(),
            },
        );
    }
    Ok(entries)
}

fn effective_file_mode(index_mode: &str, metadata: &fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 != 0 {
            "100755".into()
        } else {
            "100644".into()
        }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        index_mode.to_string()
    }
}

fn untracked_paths(repo: &Path, scopes: &[String]) -> Result<Vec<String>, SnapshotError> {
    let mut args = vec![
        "ls-files",
        "--others",
        "--exclude-per-directory=.gitignore",
        "-z",
        "--",
    ];
    args.extend(scopes.iter().map(String::as_str));
    decode_paths(&git_required(
        repo,
        &args,
        "enumerate untracked snapshot inputs",
    )?)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Overlay {
    tracked_modified: BTreeSet<String>,
    tracked_deleted: BTreeSet<String>,
    untracked: BTreeSet<String>,
    renamed: BTreeSet<String>,
    type_changed: BTreeSet<String>,
    staged: BTreeSet<String>,
}

impl Overlay {
    fn unversioned(paths: Vec<String>) -> Self {
        Self {
            untracked: paths.into_iter().collect(),
            ..Self::default()
        }
    }

    fn paths(&self) -> Vec<String> {
        let mut paths = BTreeSet::new();
        paths.extend(self.tracked_modified.iter().cloned());
        paths.extend(self.tracked_deleted.iter().cloned());
        paths.extend(self.untracked.iter().cloned());
        paths.extend(self.renamed.iter().cloned());
        paths.extend(self.type_changed.iter().cloned());
        paths.extend(self.staged.iter().cloned());
        paths.into_iter().collect()
    }

    fn members_json(&self) -> Value {
        json!({
            "trackedModified": self.tracked_modified,
            "trackedDeleted": self.tracked_deleted,
            "untracked": self.untracked,
            "renamed": self.renamed,
            "typeChanged": self.type_changed,
            "staged": self.staged
        })
    }
}

fn overlay_status_with<I>(
    repo: &Path,
    scopes: &[String],
    ignore_controls: &I,
) -> Result<Overlay, SnapshotError>
where
    I: Fn(&Path, &[String]) -> Result<Vec<String>, SnapshotError>,
{
    let mut args = vec![
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--",
    ];
    let mut pathspecs = scopes.to_vec();
    pathspecs.extend(ignore_controls(repo, scopes)?);
    pathspecs.extend(
        index_entries(repo, &[".".to_string()])?
            .into_keys()
            .filter(|path| ignore_control_relevant(path, scopes)),
    );
    pathspecs.sort();
    pathspecs.dedup();
    args.extend(pathspecs.iter().map(String::as_str));
    let output = git_required(repo, &args, "inspect working-tree overlay")?;
    let entries = output
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    let mut overlay = Overlay::default();
    let mut index = 0;
    while index < entries.len() {
        let entry = entries[index];
        if entry.len() < 4 {
            return Err(SnapshotError::Unavailable(
                "Git returned malformed porcelain status".into(),
            ));
        }
        let status = &entry[..2];
        let path = String::from_utf8(entry[3..].to_vec())
            .map_err(|error| SnapshotError::Unavailable(format!("Git path is not UTF-8: {error}")))?
            .replace('\\', "/");
        classify_status(&mut overlay, status, &path)?;
        if status.contains(&b'R') || status.contains(&b'C') {
            index += 1;
            let source = entries.get(index).ok_or_else(|| {
                SnapshotError::Unavailable("Git rename status omitted source path".into())
            })?;
            overlay.renamed.insert(
                String::from_utf8(source.to_vec())
                    .map_err(|error| {
                        SnapshotError::Unavailable(format!("Git path is not UTF-8: {error}"))
                    })?
                    .replace('\\', "/"),
            );
        }
        index += 1;
    }
    overlay.untracked.extend(untracked_paths(repo, scopes)?);
    let indexed = index_entries(repo, &[".".to_string()])?;
    overlay.untracked.extend(
        ignore_controls(repo, scopes)?
            .into_iter()
            .filter(|path| !indexed.contains_key(path)),
    );
    Ok(overlay)
}

fn classify_status(overlay: &mut Overlay, status: &[u8], path: &str) -> Result<(), SnapshotError> {
    if matches!(
        status,
        b"DD" | b"AU" | b"UD" | b"UA" | b"DU" | b"AA" | b"UU"
    ) {
        return Err(SnapshotError::Unavailable(format!(
            "unmerged Git status cannot produce a snapshot identity: {}",
            String::from_utf8_lossy(status)
        )));
    }
    if status == b"!!" {
        return Ok(());
    }
    let x = status[0];
    let y = status[1];
    if status == b"??" {
        overlay.untracked.insert(path.into());
        return Ok(());
    }
    if x != b' ' {
        overlay.staged.insert(path.into());
    }
    if x == b'D' || y == b'D' {
        overlay.tracked_deleted.insert(path.into());
    }
    if x == b'R' || y == b'R' || x == b'C' || y == b'C' {
        overlay.renamed.insert(path.into());
    }
    if x == b'T' || y == b'T' {
        overlay.type_changed.insert(path.into());
    }
    if matches!(x, b'M' | b'A') || matches!(y, b'M' | b'A') {
        overlay.tracked_modified.insert(path.into());
    }
    Ok(())
}

fn decode_paths(bytes: &[u8]) -> Result<Vec<String>, SnapshotError> {
    let mut paths = bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec()).map(|path| {
                let normalized = path.replace('\\', "/");
                normalized
                    .strip_prefix("./")
                    .unwrap_or(&normalized)
                    .to_string()
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| SnapshotError::Unavailable(format!("Git path is not UTF-8: {error}")))?;
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn inventory_unversioned(repo: &Path, scopes: &[String]) -> Result<Vec<String>, SnapshotError> {
    let rg = if cfg!(windows) { "rg.exe" } else { "rg" };
    let output = Command::new(rg)
        .args([
            "--files",
            "--hidden",
            "--null",
            "--no-require-git",
            "--no-ignore-parent",
            "--no-ignore-global",
            "--no-ignore-exclude",
            "-g",
            "!**/.git/**",
        ])
        .args(scopes)
        .env_remove("RIPGREP_CONFIG_PATH")
        .current_dir(repo)
        .output()
        .map_err(|error| SnapshotError::Unavailable(format!("cannot launch {rg}: {error}")))?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(SnapshotError::Unavailable(format!(
            "unversioned inventory failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let mut paths = decode_paths(&output.stdout)?;
    paths.extend(filesystem_ignore_controls(repo, scopes)?);
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn digest_unversioned(repo: &Path, scopes: &[String]) -> Result<String, SnapshotError> {
    let paths = inventory_unversioned(repo, scopes)?;
    let mut records = vec![b"code-intel-unversioned-input.v1".to_vec()];
    for relative in paths {
        let path = repo.join(&relative);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| SnapshotError::Io(format!("inspect {relative}: {error}")))?;
        let (kind, bytes) = if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)
                .map_err(|error| SnapshotError::Io(format!("read symlink {relative}: {error}")))?;
            (
                "symlink",
                target
                    .to_str()
                    .ok_or_else(|| SnapshotError::Io(format!("non-UTF-8 symlink: {relative}")))?
                    .replace('\\', "/")
                    .into_bytes(),
            )
        } else if metadata.is_file() {
            (
                "file",
                fs::read(&path)
                    .map_err(|error| SnapshotError::Io(format!("read {relative}: {error}")))?,
            )
        } else {
            continue;
        };
        let mut record = kind.as_bytes().to_vec();
        record.push(0);
        record.extend_from_slice(relative.as_bytes());
        record.push(0);
        record.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        record.extend_from_slice(&bytes);
        records.push(record);
    }
    Ok(hash_records(&records))
}

fn stable_unversioned_snapshot(
    repo: &Path,
    scopes: &[String],
) -> Result<(String, Vec<String>), SnapshotError> {
    let before_paths = inventory_unversioned(repo, scopes)?;
    let first = digest_unversioned(repo, scopes)?;
    let second = digest_unversioned(repo, scopes)?;
    let after_paths = inventory_unversioned(repo, scopes)?;
    if first != second || before_paths != after_paths {
        return Err(SnapshotError::Io(
            "unversioned tree changed while snapshot identity was being computed; retry".into(),
        ));
    }
    Ok((first, after_paths))
}

fn git_output(repo: &Path, args: &[&str]) -> Result<std::process::Output, SnapshotError> {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|error| SnapshotError::Unavailable(format!("cannot launch Git: {error}")))
}

fn git_required(repo: &Path, args: &[&str], action: &str) -> Result<Vec<u8>, SnapshotError> {
    let output = git_output(repo, args)?;
    if !output.status.success() {
        return Err(SnapshotError::Unavailable(format!(
            "cannot {action}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map(|index| index + 1)
        .unwrap_or(start);
    &bytes[start..end]
}

fn hash_records(records: &[Vec<u8>]) -> String {
    let mut canonical = Vec::new();
    for record in records {
        canonical.extend_from_slice(&(record.len() as u64).to_be_bytes());
        canonical.extend_from_slice(record);
    }
    sha256_hex(&canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn overlay_rejects_a_change_between_complete_reads() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repo = std::env::temp_dir().join(format!("code-intel-snapshot-race-{nonce}"));
        fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.name", "Snapshot Test"],
            vec!["config", "user.email", "snapshot@example.invalid"],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        fs::write(repo.join("file.txt"), "one").unwrap();
        assert!(Command::new("git")
            .args(["add", "."])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["commit", "--quiet", "-m", "fixture"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        let result = stable_overlay_snapshot_with(
            &repo,
            &[".".into()],
            || {
                fs::write(
                    repo.join("file.txt"),
                    "content changed between complete reads",
                )
                .unwrap();
            },
            |_, _| Ok(Vec::new()),
        );
        assert!(
            matches!(&result, Err(SnapshotError::Io(message)) if message.contains("changed")),
            "unexpected stable overlay result: {result:?}"
        );
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn porcelain_xy_table_classifies_every_supported_state() {
        let cases: &[(&[u8], &str)] = &[
            (b"??", "untracked"),
            (b"!!", "ignored"),
            (b" M", "modified"),
            (b" A", "modified"),
            (b" D", "deleted"),
            (b"M ", "staged_modified"),
            (b"A ", "staged_modified"),
            (b"D ", "staged_deleted"),
            (b"R ", "renamed"),
            (b"C ", "renamed"),
            (b" T", "type"),
            (b"T ", "staged_type"),
            (b"MM", "staged_modified"),
            (b"AM", "staged_modified"),
            (b"RM", "renamed"),
            (b"MD", "staged_deleted"),
            (b"AD", "staged_deleted"),
            (b"RD", "renamed_deleted"),
            (b"CD", "renamed_deleted"),
        ];
        for (status, expected) in cases {
            let mut overlay = Overlay::default();
            classify_status(&mut overlay, status, "file").unwrap();
            let present = |set: &BTreeSet<String>| set.contains("file");
            match *expected {
                "untracked" => assert!(present(&overlay.untracked)),
                "ignored" => assert!(overlay.paths().is_empty()),
                "modified" => assert!(present(&overlay.tracked_modified)),
                "deleted" => assert!(present(&overlay.tracked_deleted)),
                "renamed" => assert!(present(&overlay.renamed)),
                "type" => assert!(present(&overlay.type_changed)),
                "staged_modified" => {
                    assert!(present(&overlay.staged));
                    assert!(present(&overlay.tracked_modified));
                }
                "staged_deleted" => {
                    assert!(present(&overlay.staged));
                    assert!(present(&overlay.tracked_deleted));
                }
                "staged_type" => {
                    assert!(present(&overlay.staged));
                    assert!(present(&overlay.type_changed));
                }
                "renamed_deleted" => {
                    assert!(present(&overlay.staged));
                    assert!(present(&overlay.renamed));
                    assert!(present(&overlay.tracked_deleted));
                }
                _ => unreachable!(),
            }
        }
        for status in [b"DD", b"AU", b"UD", b"UA", b"DU", b"AA", b"UU"] {
            let mut overlay = Overlay::default();
            assert!(classify_status(&mut overlay, status, "file").is_err());
        }
    }
}
