use super::*;
use std::fs;

#[test]
fn weco_byok_is_unconfigured_when_no_recognized_key_is_present() {
    assert!(!weco_byok_configured_from(|_| false));
}

#[test]
fn weco_byok_is_configured_when_any_recognized_provider_key_is_present() {
    for name in WECO_BYOK_ENV_VARS {
        assert!(weco_byok_configured_from(|candidate| candidate == *name));
    }
}

#[test]
fn weco_account_is_unconfigured_when_neither_env_nor_credentials_file_is_present() {
    assert!(!weco_account_configured_from(false, false));
}

#[test]
fn weco_account_is_configured_by_either_the_env_var_or_the_credentials_file() {
    assert!(weco_account_configured_from(true, false));
    assert!(weco_account_configured_from(false, true));
    assert!(weco_account_configured_from(true, true));
}

#[test]
fn weco_reason_distinguishes_not_installed_from_each_missing_auth_gate() {
    assert_eq!(weco_reason(false, false, false), "weco not found on PATH");
    assert_eq!(weco_reason(false, true, true), "weco not found on PATH");
    assert_eq!(
        weco_reason(true, false, false),
        "weco installed but neither an LLM provider key (BYOK) nor a weco.ai account (WECO_API_KEY) is configured"
    );
    assert_eq!(
        weco_reason(true, false, true),
        "weco installed but no LLM provider key configured (BYOK)"
    );
    assert_eq!(
        weco_reason(true, true, false),
        "weco installed but no weco.ai account configured (WECO_API_KEY) -- weco's run loop is server-tracked and requires this even with your own LLM key"
    );
    assert_eq!(weco_reason(true, true, true), "");
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "code-intel-doctor-bootstrap-{}-{tag}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn home(set: bool, exists: bool) -> CodeIntelHome {
    CodeIntelHome {
        value: "C:/nope".into(),
        resolved: "C:/nope".into(),
        set,
        exists,
        matches_default: false,
        expected: "C:/expected".into(),
    }
}

fn strict(require_understand: bool) -> Options {
    let mut options = Options::new(PathBuf::from("."));
    options.require_understand = require_understand;
    options
}

#[test]
fn missing_list_preserves_the_retired_scripts_wording_and_order() {
    let checks = json!({
        "pipelineScript": {"found": false},
        "config": {"found": true, "parsed": false, "parseError": "bad json"},
        "sentrux": {"core": {"found": false}, "pro": {"found": false}},
        "graphProvider": {"sourceFound": false, "cargoFound": false},
        "repo": {"path": "x", "exists": false}
    });
    let tools = vec![
        json!({"name": "rg", "required": true, "found": false}),
        json!({"name": "repomix", "required": false, "found": false}),
    ];
    assert_eq!(
        missing_list(&checks, &tools, false, &strict(true), &home(true, false)),
        vec![
            "pipeline script".to_string(),
            "pipeline config: invalid JSON (bad json)".to_string(),
            "rg".to_string(),
            "sentrux core".to_string(),
            "sentrux pro auto-activation".to_string(),
            "internal graph provider source".to_string(),
            "code-intel Rust runtime".to_string(),
            "Understand Anything skill or plugin".to_string(),
            "repo path".to_string(),
            "CODE_INTEL_HOME: directory does not exist (C:/nope)".to_string(),
        ]
    );
}

#[test]
fn builtin_sentrux_makes_the_external_overlay_optional() {
    let checks = json!({
        "pipelineScript": {"found": true},
        "config": {"found": true, "parsed": true},
        "sentrux": {"core": {"found": false}, "pro": {"found": false}},
        "graphProvider": {"sourceFound": true, "cargoFound": true},
        "repo": {"exists": true}
    });
    let tools = vec![json!({"name": "sentrux", "required": false, "found": false})];
    assert!(missing_list(&checks, &tools, true, &strict(false), &home(false, false)).is_empty());
}

