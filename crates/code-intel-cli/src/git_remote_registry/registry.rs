use super::host::HostOverride;
use super::resolve::{normalize_local_path, resolve_remote, RemoteInfo};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

fn data_root() -> PathBuf {
    if let Ok(value) = env::var("CODE_INTEL_DATA_ROOT") {
        if !value.trim().is_empty() {
            return PathBuf::from(value);
        }
    }
    if cfg!(windows) {
        if let Some(base) = env::var_os("LOCALAPPDATA") {
            if !base.is_empty() {
                return PathBuf::from(base).join("code-intel");
            }
        }
    }
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".code-intel")
}

pub fn default_registry_path() -> PathBuf {
    data_root().join("remote-links").join("registry.json")
}

/// The JSON sidecar (design doc §4). Never reads or writes repowise's own
/// `wiki.db` -- entries here are keyed by normalized `local_path`, disjoint
/// from repowise's storage.
pub struct GitRemoteRegistry {
    path: PathBuf,
    entries: HashMap<String, RemoteInfo>,
}

impl GitRemoteRegistry {
    pub fn load(path: PathBuf) -> Self {
        let entries = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|v| v.as_object().cloned())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| RemoteInfo::from_json(v).map(|info| (k.clone(), info)))
                    .collect()
            })
            .unwrap_or_default();
        GitRemoteRegistry { path, entries }
    }

    pub fn get(&self, local_path: &str) -> Option<&RemoteInfo> {
        self.entries.get(&normalize_local_path(local_path))
    }

    pub fn upsert(&mut self, local_path: &str, info: RemoteInfo) {
        self.entries.insert(normalize_local_path(local_path), info);
    }

    /// Writes via a temp-file-then-rename, not a direct `fs::write`: the
    /// proxy's warm-up thread, `get_or_resolve`, and every
    /// `/__code-intel/remote-links.json` request (`GitRemoteRegistry::load`)
    /// all touch this same file, potentially concurrently. A direct write
    /// lets a reader observe a partially-written file mid-save -- `load`
    /// then fails to parse it and silently falls back to an empty registry,
    /// and a save interrupted mid-write (process killed) would leave a
    /// truncated file on disk. `rename` is atomic on both Windows and POSIX
    /// filesystems, so readers only ever see a complete old or new version.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut obj = Map::new();
        for (k, v) in &self.entries {
            obj.insert(k.clone(), v.to_json());
        }
        let bytes = serde_json::to_vec_pretty(&Value::Object(obj))?;

        let mut tmp_name = self
            .path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_else(|| "registry.json".into());
        tmp_name.push(format!(".tmp-{}", std::process::id()));
        let tmp_path = self.path.with_file_name(tmp_name);

        fs::write(&tmp_path, bytes)?;
        fs::rename(&tmp_path, &self.path)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The `/__code-intel/remote-links.json` route body (design doc §6.2):
    /// `{ "<repo_id>": { "web_base_url": ..., "host_type": ... }, ... }`.
    /// Keyed by `repo_id`, not `local_path` -- the client-side script joins
    /// against the DOM's own `/repos/{id}/overview` card links, not paths.
    /// Entries with no resolvable `repo_id` or no usable `web_base_url`
    /// (no origin, `host_type: none`) are omitted rather than emitted as
    /// null, so the client's map only ever contains real, linkable entries.
    pub fn to_remote_links_json(&self) -> Value {
        let mut obj = Map::new();
        for info in self.entries.values() {
            let (Some(repo_id), Some(link)) = (info.repo_id.as_deref(), info.to_link_json()) else {
                continue;
            };
            obj.insert(repo_id.to_string(), link);
        }
        Value::Object(obj)
    }

    /// Cache-miss path (design doc §6.1): resolves one repo on demand and
    /// persists the result, instead of re-scanning the whole workspace.
    /// Not wired to any HTTP surface yet (that's Phase 3) -- exists now so
    /// Phase 3 has a ready-made entry point.
    pub fn get_or_resolve(
        &mut self,
        local_path: &str,
        overrides: &HashMap<String, HostOverride>,
    ) -> &RemoteInfo {
        let key = normalize_local_path(local_path);
        if !self.entries.contains_key(&key) {
            let raw = git_remote_origin(local_path);
            let info = resolve_remote(raw.as_deref(), overrides);
            self.entries.insert(key.clone(), info);
            let _ = self.save();
        }
        self.entries
            .get(&key)
            .expect("just inserted or already present")
    }
}

/// Shells out to `git -C <local_path> remote get-url origin`. Returns
/// `None` if git isn't available, the path isn't a git repo, or no
/// `origin` is configured -- all legitimate states in the live corpus
/// (design doc §3: 21/136 repos have no origin at all).
pub fn git_remote_origin(local_path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", local_path, "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

/// Startup warm-up pass (design doc §6.1): fetches the full repo list from
/// repowise's own `/api/repos` and resolves a remote-link entry for each,
/// via a `git remote get-url origin` shell-out per repo. `repowise_proxy_server`
/// runs this once in a background thread at startup so it never blocks the
/// first proxied request. Returns `(local_path, RemoteInfo)` pairs rather
/// than mutating a registry directly, so this stays testable independent of
/// the sidecar file; an empty upstream repo list (`/api/repos` unreachable,
/// or repowise not finished starting yet) returns an empty `Vec` rather than
/// erroring -- callers should treat that as "try again later," not fatal.
pub fn warm_up_from_upstream(
    upstream_url: &str,
    overrides: &HashMap<String, HostOverride>,
) -> Vec<(String, RemoteInfo)> {
    let repos: Value = match ureq::get(&format!("{}/api/repos", upstream_url)).call() {
        Ok(resp) => match resp.into_json() {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        },
        Err(_) => return Vec::new(),
    };
    let mut results = Vec::new();
    if let Some(arr) = repos.as_array() {
        for repo in arr {
            if let Some(local_path) = repo.get("local_path").and_then(|v| v.as_str()) {
                let raw = git_remote_origin(local_path);
                let mut info = resolve_remote(raw.as_deref(), overrides);
                info.repo_id = repo
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                results.push((local_path.to_string(), info));
            }
        }
    }
    results
}
