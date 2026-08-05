//! Phase 1 of the GitHub/GitLab/Gitea remote-linkage design
//! (`docs/github-gitlab-remote-linkage-design.md`, tracked in issue #191).
//!
//! Resolves a repo's `origin` remote into a normalized, credential-free
//! record (host type, owner/repo, web base URL) and persists it in a
//! code-intel-owned JSON sidecar under `CODE_INTEL_DATA_ROOT`. Never reads
//! from or writes to repowise's own `wiki.db` -- see design doc §4.1 for why.
//!
//! Phase 2 (cache warm-up) is wired into `repowise_proxy_server::start_proxy`.
//! Phase 3 (the `/__code-intel/remote-links.json` route and client-side
//! injection) is implemented here as [`GitRemoteRegistry::to_remote_links_json`]
//! and [`build_remote_link_injection_script`]; `repowise_proxy_server`
//! serves the route and appends the script, but owns none of this logic.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostType {
    Github,
    Gitlab,
    Gitea,
    Generic,
    None,
}

impl HostType {
    fn as_str(self) -> &'static str {
        match self {
            HostType::Github => "github",
            HostType::Gitlab => "gitlab",
            HostType::Gitea => "gitea",
            HostType::Generic => "generic",
            HostType::None => "none",
        }
    }

    fn from_str(s: &str) -> HostType {
        match s {
            "github" => HostType::Github,
            "gitlab" => HostType::Gitlab,
            "gitea" => HostType::Gitea,
            "none" => HostType::None,
            _ => HostType::Generic,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostOverride {
    pub host_type: HostType,
    pub web_base_url: String,
}

#[derive(Debug, Clone)]
pub struct RemoteInfo {
    pub remote_url_normalized: Option<String>,
    pub host_type: HostType,
    pub host: Option<String>,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub web_base_url: Option<String>,
    pub has_credentials_stripped: bool,
    pub is_plaintext_transport: bool,
    /// repowise's own `repositories.id`, recorded opportunistically when a
    /// proxied response carries it (design doc §4.2: never the durable
    /// identity, but useful as a fast join key). The web UI's repo cards
    /// link to `/repos/{id}/overview` -- confirmed by inspecting the live
    /// DOM -- so this is exactly the key the client-side injection script
    /// needs to attach a deep link to the right card.
    pub repo_id: Option<String>,
}

impl RemoteInfo {
    fn none() -> Self {
        RemoteInfo {
            remote_url_normalized: None,
            host_type: HostType::None,
            host: None,
            owner: None,
            repo: None,
            web_base_url: None,
            has_credentials_stripped: false,
            is_plaintext_transport: false,
            repo_id: None,
        }
    }

    fn to_json(&self) -> Value {
        let mut obj = Map::new();
        obj.insert(
            "remote_url_normalized".into(),
            match &self.remote_url_normalized {
                Some(v) => json!(v),
                None => Value::Null,
            },
        );
        obj.insert("host_type".into(), json!(self.host_type.as_str()));
        obj.insert(
            "host".into(),
            self.host
                .as_deref()
                .map(|s| json!(s))
                .unwrap_or(Value::Null),
        );
        obj.insert(
            "owner".into(),
            self.owner
                .as_deref()
                .map(|s| json!(s))
                .unwrap_or(Value::Null),
        );
        obj.insert(
            "repo".into(),
            self.repo
                .as_deref()
                .map(|s| json!(s))
                .unwrap_or(Value::Null),
        );
        obj.insert(
            "web_base_url".into(),
            self.web_base_url
                .as_deref()
                .map(|s| json!(s))
                .unwrap_or(Value::Null),
        );
        obj.insert(
            "has_credentials_stripped".into(),
            json!(self.has_credentials_stripped),
        );
        obj.insert(
            "is_plaintext_transport".into(),
            json!(self.is_plaintext_transport),
        );
        obj.insert(
            "repo_id".into(),
            self.repo_id
                .as_deref()
                .map(|s| json!(s))
                .unwrap_or(Value::Null),
        );
        Value::Object(obj)
    }

    fn from_json(v: &Value) -> Option<RemoteInfo> {
        let obj = v.as_object()?;
        let get_str = |key: &str| -> Option<String> {
            obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
        };
        Some(RemoteInfo {
            remote_url_normalized: get_str("remote_url_normalized"),
            host_type: HostType::from_str(&get_str("host_type").unwrap_or_default()),
            host: get_str("host"),
            owner: get_str("owner"),
            repo: get_str("repo"),
            web_base_url: get_str("web_base_url"),
            has_credentials_stripped: obj
                .get("has_credentials_stripped")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            is_plaintext_transport: obj
                .get("is_plaintext_transport")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            repo_id: get_str("repo_id"),
        })
    }

    /// The subset the client-side injection script actually needs, keyed
    /// by `repo_id` rather than `local_path` -- see design doc §6.2 and the
    /// doc comment on `repo_id` above for why `id` is the right join key
    /// for the DOM side specifically.
    fn to_link_json(&self) -> Option<Value> {
        let web_base_url = self.web_base_url.as_deref()?;
        Some(json!({
            "web_base_url": web_base_url,
            "host_type": self.host_type.as_str(),
        }))
    }
}

struct ParsedUrl {
    scheme: String,
    host: String,
    port: Option<String>,
    path: String,
    had_credentials: bool,
}

/// Strips `user[:pass]@` from a remote URL and splits it into scheme/host/
/// port/path, without pulling in a URL-parsing crate. Handles the two shapes
/// actually seen in this environment's 136-repo corpus (design doc §3):
/// `scheme://[user[:pass]@]host[:port]/path` and the SCP-like SSH form
/// `git@host:path`.
fn parse_remote_url(url: &str) -> ParsedUrl {
    let url = url.trim();

    if let Some(rest) = url.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            return ParsedUrl {
                scheme: "ssh".to_string(),
                host: host.to_string(),
                port: None,
                path: path.trim_start_matches('/').to_string(),
                had_credentials: false,
            };
        }
    }

    if let Some(idx) = url.find("://") {
        let scheme = url[..idx].to_string();
        let after_scheme = &url[idx + 3..];
        let (authority, path) = match after_scheme.find('/') {
            Some(slash_idx) => (
                &after_scheme[..slash_idx],
                after_scheme[slash_idx + 1..].to_string(),
            ),
            None => (after_scheme, String::new()),
        };
        let (had_credentials, host_port) = match authority.rfind('@') {
            Some(at_idx) => (true, &authority[at_idx + 1..]),
            None => (false, authority),
        };
        let (host, port) = match host_port.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
                (h.to_string(), Some(p.to_string()))
            }
            _ => (host_port.to_string(), None),
        };
        return ParsedUrl {
            scheme,
            host,
            port,
            path,
            had_credentials,
        };
    }

    // Unrecognized shape: still flag potential credentials conservatively so
    // an unusual remote never leaks a userinfo-like substring downstream.
    ParsedUrl {
        scheme: String::new(),
        host: String::new(),
        port: None,
        path: url.to_string(),
        had_credentials: url.contains('@'),
    }
}

