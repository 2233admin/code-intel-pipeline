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

use std::env;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

#[path = "tool_path.rs"]
mod tool_path;

/// Marker the doctor capability adapter matches on. Kept as a constant so the
/// probe and the adapter's contract check cannot drift.
pub(crate) const BOOTSTRAP_SCHEMA: &str = "code-intel-doctor-bootstrap-observation.v1";

/// Substring `sentrux check --help` must print for the core overlay to count
/// as conforming. PowerShell's `-match` is case-insensitive, so this compares
/// case-insensitively too.
const SENTRUX_CORE_MARKER: &str = "Enforce architectural rules";

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
    let platform = resolve_platform(&options.platform)?;
    let prefix = options.tool_path_prefix.as_deref();

    let config_path = match &options.config {
        Some(path) => path.clone(),
        None => options.pipeline_root.join("pipeline.config.json"),
    };
    let (config_data, config_parse_error) = load_config(&config_path);

    let repo_path = resolve_repo_path(options, config_data.as_ref());
    let repo_config = match (&options.repo_path, &repo_path) {
        // An explicit -RepoPath wins over the alias, so the config entry has
        // to be found by reverse path lookup rather than by name.
        (Some(_), Some(path)) => find_repo_config_by_path(config_data.as_ref(), path),
        _ => options
            .repo
            .as_deref()
            .and_then(|alias| repo_config_by_alias(config_data.as_ref(), alias)),
    };
    let sentrux_scope = repo_path
        .as_ref()
        .map(|path| resolve_sentrux_scope(path, repo_config.as_ref()));

    let pipeline_script = options
        .pipeline_root
        .join("archive")
        .join("run-code-intel.ps1");
    let cli_root = options.pipeline_root.join("crates").join("code-intel-cli");
    let graph_source = cli_root.join("src").join("graph.rs");
    let graph_cargo = cli_root.join("Cargo.toml");
    let binary_name = binary_name(&platform);
    let binary_candidates = [
        options
            .pipeline_root
            .join("target")
            .join("release")
            .join(&binary_name),
        options
            .pipeline_root
            .join("target")
            .join("debug")
            .join(&binary_name),
    ];
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
    let builtin_sentrux = tool_path::locate("code-intel", prefix).is_some()
        || ["release", "debug"].iter().any(|profile| {
            ["code-intel.exe", "code-intel"].iter().any(|name| {
                options
                    .pipeline_root
                    .join("target")
                    .join(profile)
                    .join(name)
                    .is_file()
            })
        });

    let tools = vec![
        probe_tool("rg", true, prefix),
        probe_tool("git", true, prefix),
        probe_python(prefix),
        probe_tool("repowise", options.require_repowise, prefix),
        probe_tool("repomix", false, prefix),
        probe_tool("sentrux", !builtin_sentrux, prefix),
    ];

    let sentrux_core = probe_command_output(
        "sentrux-core",
        "sentrux",
        &["check", "--help"],
        prefix,
        |text| contains_ignore_case(text, SENTRUX_CORE_MARKER),
    );
    // Tier: free is healthy without the SENTRUX_AUTO_PRO opt-in (Pro
    // auto-activation is opt-in; see archive/tools/sentrux-shim/sentrux-shim.ps1).
    let require_pro_tier = matches!(
        env::var("SENTRUX_AUTO_PRO").unwrap_or_default().as_str(),
        "1" | "true" | "True" | "TRUE"
    );
    let sentrux_pro = probe_command_output(
        "sentrux-pro",
        "sentrux",
        &["pro", "status"],
        prefix,
        |text| matches_tier(text, require_pro_tier),
    );

    let home_dir = home_directory();
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

    let code_intel_home_default = resolve_code_intel_path(&options.pipeline_root);
    let code_intel_home_value = env::var("CODE_INTEL_HOME").unwrap_or_default();
    let code_intel_home_set = !code_intel_home_value.trim().is_empty();
    let code_intel_home_resolved = if code_intel_home_set {
        display(&resolve_code_intel_path(Path::new(&code_intel_home_value)))
    } else {
        String::new()
    };
    let code_intel_home_exists =
        code_intel_home_set && Path::new(&code_intel_home_resolved).is_dir();
    let code_intel_home_matches_default =
        code_intel_home_set && code_intel_home_resolved == display(&code_intel_home_default);

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
        "env": {
            "codeIntelHome": {
                "expected": display(&code_intel_home_default),
                "value": if code_intel_home_set { code_intel_home_value.clone() } else { String::new() },
                "resolved": code_intel_home_resolved.clone(),
                "exists": code_intel_home_exists,
                "matchesDefault": code_intel_home_matches_default,
                "ok": code_intel_home_exists && code_intel_home_matches_default
            }
        }
    });

    let missing = missing_list(
        &checks,
        &tools,
        builtin_sentrux,
        options.require_understand,
        config_parse_error.as_deref(),
        code_intel_home_set,
        code_intel_home_exists,
        &code_intel_home_resolved,
    );

    let paths = platform_paths(&platform, &options.pipeline_root);
    Ok(json!({
        "schema": BOOTSTRAP_SCHEMA,
        "authority": "observation_only",
        "source": "native",
        "ok": missing.is_empty(),
        "missing": missing,
        "platform": {
            "os": platform,
            "shell": "Rust",
            "psVersion": ""
        },
        "paths": paths,
        "checks": checks,
        "strict": {
            "requireRepowise": options.require_repowise,
            "requireUnderstand": options.require_understand
        }
    }))
}

