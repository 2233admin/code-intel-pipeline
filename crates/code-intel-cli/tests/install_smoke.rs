//! Install-topology coverage (DR-0001). An extracted release invoked by
//! absolute path, with only `git` and `rg` on PATH (no Python, no PowerShell,
//! the release's bin directory not on PATH), must pass doctor bootstrap (#396).
//! Checkout tests always have `target/` builds and Python, so only this gate
//! sees that regression. CI points `CODE_INTEL_SMOKE_BIN` at the installed
//! release's `bin/` after the packaged install and runs it with `--ignored`.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
            .expect("CODE_INTEL_SMOKE_BIN must point at the installed release bin directory"),
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
