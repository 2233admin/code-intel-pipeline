//! Native bootstrap/environment probe — the Rust owner of what
//! `archive/check-code-intel-tools.ps1` used to compute in PowerShell.
//!
//! Emits `code-intel-doctor-bootstrap-observation.v1`, the same
//! non-authoritative observation the doctor capability adapter consumes. The
//! PowerShell entry point is now a thin forwarder onto this module, so there
//! is exactly one implementation of the probe instead of a script plus a
//! divergent in-process fallback that hardcoded its graph-provider answers.
//!
//! Everything here is observation only: it reports presence and readiness of
//! tools, providers, config and repository state. It never writes, never
//! claims admissibility, and never emits engineering facts — those boundaries
//! belong to `doctor_adapter`.
//!
//! This file assembles the envelope; the three submodules own the concerns it
//! composes — `config` resolves the pipeline config and repository, `paths`
//! derives platform locations, `probe` observes tools and command output.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

mod config;
mod paths;
mod probe;

use paths::display;

/// Marker the doctor capability adapter matches on. Kept as a constant so the
/// probe and the adapter's contract check cannot drift.
pub(crate) const BOOTSTRAP_SCHEMA: &str = "code-intel-doctor-bootstrap-observation.v1";

pub(crate) struct Options {
    /// Repo alias resolved through `pipeline.config.json`'s `repos` map.
    pub(crate) repo: Option<String>,
    /// Explicit repository path; takes precedence over `repo`.
    pub(crate) repo_path: Option<String>,
    /// Pipeline config path; defaults to `<pipeline root>/pipeline.config.json`.
    pub(crate) config: Option<PathBuf>,
    /// `auto` | `windows` | `macos` | `linux`.
    pub(crate) platform: String,
    pub(crate) require_repowise: bool,
    pub(crate) require_understand: bool,
    /// Directory searched ahead of `PATH` when probing for tools. Lets a test
    /// stand up a fixture toolchain without mutating the process environment.
    pub(crate) tool_path_prefix: Option<PathBuf>,
    /// Repository root holding `crates/`, `target/` and `archive/`.
    pub(crate) pipeline_root: PathBuf,
}

impl Options {
    pub(crate) fn new(pipeline_root: PathBuf) -> Self {
        Self {
            repo: None,
            repo_path: None,
            config: None,
            platform: "auto".into(),
            require_repowise: true,
            require_understand: false,
            tool_path_prefix: None,
            pipeline_root,
        }
    }
}