/// The `missing` list, in the order the PowerShell probe emitted it — several
/// callers (installer checks, CI logs) read it as a comma-joined string.
#[allow(clippy::too_many_arguments)]
fn missing_list(
    checks: &Value,
    tools: &[Value],
    builtin_sentrux: bool,
    require_understand: bool,
    config_parse_error: Option<&str>,
    code_intel_home_set: bool,
    code_intel_home_exists: bool,
    code_intel_home_resolved: &str,
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
        missing.push(format!(
            "pipeline config: invalid JSON ({})",
            config_parse_error.unwrap_or_default()
        ));
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
    if require_understand && !flag("/graphProvider/sourceFound") {
        missing.push("internal graph provider source".to_string());
    }
    if require_understand && !flag("/graphProvider/cargoFound") {
        missing.push("code-intel Rust runtime".to_string());
    }
    if checks["repo"].is_object() && !flag("/repo/exists") {
        missing.push("repo path".to_string());
    }
    if code_intel_home_set && !code_intel_home_exists {
        missing.push(format!(
            "CODE_INTEL_HOME: directory does not exist ({code_intel_home_resolved})"
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

fn load_config(path: &Path) -> (Option<Value>, Option<String>) {
    if !path.is_file() {
        return (None, None);
    }
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error.to_string())),
        },
        Err(error) => (None, Some(error.to_string())),
    }
}

fn repo_config_by_alias<'a>(config: Option<&'a Value>, alias: &str) -> Option<&'a Value> {
    config?.get("repos")?.get(alias)
}

/// Reverse lookup: which configured repo entry points at `repo_path`. Mirrors
/// the PowerShell `Find-RepoConfigByPath`, including its trailing-separator
/// trim and case-insensitive comparison.
fn find_repo_config_by_path<'a>(config: Option<&'a Value>, repo_path: &Path) -> Option<&'a Value> {
    let repos = config?.get("repos")?.as_object()?;
    let target = trim_trailing_separator(&display(repo_path));
    repos.values().find(|entry| {
        entry
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .is_some_and(|path| {
                let resolved = display(&resolve_code_intel_path(Path::new(path)));
                trim_trailing_separator(&resolved).eq_ignore_ascii_case(&target)
            })
    })
}

