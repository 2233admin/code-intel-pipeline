//! Contract tests for the installer's version gate.
//!
//! Before this gate existed, `Install-MissingTool` returned on presence alone:
//! a machine with `repowise` 0.32.0 on PATH reported `already_present` while
//! the supply-chain-003 pin declared 0.38.0, so the pin was a declaration
//! nothing enforced. Presence and correctness were indistinguishable in the
//! install report.
//!
//! The functions under test live inside `legacy/install-code-intel-pipeline.ps1`,
//! whose top level performs a real installation — dot-sourcing it would install.
//! The driver written here lifts the two functions out by AST and evaluates
//! them with their collaborators stubbed. Per AGENTS.md the driver is generated
//! into a temp directory rather than committed, so this adds no new PowerShell
//! to the tree; assertions live on the Rust side, matching the #78/#80
//! direction of porting PowerShell call points to Rust.

#[path = "common/installer_version.rs"]
mod installer_version;

use installer_version::{scenario, scenario_from_installer};
use std::path::PathBuf;

#[test]
fn tool_version_parses_the_formats_the_gate_actually_meets() {
    let standard = scenario("parse-standard");
    assert_eq!(standard["refused"], false);
    assert_eq!(
        standard["parsed"], "0.32.0",
        "`repowise, version X` is what the pinned tool prints"
    );

    let prerelease = scenario("parse-prerelease");
    assert_eq!(prerelease["refused"], false);
    assert_eq!(
        prerelease["parsed"], "0.7.0-beta.2",
        "a prerelease suffix must survive whole, or every beta reads as drift"
    );
}

#[test]
fn the_probe_refuses_sources_the_project_forbids_launching() {
    // tool_path.rs states the rule for every tool launch in this project:
    // "only ever launches by absolute path", "relative PATH entries are
    // skipped outright". A `.ps1` is worse than merely relative — `& $Source`
    // would execute it inside the installer's own process.
    for tag in [
        "parse-empty-source",
        "parse-missing-tool",
        "parse-relative-source",
        "parse-script-source",
    ] {
        assert_eq!(
            scenario(tag)["refused"],
            true,
            "{tag} must be refused, not executed"
        );
    }
}

#[test]
fn a_version_shaped_number_in_a_banner_does_not_win_the_match() {
    let result = scenario("parse-noise-before-version");
    assert_eq!(result["refused"], false);
    assert_eq!(
        result["parsed"], "0.38.0",
        "the name-anchored line wins over `setuptools 3.11.0` noise"
    );
}

#[test]
fn a_probe_that_ran_but_read_nothing_is_unknown_not_a_match() {
    // The gate exists to surface exactly this state; treating it as a match
    // would restore the presence-only behaviour it replaces.
    for tag in ["parse-unparseable"] {
        assert_eq!(
            scenario(tag)["refused"],
            false,
            "{tag} is executable; it ran"
        );
        assert_eq!(
            scenario(tag)["parsed"],
            "",
            "{tag} must read as unknown, not as a version"
        );
    }
}

#[test]
fn a_matching_pinned_version_stays_already_present() {
    let result = scenario("match");
    assert_eq!(result["status"], "already_present");
}

#[test]
fn newer_than_pin_stays_already_present_and_is_never_downgraded() {
    // The pin is a floor, not an exact target. A user who upgraded past the
    // pin must not be reported as drifted, and -InstallMissing must not
    // downgrade them back to the pin on every rerun — the scenario's
    // installer block throws if invoked.
    let result = scenario("newer");
    assert_eq!(result["status"], "already_present");
}

#[test]
fn drift_is_reported_even_when_the_gate_may_not_fix_it() {
    // Without -InstallMissing the installer must not touch the system, but
    // staying silent is the failure this branch exists to prevent: a
    // present-but-wrong version previously read as already_present.
    let result = scenario("drift");
    assert_eq!(result["status"], "version_drift");
}

#[test]
fn an_unreadable_version_reports_drift_rather_than_passing() {
    let result = scenario("drift-unknown");
    assert_eq!(result["status"], "version_drift");
}

#[test]
fn tools_without_a_pin_keep_their_previous_behaviour() {
    let result = scenario("unpinned");
    assert_eq!(result["status"], "already_present");
}

#[test]
fn install_missing_upgrades_a_drifted_tool_to_the_pin() {
    let result = scenario("upgrade");
    assert_eq!(result["status"], "upgraded");
}

#[test]
fn a_reinstall_that_does_not_reach_the_pin_fails_loudly() {
    assert_eq!(scenario("upgrade-failed")["status"], "upgrade_failed");
}

#[test]
fn confirmed_drift_reaches_the_ok_computation_but_uncertainty_does_not() {
    // The installer's `ok` is derived from `$checks`, never from
    // `$installActions`, and bootstrap-new-machine.ps1 reads only
    // `installResult.ok`. Drift that stays in installActions is invisible to
    // every consumer.
    let result = scenario("compliance-checks");

    let emitted: Vec<&str> = result["emitted"]
        .as_array()
        .expect("emitted")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        emitted,
        vec!["version:repowise", "version:mystery"],
        "only drift and upgrade_failed become checks; already_present and upgraded do not"
    );

    let required: Vec<&str> = result["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        required,
        vec!["version:repowise"],
        "a measured mismatch fails the install; an unreadable version is uncertainty and must not"
    );
}

#[test]
fn compatibility_forwarding_preserves_python_prerelease_and_postrelease_decisions() {
    assert_eq!(scenario("pep-floor-rc")["status"], "version_drift");
    assert_eq!(scenario("pep-floor-post")["status"], "already_present");
    assert_eq!(scenario("pep-upgrade-newer")["status"], "upgraded");
}

#[test]
fn a_missing_native_version_owner_cannot_trigger_installation() {
    let result = scenario("missing-owner");
    assert_eq!(result["blocked"], true);
    assert_eq!(result["installerCalled"], false);
}

#[test]
#[ignore = "DR-0001 topology gate; CI supplies the installed release root"]
fn packaged_install_preserves_repowise_python_version_floor() {
    let root = PathBuf::from(
        std::env::var("CODE_INTEL_SMOKE_RELEASE_ROOT").expect("installed release root"),
    );
    let installer = root.join("legacy/install-code-intel-pipeline.ps1");
    assert_eq!(
        scenario_from_installer("pep-floor-rc", &installer, Some(&root))["status"],
        "version_drift"
    );
    assert_eq!(
        scenario_from_installer("pep-floor-post", &installer, Some(&root))["status"],
        "already_present"
    );
    assert_eq!(
        scenario_from_installer("pep-upgrade-newer", &installer, Some(&root))["status"],
        "upgraded"
    );
}
