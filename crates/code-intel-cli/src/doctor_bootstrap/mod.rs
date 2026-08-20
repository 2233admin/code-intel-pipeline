//! Native bootstrap/environment probe — the Rust owner of what
//! `legacy/check-code-intel-tools.ps1` used to compute in PowerShell.
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
mod identity;
mod paths;
mod probe;

// Included by path so every crate root that pulls this module in — the binary
// and the adapter copies used by integration tests — resolves it the same way.
#[path = "../doctor_provider_rows.rs"]
mod doctor_provider_rows;

use paths::display;
// Re-exported so `language_pref` derives the user-level config root from the
// same OS/`CODE_INTEL_DATA_ROOT` switch this probe already ported from
// `Get-CodeIntelDataRoot` in `code-intel-platform.psm1`, instead of a third
// independent copy of that logic.
pub(crate) use paths::{data_root, home_directory, resolve_platform};

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
    /// Repository root holding `crates/`, `target/` and `legacy/`.
    pub(crate) pipeline_root: PathBuf,
}

impl Options {
    pub(crate) fn new(pipeline_root: PathBuf) -> Self {
        Self {
            repo: None,
            repo_path: None,
            config: None,
            platform: "auto".into(),
            require_repowise: false,
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
        .join("legacy")
        .join("run-code-intel.ps1");
    let cli_root = options.pipeline_root.join("crates").join("code-intel-cli");
    let graph_source = cli_root.join("src").join("graph").join("mod.rs");
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

    // #300: optional weco perf-optimize provider (WecoAI's AIDE-style
    // tree-search CLI). Never required — the built-in pipeline has no hard
    // dependency on it, mirroring repomix/ast-grep below. Captured separately
    // (rather than inline in `tools`) because `checks.weco.reason` below
    // needs its `found` bit alongside the BYOK check.
    let weco_probe = probe::probe_tool("weco", false, prefix);
    let weco_present = weco_probe["found"].as_bool().unwrap_or(false);

    let tools = vec![
        probe::probe_tool("rg", true, prefix),
        probe::probe_tool("git", true, prefix),
        probe::probe_python(prefix),
        probe::probe_tool("repowise", options.require_repowise, prefix),
        probe::probe_tool("repomix", false, prefix),
        probe::probe_tool("sentrux", !builtin_sentrux, prefix),
        // `edit.ast-grep-plan` ships as a production capability whose runtime
        // adapter resolves `ast-grep` through `tool_path`, so a machine
        // without it only finds out at capability-exec time, as
        // `Unavailable("start ast-grep: ...")`. Optional because
        // `orchestration/toolchain-versions.v1.json` declares ast-grep
        // `required: false` — but observed, so the doctor stops omitting a
        // tool a shipped capability cannot run without.
        probe::probe_tool("ast-grep", false, prefix),
        weco_probe,
    ];

    let sentrux_core = probe::probe_command_output(
        "sentrux-core",
        "sentrux",
        &["check", "--help"],
        prefix,
        |text| probe::contains_ignore_case(text, probe::SENTRUX_CORE_MARKER),
    );
    // Tier: free is healthy without the SENTRUX_AUTO_PRO opt-in (Pro
    // auto-activation is opt-in; see legacy/tools/sentrux-shim/sentrux-shim.ps1).
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

    let assistance_plugins = probe::probe_assistance_plugins(
        &options
            .pipeline_root
            .join("orchestration")
            .join("agent-assistance-catalog.v1.json"),
        &home_dir,
    );

    let repo_state = repo_state(repo_path.as_deref(), sentrux_scope.as_deref());
    let home = code_intel_home(&options.pipeline_root);
    let weco_byok_configured = weco_byok_configured_from(|name| std::env::var_os(name).is_some());
    let weco_account_configured = weco_account_configured_from(
        std::env::var_os(WECO_ACCOUNT_ENV_VAR).is_some(),
        home_dir
            .join(".config")
            .join("weco")
            .join("credentials.json")
            .is_file(),
    );
    let weco_reason = weco_reason(weco_present, weco_byok_configured, weco_account_configured);

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
            // Deliberately not wired to `language_pref::resolve` (issue
            // #155): this module's source is `#[path]`-included as its own
            // independent compilation root by roughly a dozen integration
            // test files (each pulling it in transitively through
            // `capability_inventory::doctor_adapter`), none of which declare
            // a `language_pref` module. A `crate::language_pref` reference
            // here compiles in the real binary but fails every one of those
            // test targets with `cannot find language_pref in crate`. The
            // command below is illustrative only (this probe never writes),
            // so it stays a static example rather than gaining a fragile
            // cross-module dependency for a cosmetic string.
            "command": format!(
                "{} graph --repo <repo-path> --language zh --write --json",
                display(&graph_command_binary)
            )
        },
        "assistancePlugins": assistance_plugins,
        "repo": repo_state,
        "env": {"codeIntelHome": home.observation()},
        // #300: BYOK is presence, not validity — this never calls out to an
        // LLM provider to confirm the key works, matching how `tools` above
        // never confirms a found binary actually runs. `reason` is empty
        // once ready; the operator-facing `doctor_provider_rows` provider row
        // deliberately excludes this text (its schema forbids extra
        // properties), so it only lives here.
        "weco": {
            "byokConfigured": weco_byok_configured,
            "accountConfigured": weco_account_configured,
            "reason": weco_reason
        }
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

/// Env vars weco's own BYOK (bring-your-own-key) contract recognizes for its
/// supported providers (OpenAI, Anthropic, Gemini). Presence only — this
/// never validates that a key actually authenticates, mirroring how
/// `probe::probe_tool` never validates that a found binary actually runs.
const WECO_BYOK_ENV_VARS: &[&str] = &["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "GEMINI_API_KEY"];

/// Pure over an injected `present` predicate so the three-state behavior
/// (#300: not-installed / installed-but-unauthenticated / available) is
/// testable without mutating process-global env vars, which would race
/// against Rust's default parallel test execution.
fn weco_byok_configured_from(present: impl Fn(&str) -> bool) -> bool {
    WECO_BYOK_ENV_VARS.iter().any(|name| present(name))
}

/// weco.ai's own account token -- distinct from `WECO_BYOK_ENV_VARS`, and
/// required unconditionally: verified against the weco-cli source (#301
/// research) that weco's optimization loop is not local at all -- the
/// decision logic runs server-side, polled over HTTP, with a heartbeat
/// thread keeping the run alive on weco's backend. The BYOK key only pays
/// for LLM generation; this token is what creates/tracks the run at all,
/// with or without BYOK. Resolution order matches weco's own
/// `config.py::load_weco_api_key`: the env var first, then the credentials
/// file `weco login` writes.
const WECO_ACCOUNT_ENV_VAR: &str = "WECO_API_KEY";

/// Pure over injected presence bits, same reasoning as
/// `weco_byok_configured_from`: testable without mutating process-global env
/// vars or touching the real filesystem.
fn weco_account_configured_from(env_present: bool, credentials_file_present: bool) -> bool {
    env_present || credentials_file_present
}

/// Human-readable cause for #300's "unavailable" states, distinguishing
/// not-installed from the two independent auth gates weco actually has
/// (#301 research corrected the original assumption that BYOK alone was
/// sufficient). Lives in `checks.weco.reason` rather than the
/// `doctor_provider_rows` provider row: that row's schema
/// (`providerObservation`, code-intel-doctor-observation.v1.schema.json)
/// sets `additionalProperties: false` on exactly
/// `[id,presence,readiness,conformance,admissibility]`, so a free-text field
/// there would fail schema validation.
fn weco_reason(present: bool, byok_configured: bool, account_configured: bool) -> &'static str {
    if !present {
        "weco not found on PATH"
    } else if !byok_configured && !account_configured {
        "weco installed but neither an LLM provider key (BYOK) nor a weco.ai account (WECO_API_KEY) is configured"
    } else if !byok_configured {
        "weco installed but no LLM provider key configured (BYOK)"
    } else if !account_configured {
        "weco installed but no weco.ai account configured (WECO_API_KEY) -- weco's run loop is server-tracked and requires this even with your own LLM key"
    } else {
        ""
    }
}

/// The packaged release binary first, then a source checkout's
/// `target/release` and `target/debug`, with the platform-correct name.
fn binary_candidates(pipeline_root: &Path, platform: &str) -> [PathBuf; 3] {
    let name = paths::binary_name(platform);
    [
        pipeline_root.join("bin").join(&name),
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
    // The two checks above only prove the *internal* graph provider ships in
    // this checkout; inside the pipeline repo they are trivially true, which
    // left `--require-understand` a fail-open no-op that never once consulted
    // the `understandAnything` block it computes. The retired PowerShell probe
    // had the same hole. Understand Anything ships either as an agent skill or
    // as a plugin directory, so either one satisfies the requirement — this is
    // the same "skill/plugin" wording the installer's own RequireUnderstand
    // check remediates with.
    if options.require_understand
        && !flag("/understandAnything/skillFound")
        && !flag("/understandAnything/pluginFound")
    {
        missing.push("Understand Anything skill or plugin".to_string());
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
/// `legacy/check-code-intel-tools.ps1`. Exits 1 when the probe reports
/// missing prerequisites, matching the script it retired.
pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let mut options = Options::new(default_pipeline_root());
    let mut json_output = false;
    // Bootstrap deliberately reports `ok` while an external provider overlay
    // is broken, because the built-in engines make the overlay optional. The
    // strict verdict — a *present* provider must conform — lives in the DAG
    // doctor node, which CI runs before installing anything. This flag makes
    // that same verdict callable after an install, so the branch every
    // installed machine takes is reachable without a full pipeline run.
    let mut require_provider_conformance = false;
    // Caller-supplied cache-buster (#197): a host command proxy that keys its
    // replay cache on command-line bytes can never hit when the caller varies
    // this value, and the echo in `invocationIdentity.nonce` proves the value
    // reached a live process instead of a cached capture.
    let mut nonce: Option<String> = None;
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
            ("--require-provider-conformance", _) => {
                require_provider_conformance = true;
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
            ("--nonce", Some(value)) => {
                nonce = Some(value.clone());
                2
            }
            (
                "--repo" | "--repo-path" | "--config" | "--platform" | "--pipeline-root"
                | "--nonce",
                None,
            ) => return fail(&format!("{token} requires a value")),
            (other, _) => return fail(&format!("unknown argument for doctor bootstrap: {other}")),
        };
    }

    let mut observation = match observe(&options) {
        Ok(observation) => observation,
        Err(error) => return fail(&error),
    };
    identity::attach(&mut observation, nonce);
    let rendered = if json_output {
        match serde_json::to_string_pretty(&observation) {
            Ok(text) => text,
            Err(error) => return fail(&format!("serialize doctor observation: {error}")),
        }
    } else {
        format!(
            "{}\n{}",
            render_human(&observation),
            identity::human_line(&observation)
        )
    };
    println!("{rendered}");
    if require_provider_conformance {
        let nonconforming = doctor_provider_rows::nonconforming_providers(&observation);
        if !nonconforming.is_empty() {
            return fail(&format!(
                "provider conformance failed: {} present but nonconforming. A broken overlay on PATH is a verdict-drift hazard: reinstall it, or take it off PATH so the built-in engine applies.",
                nonconforming.join(", ")
            ));
        }
    }
    i32::from(!observation["ok"].as_bool().unwrap_or(false))
}

fn fail(message: &str) -> i32 {
    eprintln!("error: {message}");
    65
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