fn resolve_repo_path(options: &Options, config: Option<&Value>) -> Option<PathBuf> {
    if let Some(repo_path) = options
        .repo_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let path = PathBuf::from(repo_path);
        return Some(if path.is_dir() {
            resolve_code_intel_path(&path)
        } else {
            path
        });
    }
    let alias = options
        .repo
        .as_deref()
        .filter(|value| !value.trim().is_empty())?;
    let configured = repo_config_by_alias(config, alias)
        .and_then(|entry| entry.get("path"))
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty());
    let path = PathBuf::from(configured.unwrap_or(alias));
    Some(if path.is_dir() {
        resolve_code_intel_path(&path)
    } else {
        path
    })
}

fn resolve_sentrux_scope(repo_path: &Path, repo_config: Option<&&Value>) -> PathBuf {
    let configured = repo_config
        .and_then(|entry| entry.get("sentruxPath"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let Some(configured) = configured else {
        return repo_path.to_path_buf();
    };
    let scope = if Path::new(configured).is_absolute() {
        PathBuf::from(configured)
    } else {
        repo_path.join(configured)
    };
    if scope.is_dir() {
        resolve_code_intel_path(&scope)
    } else {
        scope
    }
}

fn probe_tool(name: &str, required: bool, prefix: Option<&Path>) -> Value {
    let found = tool_path::locate(name, prefix);
    json!({
        "name": name,
        "required": required,
        "found": found.is_some(),
        "source": found.as_deref().map(display).unwrap_or_default()
    })
}

/// `python` falls back to `python3`, matching `Get-CodeIntelPythonCommand`.
/// The reported `name` stays `python` so the `missing` list wording does not
/// change with which interpreter happened to be installed.
fn probe_python(prefix: Option<&Path>) -> Value {
    let found =
        tool_path::locate("python", prefix).or_else(|| tool_path::locate("python3", prefix));
    json!({
        "name": "python",
        "required": true,
        "found": found.is_some(),
        "source": found.as_deref().map(display).unwrap_or_default()
    })
}

/// Run `program args...` and decide `found` from exit status plus a predicate
/// over the merged stdout/stderr text. A program that cannot be located or
/// launched is a `found: false` observation, never an error: absence of an
/// optional overlay is exactly what this probe exists to report.
fn probe_command_output(
    name: &str,
    program: &str,
    args: &[&str],
    prefix: Option<&Path>,
    matches: impl Fn(&str) -> bool,
) -> Value {
    let Some(binary) = tool_path::locate(program, prefix) else {
        return json!({
            "name": name,
            "found": false,
            "output": format!("{program} was not found on PATH")
        });
    };
    let mut command = Command::new(&binary);
    command.args(args);
    if let Some(prefix) = prefix {
        if let Some(path) = prefixed_path(prefix) {
            command
                .env_remove("PATH")
                .env_remove("Path")
                .env("PATH", path);
        }
    }
    match command.output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            let text = text.trim().to_string();
            json!({
                "name": name,
                "found": output.status.success() && matches(&text),
                "output": text
            })
        }
        Err(error) => json!({"name": name, "found": false, "output": error.to_string()}),
    }
}

fn prefixed_path(prefix: &Path) -> Option<std::ffi::OsString> {
    let mut paths = vec![prefix.to_path_buf()];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    env::join_paths(paths).ok()
}

/// `Tier:\s+pro` when Pro auto-activation is opted into, `Tier:\s+(pro|free)`
/// otherwise. Hand-rolled because the crate carries no regex dependency, and
/// case-insensitive to match PowerShell `-match` semantics.
fn matches_tier(text: &str, require_pro: bool) -> bool {
    let lower = text.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(index) = rest.find("tier:") {
        let after = &rest[index + "tier:".len()..];
        let trimmed = after.trim_start_matches([' ', '\t', '\r', '\n']);
        if trimmed.len() < after.len() {
            if trimmed.starts_with("pro") || (!require_pro && trimmed.starts_with("free")) {
                return true;
            }
        }
        rest = after;
    }
    false
}

