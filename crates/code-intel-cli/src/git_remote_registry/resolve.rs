use super::host::{HostOverride, HostType};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

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

    pub(crate) fn to_json(&self) -> Value {
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

    pub(crate) fn from_json(v: &Value) -> Option<RemoteInfo> {
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
    pub(crate) fn to_link_json(&self) -> Option<Value> {
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

/// Canonicalizes a `local_path` for use as a cache key: lowercase (Windows
/// paths are case-insensitive) and forward slashes, so a path independently
/// derived by this module (e.g. from a `git -C` invocation) joins correctly
/// against a `local_path` value repowise handed back verbatim. See design
/// doc §7.3 -- this project has hit path-normalization bugs across
/// worktrees/environments before.
pub fn normalize_local_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}
