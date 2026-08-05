use std::collections::HashMap;
use std::env;
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
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            HostType::Github => "github",
            HostType::Gitlab => "gitlab",
            HostType::Gitea => "gitea",
            HostType::Generic => "generic",
            HostType::None => "none",
        }
    }

    pub(crate) fn from_str(s: &str) -> HostType {
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
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return out,
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
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