fn contains_ignore_case(text: &str, needle: &str) -> bool {
    text.to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn platform_paths(platform: &str, pipeline_root: &Path) -> Value {
    let home = home_directory();
    let data_root = data_root(platform, &home);
    let bin = match env::var("CODE_INTEL_BIN") {
        Ok(value) if !value.trim().is_empty() => resolve_code_intel_path(Path::new(&value)),
        _ => data_root.join("bin"),
    };
    let code_intel_home = match env::var("CODE_INTEL_HOME") {
        Ok(value) if !value.trim().is_empty() => resolve_code_intel_path(Path::new(&value)),
        _ => resolve_code_intel_path(pipeline_root),
    };
    json!({
        "home": display(&home),
        "dataRoot": display(&data_root),
        "bin": display(&bin),
        "codeIntelHome": display(&code_intel_home)
    })
}

fn data_root(platform: &str, home: &Path) -> PathBuf {
    if let Ok(value) = env::var("CODE_INTEL_DATA_ROOT") {
        if !value.trim().is_empty() {
            return resolve_code_intel_path(Path::new(&value));
        }
    }
    match platform {
        "windows" => env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|base| !base.as_os_str().is_empty())
            .unwrap_or_else(|| home.join(".code-intel"))
            .join("code-intel"),
        "macos" => home
            .join("Library")
            .join("Application Support")
            .join("code-intel"),
        _ => env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|base| !base.as_os_str().is_empty())
            .unwrap_or_else(|| home.join(".local").join("share"))
            .join("code-intel"),
    }
}

fn home_directory() -> PathBuf {
    let raw = if cfg!(windows) {
        env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"))
    } else {
        env::var_os("HOME")
    };
    raw.map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| resolve_code_intel_path(&path))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn binary_name(platform: &str) -> String {
    if platform == "windows" {
        "code-intel.exe".into()
    } else {
        "code-intel".into()
    }
}

pub(crate) fn resolve_platform(requested: &str) -> Result<String, String> {
    match requested {
        "windows" | "macos" | "linux" => Ok(requested.to_string()),
        "auto" => {
            if cfg!(windows) {
                Ok("windows".into())
            } else if cfg!(target_os = "macos") {
                Ok("macos".into())
            } else if cfg!(target_os = "linux") {
                Ok("linux".into())
            } else {
                Err("Unsupported platform. Pass --platform windows|macos|linux.".into())
            }
        }
        other => Err(format!(
            "--platform must be auto|windows|macos|linux, got {other}"
        )),
    }
}

/// `Resolve-CodeIntelPath`: the on-disk absolute path when it exists, an
/// absolute lexically-normalized path when it does not. Windows verbatim
/// (`\\?\`) prefixes are stripped so the value stays comparable to the
/// path strings every other producer in this pipeline emits.
fn resolve_code_intel_path(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(resolved) => strip_verbatim(&resolved),
        Err(_) => normalize(&absolute_from_cwd(path)),
    }
}

fn absolute_from_cwd(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Lexical `.`/`..` collapse, matching `[Path]::GetFullPath` for paths that do
/// not exist on disk (where `canonicalize` cannot help).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir) | Some(Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn strip_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_path_buf())
}

