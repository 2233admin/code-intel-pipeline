//! Shared test support: spawning the CLI without the developer's environment
//! leaking into it.
//!
//! `src/env_contract.rs` is included directly rather than copied, so the list
//! of variables this module clears is the same list the binary declares.

#[path = "../../src/env_contract.rs"]
pub mod env_contract;

use std::path::Path;
use std::process::Command;

/// The CLI under test, with every pipeline-owned variable removed.
///
/// Tests used to build `Command::new(env!("CARGO_BIN_EXE_code-intel"))`
/// directly, which inherits the whole ambient environment. Any variable in
/// [`env_contract::PIPELINE_VARS`] that the developer happened to export then
/// redirected the child at their real installation — a root, a manifest or a
/// binary — and the default branch went untested. Worse, it only reproduced
/// for people whose shell exported it, which is why `CODE_INTEL_HOME` took two
/// separate investigations to pin down.
///
/// Ambient variables (`PATH`, `HOME`, …) are deliberately left alone: a child
/// with no `PATH` cannot find `git`, and tests that care set them explicitly.
///
/// A test that is genuinely exercising one of these variables sets it back
/// with `.env(...)` after calling this — the point is that it becomes a
/// property of the test rather than of the shell.
pub fn cli() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_code-intel"));
    for name in env_contract::PIPELINE_VARS {
        command.env_remove(name);
    }
    command
}

/// [`cli`] with the working directory set.
pub fn cli_in(dir: &Path) -> Command {
    let mut command = cli();
    command.current_dir(dir);
    command
}

/// Values that must not change the CLI's behaviour when they leak in from the
/// surrounding shell. Used by the hermeticity regression test to prove the
/// clearing above actually holds.
pub fn hostile_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("CODE_INTEL_ARTIFACT_ROOT", "/nonexistent/hostile/artifacts"),
        ("CODE_INTEL_BIN", "/nonexistent/hostile/bin"),
        ("CODE_INTEL_DATA_ROOT", "/nonexistent/hostile/data"),
        ("CODE_INTEL_HOME", "/nonexistent/hostile/home"),
        (
            "CODE_INTEL_INTEGRATIONS_MANIFEST",
            "/nonexistent/hostile/manifest.json",
        ),
        ("CODE_INTEL_TEST_RG_EXTRA_PATH", "hostile/extra.rs"),
        ("SENTRUX_AUTO_PRO", "1"),
    ]
}