#[test]
fn observation_carries_the_v1_contract_and_every_retired_check() {
    let root = scratch("contract");
    let mut options = Options::new(root.clone());
    options.repo_path = Some(display(&root));
    let observation = observe(&options).unwrap();
    assert_eq!(observation["schema"], BOOTSTRAP_SCHEMA);
    assert_eq!(observation["authority"], "observation_only");
    assert!(observation["ok"].is_boolean());
    // `serde_json::Map` is a `BTreeMap` here, so the key order is stable.
    // A silently dropped check has to surface as a test failure rather
    // than as a missing field downstream.
    let checks = observation["checks"]
        .as_object()
        .expect("checks object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        checks,
        vec![
            "assistancePlugins".to_string(),
            "config".to_string(),
            "env".to_string(),
            "graphProvider".to_string(),
            "pipelineScript".to_string(),
            "repo".to_string(),
            "sentrux".to_string(),
            "tools".to_string(),
            "understandAnything".to_string(),
            "weco".to_string(),
        ]
    );
    let names = observation["checks"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["rg", "git", "python", "repowise", "repomix", "sentrux", "ast-grep", "weco"]
    );
    assert!(observation["checks"]["weco"]["byokConfigured"].is_boolean());
    assert!(observation["checks"]["weco"]["accountConfigured"].is_boolean());
    assert!(observation["checks"]["weco"]["reason"].is_string());
    fs::remove_dir_all(root).ok();
}

#[test]
fn a_missing_repo_path_is_a_domain_observation_not_an_error() {
    let root = scratch("absent");
    let mut options = Options::new(root.clone());
    options.repo_path = Some(display(&root.join("does-not-exist")));
    let observation = observe(&options).unwrap();
    assert_eq!(observation["checks"]["repo"]["exists"], json!(false));
    assert_eq!(observation["ok"], json!(false));
    assert!(observation["missing"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "repo path"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn configured_sentrux_path_resolves_the_scope_and_finds_scoped_rules() {
    let root = scratch("scope");
    let repo = root.join("ConfiguredRepo");
    let sentrux = repo.join("backend").join(".sentrux");
    fs::create_dir_all(&sentrux).unwrap();
    fs::write(sentrux.join("rules.toml"), b"").unwrap();
    fs::write(sentrux.join("baseline.json"), b"{}").unwrap();
    let config_path = root.join("pipeline.config.json");
    fs::write(
        &config_path,
        serde_json::to_vec(&json!({"repos": {"fixture": {
            "path": format!("{}{}", display(&repo), std::path::MAIN_SEPARATOR),
            "sentruxPath": "backend"
        }}}))
        .unwrap(),
    )
    .unwrap();

    let mut options = Options::new(root.clone());
    options.config = Some(config_path);
    // A `.` segment in the argument, exactly the shape the retired
    // PowerShell contract test exercised.
    options.repo_path = Some(display(&repo.join(".")));
    let observation = observe(&options).unwrap();

    let expected = display(&paths::resolve_code_intel_path(&repo.join("backend")));
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
fn human_rendering_keeps_the_retired_scripts_first_line() {
    let ok = json!({"ok": true, "missing": [], "checks": {}});
    assert!(render_human(&ok).starts_with("Code intel doctor: OK"));
    let bad = json!({"ok": false, "missing": ["rg", "git"], "checks": {}});
    assert!(render_human(&bad).starts_with("Code intel doctor: missing rg, git"));
}

#[test]
fn binary_candidates_prefer_packaged_release_over_checkout_builds() {
    let candidates = binary_candidates(Path::new("root"), "windows");
    assert!(candidates[0].ends_with(Path::new("bin/code-intel.exe")));
    assert!(candidates[1].ends_with(Path::new("target/release/code-intel.exe")));
    assert!(candidates[2].ends_with(Path::new("target/debug/code-intel.exe")));
}