/// Run the probe and return the observation document.
pub(crate) fn observe(options: &Options) -> Result<Value, String> {
    let platform = paths::resolve_platform(&options.platform)?;
    let prefix = options.tool_path_prefix.as_deref();

    let config_path = match &options.config {
        Some(path) => path.clone(),
        None => options.pipeline_root.join("pipeline.config.json"),
    };
    let (config_data, config_parse_error) = config::load_config(&config_path);

    let repo_path = config::resolve_repo_path(options, config_data.as_ref());
    let repo_config = match (&options.repo_path, &repo_path) {
        // An explicit --repo-path wins over the alias, so the config entry has
        // to be found by reverse path lookup rather than by name.
        (Some(_), Some(path)) => config::find_repo_config_by_path(config_data.as_ref(), path),
        _ => options
            .repo
            .as_deref()
            .and_then(|alias| config::repo_config_by_alias(config_data.as_ref(), alias)),
    };
    let sentrux_scope = repo_path
        .as_ref()
        .map(|path| config::resolve_sentrux_scope(path, repo_config.as_ref()));

    let pipeline_script = options
        .pipeline_root
        .join("archive")
        .join("run-code-intel.ps1");
    let cli_root = options.pipeline_root.join("crates").join("code-intel-cli");
    let graph_source = cli_root.join("src").join("graph.rs");
    let graph_cargo = cli_root.join("Cargo.toml");
    let binary_candidates = binary_candidates(&options.pipeline_root, &platform);
    let graph_binary = binary_candidates
        .iter()
        .find(|path| path.is_file())
        .cloned();
    let graph_command_binary = graph_binary
        .clone()
        .unwrap_or_else(|| binary_candidates[0].clone());

    // The structural gate engine ships inside the code-intel binary; an
    // external sentrux on PATH is an optional overlay, not a bootstrap
    // requirement.
    let builtin_sentrux = probe::locate("code-intel", prefix).is_some()
        || built_binaries(&options.pipeline_root).any(|path| path.is_file());

    let tools = vec![
        probe::probe_tool("rg", true, prefix),
        probe::probe_tool("git", true, prefix),
        probe::probe_python(prefix),
        probe::probe_tool("repowise", options.require_repowise, prefix),
        probe::probe_tool("repomix", false, prefix),
        probe::probe_tool("sentrux", !builtin_sentrux, prefix),
    ];

    let sentrux_core = probe::probe_command_output(
        "sentrux-core",
        "sentrux",
        &["check", "--help"],
        prefix,
        |text| probe::contains_ignore_case(text, probe::SENTRUX_CORE_MARKER),
    );
    // Tier: free is healthy without the SENTRUX_AUTO_PRO opt-in (Pro
    // auto-activation is opt-in; see archive/tools/sentrux-shim/sentrux-shim.ps1).
    let require_pro_tier = probe::requires_pro_tier();
    let sentrux_pro = probe::probe_command_output(
        "sentrux-pro",
        "sentrux",
        &["pro", "status"],
        prefix,
        |text| probe::matches_tier(text, require_pro_tier),
    );

    let home_dir = paths::home_directory();
    let understand_skill = [".claude", ".agents", ".codex"]
        .iter()
        .map(|agent| {
            home_dir
                .join(agent)
                .join("skills")
                .join("understand")
                .join("SKILL.md")
        })
        .find(|path| path.is_file());
    let repo_parent = repo_path
        .as_ref()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| options.pipeline_root.clone());
    let understand_plugin = [
        home_dir
            .join(".claude")
            .join("plugins")
            .join("cache")
            .join("understand-anything"),
        home_dir.join(".understand-anything-plugin"),
        repo_parent.join("Understand-Anything"),
    ]
    .into_iter()
    .find(|path| path.is_dir());

    let repo_state = repo_state(repo_path.as_deref(), sentrux_scope.as_deref());
    let home = code_intel_home(&options.pipeline_root);

    let checks = json!({
        "pipelineScript": {
            "path": display(&pipeline_script),
            "found": pipeline_script.is_file()
        },
        "config": {
            "path": display(&config_path),
            "found": config_path.is_file(),
            "parsed": config_data.is_some() || config_parse_error.is_none(),
            "parseError": config_parse_error.clone().unwrap_or_default()
        },
        "tools": tools,
        "sentrux": {
            "core": sentrux_core,
            "pro": sentrux_pro,
            "builtin": {"found": builtin_sentrux}
        },
        "understandAnything": {
            "skillFound": understand_skill.is_some(),
            "skillPath": understand_skill.as_deref().map(display).unwrap_or_default(),
            "pluginFound": understand_plugin.is_some(),
            "pluginPath": understand_plugin.as_deref().map(display).unwrap_or_default()
        },
        "graphProvider": {
            "sourceFound": graph_source.is_file(),
            "cargoFound": graph_cargo.is_file(),
            "binaryFound": graph_binary.is_some(),
            "binaryPath": graph_binary.as_deref().map(display).unwrap_or_default(),
            "command": format!(
                "{} graph --repo <repo-path> --language zh --write --json",
                display(&graph_command_binary)
            )
        },
        "repo": repo_state,
        "env": {"codeIntelHome": home.observation()}
    });

    let missing = missing_list(&checks, &tools, builtin_sentrux, options, &home);
    Ok(json!({
        "schema": BOOTSTRAP_SCHEMA,
        "authority": "observation_only",
        "source": "native",
        "ok": missing.is_empty(),
        "missing": missing,
        "platform": {"os": platform, "shell": "Rust", "psVersion": ""},
        "paths": paths::platform_paths(&platform, &options.pipeline_root),
        "checks": checks,
        "strict": {
            "requireRepowise": options.require_repowise,
            "requireUnderstand": options.require_understand
        }
    }))
}

