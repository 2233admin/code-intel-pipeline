//! Environment isolation is checked through the actual child process.
//! Pipeline-owned variables use the authoritative registry; ambient OS variables
//! remain available so Git and Windows networking can initialize.

mod common;

use std::collections::BTreeSet;

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
fn version_result_is_independent_of_hostile_installation_paths() {
    // Users must be able to identify the running binary even when the shell
    // points other commands at a stale installation or unusable roots.
    // This checks the public version result, not a command builder's shape.
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
    let hostile = command
        .arg("--version")
        .output()
        .expect("run hostile --version");

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
