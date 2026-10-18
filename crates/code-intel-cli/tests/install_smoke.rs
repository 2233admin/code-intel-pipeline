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

fn locate_on_path(name: &str) -> PathBuf {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(&exe))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("{name} must be on PATH for this smoke"))
}

/// PATH exposing only `git` and `rg`: the minimal host the native core path
/// claims to need (#396). On Unix the tools are symlinked into a fresh
/// directory, because standard runners keep `git` in `/usr/bin` next to
/// `python3`. On Windows `git.exe` locates its install relative to itself, so
/// the real directories are used and must not expose Python or PowerShell.
fn git_and_rg_only_path(scratch: &Path) -> OsString {
    let tools = [locate_on_path("git"), locate_on_path("rg")];
    #[cfg(unix)]
    {
        let dir = scratch.join("path");
        fs::create_dir_all(&dir).expect("create isolated PATH dir");
        for tool in &tools {
            let link = dir.join(tool.file_name().expect("tool file name"));
            std::os::unix::fs::symlink(tool, &link)
                .unwrap_or_else(|error| panic!("link {}: {error}", tool.display()));
        }
        dir.into_os_string()
    }
    #[cfg(not(unix))]
    {
        let _ = scratch;
        let dirs: Vec<PathBuf> = tools
            .iter()
            .map(|tool| tool.parent().expect("tool dir").to_path_buf())
            .collect();
        for dir in &dirs {
            for forbidden in ["python.exe", "python3.exe", "pwsh.exe"] {
                assert!(
                    !dir.join(forbidden).is_file(),
                    "{} also provides {forbidden}; cannot build a python/pwsh-free PATH",
                    dir.display()
                );
            }
        }
        env::join_paths(dirs).expect("join git/rg PATH")
    }
}

#[test]
#[ignore = "DR-0001 topology gate; CI sets CODE_INTEL_SMOKE_* after packaged install"]
fn packaged_doctor_is_ready_without_python_pwsh_or_bin_on_path() {
    let bin = PathBuf::from(
        env::var("CODE_INTEL_SMOKE_BIN")
            .expect("CODE_INTEL_SMOKE_BIN must point at the installed bin directory"),
    );
    let binary = bin.join(if cfg!(windows) {
        "code-intel.exe"
    } else {
        "code-intel"
    });
    assert!(
        binary.is_file(),
        "installed binary missing: {}",
        binary.display()
    );

    let scratch = env::temp_dir().join(format!("code-intel-396-{}", std::process::id()));
    let fixture = scratch.join("repo");
    fs::create_dir_all(&fixture).expect("create fixture repo");
    fs::write(fixture.join("main.py"), "print('hi')\n").expect("write fixture source");
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&fixture)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=smoke",
        "-c",
        "user.email=smoke@example.invalid",
        "commit",
        "-qm",
        "fixture",
    ]);

    // Absolute-path invocation, bin directory NOT on PATH, no Python or
    // PowerShell anywhere on PATH: the clean-package topology from #396.
    let output = Command::new(&binary)
        .args(["doctor", "bootstrap", "--repo-path"])
        .arg(&fixture)
        .args(["--no-require-repowise", "--json"])
        .current_dir(&fixture)
        .env("PATH", git_and_rg_only_path(&scratch))
        .env_remove("CODE_INTEL_HOME")
        .output()
        .unwrap_or_else(|error| panic!("launch {}: {error}", binary.display()));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "doctor did not emit JSON ({error}):\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let missing = report["missing"].as_array().cloned().unwrap_or_default();
    assert!(
        report["ok"] == true && missing.is_empty() && output.status.success(),
        "packaged doctor must be ready with only git + rg (#396); exit {:?}, missing {missing:?}:\n{stdout}",
        output.status.code()
    );
    let _ = fs::remove_dir_all(&scratch);
}