/// `target/release` then `target/debug`, with the platform-correct name — the
/// order the retired script probed and the order `binaryPath` reports.
fn binary_candidates(pipeline_root: &Path, platform: &str) -> [PathBuf; 2] {
    let name = paths::binary_name(platform);
    [
        pipeline_root.join("target").join("release").join(&name),
        pipeline_root.join("target").join("debug").join(&name),
    ]
}

/// Both platform spellings under both profiles: the built-in engine check must
/// not depend on which platform's binary name this process was built for.
fn built_binaries(pipeline_root: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    ["release", "debug"].into_iter().flat_map(move |profile| {
        ["code-intel.exe", "code-intel"]
            .into_iter()
            .map(move |name| pipeline_root.join("target").join(profile).join(name))
    })
}

/// `CODE_INTEL_HOME` compared against the default derivation — the pipeline
/// root — rather than against its own env-derived value, which would have
/// matched any set value including a deleted directory.
struct CodeIntelHome {
    value: String,
    resolved: String,
    set: bool,
    exists: bool,
    matches_default: bool,
    expected: String,
}

impl CodeIntelHome {
    fn observation(&self) -> Value {
        json!({
            "expected": self.expected,
            "value": self.value,
            "resolved": self.resolved,
            "exists": self.exists,
            "matchesDefault": self.matches_default,
            "ok": self.exists && self.matches_default
        })
    }
}

fn code_intel_home(pipeline_root: &Path) -> CodeIntelHome {
    let expected = display(&paths::resolve_code_intel_path(pipeline_root));
    let value = std::env::var("CODE_INTEL_HOME").unwrap_or_default();
    let set = !value.trim().is_empty();
    let resolved = if set {
        display(&paths::resolve_code_intel_path(Path::new(&value)))
    } else {
        String::new()
    };
    let exists = set && Path::new(&resolved).is_dir();
    let matches_default = set && resolved == expected;
    CodeIntelHome {
        value: if set { value } else { String::new() },
        resolved,
        set,
        exists,
        matches_default,
        expected,
    }
}

