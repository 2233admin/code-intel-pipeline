//! Contract tests for `code-intel doctor bootstrap`, the subcommand that
//! replaced `legacy/check-code-intel-tools.ps1` under T3 (issue #48).
//!
//! What is pinned here is what other components actually read: the
//! observation schema and `observation_only` authority the doctor capability
//! adapter checks, the `missing`/`ok` pair the installer parses, the
//! `checks.repo.*` fields the repo-config contract test asserts on, and the
//! exit code CI gates on.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "code-intel-doctor-cli-{tag}-{}-{nonce}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn pipeline_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("pipeline root")
}

fn doctor(args: &[&str]) -> (i32, Value, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["doctor", "bootstrap", "--pipeline-root"])
        .arg(pipeline_root())
        .args(args)
        .output()
        .expect("run doctor bootstrap");
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let value = serde_json::from_str::<Value>(&stdout).unwrap_or(Value::Null);
    (output.status.code().unwrap_or(-1), value, stderr)
}

#[test]
fn emits_one_observation_only_document_the_adapter_contract_accepts() {
    let repo = temp_dir("ok");
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    let (code, observation, stderr) = doctor(&[
        "--repo-path",
        repo.to_str().unwrap(),
        "--no-require-repowise",
        "--json",
    ]);

    assert!(stderr.is_empty(), "{stderr}");
    assert!(matches!(code, 0 | 1), "unexpected exit code {code}");
    assert_eq!(
        observation["schema"],
        "code-intel-doctor-bootstrap-observation.v1"
    );
    assert_eq!(observation["authority"], "observation_only");
    assert!(observation["ok"].is_boolean());
    assert!(observation["missing"].is_array());
    // The three pointers doctor_adapter reads out of the raw observation.
    assert!(observation.pointer("/checks/tools").unwrap().is_array());
    assert!(observation
        .pointer("/checks/sentrux/builtin/found")
        .unwrap()
        .is_boolean());
    assert!(observation
        .pointer("/checks/graphProvider/sourceFound")
        .unwrap()
        .is_boolean());
    assert_eq!(observation["checks"]["repo"]["exists"], json!(true));
    fs::remove_dir_all(repo).ok();
}

