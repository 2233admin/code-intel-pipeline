use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

use super::{process::hash_file, Failure};

pub(super) const MARKER: &str = "code-intel-snapshot.json";
const CACHE_IGNORE: &str = "# CodeGraph data files — local to each machine, not for committing.\n# Ignore everything in .codegraph/ except this file itself, so transient\n# files (the database, daemon.pid, sockets, logs) never show up in git.\n*\n!.gitignore\n";

pub(super) fn resolve_root(explicit: Option<&Path>, repo: &Path) -> Result<PathBuf, Failure> {
    let requested = crate::artifacts::resolve_artifact_root(explicit)
        .map_err(|e| Failure::io(format!("resolve artifact root: {e}")))?;
    let resolved = resolve_missing(&requested)?;
    if resolved.starts_with(repo) {
        return Err(Failure::contract(
            "artifact root must be outside the source repository",
        ));
    }
    let root = crate::artifacts::ensure_directory(&resolved)
        .map_err(|e| Failure::io(format!("create artifact root: {e}")))?;
    let root = fs::canonicalize(root)
        .map_err(|e| Failure::io(format!("resolve created artifact root: {e}")))?;
    if root.starts_with(repo) {
        return Err(Failure::contract(
            "artifact root resolves inside the source repository",
        ));
    }
    Ok(root)
}

// Canonicalize existing ancestors, including symlinks, for deleted changed
// paths and not-yet-created artifact directories. Never create source paths.
pub(super) fn resolve_missing(path: &Path) -> Result<PathBuf, Failure> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| Failure::io(e.to_string()))?
            .join(path)
    };
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| Failure::contract("path has no existing ancestor"))?
                        .to_os_string(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| Failure::contract("path has no existing ancestor"))?;
            }
            Err(e) => {
                return Err(Failure::io(format!(
                    "inspect path {}: {e}",
                    ancestor.display()
                )))
            }
        }
    }
    let mut resolved = fs::canonicalize(ancestor)
        .map_err(|e| Failure::io(format!("resolve path {}: {e}", ancestor.display())))?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    let mut normalized = PathBuf::new();
    for component in resolved.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

pub(super) fn check_source_path(repo: &Path, relative: &str) -> Result<(), Failure> {
    let resolved = resolve_missing(&repo.join(relative))?;
    if !resolved.starts_with(repo) {
        return Err(Failure::usage(format!(
            "repository-relative path escapes repository through a symlink: {relative}"
        )));
    }
    if relative == ".codegraph"
        || relative.starts_with(".codegraph/")
        || relative == ".git"
        || relative.starts_with(".git/")
    {
        return Err(Failure::usage(
            "--file/--changed must name source, not repository/cache metadata",
        ));
    }
    Ok(())
}

pub(super) fn cache_path(repo: &Path) -> Result<PathBuf, Failure> {
    let cache = repo.join(".codegraph");
    if let Ok(metadata) = fs::symlink_metadata(&cache) {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Failure::contract(
                ".codegraph must be a real local directory, not a symlink",
            ));
        }
        if fs::canonicalize(&cache).map_err(|e| Failure::io(e.to_string()))? != cache {
            return Err(Failure::contract(
                ".codegraph must not redirect outside its repository",
            ));
        }
    }
    Ok(cache)
}

pub(super) fn initialize_cache(cache: &Path) -> Result<(), Failure> {
    crate::artifacts::ensure_directory(cache)
        .map_err(|e| Failure::io(format!("create CodeGraph cache: {e}")))?;
    let path = cache.join(".gitignore");
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(CACHE_IGNORE.as_bytes())
                .map_err(|e| Failure::io(format!("write cache ignore control: {e}")))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(Failure::io(format!("create cache ignore control: {e}"))),
    }
    let metadata = fs::symlink_metadata(&path).map_err(|e| Failure::io(e.to_string()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Failure::contract(
            "cache .gitignore must be a regular local file",
        ));
    }
    let ignore = fs::read_to_string(path).map_err(|e| Failure::io(e.to_string()))?;
    if !ignore.lines().any(|line| line.trim() == "*") {
        return Err(Failure::contract(
            "custom cache .gitignore must ignore all cache entries with '*'; preserved unchanged",
        ));
    }
    Ok(())
}