fn split_owner_repo(path: &str) -> Option<(String, String)> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let repo_raw = trimmed.rsplit('/').next()?;
    let repo = repo_raw.strip_suffix(".git").unwrap_or(repo_raw);
    let owner_len = trimmed.len().saturating_sub(repo_raw.len());
    let owner = trimmed[..owner_len].trim_end_matches('/');
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

/// Resolves a raw `git remote get-url origin` string (or `None`, for repos
/// with no origin configured) into a [`RemoteInfo`]. `overrides` is keyed by
/// `host` or `host:port` (design doc §5.2); auto-detection alone only ever
/// recognizes `github.com` and `gitlab.com`.
pub fn resolve_remote(
    raw_remote_url: Option<&str>,
    overrides: &HashMap<String, HostOverride>,
) -> RemoteInfo {
    let raw = match raw_remote_url {
        Some(r) if !r.trim().is_empty() => r,
        _ => return RemoteInfo::none(),
    };

    let parsed = parse_remote_url(raw);
    let (owner, repo) = match split_owner_repo(&parsed.path) {
        Some(pair) => pair,
        None => return RemoteInfo::none(),
    };

    let host_key = match &parsed.port {
        Some(port) => format!("{}:{}", parsed.host, port),
        None => parsed.host.clone(),
    };

    let (host_type, web_base_url) = if parsed.host == "github.com" {
        (HostType::Github, "https://github.com".to_string())
    } else if parsed.host == "gitlab.com" {
        (HostType::Gitlab, "https://gitlab.com".to_string())
    } else if let Some(over) = overrides
        .get(&host_key)
        .or_else(|| overrides.get(&parsed.host))
    {
        (
            over.host_type,
            over.web_base_url.trim_end_matches('/').to_string(),
        )
    } else if parsed.host.is_empty() {
        return RemoteInfo::none();
    } else {
        // Generic: still linkable to the repo root, using the transport's
        // own scheme/host/port -- see design doc §5.3 on what this can and
        // can't do (no per-file/line deep links without a known host type).
        //
        // Scheme is deliberately NOT passed through verbatim: `parsed.scheme`
        // comes from a `git remote get-url origin` string, which is
        // attacker-influenceable (anyone who controls a `.git/config` in the
        // indexed workspace controls this string). This value ends up as
        // `link.href` on a real anchor element in the proxied page (see
        // `build_remote_link_injection_script`) -- an unfiltered scheme like
        // `javascript:` would be a stored-XSS vector the moment a user
        // clicks the injected "open remote" link. Only `http`/`https` pass
        // through; everything else (ssh, git, file, javascript, ...) maps to
        // `https`, since the web UI a browser navigates to is https
        // regardless of which transport `git` itself used.
        let scheme = match parsed.scheme.as_str() {
            "http" => "http".to_string(),
            "https" => "https".to_string(),
            _ => "https".to_string(),
        };
        let base = match &parsed.port {
            Some(port) => format!("{}://{}:{}", scheme, parsed.host, port),
            None => format!("{}://{}", scheme, parsed.host),
        };
        (HostType::Generic, base)
    };

    let remote_url_normalized = format!("{}/{}/{}", web_base_url, owner, repo);
    let full_web_url = remote_url_normalized.clone();
    let is_plaintext_transport = parsed.scheme == "http";

    RemoteInfo {
        remote_url_normalized: Some(remote_url_normalized),
        host_type,
        host: Some(parsed.host),
        owner: Some(owner),
        repo: Some(repo),
        web_base_url: Some(full_web_url),
        has_credentials_stripped: parsed.had_credentials,
        is_plaintext_transport,
        repo_id: None,
    }
}