#[test]
fn exit_code_tracks_ok_so_ci_gates_on_it() {
    let root = temp_dir("absent");
    let missing_repo = root.join("not-here");
    let (code, observation, _) = doctor(&[
        "--repo-path",
        missing_repo.to_str().unwrap(),
        "--no-require-repowise",
        "--json",
    ]);

    assert_eq!(code, 1);
    assert_eq!(observation["ok"], json!(false));
    assert!(observation["missing"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "repo path"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn configured_sentrux_path_is_resolved_through_a_reverse_repo_lookup() {
    let root = temp_dir("scope");
    let repo = root.join("ConfiguredRepo");
    let sentrux = repo.join("backend").join(".sentrux");
    fs::create_dir_all(&sentrux).unwrap();
    fs::write(sentrux.join("rules.toml"), b"").unwrap();
    fs::write(sentrux.join("baseline.json"), b"{}").unwrap();

    let config_path = root.join("pipeline.config.json");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({
            "repos": {
                "fixture": {
                    "path": format!("{}{}", repo.display(), std::path::MAIN_SEPARATOR),
                    "sentruxPath": "backend"
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let (_, observation, _) = doctor(&[
        "--config",
        config_path.to_str().unwrap(),
        "--repo-path",
        repo.join(".").to_str().unwrap(),
        "--no-require-repowise",
        "--json",
    ]);

    let expected = repo.join("backend").canonicalize().unwrap();
    let expected = expected
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();
    assert_eq!(
        observation["checks"]["repo"]["sentruxScope"],
        json!(expected)
    );
    assert_eq!(observation["checks"]["repo"]["sentruxRules"], json!(true));
    assert_eq!(
        observation["checks"]["repo"]["sentruxBaseline"],
        json!(true)
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn unparsable_config_is_a_domain_finding_not_a_crash() {
    let root = temp_dir("badconfig");
    let config_path = root.join("pipeline.config.json");
    fs::write(&config_path, b"{ not json").unwrap();

    let (code, observation, stderr) = doctor(&[
        "--config",
        config_path.to_str().unwrap(),
        "--repo-path",
        root.to_str().unwrap(),
        "--no-require-repowise",
        "--json",
    ]);

    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(code, 1);
    assert_eq!(observation["checks"]["config"]["found"], json!(true));
    assert_eq!(observation["checks"]["config"]["parsed"], json!(false));
    assert!(observation["missing"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value
            .as_str()
            .is_some_and(|text| text.starts_with("pipeline config: invalid JSON"))));
    fs::remove_dir_all(root).ok();
}

/// Regression: an installed `code-intel` sits next to a copy of
/// `orchestration/integrations.json` that the installer places in its bin
/// directory, so deriving the pipeline root from the executable resolved that
/// bin directory and reported `pipeline script` / `pipeline config` missing —
/// which is exactly how CI's Doctor step failed on all four runners. Without
/// `--pipeline-root`, the checkout the caller is standing in must win.
#[test]
fn the_pipeline_root_defaults_to_the_checkout_the_caller_stands_in() {
    let root = temp_dir("cwd-root");
    let checkout = root.join("checkout");
    let bin = root.join("bin");
    // A decoy manifest beside the "installed" binary location, mirroring what
    // install-code-intel-pipeline.ps1 writes.
    for dir in [&checkout, &bin] {
        fs::create_dir_all(dir.join("orchestration")).unwrap();
        fs::write(dir.join("orchestration").join("integrations.json"), b"{}").unwrap();
    }
    fs::create_dir_all(checkout.join("legacy")).unwrap();
    fs::write(checkout.join("legacy").join("run-code-intel.ps1"), b"").unwrap();
    fs::write(checkout.join("pipeline.config.json"), b"{}").unwrap();
    let crate_src = checkout.join("crates").join("code-intel-cli").join("src");
    fs::create_dir_all(&crate_src).unwrap();
    fs::write(crate_src.join("graph.rs"), b"").unwrap();
    fs::write(
        checkout
            .join("crates")
            .join("code-intel-cli")
            .join("Cargo.toml"),
        b"[package]",
    )
    .unwrap();

    // No --pipeline-root, and the working directory is a subdirectory of the
    // checkout so the upward walk is exercised too.
    let nested = checkout.join("crates");
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["doctor", "bootstrap", "--no-require-repowise", "--json"])
        .current_dir(&nested)
        .output()
        .expect("run doctor bootstrap");
    let observation: Value =
        serde_json::from_slice(&output.stdout).expect("observation is one JSON document");

    assert_eq!(
        observation["checks"]["pipelineScript"]["found"],
        json!(true)
    );
    assert_eq!(observation["checks"]["config"]["found"], json!(true));
    assert_eq!(
        observation["checks"]["graphProvider"]["sourceFound"],
        json!(true)
    );
    let missing = observation["missing"].as_array().unwrap();
    for absent in ["pipeline script", "pipeline config"] {
        assert!(
            !missing.iter().any(|value| value == absent),
            "{absent} must not be reported when standing in the checkout: {missing:?}"
        );
    }
    fs::remove_dir_all(root).ok();
}

#[test]
fn an_unknown_flag_fails_closed_without_emitting_an_observation() {
    let (code, observation, stderr) = doctor(&["--not-a-flag"]);
    assert_eq!(code, 65);
    assert_eq!(observation, Value::Null);
    assert!(stderr.contains("unknown argument for doctor bootstrap"));
}

#[test]
fn the_powershell_entry_point_is_a_thin_forwarder() {
    // The retirement bar for T3 is "<=50 lines shim or deleted". Assert the
    // shim stays thin and stays a forwarder rather than growing logic back.
    let script = pipeline_root().join("legacy/check-code-intel-tools.ps1");
    let text = fs::read_to_string(&script).expect("read forwarder");
    let code_lines = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .count();
    assert!(
        code_lines <= 50,
        "{} has {code_lines} code lines; T3 caps the shim at 50",
        script.display()
    );
    assert!(text.contains("doctor"), "forwarder must invoke the binary");
    assert!(
        text.contains("bootstrap"),
        "forwarder must invoke the binary"
    );
}
