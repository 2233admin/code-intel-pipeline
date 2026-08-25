//! Install-topology coverage for the relocated Sentrux shim (#216 / #274).
//!
//! Checkout tests lock the payload path and keep root PowerShell from growing
//! orchestration. The ignored test is the DR-0001 reproduction: after a
//! packaged install, the release still ships `legacy/tools/sentrux-shim` and
//! the installed `sentrux` launcher can run `check --help` plus `pro status`.
//! CI sets `CODE_INTEL_SMOKE_RELEASE_ROOT` / `CODE_INTEL_SMOKE_BIN` and runs
//! that test with `--ignored`.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root")
}

fn shim_payload_files() -> [&'static str; 4] {
    [
        "legacy/tools/sentrux-shim/sentrux-shim.ps1",
        "legacy/tools/sentrux-shim/sentrux-lite-core.ps1",
        "legacy/tools/sentrux-shim/sentrux.cmd",
        "legacy/tools/sentrux-shim/sentrux",
    ]
}

fn assert_shim_payload(root: &Path) {
    for relative in shim_payload_files() {
        let path = root.join(relative);
        assert!(
            path.is_file(),
            "missing relocated sentrux-shim payload {relative} under {}",
            root.display()
        );
    }
}

fn contains_ignore_case(text: &str, needle: &str) -> bool {
    text.to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn matches_tier(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(index) = rest.find("tier:") {
        let after = &rest[index + "tier:".len()..];
        let trimmed = after.trim_start_matches([' ', '\t', '\r', '\n']);
        if trimmed.len() < after.len()
            && (trimmed.starts_with("pro") || trimmed.starts_with("free"))
        {
            return true;
        }
        rest = after;
    }
    false
}

fn prepend_path(bin: &Path) -> OsString {
    let mut entries = vec![bin.to_path_buf()];
    if let Some(existing) = env::var_os("PATH") {
        entries.extend(env::split_paths(&existing));
    }
    env::join_paths(entries).expect("join PATH with installed bin first")
}

fn installed_launcher(bin: &Path) -> PathBuf {
    if cfg!(windows) {
        bin.join("sentrux.cmd")
    } else {
        bin.join("sentrux")
    }
}

fn run_installed_sentrux(bin: &Path, args: &[&str]) -> (i32, String) {
    let launcher = installed_launcher(bin);
    assert!(
        launcher.is_file(),
        "installed sentrux launcher missing: {}",
        launcher.display()
    );
    let output = Command::new(&launcher)
        .args(args)
        .env("PATH", prepend_path(bin))
        .output()
        .unwrap_or_else(|error| panic!("launch {}: {error}", launcher.display()));
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), text.trim().to_string())
}

#[test]
fn checkout_ships_relocated_sentrux_shim_and_no_root_orchestration() {
    let root = repo_root();
    assert_shim_payload(&root);

    let installer = fs::read_to_string(root.join("legacy/install-code-intel-pipeline.ps1"))
        .expect("read installer");
    assert!(
        installer
            .contains(r#"Join-Path (Join-Path (Join-Path $Root "legacy") "tools") "sentrux-shim""#),
        "installer must resolve shim source under legacy/tools/sentrux-shim"
    );
    assert!(
        installer.contains(r#"legacy/tools/sentrux-shim/sentrux-shim.ps1"#),
        "installer forwarder must target the relocated shim"
    );
    assert!(
        !installer.contains(r#"Join-Path (Join-Path $Root "tools") "sentrux-shim""#),
        "installer still references the pre-move tools/sentrux-shim path"
    );

    assert!(
        !root.join("invoke-code-intel.ps1").is_file(),
        "root invoke-code-intel.ps1 must stay absent so orchestration stays in the compiled CLI"
    );
}

#[test]
fn checkout_ships_legacy_pipeline_entrypoint_deploy_step() {
    let root = repo_root();
    let installer = fs::read_to_string(root.join("legacy/install-code-intel-pipeline.ps1"))
        .expect("read installer");
    assert!(
        installer.contains("function Install-LegacyPipelineEntrypoint"),
        "installer must deploy legacy/run-code-intel.ps1 and pipeline.config.json into <bin> (#232)"
    );
    assert!(
        installer.contains("Install-LegacyPipelineEntrypoint $Actions $Root $binDir"),
        "Install-CodeIntelBinary must call the legacy pipeline entrypoint deploy step"
    );
}

#[test]
#[ignore = "DR-0001 topology gate; CI sets CODE_INTEL_SMOKE_* after packaged install"]
fn packaged_install_deploys_legacy_pipeline_entrypoint() {
    let bin = env::var("CODE_INTEL_SMOKE_BIN")
        .expect("CODE_INTEL_SMOKE_BIN must point at the installed bin directory");
    let bin = PathBuf::from(bin);

    let script = bin.join("legacy").join("run-code-intel.ps1");
    assert!(
        script.is_file(),
        "installed bin is missing legacy/run-code-intel.ps1 (#232): {}",
        script.display()
    );
    let config = bin.join("pipeline.config.json");
    assert!(
        config.is_file(),
        "installed bin is missing pipeline.config.json (#232): {}",
        config.display()
    );
}

#[test]
#[ignore = "DR-0001 topology gate; CI sets CODE_INTEL_SMOKE_* after packaged install"]
fn packaged_install_runs_relocated_sentrux_shim() {
    let release_root = env::var("CODE_INTEL_SMOKE_RELEASE_ROOT")
        .expect("CODE_INTEL_SMOKE_RELEASE_ROOT must point at the packaged release root");
    let bin = env::var("CODE_INTEL_SMOKE_BIN")
        .expect("CODE_INTEL_SMOKE_BIN must point at the installed bin directory");
    let release_root = PathBuf::from(release_root);
    let bin = PathBuf::from(bin);

    assert_shim_payload(&release_root);

    let forwarder = fs::read_to_string(bin.join("sentrux-shim.ps1")).unwrap_or_else(|error| {
        panic!(
            "read installed sentrux-shim forwarder {}: {error}",
            bin.join("sentrux-shim.ps1").display()
        )
    });
    assert!(
        forwarder.contains("legacy/tools/sentrux-shim/sentrux-shim.ps1"),
        "installed forwarder does not target the relocated shim:\n{forwarder}"
    );

    let (check_code, check_text) = run_installed_sentrux(&bin, &["check", "--help"]);
    assert_eq!(
        check_code, 0,
        "installed sentrux check --help exited {check_code}:\n{check_text}"
    );
    assert!(
        contains_ignore_case(&check_text, "Enforce architectural rules"),
        "installed sentrux check --help missed the core marker:\n{check_text}"
    );

    let (status_code, status_text) = run_installed_sentrux(&bin, &["pro", "status"]);
    assert_eq!(
        status_code, 0,
        "installed sentrux pro status exited {status_code}:\n{status_text}"
    );
    assert!(
        matches_tier(&status_text),
        "installed sentrux pro status missed a Tier line:\n{status_text}"
    );
}