/// Mirrors `capability::discover_manifest`'s discovery order, using the
/// `CODE_INTEL_GIT_HOST_OVERRIDES` env var instead of
/// `CODE_INTEL_INTEGRATIONS_MANIFEST`. A file path, not inline JSON -- see
/// design doc §5.2 for why (Windows env-var JSON quoting).
pub fn discover_git_host_overrides(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return path.is_file().then(|| path.to_path_buf());
    }
    if let Some(path) = env::var_os("CODE_INTEL_GIT_HOST_OVERRIDES") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    if let Some(home) = env::var_os("CODE_INTEL_HOME") {
        let candidate = PathBuf::from(home)
            .join("orchestration")
            .join("git-hosts.json");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Loads a `{ "host[:port]": { "type": ..., "web_base_url": ... } }` manifest.
/// Malformed entries are skipped rather than failing the whole load, since a
/// missing/bad override should degrade to `generic` detection, not break the
/// registry.
pub fn load_git_host_overrides(path: &Path) -> HashMap<String, HostOverride> {
    let mut out = HashMap::new();
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return out,
    };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return out,
    };
    let obj = match value.as_object() {
        Some(o) => o,
        None => return out,
    };
    for (host_key, entry) in obj {
        let host_type = entry
            .get("type")
            .and_then(|v| v.as_str())
            .map(HostType::from_str);
        let web_base_url = entry
            .get("web_base_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let (Some(host_type), Some(web_base_url)) = (host_type, web_base_url) {
            out.insert(
                host_key.clone(),
                HostOverride {
                    host_type,
                    web_base_url,
                },
            );
        }
    }
    out
}

/// Canonicalizes a `local_path` for use as a cache key: lowercase (Windows
/// paths are case-insensitive) and forward slashes, so a path independently
/// derived by this module (e.g. from a `git -C` invocation) joins correctly
/// against a `local_path` value repowise handed back verbatim. See design
/// doc §7.3 -- this project has hit path-normalization bugs across
/// worktrees/environments before.
pub fn normalize_local_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

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

