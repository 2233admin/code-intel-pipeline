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

fn run_packaged_legacy_session(
    release_root: &Path,
    bin: &Path,
    repo: &Path,
    operation: &str,
    session_id: &str,
) -> (i32, String) {
    let script = release_root
        .join("legacy")
        .join("Invoke-SentruxAgentTool.ps1");
    assert!(
        script.is_file(),
        "packaged legacy session script missing: {}",
        script.display()
    );
    let native = if cfg!(windows) {
        bin.join("code-intel.exe")
    } else {
        bin.join("code-intel")
    };
    assert!(
        native.is_file(),
        "installed native CLI missing: {}",
        native.display()
    );
    let output = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(script)
        .arg(operation)
        .arg(repo)
        .args(["-SessionId", session_id])
        .env("PATH", prepend_path(bin))
        .env("CODE_INTEL_RUST_CLI", native)
        .output()
        .unwrap_or_else(|error| panic!("launch packaged legacy session: {error}"));
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), text.trim().to_string())
}

fn parse_session_json(code_and_text: &(i32, String), operation: &str) -> serde_json::Value {
    assert_eq!(
        code_and_text.0, 0,
        "packaged {operation} exited {}: {}",
        code_and_text.0, code_and_text.1
    );
    serde_json::from_str(&code_and_text.1).unwrap_or_else(|error| {
        panic!(
            "packaged {operation} must emit JSON: {error}; output={}",
            code_and_text.1
        )
    })
}

fn metric_i64(value: &serde_json::Value, path: &str) -> i64 {
    let mut current = value;
    for key in path.split('.') {
        current = current
            .get(key)
            .unwrap_or_else(|| panic!("missing session metric {path} in {value}"));
    }
    current
        .as_i64()
        .unwrap_or_else(|| panic!("session metric {path} is not an integer: {current}"))
}

#[test]
#[ignore = "packaged installed topology gate; CI sets CODE_INTEL_SMOKE_* after package install"]
fn packaged_install_legacy_sessions_use_native_metrics_and_preserve_baselines() {
    let release_root = PathBuf::from(
        env::var("CODE_INTEL_SMOKE_RELEASE_ROOT")
            .expect("CODE_INTEL_SMOKE_RELEASE_ROOT must point at the packaged release root"),
    );
    let bin = PathBuf::from(
        env::var("CODE_INTEL_SMOKE_BIN")
            .expect("CODE_INTEL_SMOKE_BIN must point at the installed bin directory"),
    );
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let repo = env::temp_dir().join(format!(
        "code-intel-installed-session-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(repo.join("src")).expect("create packaged session source tree");
    fs::create_dir_all(repo.join("tests")).expect("create packaged session test tree");
    fs::create_dir_all(repo.join(".sentrux/cache")).expect("create baseline cache");
    fs::write(
        repo.join("src/lib.rs"),
        "mod helper;\nuse helper::value;\npub fn run() -> i32 { value() }\n",
    )
    .expect("write production source");
    fs::write(repo.join("src/helper.rs"), "pub fn value() -> i32 { 1 }\n")
        .expect("write production helper");
    fs::write(
        repo.join("tests/session_imports.rs"),
        "use crate::test_dependency_one;\nuse crate::test_dependency_two;\n#[test]\nfn session_contract() {}\n",
    )
    .expect("write test-only imports");
    fs::write(
        repo.join(".sentrux/rules.toml"),
        "[constraints]\nignore_test_dependencies = true\n",
    )
    .expect("write coupling policy");
    let canonical = b"{\n  \"schema\": \"canonical-fixture\",\n  \"owner\": \"install-smoke\"\n}\n";
    let lite = b"{\n  \"tool\": \"sentrux-lite\",\n  \"quality_signal\": 100\n}\n";
    let canonical_path = repo.join(".sentrux/baseline.json");
    let lite_path = repo.join(".sentrux/cache/lite-baseline.json");
    fs::write(&canonical_path, canonical).expect("write canonical baseline");
    fs::write(&lite_path, lite).expect("write lite baseline");

    let start = parse_session_json(
        &run_packaged_legacy_session(
            &release_root,
            &bin,
            &repo,
            "session_start",
            "installed-session",
        ),
        "session_start",
    );
    assert_eq!(start["tool"], "session_start");
    assert_eq!(start["gate"]["pass"], true);
    assert!(metric_i64(&start, "gate.metrics_observed_count") >= 4);
    assert!(repo
        .join(".sentrux/cache/native-session-baseline.json")
        .is_file());

    let native_baseline_path = repo.join(".sentrux/cache/native-session-baseline.json");
    let native_baseline: serde_json::Value = serde_json::from_slice(
        &fs::read(&native_baseline_path).expect("read native session baseline"),
    )
    .expect("native session baseline JSON");
    assert_eq!(native_baseline["schema"], "code-intel-sentrux-baseline.v7");
    assert_eq!(native_baseline["engine"]["id"], "sentrux-native");
    assert_eq!(
        native_baseline["couplingPolicy"]["ignore_test_dependencies"],
        true
    );
    assert!(metric_i64(&native_baseline, "metrics.test_files") > 0);
    assert_eq!(metric_i64(&native_baseline, "metrics.coupling_files"), 2);
    assert!(
        metric_i64(&native_baseline, "metrics.coupling_import_edges")
            < metric_i64(&native_baseline, "metrics.total_import_edges")
    );

    let end = parse_session_json(
        &run_packaged_legacy_session(
            &release_root,
            &bin,
            &repo,
            "session_end",
            "installed-session",
        ),
        "session_end",
    );
    assert_eq!(end["tool"], "session_end");
    assert_eq!(end["pass"], true);
    assert!(metric_i64(&end, "gate.metrics_observed_count") >= 4);
    assert_eq!(end["signal_before"], end["signal_after"]);
    assert_eq!(
        fs::read(&canonical_path).expect("read canonical baseline"),
        canonical
    );
    assert_eq!(fs::read(&lite_path).expect("read lite baseline"), lite);
    let _ = fs::remove_dir_all(repo);
}
