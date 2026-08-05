//! The authoritative list of environment variables this CLI reads.
//!
//! Every `env::var` / `env::var_os` name in `src/` must appear here. A guard
//! test walks the source tree and fails when one does not, so the registry
//! cannot fall behind the code that reads the environment.
//!
//! This exists because propagation and validation kept sharing a blind spot:
//! a test would spawn the binary with the ambient environment and override two
//! or three keys, and any *other* variable the binary branches on stayed
//! whatever the developer's shell happened to export. The default branch then
//! went untested, and the suite passed or failed depending on who ran it —
//! see `PIPELINE_VARS` for the ones that have actually bitten us.
//!
//! The file is compiled into the binary and included directly by the test
//! support module (`#[path = "../src/env_contract.rs"]`), so both sides read
//! one list rather than two that agree until they don't.

/// Variables this project defines. A test that spawns the CLI must neutralize
/// every one of these unless it is deliberately exercising that variable —
/// they redirect roots, manifests and binaries, so an inherited value silently
/// points the child at the developer's real installation.
pub const PIPELINE_VARS: &[&str] = &[
    "CODE_INTEL_ARTIFACT_ROOT",
    "CODE_INTEL_BIN",
    "CODE_INTEL_CC_SWITCH_API_KEY",
    "CODE_INTEL_CC_SWITCH_ENDPOINT",
    "CODE_INTEL_DATA_ROOT",
    "CODE_INTEL_GIT_HOST_OVERRIDES",
    "CODE_INTEL_HOME",
    "CODE_INTEL_INTEGRATIONS_MANIFEST",
    "CODE_INTEL_LANG",
    "CODE_INTEL_TEST_RG_EXTRA_PATH",
    "SENTRUX_AUTO_PRO",
];

/// Variables the operating system or shell owns. Tests inherit or set these
/// deliberately (a child with no `PATH` cannot find `git`), so they are
/// registered but never blanket-cleared.
pub const AMBIENT_VARS: &[&str] = &[
    "HOME",
    "LANG",
    "LC_ALL",
    "LOCALAPPDATA",
    "PATH",
    "PROCESSOR_IDENTIFIER",
    "PSUICulture",
    "USERPROFILE",
    "XDG_DATA_HOME",
];

/// Every registered variable, pipeline-owned first.
pub fn all_vars() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = PIPELINE_VARS
        .iter()
        .chain(AMBIENT_VARS.iter())
        .copied()
        .collect();
    names.sort_unstable();
    names
}

/// True when `name` is registered above.
pub fn is_registered(name: &str) -> bool {
    PIPELINE_VARS.contains(&name) || AMBIENT_VARS.contains(&name)
}