/// The proxy-served route path for [`GitRemoteRegistry::to_remote_links_json`]
/// (design doc §6.2). Namespaced under `/__code-intel/` so it can never
/// collide with a real repowise API route as repowise evolves.
pub const REMOTE_LINKS_ROUTE: &str = "/__code-intel/remote-links.json";

/// Client-side injection script (design doc §6.2): fetches
/// [`REMOTE_LINKS_ROUTE`] once, then decorates every repo-card link
/// (`/repos/{id}/overview`, confirmed by inspecting the live DOM) with a
/// small "open on GitHub/GitLab/Gitea" anchor.
///
/// Deliberately does *not* reuse `repowise_i18n_proxy`'s MutationObserver --
/// this is a separate concern with a separate dictionary (an id->link map
/// instead of a string->string one), and keeping them independent matches
/// this codebase's existing file-per-plugin shape rather than coupling two
/// plugins that don't need to share state.
///
/// Idempotency is content-based, not marker-based, for the same reason
/// `repowise_i18n_proxy`'s text-node translation is: repowise's repo list
/// is a large virtualized list, so React can recycle a card's DOM node for
/// a different repo on scroll. A one-time "already decorated" flag on the
/// node would then wrongly keep a stale link attached to a recycled card.
/// Instead, every pass compares the *current* desired href against what's
/// already appended and only touches the DOM when they differ.
pub fn build_remote_link_injection_script() -> String {
    format!(
        r#"<script id="code-intel-remote-links-injected">
(function() {{
  fetch({route}).then(function(r) {{ return r.ok ? r.json() : {{}}; }}).then(function(map) {{
    if (!map || Object.keys(map).length === 0) return;
    var ID_RE = /\/repos\/([0-9a-f]{{32}})\/overview/;

    function decorate(a) {{
      var href = a.getAttribute && a.getAttribute('href');
      var existing = a.querySelector(':scope > .ci-remote-link');
      var m = href && href.match(ID_RE);
      var info = m && map[m[1]];
      var desired = info && info.web_base_url;
      // Second layer, redundant with the server-side scheme allowlist in
      // resolve_remote: never assign an href whose scheme isn't http/https,
      // so a payload that somehow bypassed the server check still can't
      // become a clickable javascript: link here.
      if (desired && !/^https?:\/\//i.test(desired)) desired = null;
      if (!desired) {{
        if (existing) existing.remove();
        return;
      }}
      if (existing && existing.getAttribute('href') === desired) return;
      if (existing) existing.remove();
      var link = document.createElement('a');
      link.className = 'ci-remote-link';
      link.href = desired;
      link.target = '_blank';
      link.rel = 'noopener noreferrer';
      var label = info.host_type === 'github' ? 'GitHub'
        : info.host_type === 'gitlab' ? 'GitLab'
        : info.host_type === 'gitea' ? 'Gitea'
        : 'remote';
      link.title = 'Open on ' + label;
      link.textContent = ' ↗';
      link.style.marginLeft = '4px';
      link.style.opacity = '0.6';
      link.addEventListener('click', function(ev) {{ ev.stopPropagation(); }});
      a.appendChild(link);
    }}

    function scan(root) {{
      if (!root || root.nodeType !== Node.ELEMENT_NODE) return;
      if (root.tagName === 'A') decorate(root);
      if (!root.querySelectorAll) return;
      var links = root.querySelectorAll('a[href*="/repos/"]');
      for (var i = 0; i < links.length; i++) decorate(links[i]);
    }}

    scan(document.body);
    var observer = new MutationObserver(function(mutations) {{
      for (var i = 0; i < mutations.length; i++) {{
        var m = mutations[i];
        if (m.type === 'characterData') {{
          scan(m.target.parentElement);
        }} else {{
          for (var j = 0; j < m.addedNodes.length; j++) scan(m.addedNodes[j]);
        }}
      }}
    }});
    observer.observe(document.body, {{ childList: true, subtree: true, characterData: true }});
  }}).catch(function() {{}});
}})();
</script>"#,
        route = json!(REMOTE_LINKS_ROUTE)
    )
}

#[cfg(test)]
mod tests;
