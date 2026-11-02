//! Contract tests for `code-intel doctor bootstrap`.
//!
//! What is pinned here is what other components actually read: the
//! observation schema and `observation_only` authority the doctor capability
//! adapter checks, the `missing`/`ok` pair the installer parses, the
//! `checks.repo.*` fields the repo-config contract test asserts on, and the
//! exit code CI gates on.
mod common;

use common::TempDir;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

fn pipeline_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("pipeline root")
}

fn doctor(args: &[&str]) -> (i32, Value, String) {
    let output = common::cli()
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
    let repo_dir = TempDir::new("ok");
    let repo = repo_dir.path();
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
        .pointer("/checks/graphProvider/binaryFound")
        .unwrap()
        .is_boolean());
    assert_eq!(observation["checks"]["repo"]["exists"], json!(true));
}

#[test]
fn invocation_identity_is_cli_layer_unique_and_echoes_the_nonce() {
    let repo_dir = TempDir::new("identity");
    let repo = repo_dir.path();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    let args = [
        "--repo-path",
        repo.to_str().unwrap(),
        "--no-require-repowise",
        "--json",
        "--nonce",
        "night-7",
    ];
    let (first_code, first, first_stderr) = doctor(&args);
    let (_, second, _) = doctor(&args);

    // Doctor's identity rides inside the observation, unlike the verdict
    // commands' stderr echo (#197) — stderr stays clean for envelope readers.
    assert!(first_stderr.is_empty(), "{first_stderr}");
    assert!(
        matches!(first_code, 0 | 1),
        "unexpected exit code {first_code}"
    );
    assert_eq!(first["invocationIdentity"]["nonce"], json!("night-7"));
    let first_id = first["invocationIdentity"]["id"].as_str().expect("id");
    let second_id = second["invocationIdentity"]["id"].as_str().expect("id");
    assert!(!first_id.is_empty());
    // Byte-identical argv, distinct identity: a replayed capture repeats the
    // id, a live rerun cannot.
    assert_ne!(first_id, second_id);
}

#[test]
fn nonce_without_a_value_fails_closed() {
    let (code, _, stderr) = doctor(&["--json", "--nonce"]);
    assert_eq!(code, 65);
    assert!(stderr.contains("--nonce requires a value"), "{stderr}");
}

#[test]
fn exit_code_tracks_ok_so_ci_gates_on_it() {
    let root_dir = TempDir::new("absent");
    let root = root_dir.path();
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
}

#[test]
fn configured_sentrux_path_is_resolved_through_a_reverse_repo_lookup() {
    let root_dir = TempDir::new("scope");
    let root = root_dir.path();
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
}

#[test]
fn unparsable_config_is_a_domain_finding_not_a_crash() {
    let root_dir = TempDir::new("badconfig");
    let root = root_dir.path();
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
}

/// Regression: an installed `code-intel` sits next to a copy of
/// `orchestration/integrations.json` that the installer places in its bin
/// directory, so deriving the pipeline root from the executable resolved that
/// bin directory and reported `pipeline script` / `pipeline config` missing —
/// which is exactly how CI's Doctor step failed on all four runners. Without
/// `--pipeline-root`, the checkout the caller is standing in must win.
#[test]
fn the_pipeline_root_defaults_to_the_checkout_the_caller_stands_in() {
    let root_dir = TempDir::new("cwd-root");
    let root = root_dir.path();
    let checkout = root.join("checkout");
    let bin = root.join("bin");
    // A decoy manifest beside the "installed" binary location, mirroring what
    // the installer writes.
    for dir in [&checkout, &bin] {
        fs::create_dir_all(dir.join("orchestration")).unwrap();
        fs::write(dir.join("orchestration").join("integrations.json"), b"{}").unwrap();
    }
    fs::write(checkout.join("pipeline.config.json"), b"{}").unwrap();
    fs::create_dir_all(checkout.join("crates")).unwrap();

    // No --pipeline-root, and the working directory is a subdirectory of the
    // checkout so the upward walk is exercised too.
    let nested = checkout.join("crates");
    let output = common::cli()
        .args(["doctor", "bootstrap", "--no-require-repowise", "--json"])
        .current_dir(&nested)
        .output()
        .expect("run doctor bootstrap");
    let observation: Value =
        serde_json::from_slice(&output.stdout).expect("observation is one JSON document");

    assert!(observation["checks"].get("pipelineScript").is_none());
    assert_eq!(observation["checks"]["config"]["found"], json!(true));
    let missing = observation["missing"].as_array().unwrap();
    for absent in ["pipeline script", "pipeline config"] {
        assert!(
            !missing.iter().any(|value| value == absent),
            "{absent} must not be reported when standing in the checkout: {missing:?}"
        );
    }
}