pub(super) struct CacheLock {
    path: PathBuf,
}
impl CacheLock {
    pub fn acquire(cache: &Path) -> Result<Self, Failure> {
        let path = cache.join("code-intel.lock");
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists { Failure::contract("another Pipeline CodeGraph operation holds code-intel.lock; a crashed operation requires owner removal of that cache lock") }
            else { Failure::io(format!("acquire CodeGraph cache lock: {e}")) }
        })?;
        if let Err(e) = writeln!(file, "{}", std::process::id()) {
            let _ = fs::remove_file(&path);
            return Err(Failure::io(e.to_string()));
        }
        Ok(Self { path })
    }
}
impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Database state at the cache path, decided **before** any writer runs.
///
/// `Path::is_file()` follows symlinks, so a dangling `codegraph.db` link reports
/// `false` and would be treated as "no database" — which routes `index` into the
/// initial-index writer before `database_identity` gets to reject the link. This
/// guard uses `symlink_metadata` so a link, directory, or other non-regular entry
/// is refused up front instead of after the write.
pub(super) enum DatabaseState {
    /// No entry at the path: a writer may create the database.
    Absent,
    /// A regular local file: use `database_identity` for its content identity.
    Regular,
}

pub(super) fn database_state(cache: &Path) -> Result<DatabaseState, Failure> {
    match fs::symlink_metadata(cache.join("codegraph.db")) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(Failure::contract(
                    "CodeGraph database must be a regular local file, not a symlink or directory",
                ));
            }
            Ok(DatabaseState::Regular)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DatabaseState::Absent),
        Err(e) => Err(Failure::io(format!("inspect CodeGraph database: {e}"))),
    }
}

pub(super) fn database_identity(cache: &Path) -> Result<Value, Failure> {
    // Re-checks what `database_state` already established for `codegraph.db`, and
    // additionally constrains `codegraph.db-wal`. Kept independent so a caller
    // that reaches identity hashing still refuses a link or non-regular entry.
    let mut identities = serde_json::Map::new();
    for name in ["codegraph.db", "codegraph.db-wal"] {
        let path = cache.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(Failure::contract(
                        "CodeGraph database/sidecar must be a regular local file",
                    ));
                }
                // Read-only SQLite opens may create an empty WAL. It carries no
                // database content; absence and zero bytes have the same identity.
                if name == "codegraph.db-wal" && metadata.len() == 0 {
                    continue;
                }
                identities.insert(name.into(), json!(hash_file(&path)?));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && name != "codegraph.db" => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Failure::contract(
                    "CodeGraph cache has no database; explicit index required",
                ))
            }
            Err(e) => {
                return Err(Failure::io(format!(
                    "inspect CodeGraph database identity: {e}"
                )))
            }
        }
    }
    Ok(Value::Object(identities))
}

pub(super) fn read_marker(cache: &Path) -> Result<Option<Value>, Failure> {
    let path = cache.join(MARKER);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Failure::io(format!("inspect snapshot binding: {e}"))),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1024 * 1024 {
        return Err(Failure::contract(
            "snapshot binding must be a regular local JSON file below 1 MiB",
        ));
    }
    let bytes = fs::read(path).map_err(|e| Failure::io(format!("read snapshot binding: {e}")))?;
    let text = std::str::from_utf8(&bytes).map_err(|e| Failure::contract(e.to_string()))?;
    crate::capability::reject_duplicate_json_keys(text).map_err(Failure::contract)?;
    let marker: Value = serde_json::from_str(text)
        .map_err(|e| Failure::contract(format!("malformed snapshot binding: {e}")))?;
    if marker["schema"] != "code-intel-codegraph-snapshot-binding.v1"
        || marker["scope"] != json!(["."])
        || !marker["snapshot"].is_object()
        || !marker["provider"].is_object()
        || !marker["database"].is_object()
        || !marker["repository"].is_string()
        || marker["boundAt"].as_u64().is_none()
        || marker.as_object().map_or(true, |fields| fields.len() != 7)
    {
        return Err(Failure::contract(
            "malformed CodeGraph snapshot binding contract",
        ));
    }
    crate::capability::validate_snapshot(&marker["snapshot"]).map_err(Failure::contract)?;
    let digest = |value: &Value| {
        value.as_str().is_some_and(|text| {
            text.len() == 64
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    };
    let implementation = &marker["provider"]["implementation"];
    if marker["provider"]["id"] != "codegraph.cli"
        || implementation["id"] != "colbymchenry/codegraph"
        || !implementation["version"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
        || !digest(&implementation["digest"])
        || !digest(&marker["database"]["codegraph.db"])
        || marker["database"]
            .as_object()
            .unwrap()
            .iter()
            .any(|(name, value)| {
                !matches!(name.as_str(), "codegraph.db" | "codegraph.db-wal") || !digest(value)
            })
    {
        return Err(Failure::contract(
            "malformed CodeGraph provider/database binding identity",
        ));
    }
    Ok(Some(marker))
}

pub(super) fn invalidate(cache: &Path) -> Result<(), Failure> {
    match fs::remove_file(cache.join(MARKER)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Failure::io(format!("invalidate CodeGraph binding: {e}"))),
    }
}

pub(super) fn publish_marker(cache: &Path, marker: &Value) -> Result<(), Failure> {
    let bytes = serde_json::to_vec(marker).map_err(|e| Failure::contract(e.to_string()))?;
    let path = cache.join(MARKER);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Failure::io(format!("publish CodeGraph binding: {e}")))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| Failure::io(format!("persist CodeGraph binding: {e}")))
}