fn trim_trailing_separator(value: &str) -> String {
    let trimmed = value.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        value.to_string()
    } else {
        trimmed.to_string()
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The human-readable rendering the PowerShell probe printed without `-Json`.
/// CI reads these lines, so the wording is preserved verbatim.
pub(crate) fn render_human(observation: &Value) -> String {
    let mut lines = Vec::new();
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
    if observation["ok"].as_bool().unwrap_or(false) {
        lines.push("Code intel doctor: OK".to_string());
    } else {
        lines.push(format!("Code intel doctor: missing {missing}"));
    }

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
    if observation["checks"]["repo"].is_object() {
        lines.push(format!("Repo: {}", text("/checks/repo/path")));
        lines.push(format!("Repo exists: {}", flag("/checks/repo/exists")));
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
    }
    lines.join("\n")
}

/// `code-intel doctor bootstrap [...]` — the direct CLI surface that replaced
/// `archive/check-code-intel-tools.ps1`. Exits 1 when the probe reports
/// missing prerequisites, matching the script it retired.
pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let mut options = Options::new(pipeline_root());
    let mut json_output = false;
    let mut index = 0;
    while index < raw.len() {
        let token = raw[index].as_str();
        let value = || -> Result<String, String> {
            raw.get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .cloned()
                .ok_or_else(|| format!("{token} requires a value"))
        };
        let step = match token {
            "--json" => {
                json_output = true;
                1
            }
            "--require-repowise" => {
                options.require_repowise = true;
                1
            }
            "--no-require-repowise" => {
                options.require_repowise = false;
                1
            }
            "--require-understand" => {
                options.require_understand = true;
                1
            }
            "--repo" => match value() {
                Ok(found) => {
                    options.repo = Some(found);
                    2
                }
                Err(error) => return fail(&error),
            },
            "--repo-path" => match value() {
                Ok(found) => {
                    options.repo_path = Some(found);
                    2
                }
                Err(error) => return fail(&error),
            },
            "--config" => match value() {
                Ok(found) => {
                    options.config = Some(PathBuf::from(found));
                    2
                }
                Err(error) => return fail(&error),
            },
            "--platform" => match value() {
                Ok(found) => {
                    options.platform = found;
                    2
                }
                Err(error) => return fail(&error),
            },
            "--pipeline-root" => match value() {
                Ok(found) => {
                    options.pipeline_root = PathBuf::from(found);
                    2
                }
                Err(error) => return fail(&error),
            },
            other => return fail(&format!("unknown argument for doctor bootstrap: {other}")),
        };
        index += step;
    }

    let observation = match observe(&options) {
        Ok(observation) => observation,
        Err(error) => return fail(&error),
    };
    if json_output {
        match serde_json::to_string_pretty(&observation) {
            Ok(text) => println!("{text}"),
            Err(error) => return fail(&format!("serialize doctor observation: {error}")),
        }
    } else {
        println!("{}", render_human(&observation));
    }
    if observation["ok"].as_bool().unwrap_or(false) {
        0
    } else {
        1
    }
}

fn fail(message: &str) -> i32 {
    eprintln!("error: {message}");
    65
}

