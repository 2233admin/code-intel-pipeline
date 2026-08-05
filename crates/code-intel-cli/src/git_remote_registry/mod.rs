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
//!
//! Split into one file per concern (host detection/override, URL
//! resolution, sidecar persistence, client-side injection) rather than one
//! flat module -- this crate's own architecture gate flags any file over
//! 25 functions and 400 lines as a new "god file", and the original
//! single-file version of this module (26 functions, 659 lines) tripped
//! exactly that ratchet.

mod host;
mod injection;
mod registry;
mod resolve;

pub use host::{discover_git_host_overrides, load_git_host_overrides, HostOverride, HostType};
pub use injection::{build_remote_link_injection_script, REMOTE_LINKS_ROUTE};
pub use registry::{
    default_registry_path, git_remote_origin, warm_up_from_upstream, GitRemoteRegistry,
};
pub use resolve::{normalize_local_path, resolve_remote, RemoteInfo};

#[cfg(test)]
mod tests;