pub(super) fn persist_payload(
    root: &Path,
    snapshot_identity: &str,
    payload: &Value,
) -> Result<Value, Failure> {
    let bytes = serde_json::to_vec(payload).map_err(|e| Failure::contract(e.to_string()))?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(Failure::contract(
            "CodeGraph evidence payload exceeds A04 64 MiB bound",
        ));
    }
    let digest = crate::capability::sha256_hex(&bytes);
    let directory = crate::artifacts::ensure_directory(&root.join("codegraph/payloads"))
        .map_err(|e| Failure::io(format!("create evidence directory: {e}")))?;
    let canonical = fs::canonicalize(&directory).map_err(|e| Failure::io(e.to_string()))?;
    if !canonical.starts_with(root) {
        return Err(Failure::contract(
            "evidence directory escapes artifact root through a symlink",
        ));
    }
    let path = directory.join(format!("{digest}.json"));
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
                drop(file);
                let _ = fs::remove_file(&path);
                return Err(Failure::io(format!("persist CodeGraph evidence: {error}")));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&path).map_err(|e| Failure::io(e.to_string()))?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || hash_file(&path)? != digest
            {
                return Err(Failure::contract("content-addressed evidence path has conflicting bytes or is not a regular file"));
            }
        }
        Err(e) => return Err(Failure::io(format!("create CodeGraph evidence: {e}"))),
    }
    Ok(
        json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":"code-intel-evidence-payload.v1","type":"observed.evidence.payload","path":format!("codegraph/payloads/{digest}.json"),"sha256":digest,"consumedSnapshotIdentity":snapshot_identity}),
    )
}

#[cfg(test)]
mod tests {
    use super::{database_state, DatabaseState};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    /// Temp cache directory removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "code-intel-database-state-{}-{nonce}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(original, link)
        }
        #[cfg(not(windows))]
        {
            std::os::unix::fs::symlink(original, link)
        }
    }

    #[test]
    fn absent_database_reports_absent() {
        let scratch = Scratch::new();
        match database_state(scratch.path()) {
            Ok(DatabaseState::Absent) => {}
            Ok(DatabaseState::Regular) => panic!("expected Absent, got Regular"),
            Err(_) => panic!("expected Absent, got a failure"),
        }
    }

    #[test]
    fn regular_database_reports_regular() {
        let scratch = Scratch::new();
        fs::write(scratch.path().join("codegraph.db"), b"database bytes").unwrap();
        match database_state(scratch.path()) {
            Ok(DatabaseState::Regular) => {}
            Ok(DatabaseState::Absent) => panic!("expected Regular, got Absent"),
            Err(_) => panic!("expected Regular, got a failure"),
        }
    }

    /// The regression this guard exists for: `Path::is_file()` follows the link,
    /// so a dangling `codegraph.db` used to look absent and route `index` into the
    /// initial-index writer before the identity check could reject it.
    #[test]
    fn dangling_database_symlink_is_rejected_before_a_writer_runs() {
        let scratch = Scratch::new();
        let target = scratch.path().join("outside-target.db");
        let link = scratch.path().join("codegraph.db");
        match symlink_file(&target, &link) {
            Ok(()) => {}
            // Windows needs Developer Mode or elevation to create symlinks; the
            // contract cannot be exercised here, so it is not claimed.
            Err(error) => {
                eprintln!("skipping dangling-link case: {error}");
                return;
            }
        }
        assert!(!target.exists(), "the link must be dangling for this case");
        // Precondition that made the bug reachable.
        assert!(
            !link.is_file(),
            "is_file() must report false for a dangling link"
        );
        assert!(
            database_state(scratch.path()).is_err(),
            "a dangling database link must be rejected before any writer runs"
        );
        assert!(
            !target.exists(),
            "the guard must not follow the link into a write"
        );
    }

    #[test]
    fn database_directory_entry_is_rejected() {
        let scratch = Scratch::new();
        fs::create_dir_all(scratch.path().join("codegraph.db")).unwrap();
        assert!(
            database_state(scratch.path()).is_err(),
            "a directory at the database path must be rejected"
        );
    }
}