/// The `missing` list, in the order the PowerShell probe emitted it — several
/// callers (installer checks, CI logs) read it as a comma-joined string.
fn missing_list(
    checks: &Value,
    tools: &[Value],
    builtin_sentrux: bool,
    options: &Options,
    home: &CodeIntelHome,
) -> Vec<String> {
    let flag = |pointer: &str| {
        checks
            .pointer(pointer)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let mut missing = Vec::new();
    if !flag("/pipelineScript/found") {
        missing.push("pipeline script".to_string());
    }
    if !flag("/config/found") {
        missing.push("pipeline config".to_string());
    }
    if flag("/config/found") && !flag("/config/parsed") {
        let detail = checks
            .pointer("/config/parseError")
            .and_then(Value::as_str)
            .unwrap_or_default();
        missing.push(format!("pipeline config: invalid JSON ({detail})"));
    }
    for tool in tools {
        if tool["required"].as_bool().unwrap_or(false) && !tool["found"].as_bool().unwrap_or(false)
        {
            missing.push(tool["name"].as_str().unwrap_or_default().to_string());
        }
    }
    if !flag("/sentrux/core/found") && !builtin_sentrux {
        missing.push("sentrux core".to_string());
    }
    if !flag("/sentrux/pro/found") && !builtin_sentrux {
        missing.push("sentrux pro auto-activation".to_string());
    }
    if options.require_understand && !flag("/graphProvider/sourceFound") {
        missing.push("internal graph provider source".to_string());
    }
    if options.require_understand && !flag("/graphProvider/cargoFound") {
        missing.push("code-intel Rust runtime".to_string());
    }
    if checks["repo"].is_object() && !flag("/repo/exists") {
        missing.push("repo path".to_string());
    }
    if home.set && !home.exists {
        missing.push(format!(
            "CODE_INTEL_HOME: directory does not exist ({})",
            home.resolved
        ));
    }
    missing
}

fn repo_state(repo_path: Option<&Path>, sentrux_scope: Option<&Path>) -> Value {
    let Some(repo_path) = repo_path else {
        return Value::Null;
    };
    if !repo_path.is_dir() {
        return json!({"path": display(repo_path), "exists": false});
    }
    let scope = sentrux_scope.unwrap_or(repo_path);
    let sentrux_dir = scope.join(".sentrux");
    json!({
        "path": display(repo_path),
        "exists": true,
        "isGitRepo": repo_path.join(".git").exists(),
        "understandGraph": repo_path
            .join(".understand-anything")
            .join("knowledge-graph.json")
            .is_file(),
        "repowiseState": repo_path.join(".repowise").is_dir(),
        "sentruxScope": display(scope),
        "sentruxRules": sentrux_dir.join("rules.toml").is_file(),
        "sentruxBaseline": sentrux_dir.join("baseline.json").is_file()
    })
}

/// Repository root: the directory holding `orchestration/`, discovered the
/// same way the capability layer discovers its manifest.
pub(crate) fn pipeline_root() -> PathBuf {
    crate::capability::discover_manifest(None)
        .and_then(|manifest| manifest.parent()?.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."))
}

/// The pipeline checkout a bare `code-intel doctor bootstrap` should inspect.
///
/// Deliberately NOT `pipeline_root()`: that walks up from the executable, and
/// the installer copies `orchestration/integrations.json` next to the
/// installed binary, so an installed `code-intel` would resolve its own bin
/// directory as the pipeline and report every repository-side check missing.
/// The retired script derived the root from its own location inside the
/// checkout; the closest CLI analogue is the checkout the caller is standing
/// in, so walk up from the working directory first and only then fall back to
/// manifest discovery. `--pipeline-root` overrides both.
fn default_pipeline_root() -> PathBuf {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| {
            cwd.ancestors()
                .find(|dir| {
                    dir.join("orchestration")
                        .join("integrations.json")
                        .is_file()
                })
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(pipeline_root)
}

/// The human-readable rendering the PowerShell probe printed without `-Json`.
/// CI reads these lines, so the wording is preserved verbatim.
pub(crate) fn render_human(observation: &Value) -> String {
    let text = |pointer: &str| {
        observation
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let flag = |pointer: &str| {
        observation
            .pointer(pointer)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let mark = |ok: bool| if ok { "OK" } else { "MISSING" };

    let mut lines = vec![headline(observation)];
    lines.push(format!("Pipeline: {}", text("/checks/pipelineScript/path")));
    lines.push(format!("Config: {}", text("/checks/config/path")));
    if let Some(tools) = observation
        .pointer("/checks/tools")
        .and_then(Value::as_array)
    {
        for tool in tools {
            lines.push(format!(
                "{} {} {}",
                mark(tool["found"].as_bool().unwrap_or(false)),
                tool["name"].as_str().unwrap_or_default(),
                tool["source"].as_str().unwrap_or_default()
            ));
        }
    }
    let builtin = flag("/checks/sentrux/builtin/found");
    lines.push(format!(
        "{} sentrux-core {}",
        mark(flag("/checks/sentrux/core/found") || builtin),
        text("/checks/sentrux/core/output")
    ));
    lines.push(format!(
        "{} sentrux-pro {}",
        mark(flag("/checks/sentrux/pro/found") || builtin),
        text("/checks/sentrux/pro/output")
    ));
    lines.push(format!(
        "{} internal graph provider source={} cargo={} binary={}",
        mark(flag("/checks/graphProvider/sourceFound") && flag("/checks/graphProvider/cargoFound")),
        flag("/checks/graphProvider/sourceFound"),
        flag("/checks/graphProvider/cargoFound"),
        flag("/checks/graphProvider/binaryFound")
    ));
    lines.push(format!(
        "{} external Understand fallback skill={} plugin={}",
        mark(
            flag("/checks/understandAnything/skillFound")
                && flag("/checks/understandAnything/pluginFound")
        ),
        text("/checks/understandAnything/skillPath"),
        text("/checks/understandAnything/pluginPath")
    ));
    lines.extend(repo_lines(observation, &text, &flag));
    lines.join("\n")
}

fn headline(observation: &Value) -> String {
    if observation["ok"].as_bool().unwrap_or(false) {
        return "Code intel doctor: OK".to_string();
    }
    let missing = observation["missing"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    format!("Code intel doctor: missing {missing}")
}

fn repo_lines(
    observation: &Value,
    text: &impl Fn(&str) -> String,
    flag: &impl Fn(&str) -> bool,
) -> Vec<String> {
    if !observation["checks"]["repo"].is_object() {
        return Vec::new();
    }
    let mut lines = vec![
        format!("Repo: {}", text("/checks/repo/path")),
        format!("Repo exists: {}", flag("/checks/repo/exists")),
    ];
    if flag("/checks/repo/exists") {
        lines.push(format!(
            "Understand graph: {}",
            flag("/checks/repo/understandGraph")
        ));
        lines.push(format!(
            "Repowise state: {}",
            flag("/checks/repo/repowiseState")
        ));
        lines.push(format!(
            "Sentrux scope: {}",
            text("/checks/repo/sentruxScope")
        ));
        lines.push(format!(
            "Sentrux rules: {}",
            flag("/checks/repo/sentruxRules")
        ));
        lines.push(format!(
            "Sentrux baseline: {}",
            flag("/checks/repo/sentruxBaseline")
        ));
    }
    lines
}

/// `code-intel doctor bootstrap [...]` — the direct CLI surface that replaced
/// `archive/check-code-intel-tools.ps1`. Exits 1 when the probe reports
/// missing prerequisites, matching the script it retired.
pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let mut options = Options::new(default_pipeline_root());
    let mut json_output = false;
    let mut index = 0;
    while index < raw.len() {
        let token = raw[index].as_str();
        // A following token that itself looks like a flag is not a value, so
        // `--repo --json` fails closed instead of consuming `--json`.
        let value = raw.get(index + 1).filter(|value| !value.starts_with("--"));
        index += match (token, value) {
            ("--json", _) => {
                json_output = true;
                1
            }
            ("--require-repowise", _) => {
                options.require_repowise = true;
                1
            }
            ("--no-require-repowise", _) => {
                options.require_repowise = false;
                1
            }
            ("--require-understand", _) => {
                options.require_understand = true;
                1
            }
            ("--repo", Some(value)) => {
                options.repo = Some(value.clone());
                2
            }
            ("--repo-path", Some(value)) => {
                options.repo_path = Some(value.clone());
                2
            }
            ("--config", Some(value)) => {
                options.config = Some(PathBuf::from(value));
                2
            }
            ("--platform", Some(value)) => {
                options.platform = value.clone();
                2
            }
            ("--pipeline-root", Some(value)) => {
                options.pipeline_root = PathBuf::from(value);
                2
            }
            ("--repo" | "--repo-path" | "--config" | "--platform" | "--pipeline-root", None) => {
                return fail(&format!("{token} requires a value"))
            }
            (other, _) => return fail(&format!("unknown argument for doctor bootstrap: {other}")),
        };
    }

    let observation = match observe(&options) {
        Ok(observation) => observation,
        Err(error) => return fail(&error),
    };
    let rendered = if json_output {
        match serde_json::to_string_pretty(&observation) {
            Ok(text) => text,
            Err(error) => return fail(&format!("serialize doctor observation: {error}")),
        }
    } else {
        render_human(&observation)
    };
    println!("{rendered}");
    i32::from(!observation["ok"].as_bool().unwrap_or(false))
}

fn fail(message: &str) -> i32 {
    eprintln!("error: {message}");
    65
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
        assert!(
            missing_list(&checks, &tools, true, &strict(false), &home(false, false)).is_empty()
        );
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
                "config".to_string(),
                "env".to_string(),
                "graphProvider".to_string(),
                "pipelineScript".to_string(),
                "repo".to_string(),
                "sentrux".to_string(),
                "tools".to_string(),
                "understandAnything".to_string(),
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
            vec!["rg", "git", "python", "repowise", "repomix", "sentrux"]
        );
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
    fn binary_candidates_prefer_release_over_debug() {
        let candidates = binary_candidates(Path::new("root"), "windows");
        assert!(candidates[0].ends_with(Path::new("target/release/code-intel.exe")));
        assert!(candidates[1].ends_with(Path::new("target/debug/code-intel.exe")));
    }
}
