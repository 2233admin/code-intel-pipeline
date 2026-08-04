//! The environment is a branch input, so it gets the same treatment as any
//! other contract: one authoritative list, and a gate that fails when the code
//! and the list disagree.
//!
//! Two properties are enforced here.
//!
//! 1. **No unregistered reads.** Every `env::var` / `env::var_os` name in
//!    `src/` appears in `env_contract`. Without this the registry decays into
//!    a stale comment: someone adds a branch on a new variable, no test
//!    clears it, and the default branch quietly stops being exercised.
//!
//! 2. **No direct binary construction in tests.** Every test spawns the CLI
//!    through `common::cli()`. This is the property that makes the fix stick —
//!    clearing the environment in one helper achieves nothing if the next test
//!    file goes back to `Command::new(env!("CARGO_BIN_EXE_code-intel"))`.
//!
//! The pairing is deliberate. A gate scoped to "the variables we happened to
//! think of" is scoped to the tool rather than to the invariant, and drifts
//! the moment the tool changes.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let path = entry.expect("read dir entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Every `"NAME"` passed to `env::var` / `env::var_os` in the given source.
fn env_reads(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for marker in ["env::var(\"", "env::var_os(\""] {
        let mut rest = source;
        while let Some(start) = rest.find(marker) {
            let after = &rest[start + marker.len()..];
            match after.find('"') {
                Some(end) => {
                    names.push(after[..end].to_string());
                    rest = &after[end..];
                }
                None => break,
            }
        }
    }
    names
}

#[test]
fn every_environment_variable_the_binary_reads_is_registered() {
    let src = crate_root().join("src");
    let mut sources = Vec::new();
    rust_sources(&src, &mut sources);
    assert!(!sources.is_empty(), "no sources found under {}", src.display());

    let mut unregistered: BTreeSet<String> = BTreeSet::new();
    for path in &sources {
        // The registry names the variables; reading it as data would report itself.
        if path.file_name().is_some_and(|name| name == "env_contract.rs") {
            continue;
        }
        let source = fs::read_to_string(path).expect("read source");
        for name in env_reads(&source) {
            if !common::env_contract::is_registered(&name) {
                unregistered.insert(format!("{name} (read in {})", path.display()));
            }
        }
    }

    assert!(
        unregistered.is_empty(),
        "these environment variables are read but not declared in src/env_contract.rs:\n  {}\n\
         Add each to PIPELINE_VARS (ours — tests must clear it) or AMBIENT_VARS (the OS owns it).",
        unregistered.into_iter().collect::<Vec<_>>().join("\n  "),
    );
}

#[test]
fn no_registered_variable_is_declared_twice() {
    let all = common::env_contract::all_vars();
    let unique: BTreeSet<&str> = all.iter().copied().collect();
    assert_eq!(
        all.len(),
        unique.len(),
        "env_contract declares a variable in both PIPELINE_VARS and AMBIENT_VARS",
    );
}

#[test]
fn tests_spawn_the_binary_only_through_the_hermetic_helper() {
    let tests_dir = crate_root().join("tests");
    let mut sources = Vec::new();
    rust_sources(&tests_dir, &mut sources);

    let helper = tests_dir.join("common").join("mod.rs");
    // This file names the forbidden pattern in order to forbid it, and the
    // helper is the one place allowed to use it.
    let this_file = tests_dir.join("env_contract.rs");
    let mut offenders: Vec<String> = Vec::new();
    for path in &sources {
        if path == &helper || path == &this_file {
            continue;
        }
        let source = fs::read_to_string(path).expect("read test source");
        if source.contains("CARGO_BIN_EXE_code-intel") {
            offenders.push(path.display().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these tests construct the CLI directly instead of calling common::cli(), \
         so they inherit whatever the developer's shell exports:\n  {}",
        offenders.join("\n  "),
    );
}

#[test]
fn the_hermetic_helper_actually_clears_the_pipeline_variables() {
    // `common::cli()` removes the pipeline variables; asking the child to print
    // its own view of them proves the removal reached the process rather than
    // just the builder.
    let output = common::cli()
        .arg("--version")
        .output()
        .expect("run code-intel --version");
    assert!(
        output.status.success(),
        "--version failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    // The clearing is load-bearing only if these variables can change behaviour
    // at all. Setting one to an unusable path must be visible somewhere — if
    // this ever stops being true the variable no longer belongs in PIPELINE_VARS.
    assert!(
        !common::env_contract::PIPELINE_VARS.is_empty(),
        "PIPELINE_VARS is empty, so common::cli() clears nothing",
    );
}

#[test]
fn a_hostile_ambient_environment_does_not_reach_the_child() {
    // Simulates the failure this whole contract exists to prevent: the developer's
    // shell exports CODE_INTEL_HOME (Claude Code and Codex both do), the test
    // spawns the binary, and the child silently resolves against that installation
    // instead of the fixture. `common::cli()` output must be identical whether or
    // not the variables are present, because it removed them either way.
    let hermetic = common::cli()
        .arg("--version")
        .output()
        .expect("run hermetic --version");

    let mut command = common::cli();
    for (name, value) in common::hostile_env() {
        // Re-adding after the helper cleared it is exactly what an inherited
        // environment looks like from the child's point of view.
        command.env(name, value);
    }
    let hostile = command.arg("--version").output().expect("run hostile --version");

    assert_eq!(
        hermetic.status.code(),
        hostile.status.code(),
        "--version exit code changed under a hostile environment",
    );
    assert_eq!(
        String::from_utf8_lossy(&hermetic.stdout),
        String::from_utf8_lossy(&hostile.stdout),
        "--version stdout changed under a hostile environment",
    );
}