/// A release payload has the compiled `code-intel` binary in `bin/`, not in a
/// source checkout's `target/{release,debug}` directories. Doctor must report
/// that installed binary instead of describing a path every release install
/// necessarily lacks (issue #232).
#[test]
fn release_layout_reports_the_packaged_binary() {
    let release_dir = TempDir::new("release-layout");
    let release = release_dir.path();
    let repo_dir = TempDir::new("release-repo");
    let repo = repo_dir.path();
    let binary = release.join("bin").join("code-intel");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(release.join("pipeline.config.json"), b"{}").unwrap();
    fs::write(&binary, b"packaged binary").unwrap();
    let binary_path = binary.to_string_lossy().into_owned();

    let output = common::cli()
        .args(["doctor", "bootstrap", "--pipeline-root"])
        .arg(&release)
        .args([
            "--repo-path",
            repo.to_str().unwrap(),
            "--platform",
            "macos",
            "--no-require-repowise",
            "--json",
        ])
        .output()
        .expect("run doctor bootstrap against release layout");
    let observation: Value =
        serde_json::from_slice(&output.stdout).expect("observation is one JSON document");

    assert_eq!(
        observation["checks"]["graphProvider"]["binaryFound"],
        json!(true),
        "release layout must discover its packaged binary: {observation}"
    );
    assert_eq!(
        observation["checks"]["graphProvider"]["binaryPath"],
        json!(binary_path)
    );
}

/// Regression (F4): `orchestration/integrations.json` ships
/// `edit.ast-grep-plan` with a `runtimeAdapter`, and that adapter resolves
/// `ast-grep` through `tool_path` before shelling out to it. The doctor's
/// probe list was hardcoded without it, so a machine with no ast-grep got
/// `ok: true, missing: []` and only learned the truth at capability-exec
/// time as `Unavailable("start ast-grep: ...")`.
///
/// The manifest read is the point: this asserts the probe list against the
/// capability that actually needs the tool, so retiring `edit.ast-grep-plan`
/// relaxes the test instead of leaving a restated constant behind.
#[test]
fn the_probe_list_covers_the_external_tool_a_shipped_capability_shells_out_to() {
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(pipeline_root().join("orchestration/integrations.json"))
            .expect("read integrations manifest"),
    )
    .expect("integrations manifest is JSON");
    let ships_ast_grep_plan = manifest["integrations"]
        .as_array()
        .expect("integrations array")
        .iter()
        .any(|entry| entry["id"] == "edit.ast-grep-plan" && entry["runtimeAdapter"].is_string());
    assert!(
        ships_ast_grep_plan,
        "edit.ast-grep-plan is no longer a shipped runtime capability; drop this test rather than weakening it"
    );

    let repo_dir = TempDir::new("ast-grep");
    let repo = repo_dir.path();
    let (_, observation, _) = doctor(&[
        "--repo-path",
        repo.to_str().unwrap(),
        "--no-require-repowise",
        "--json",
    ]);
    let probed = observation["checks"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .find(|tool| tool["name"] == "ast-grep")
        .unwrap_or_else(|| {
            panic!(
                "doctor bootstrap never probes ast-grep: {}",
                observation["checks"]["tools"]
            )
        })
        .clone();
    // Optional, matching the `required: false` that
    // `orchestration/toolchain-versions.v1.json` already declares for it — the
    // fix is that the doctor reports on the tool, not that it starts failing.
    assert_eq!(probed["required"], json!(false));
    assert!(probed["found"].is_boolean());
}

#[test]
fn an_unknown_flag_fails_closed_without_emitting_an_observation() {
    let (code, observation, stderr) = doctor(&["--not-a-flag"]);
    assert_eq!(code, 65);
    assert_eq!(observation, Value::Null);
    assert!(stderr.contains("unknown argument for doctor bootstrap"));
}