/// Repository root: the directory holding `orchestration/`, discovered the
/// same way the capability layer discovers its manifest.
pub(crate) fn pipeline_root() -> PathBuf {
    crate::capability::discover_manifest(None)
        .and_then(|manifest| manifest.parent()?.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."))
}

/// Sorted view of an observation's `checks` keys — used by the coverage
/// assertion so a silently dropped check surfaces as a test failure rather
/// than as a missing field downstream. `serde_json::Map` is a `BTreeMap`
/// here, so iteration is already ordered.
pub(crate) fn check_names(observation: &Value) -> Vec<String> {
    observation["checks"]
        .as_object()
        .map(|checks| checks.keys().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
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

    #[test]
    fn tier_pattern_accepts_free_only_without_the_pro_opt_in() {
        assert!(matches_tier("Sentrux\nTier:   free\n", false));
        assert!(matches_tier("Tier: pro", false));
        assert!(matches_tier("Tier: pro", true));
        assert!(!matches_tier("Tier:   free", true));
        // No whitespace after the colon is not a match, same as `\s+`.
        assert!(!matches_tier("Tier:free", false));
        assert!(!matches_tier("no tier line here", false));
    }

    #[test]
    fn core_marker_comparison_is_case_insensitive_like_powershell_match() {
        assert!(contains_ignore_case(
            "  ENFORCE ARCHITECTURAL RULES for a repo",
            SENTRUX_CORE_MARKER
        ));
        assert!(!contains_ignore_case(
            "some other help text",
            SENTRUX_CORE_MARKER
        ));
    }

    #[test]
    fn missing_list_preserves_the_retired_scripts_wording_and_order() {
        let checks = json!({
            "pipelineScript": {"found": false},
            "config": {"found": true, "parsed": false},
            "sentrux": {"core": {"found": false}, "pro": {"found": false}},
            "graphProvider": {"sourceFound": false, "cargoFound": false},
            "repo": {"path": "x", "exists": false}
        });
        let tools = vec![
            json!({"name": "rg", "required": true, "found": false}),
            json!({"name": "repomix", "required": false, "found": false}),
        ];
        let missing = missing_list(
            &checks,
            &tools,
            false,
            true,
            Some("bad json"),
            true,
            false,
            "C:/nope",
        );
        assert_eq!(
            missing,
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
        assert!(missing_list(&checks, &tools, true, false, None, false, false, "").is_empty());
    }

    #[test]
    fn configured_sentrux_path_resolves_the_scope_and_finds_scoped_rules() {
        let root = scratch("scope");
        let repo = root.join("ConfiguredRepo");
        let sentrux = repo.join("backend").join(".sentrux");
        fs::create_dir_all(&sentrux).unwrap();
        fs::write(sentrux.join("rules.toml"), b"").unwrap();
        fs::write(sentrux.join("baseline.json"), b"{}").unwrap();
        let config = json!({"repos": {"fixture": {
            "path": format!("{}{}", display(&repo), std::path::MAIN_SEPARATOR),
            "sentruxPath": "backend"
        }}});

        // Reverse lookup from an explicit --repo-path carrying a `.` segment,
        // exactly the shape the retired PowerShell contract test exercised.
        let mut options = Options::new(root.clone());
        options.repo_path = Some(display(&repo.join(".")));
        let repo_path = resolve_repo_path(&options, Some(&config)).unwrap();
        let entry = find_repo_config_by_path(Some(&config), &repo_path).unwrap();
        let scope = resolve_sentrux_scope(&repo_path, Some(&&entry.clone()));

        let state = repo_state(Some(&repo_path), Some(&scope));
        assert_eq!(
            state["sentruxScope"],
            json!(display(&resolve_code_intel_path(&repo.join("backend"))))
        );
        assert_eq!(state["sentruxRules"], json!(true));
        assert_eq!(state["sentruxBaseline"], json!(true));
        fs::remove_dir_all(root).ok();
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
        assert_eq!(
            check_names(&observation),
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
        let tools = observation["checks"]["tools"].as_array().unwrap();
        let names = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap_or_default())
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
    fn probe_reports_an_absent_optional_overlay_without_failing() {
        let empty = scratch("empty-bin");
        let probe = probe_command_output(
            "sentrux-core",
            "sentrux",
            &["check", "--help"],
            Some(&empty),
            |_| true,
        );
        assert_eq!(probe["name"], "sentrux-core");
        assert!(probe["found"].is_boolean());
        fs::remove_dir_all(empty).ok();
    }

    #[test]
    fn human_rendering_keeps_the_retired_scripts_first_line() {
        let ok = json!({"ok": true, "missing": [], "checks": {}});
        assert!(render_human(&ok).starts_with("Code intel doctor: OK"));
        let bad = json!({"ok": false, "missing": ["rg", "git"], "checks": {}});
        assert!(render_human(&bad).starts_with("Code intel doctor: missing rg, git"));
    }

    #[test]
    fn platform_resolution_rejects_unknown_values() {
        assert_eq!(resolve_platform("linux").unwrap(), "linux");
        assert!(resolve_platform("auto").is_ok());
        assert!(resolve_platform("solaris").is_err());
    }

    #[test]
    fn path_normalization_collapses_dot_segments_for_absent_paths() {
        let normalized = normalize(Path::new("/a/b/../c/./d"));
        assert_eq!(normalized, PathBuf::from("/a/c/d"));
    }
}
