use crate::{codenexus_adapter, graph, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

#[path = "graph_adapter.rs"]
mod graph_adapter;
#[path = "repowise_adapter.rs"]
mod repowise_adapter;
#[path = "sentrux_adapter.rs"]
mod sentrux_adapter;

pub struct Options<'a> {
    pub action: &'a str,
    pub provider: Option<&'a str>,
    pub operation: Option<&'a str>,
    pub repo: Option<&'a Path>,
    pub language: &'a str,
    pub full: bool,
    pub write: bool,
    pub json: bool,
}

#[derive(Clone, Copy)]
pub struct ProviderOperation {
    pub provider: &'static str,
    pub operation: &'static str,
    pub stage: &'static str,
    pub protocol: &'static str,
    pub method: &'static str,
    pub route: &'static str,
    pub command_template: &'static str,
    pub artifact: &'static str,
    pub required: bool,
    pub status: &'static str,
    pub source_spec: &'static str,
    pub notes: &'static str,
}

pub const OPERATIONS: &[ProviderOperation] = &[
    ProviderOperation {
        provider: "codenexus",
        operation: "adapt",
        stage: "localization",
        protocol: "provider-port+cli",
        method: "POST",
        route: "/api/providers/codenexus/adapt",
        command_template: "target/debug/code-intel.exe provider codenexus-adapt --request <native.json|-> --artifact-root <artifact-directory> --evaluated-at <unix-seconds> --max-age-seconds <seconds>",
        artifact: "code-intel-codenexus-route-result.v1",
        required: false,
        status: "active",
        source_spec: "Pipeline-owned B04 adapter over full CodeNexus or explicit lite compatibility output and A04 admissibility",
        notes: "Canonical CodeNexus evidence route. Full is primary; lite is explicit fallback/legacy rollback only. Provider process, storage, retrieval, and impact semantics remain external.",
    },
    ProviderOperation {
        provider: "sentrux",
        operation: "adapt",
        stage: "structure_governance",
        protocol: "provider-port+cli",
        method: "POST",
        route: "/api/providers/sentrux/adapt",
        command_template: "target/debug/code-intel.exe provider sentrux-adapt --request <native.json|-> --artifact-root <artifact-directory> --evaluated-at <unix-seconds> --max-age-seconds <seconds>",
        artifact: "code-intel-sentrux-route-result.v1",
        required: true,
        status: "active",
        source_spec: "Pipeline-owned B03 translation over Sentrux/shim native output and A04 admissibility",
        notes: "Canonical structural evidence route. The bundled shim and Invoke-SentruxAgentTool.ps1 remain replaceable provider implementations/rollback surfaces, never diagnosis authority.",
    },
    ProviderOperation {
        provider: "session",
        operation: "adapt",
        stage: "verification",
        protocol: "provider-port+cli",
        method: "POST",
        route: "/api/providers/session/adapt",
        command_template: "target/debug/code-intel.exe provider session-adapt --repo <repo-path> --trace <mindwalk-trace.json> [--hotspots <sentrux-hotspots-or-dsm.json>] [--out <session-evidence.json>]",
        artifact: "code-intel-session-evidence.v1",
        required: false,
        status: "active",
        source_spec: "Pipeline-owned privacy, snapshot, normalization, and structural-join logic over optional Mindwalk trace v1 input",
        notes: "Optional session-review route. Mindwalk extraction remains replaceable; raw prompts, summaries, outside paths, and provider-specific policy never enter the normalized artifact.",
    },
    ProviderOperation {
        provider: "graph",
        operation: "adapt",
        stage: "architecture_graph",
        protocol: "provider-port+cli",
        method: "POST",
        route: "/api/providers/graph/adapt",
        command_template: "target/debug/code-intel.exe provider graph-adapt --request <native.json|-> --artifact-root <artifact-directory> --evaluated-at <unix-seconds> --max-age-seconds <seconds>",
        artifact: "code-intel-graph-route-result.v1",
        required: true,
        status: "active",
        source_spec: "Pipeline-owned B02 adapter over internal Rust or explicit Understand-compatible fallback output and A04 admissibility",
        notes: "Canonical graph evidence route. Current snapshot binding is mandatory; external graph execution remains explicit fallback/legacy rollback only.",
    },
    ProviderOperation {
        provider: "repowise",
        operation: "adapt",
        stage: "semantic_memory",
        protocol: "provider-port+cli",
        method: "POST",
        route: "/api/providers/repowise/adapt",
        command_template: "target/debug/code-intel.exe provider repowise-adapt --request <native.json|-> --artifact-root <artifact-directory> --evaluated-at <unix-seconds> --max-age-seconds <seconds>",
        artifact: "code-intel-repowise-route-result.v1",
        required: false,
        status: "active",
        source_spec: "Pipeline-owned B01 adapter over Repowise-native result and A04 admissibility",
        notes: "Production evidence route; every emitted observation passes A04. Legacy provider probes and direct CLI commands remain optional diagnostics/rollback only.",
    },
    ProviderOperation {
        provider: "repowise",
        operation: "status",
        stage: "semantic_memory",
        protocol: "cli+mcp-compatible",
        method: "POST",
        route: "/api/providers/repowise/status",
        command_template: "repowise status --no-workspace <repo-path>",
        artifact: ".repowise/state.json",
        required: true,
        status: "active",
        source_spec: "Repowise CLI: status [PATH], MCP/HTTP serve surfaces for agent callers",
        notes: "Health/readiness only; not evidence. No model required; reports wiki sync and page statistics.",
    },
    ProviderOperation {
        provider: "repowise",
        operation: "index",
        stage: "semantic_memory",
        protocol: "cli+mcp-compatible",
        method: "POST",
        route: "/api/providers/repowise/index",
        command_template:
            "repowise update --no-workspace --index-only <repo-path> or repowise init --index-only <repo-path>",
        artifact: ".repowise/wiki.db",
        required: true,
        status: "active",
        source_spec: "Repowise CLI: init/update --index-only, MCP tools include semantic code retrieval",
        notes: "No model required; refreshes index artifacts. B01 translation must pass A04 before fact promotion.",
    },
    ProviderOperation {
        provider: "repowise",
        operation: "docs",
        stage: "semantic_memory",
        protocol: "cli+mcp-compatible",
        method: "POST",
        route: "/api/providers/repowise/docs",
        command_template: "repowise update --docs --no-workspace <repo-path>",
        artifact: "report.json.steps[repowise docs]",
        required: false,
        status: "compatibility",
        source_spec: "Repowise CLI: update --docs, provider/model options",
        notes: "Model-backed and separately partial/freshness-scoped; provider quota cannot disable status/index. A04 admission is required before fact promotion.",
    },
    ProviderOperation {
        provider: "codenexus",
        operation: "lite",
        stage: "localization",
        protocol: "artifact+command",
        method: "POST",
        route: "/api/providers/codenexus/lite",
        command_template: r#"pwsh -NoProfile -File "$env:CODE_INTEL_HOME\Invoke-CodeNexusLite.ps1" -RepoPath '<repo-path>'"#,
        artifact: "codenexus-context.json",
        required: false,
        status: "compatibility",
        source_spec: "Repository-owned Invoke-CodeNexusLite.ps1 localization adapter",
        notes: "Canonical compatibility route; non-blocking and replaceable, with Survival Scanner preserving localization when unavailable.",
    },
    ProviderOperation {
        provider: "repowise",
        operation: "lite",
        stage: "localization",
        protocol: "compatibility-alias",
        method: "POST",
        route: "/api/providers/repowise/lite",
        command_template: r#"pwsh -NoProfile -File "$env:CODE_INTEL_HOME\Invoke-CodeNexusLite.ps1" -RepoPath '<repo-path>'"#,
        artifact: "codenexus-context.json",
        required: false,
        status: "compatibility",
        source_spec: "Legacy Repowise-namespaced alias for the repository-owned CodeNexus-lite adapter",
        notes: "Deprecated compatibility alias for one release; migrate callers to codenexus/lite.",
    },
    ProviderOperation {
        provider: "understand",
        operation: "graph",
        stage: "architecture_graph",
        protocol: "artifact+command",
        method: "POST",
        route: "/api/providers/understand/graph",
        command_template:
            "target/debug/code-intel.exe provider --action Invoke --provider understand --operation graph --repo <repo-path> --language zh --write --json",
        artifact: ".understand-anything/knowledge-graph.json",
        required: true,
        status: "active",
        source_spec: "Understand Anything command contract: /understand <repo> --language <lang>, graph artifact",
        notes: "Internal Rust provider emits the Understand-compatible artifact first.",
    },
    ProviderOperation {
        provider: "understand",
        operation: "graph_full",
        stage: "architecture_graph",
        protocol: "artifact+command",
        method: "POST",
        route: "/api/providers/understand/graph/full",
        command_template:
            "target/debug/code-intel.exe provider --action Invoke --provider understand --operation graph_full --repo <repo-path> --language zh --write --json",
        artifact: ".understand-anything/knowledge-graph.json",
        required: false,
        status: "active",
        source_spec: "Understand Anything full graph refresh semantics",
        notes: "Use when a fresh complete graph is requested.",
    },
    ProviderOperation {
        provider: "understand",
        operation: "external_fallback",
        stage: "architecture_graph",
        protocol: "manual-command",
        method: "MANUAL",
        route: "/compat/providers/understand/external",
        command_template: "/understand <repo-path> --language zh",
        artifact: ".understand-anything/knowledge-graph.json",
        required: false,
        status: "compatibility",
        source_spec: "Understand Anything upstream/plugin command surface",
        notes: "Use only if internal graph provider fails or richer external pass is explicitly requested.",
    },
];

pub fn translate_repowise_native(
    native: &Value,
    evaluated_at: u64,
    max_age_seconds: u64,
) -> std::result::Result<Value, String> {
    repowise_adapter::translate(native, evaluated_at, max_age_seconds)
}

pub fn translate_graph_native(
    native: &Value,
    evaluated_at: u64,
    max_age_seconds: u64,
) -> std::result::Result<Value, String> {
    graph_adapter::translate(native, evaluated_at, max_age_seconds)
}

pub fn translate_sentrux_native(
    native: &Value,
    evaluated_at: u64,
    max_age_seconds: u64,
) -> std::result::Result<Value, String> {
    sentrux_adapter::translate(native, evaluated_at, max_age_seconds)
}

pub fn translate_codenexus_native(
    native: &Value,
    evaluated_at: u64,
    max_age_seconds: u64,
) -> std::result::Result<Value, String> {
    codenexus_adapter::translate(native, evaluated_at, max_age_seconds)
}

pub(crate) fn run_codenexus_adapt_raw(raw: &[String]) -> i32 {
    let cli = match parse_codenexus_adapt_cli(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let native = match read_provider_native(&cli.request, "CodeNexus") {
        Ok(value) => value,
        Err(message) => return print_codenexus_route_rejection(&message),
    };
    let mut adapter =
        match translate_codenexus_native(&native, cli.evaluated_at, cli.max_age_seconds) {
            Ok(value) => value,
            Err(message) => return print_codenexus_route_rejection(&message),
        };
    let admitted = match crate::admissibility::validate_for_consumer(
        &adapter["evidence"]["request"],
        &cli.artifact_root,
    ) {
        Ok(value) => value,
        Err(message) => return print_codenexus_route_rejection(&message),
    };
    if let Err(message) = codenexus_adapter::validate_admitted_payload(admitted.payload(), &adapter)
    {
        return print_codenexus_route_rejection(&message);
    }
    adapter["port"]["perceptionUsable"] = json!(
        admitted.result()["domainVerdict"] == "observed"
            && adapter["port"]["status"] == "current"
            && adapter["port"]["freshness"] == "current"
    );
    let result = json!({
        "schema":"code-intel-codenexus-route-result.v1",
        "status":"completed",
        "adapter":adapter,
        "admission":admitted.result(),
        "engineeringFacts":[],
        "diagnostics":[]
    });
    println!("{}", serde_json::to_string(&result).unwrap());
    0
}

struct CodeNexusAdaptCli {
    request: String,
    artifact_root: PathBuf,
    evaluated_at: u64,
    max_age_seconds: u64,
}

fn parse_codenexus_adapt_cli(raw: &[String]) -> std::result::Result<CodeNexusAdaptCli, String> {
    let mut request = None;
    let mut artifact_root = None;
    let mut evaluated_at = None;
    let mut max_age_seconds = None;
    let mut index = 0;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--request" | "--artifact-root" | "--evaluated-at" | "--max-age-seconds"
        ) {
            return Err(format!("unknown CodeNexus adapter argument: {flag}"));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires exactly one value"))?;
        match flag {
            "--request" if request.replace(value.clone()).is_some() => {
                return Err("duplicate CodeNexus adapter argument: --request".to_string())
            }
            "--artifact-root" if artifact_root.replace(PathBuf::from(value)).is_some() => {
                return Err("duplicate CodeNexus adapter argument: --artifact-root".to_string())
            }
            "--evaluated-at" => set_u64(&mut evaluated_at, value, flag, "CodeNexus")?,
            "--max-age-seconds" => set_u64(&mut max_age_seconds, value, flag, "CodeNexus")?,
            _ => {}
        }
        index += 2;
    }
    Ok(CodeNexusAdaptCli {
        request: request.ok_or("CodeNexus adapter requires --request")?,
        artifact_root: artifact_root.ok_or("CodeNexus adapter requires --artifact-root")?,
        evaluated_at: evaluated_at.ok_or("CodeNexus adapter requires --evaluated-at")?,
        max_age_seconds: max_age_seconds
            .filter(|value| *value > 0)
            .ok_or("--max-age-seconds requires a positive integer")?,
    })
}

fn print_codenexus_route_rejection(message: &str) -> i32 {
    let result = json!({
        "schema":"code-intel-codenexus-route-result.v1",
        "status":"rejected",
        "adapter":null,
        "admission":null,
        "engineeringFacts":[],
        "diagnostics":[message]
    });
    println!("{}", serde_json::to_string(&result).unwrap());
    eprintln!("{message}");
    65
}

pub(crate) fn run_sentrux_adapt_raw(raw: &[String]) -> i32 {
    let cli = match parse_sentrux_adapt_cli(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let native = match read_provider_native(&cli.request, "Sentrux") {
        Ok(value) => value,
        Err(message) => return print_sentrux_route_rejection(&message),
    };
    let mut adapter = match translate_sentrux_native(&native, cli.evaluated_at, cli.max_age_seconds)
    {
        Ok(value) => value,
        Err(message) => return print_sentrux_route_rejection(&message),
    };
    let admitted = match crate::admissibility::validate_for_consumer(
        &adapter["evidence"]["request"],
        &cli.artifact_root,
    ) {
        Ok(value) => value,
        Err(message) => return print_sentrux_route_rejection(&message),
    };
    if let Err(message) = sentrux_adapter::validate_admitted_payload(admitted.payload(), &adapter) {
        return print_sentrux_route_rejection(&message);
    }
    adapter["port"]["diagnosisEligible"] = json!(
        admitted.result()["domainVerdict"] == "observed"
            && adapter["port"]["completeness"] == "complete"
            && adapter["port"]["freshness"] == "current"
    );
    let result = json!({
        "schema":"code-intel-sentrux-route-result.v1",
        "status":"completed",
        "adapter":adapter,
        "admission":admitted.result(),
        "engineeringFacts":[],
        "diagnostics":[]
    });
    println!("{}", serde_json::to_string(&result).unwrap());
    0
}

struct SentruxAdaptCli {
    request: String,
    artifact_root: PathBuf,
    evaluated_at: u64,
    max_age_seconds: u64,
}

fn parse_sentrux_adapt_cli(raw: &[String]) -> std::result::Result<SentruxAdaptCli, String> {
    let mut request = None;
    let mut artifact_root = None;
    let mut evaluated_at = None;
    let mut max_age_seconds = None;
    let mut index = 0;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--request" | "--artifact-root" | "--evaluated-at" | "--max-age-seconds"
        ) {
            return Err(format!("unknown Sentrux adapter argument: {flag}"));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires exactly one value"))?;
        match flag {
            "--request" if request.replace(value.clone()).is_some() => {
                return Err("duplicate Sentrux adapter argument: --request".to_string())
            }
            "--artifact-root" if artifact_root.replace(PathBuf::from(value)).is_some() => {
                return Err("duplicate Sentrux adapter argument: --artifact-root".to_string())
            }
            "--evaluated-at" => set_u64(&mut evaluated_at, value, flag, "Sentrux")?,
            "--max-age-seconds" => set_u64(&mut max_age_seconds, value, flag, "Sentrux")?,
            _ => {}
        }
        index += 2;
    }
    Ok(SentruxAdaptCli {
        request: request.ok_or("Sentrux adapter requires --request")?,
        artifact_root: artifact_root.ok_or("Sentrux adapter requires --artifact-root")?,
        evaluated_at: evaluated_at.ok_or("Sentrux adapter requires --evaluated-at")?,
        max_age_seconds: max_age_seconds
            .filter(|value| *value > 0)
            .ok_or("--max-age-seconds requires a positive integer")?,
    })
}

fn print_sentrux_route_rejection(message: &str) -> i32 {
    let result = json!({
        "schema":"code-intel-sentrux-route-result.v1",
        "status":"rejected",
        "adapter":null,
        "admission":null,
        "engineeringFacts":[],
        "diagnostics":[message]
    });
    println!("{}", serde_json::to_string(&result).unwrap());
    eprintln!("{message}");
    65
}

pub(crate) fn run_graph_adapt_raw(raw: &[String]) -> i32 {
    let cli = match parse_graph_adapt_cli(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let native = match read_provider_native(&cli.request, "graph") {
        Ok(value) => value,
        Err(message) => return print_graph_route_rejection(&message),
    };
    let mut adapter = match translate_graph_native(&native, cli.evaluated_at, cli.max_age_seconds) {
        Ok(value) => value,
        Err(message) => return print_graph_route_rejection(&message),
    };
    let admitted = match crate::admissibility::validate_for_consumer(
        &adapter["evidence"]["request"],
        &cli.artifact_root,
    ) {
        Ok(value) => value,
        Err(message) => return print_graph_route_rejection(&message),
    };
    if let Err(message) = graph_adapter::validate_admitted_payload(admitted.payload(), &adapter) {
        return print_graph_route_rejection(&message);
    }
    adapter["port"]["anatomyUsable"] = json!(
        admitted.result()["domainVerdict"] == "observed"
            && adapter["port"]["status"] == "current"
            && adapter["port"]["freshness"] == "current"
    );
    let result = json!({
        "schema":"code-intel-graph-route-result.v1",
        "status":"completed",
        "adapter":adapter,
        "admission":admitted.result(),
        "engineeringFacts":[],
        "diagnostics":[]
    });
    println!("{}", serde_json::to_string(&result).unwrap());
    0
}

struct GraphAdaptCli {
    request: String,
    artifact_root: PathBuf,
    evaluated_at: u64,
    max_age_seconds: u64,
}

fn parse_graph_adapt_cli(raw: &[String]) -> std::result::Result<GraphAdaptCli, String> {
    let mut request = None;
    let mut artifact_root = None;
    let mut evaluated_at = None;
    let mut max_age_seconds = None;
    let mut index = 0;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--request" | "--artifact-root" | "--evaluated-at" | "--max-age-seconds"
        ) {
            return Err(format!("unknown graph adapter argument: {flag}"));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires exactly one value"))?;
        match flag {
            "--request" if request.replace(value.clone()).is_some() => {
                return Err("duplicate graph adapter argument: --request".to_string())
            }
            "--artifact-root" if artifact_root.replace(PathBuf::from(value)).is_some() => {
                return Err("duplicate graph adapter argument: --artifact-root".to_string())
            }
            "--evaluated-at" => set_u64(&mut evaluated_at, value, flag, "graph")?,
            "--max-age-seconds" => set_u64(&mut max_age_seconds, value, flag, "graph")?,
            _ => {}
        }
        index += 2;
    }
    Ok(GraphAdaptCli {
        request: request.ok_or("graph adapter requires --request")?,
        artifact_root: artifact_root.ok_or("graph adapter requires --artifact-root")?,
        evaluated_at: evaluated_at.ok_or("graph adapter requires --evaluated-at")?,
        max_age_seconds: max_age_seconds
            .filter(|value| *value > 0)
            .ok_or("--max-age-seconds requires a positive integer")?,
    })
}

fn print_graph_route_rejection(message: &str) -> i32 {
    let result = json!({
        "schema":"code-intel-graph-route-result.v1",
        "status":"rejected",
        "adapter":null,
        "admission":null,
        "engineeringFacts":[],
        "diagnostics":[message]
    });
    println!("{}", serde_json::to_string(&result).unwrap());
    eprintln!("{message}");
    65
}

const MAX_REPOWISE_NATIVE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PROVIDER_NATIVE_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) fn run_repowise_adapt_raw(raw: &[String]) -> i32 {
    let cli = match parse_repowise_adapt_cli(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let native = match read_repowise_native(&cli.request) {
        Ok(value) => value,
        Err(message) => return print_repowise_route_rejection(&message),
    };
    let adapter = match translate_repowise_native(&native, cli.evaluated_at, cli.max_age_seconds) {
        Ok(value) => value,
        Err(message) => return print_repowise_route_rejection(&message),
    };

    let mut admissions = Vec::new();
    let mut diagnostics = Vec::new();
    for evidence in adapter["evidence"]
        .as_array()
        .expect("the adapter always emits an evidence array")
    {
        let channel = evidence["channel"]
            .as_str()
            .expect("the adapter always emits a channel");
        match crate::admissibility::validate_for_consumer(&evidence["request"], &cli.artifact_root)
        {
            Ok(admitted) => admissions.push(json!({
                "channel":channel,
                "result":admitted.result()
            })),
            Err(message) => {
                diagnostics.push(format!("{channel}: {message}"));
                admissions.push(json!({
                    "channel":channel,
                    "result":rejected_admission(&message)
                }));
            }
        }
    }
    let rejected = !diagnostics.is_empty();
    let result = json!({
        "schema":"code-intel-repowise-route-result.v1",
        "status":if rejected { "rejected" } else { "completed" },
        "adapter":adapter,
        "admissions":admissions,
        "engineeringFacts":[],
        "diagnostics":diagnostics
    });
    println!("{}", serde_json::to_string(&result).unwrap());
    if rejected {
        65
    } else {
        0
    }
}

struct RepowiseAdaptCli {
    request: String,
    artifact_root: PathBuf,
    evaluated_at: u64,
    max_age_seconds: u64,
}

fn parse_repowise_adapt_cli(raw: &[String]) -> std::result::Result<RepowiseAdaptCli, String> {
    let mut request = None;
    let mut artifact_root = None;
    let mut evaluated_at = None;
    let mut max_age_seconds = None;
    let mut index = 0;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--request" | "--artifact-root" | "--evaluated-at" | "--max-age-seconds"
        ) {
            return Err(format!("unknown Repowise adapter argument: {flag}"));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires exactly one value"))?;
        match flag {
            "--request" if request.replace(value.clone()).is_some() => {
                return Err("duplicate Repowise adapter argument: --request".to_string())
            }
            "--artifact-root" if artifact_root.replace(PathBuf::from(value)).is_some() => {
                return Err("duplicate Repowise adapter argument: --artifact-root".to_string())
            }
            "--evaluated-at" => set_u64(&mut evaluated_at, value, flag, "Repowise")?,
            "--max-age-seconds" => set_u64(&mut max_age_seconds, value, flag, "Repowise")?,
            _ => {}
        }
        index += 2;
    }
    let max_age_seconds = max_age_seconds
        .filter(|value| *value > 0)
        .ok_or("--max-age-seconds requires a positive integer")?;
    Ok(RepowiseAdaptCli {
        request: request.ok_or("Repowise adapter requires --request")?,
        artifact_root: artifact_root.ok_or("Repowise adapter requires --artifact-root")?,
        evaluated_at: evaluated_at.ok_or("Repowise adapter requires --evaluated-at")?,
        max_age_seconds,
    })
}

fn set_u64(
    slot: &mut Option<u64>,
    value: &str,
    flag: &str,
    adapter: &str,
) -> std::result::Result<(), String> {
    if slot.is_some() {
        return Err(format!("duplicate {adapter} adapter argument: {flag}"));
    }
    *slot = Some(
        value
            .parse()
            .map_err(|_| format!("{flag} requires an unsigned integer"))?,
    );
    Ok(())
}

fn read_repowise_native(path: &str) -> std::result::Result<Value, String> {
    let mut bytes = Vec::new();
    if path == "-" {
        std::io::stdin()
            .take(MAX_REPOWISE_NATIVE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read Repowise native request: {error}"))?;
    } else {
        let path = Path::new(path);
        let metadata = fs::metadata(path)
            .map_err(|error| format!("read Repowise native request metadata: {error}"))?;
        if !metadata.is_file() {
            return Err("Repowise native request must be a regular file".to_string());
        }
        if metadata.len() > MAX_REPOWISE_NATIVE_BYTES {
            return Err("Repowise native request exceeds size limit".to_string());
        }
        bytes = fs::read(path).map_err(|error| format!("read Repowise native request: {error}"))?;
    }
    if bytes.len() as u64 > MAX_REPOWISE_NATIVE_BYTES {
        return Err("Repowise native request exceeds size limit".to_string());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("Repowise native request is not UTF-8: {error}"))?;
    crate::capability::reject_duplicate_json_keys(text)?;
    serde_json::from_str(text)
        .map_err(|error| format!("invalid Repowise native request JSON: {error}"))
}

fn read_provider_native(path: &str, provider: &str) -> std::result::Result<Value, String> {
    let mut bytes = Vec::new();
    if path == "-" {
        std::io::stdin()
            .take(MAX_PROVIDER_NATIVE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read {provider} native request: {error}"))?;
    } else {
        let path = Path::new(path);
        let metadata = fs::metadata(path)
            .map_err(|error| format!("read {provider} native request metadata: {error}"))?;
        if !metadata.is_file() {
            return Err(format!("{provider} native request must be a regular file"));
        }
        if metadata.len() > MAX_PROVIDER_NATIVE_BYTES {
            return Err(format!("{provider} native request exceeds size limit"));
        }
        bytes =
            fs::read(path).map_err(|error| format!("read {provider} native request: {error}"))?;
    }
    if bytes.len() as u64 > MAX_PROVIDER_NATIVE_BYTES {
        return Err(format!("{provider} native request exceeds size limit"));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("{provider} native request is not UTF-8: {error}"))?;
    crate::capability::reject_duplicate_json_keys(text)?;
    serde_json::from_str(text).map_err(|_| format!("invalid {provider} native request JSON"))
}

fn print_repowise_route_rejection(message: &str) -> i32 {
    let result = json!({
        "schema":"code-intel-repowise-route-result.v1",
        "status":"rejected",
        "adapter":null,
        "admissions":[],
        "engineeringFacts":[],
        "diagnostics":[message]
    });
    println!("{}", serde_json::to_string(&result).unwrap());
    eprintln!("{message}");
    65
}

fn rejected_admission(message: &str) -> Value {
    json!({
        "schema":"code-intel-evidence-admissibility-result.v1",
        "status":"rejected",
        "domainVerdict":"unknown",
        "admissionIdentity":null,
        "evidence":null,
        "verifiedPayload":null,
        "engineeringFacts":[],
        "diagnostics":[message]
    })
}

pub fn run(options: &Options<'_>) -> Result<()> {
    let action = options.action.to_ascii_lowercase();
    let value = match action.as_str() {
        "list" => list(options.provider),
        "plan" => plan(options)?,
        "validate" => validate(),
        "invoke" => invoke(options)?,
        other => return Err(format!("unknown provider action: {other}").into()),
    };

    if options.json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print_human(&value);
    }

    if action == "invoke" && value.get("ok").and_then(|value| value.as_bool()) == Some(false) {
        return Err("provider invoke failed".into());
    }

    Ok(())
}

pub fn list(provider: Option<&str>) -> Value {
    let operations: Vec<Value> = OPERATIONS
        .iter()
        .filter(|operation| {
            provider
                .map(|value| value == operation.provider)
                .unwrap_or(true)
        })
        .map(operation_json)
        .collect();

    json!({
        "ok": true,
        "schema": "code-intel-provider-api.v1",
        "operations": operations
    })
}

pub fn plan(options: &Options<'_>) -> Result<Value> {
    let operation = find_required(options)?;
    let command = render_command(operation, options.repo);

    Ok(json!({
        "ok": true,
        "schema": "code-intel-provider-api.v1",
        "operation": operation_json(operation),
        "command": command
    }))
}

pub fn validate() -> Value {
    let mut seen = std::collections::HashSet::new();
    let mut errors = Vec::new();

    for operation in OPERATIONS {
        let key = format!("{}/{}", operation.provider, operation.operation);
        if !seen.insert(key.clone()) {
            errors.push(format!("duplicate provider operation: {key}"));
        }
        if !operation.route.starts_with('/') {
            errors.push(format!("{key} route must start with /"));
        }
        if operation.command_template.trim().is_empty() {
            errors.push(format!("{key} missing command template"));
        }
        if operation.artifact.trim().is_empty() {
            errors.push(format!("{key} missing artifact contract"));
        }
        if operation.command_template.contains("code-nexus-lite.exe")
            && (operation.required || operation.status == "active")
        {
            errors.push(format!(
                "{key} references removed code-nexus-lite.exe but is active or required"
            ));
        }
    }

    validate_codenexus_registry(&mut errors);
    validate_graph_registry(&mut errors);
    validate_sentrux_registry(&mut errors);

    json!({
        "ok": errors.is_empty(),
        "schema": "code-intel-provider-api.v1",
        "operations": OPERATIONS.len(),
        "errors": errors
    })
}

fn validate_sentrux_registry(errors: &mut Vec<String>) {
    let Some(operation) = find("sentrux", "adapt") else {
        errors.push("missing canonical provider operation: sentrux/adapt".to_string());
        return;
    };
    if !operation.required || operation.status != "active" {
        errors.push("sentrux/adapt must be an active required provider route".to_string());
    }
    let (manifest_path, root) = match orchestration_manifest() {
        Ok(value) => value,
        Err(error) => {
            errors.push(error);
            return;
        }
    };
    let manifest: Value = match fs::read_to_string(&manifest_path)
        .map_err(|e| e.to_string())
        .and_then(|text| {
            serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())
        }) {
        Ok(value) => value,
        Err(error) => {
            errors.push(format!(
                "cannot read Sentrux provider manifest {}: {error}",
                manifest_path.display()
            ));
            return;
        }
    };
    let integration = manifest["integrations"].as_array().and_then(|items| {
        items
            .iter()
            .find(|item| item["id"] == "provider.sentrux-adapt")
    });
    let Some(integration) = integration else {
        errors.push("provider binding missing integration: provider.sentrux-adapt".to_string());
        return;
    };
    if integration["required"] != operation.required {
        errors.push("provider.sentrux-adapt required flag drifts from sentrux/adapt".to_string());
    }
    if integration["commands"]["adapt"] != operation.command_template {
        errors.push("provider.sentrux-adapt command drifts from sentrux/adapt".to_string());
    }
    if integration["entrypoint"] != "crates/code-intel-cli/src/providers.rs" {
        errors.push("provider.sentrux-adapt entrypoint is invalid".to_string());
    }
    for path in [
        "orchestration/schemas/code-intel-structural-evidence-port.v1.schema.json",
        "orchestration/schemas/code-intel-sentrux-route-result.v1.schema.json",
        "docs/sentrux-provider-adapter.md",
    ] {
        if !root.join(path).is_file() {
            errors.push(format!(
                "provider.sentrux-adapt contract is missing: {path}"
            ));
        }
    }
}

fn validate_graph_registry(errors: &mut Vec<String>) {
    let Some(operation) = find("graph", "adapt") else {
        errors.push("missing canonical provider operation: graph/adapt".to_string());
        return;
    };
    if !operation.required || operation.status != "active" {
        errors.push("graph/adapt must be an active required provider route".to_string());
    }

    let (manifest_path, root) = match orchestration_manifest() {
        Ok(value) => value,
        Err(error) => {
            errors.push(error);
            return;
        }
    };
    let manifest: Value = match fs::read_to_string(&manifest_path)
        .map_err(|error| error.to_string())
        .and_then(|text| {
            serde_json::from_str(text.trim_start_matches('\u{feff}'))
                .map_err(|error| error.to_string())
        }) {
        Ok(value) => value,
        Err(error) => {
            errors.push(format!(
                "cannot read graph provider manifest {}: {error}",
                manifest_path.display()
            ));
            return;
        }
    };
    let integration = manifest
        .get("integrations")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"] == "provider.graph-adapt")
        });
    let Some(integration) = integration else {
        errors.push("provider binding missing integration: provider.graph-adapt".to_string());
        return;
    };
    if integration["required"] != operation.required {
        errors.push("provider.graph-adapt required flag drifts from graph/adapt".to_string());
    }
    if integration["commands"]["adapt"] != operation.command_template {
        errors.push("provider.graph-adapt command drifts from graph/adapt".to_string());
    }
    if integration["entrypoint"] != "crates/code-intel-cli/src/providers.rs"
        || !root
            .join("crates/code-intel-cli/src/providers.rs")
            .is_file()
    {
        errors.push("provider.graph-adapt entrypoint is missing or invalid".to_string());
    }
    for schema in [
        "code-intel-architecture-graph-port.v1.schema.json",
        "code-intel-graph-route-result.v1.schema.json",
    ] {
        if !root.join("orchestration/schemas").join(schema).is_file() {
            errors.push(format!("provider.graph-adapt schema is missing: {schema}"));
        }
    }
    if !root.join("docs/graph-provider-adapter.md").is_file() {
        errors.push("provider.graph-adapt documentation is missing".to_string());
    }
}

fn validate_codenexus_registry(errors: &mut Vec<String>) {
    let Some(canonical) = find("codenexus", "lite") else {
        errors.push("missing canonical provider operation: codenexus/lite".to_string());
        return;
    };
    if canonical.required || canonical.status != "compatibility" {
        errors.push(
            "codenexus/lite must reflect its non-blocking compatibility runtime policy".to_string(),
        );
    }

    let Some(legacy) = find("repowise", "lite") else {
        errors.push("missing legacy provider operation: repowise/lite".to_string());
        return;
    };
    if legacy.required
        || legacy.status != "compatibility"
        || !legacy.notes.to_ascii_lowercase().contains("deprecated")
    {
        errors.push(
            "repowise/lite must be optional and explicitly deprecated compatibility".to_string(),
        );
    }

    let (manifest_path, root) = match orchestration_manifest() {
        Ok(value) => value,
        Err(error) => {
            errors.push(error);
            return;
        }
    };
    let manifest_text = match fs::read_to_string(&manifest_path) {
        Ok(value) => value,
        Err(error) => {
            errors.push(format!(
                "cannot read orchestration manifest {}: {error}",
                manifest_path.display()
            ));
            return;
        }
    };
    let manifest: Value = match serde_json::from_str(manifest_text.trim_start_matches('\u{feff}')) {
        Ok(value) => value,
        Err(error) => {
            errors.push(format!(
                "cannot parse orchestration manifest {}: {error}",
                manifest_path.display()
            ));
            return;
        }
    };

    validate_codenexus_adapter_integration(&manifest, &root, errors);

    validate_codenexus_integration(
        &manifest,
        &root,
        "localization.codenexus-lite",
        canonical,
        false,
        errors,
    );
    validate_codenexus_integration(
        &manifest,
        &root,
        "runtime.code-nexus-lite",
        legacy,
        false,
        errors,
    );
}

fn validate_codenexus_adapter_integration(manifest: &Value, root: &Path, errors: &mut Vec<String>) {
    let Some(operation) = find("codenexus", "adapt") else {
        errors.push("missing canonical provider operation: codenexus/adapt".to_string());
        return;
    };
    if operation.required
        || operation.status != "active"
        || operation.route != "/api/providers/codenexus/adapt"
        || operation.artifact != "code-intel-codenexus-route-result.v1"
    {
        errors.push("codenexus/adapt provider contract is invalid".to_string());
    }

    let integration = manifest
        .get("integrations")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                item.get("id").and_then(Value::as_str) == Some("provider.codenexus-adapt")
            })
        });
    let Some(integration) = integration else {
        errors.push("provider binding missing integration: provider.codenexus-adapt".to_string());
        return;
    };

    if integration.get("required").and_then(Value::as_bool) != Some(operation.required) {
        errors
            .push("provider.codenexus-adapt required flag drifts from codenexus/adapt".to_string());
    }
    if integration.get("kind").and_then(Value::as_str) != Some("internal-adapter") {
        errors.push("provider.codenexus-adapt kind must be internal-adapter".to_string());
    }
    if integration
        .get("commands")
        .and_then(|commands| commands.get("adapt"))
        .and_then(Value::as_str)
        != Some(operation.command_template)
    {
        errors.push("provider.codenexus-adapt command drifts from codenexus/adapt".to_string());
    }
    let entrypoint = "crates/code-intel-cli/src/providers.rs";
    if integration.get("entrypoint").and_then(Value::as_str) != Some(entrypoint)
        || !root.join(entrypoint).is_file()
    {
        errors.push("provider.codenexus-adapt entrypoint is missing or invalid".to_string());
    }

    let contracts = integration
        .get("artifactContract")
        .and_then(Value::as_array);
    for path in [
        "orchestration/schemas/code-intel-codenexus-port.v1.schema.json",
        "orchestration/schemas/code-intel-codenexus-route-result.v1.schema.json",
        "orchestration/schemas/code-intel-evidence-provider-port.v1.schema.json",
        "orchestration/schemas/code-intel-evidence-admissibility-result.v1.schema.json",
    ] {
        let declared =
            contracts.is_some_and(|items| items.iter().any(|item| item.as_str() == Some(path)));
        if !declared || !root.join(path).is_file() {
            errors.push(format!(
                "provider.codenexus-adapt contract is missing or undeclared: {path}"
            ));
        }
    }
    if !root.join("docs/codenexus-provider-adapter.md").is_file() {
        errors.push("provider.codenexus-adapt documentation is missing".to_string());
    }
}

fn validate_codenexus_integration(
    manifest: &Value,
    root: &Path,
    integration_id: &str,
    operation: &ProviderOperation,
    expected_required: bool,
    errors: &mut Vec<String>,
) {
    let integration = manifest
        .get("integrations")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(integration_id))
        });
    let Some(integration) = integration else {
        errors.push(format!(
            "provider binding missing integration: {integration_id}"
        ));
        return;
    };

    let entrypoint = integration
        .get("entrypoint")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let command = integration
        .get("commands")
        .and_then(|commands| commands.get("compat"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let required = integration
        .get("required")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let kind = integration
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();

    if required != expected_required {
        errors.push(format!(
            "{integration_id} required={required} drifts from provider contract required={expected_required}"
        ));
    }
    if operation.status == "compatibility" && !kind.starts_with("compatibility") {
        errors.push(format!(
            "{integration_id} kind={kind} drifts from compatibility provider status"
        ));
    }
    if entrypoint.is_empty() || !root.join(entrypoint).is_file() {
        errors.push(format!(
            "{integration_id} entrypoint missing from repository: {entrypoint}"
        ));
    }
    let entrypoint_reference = format!(r#"$env:CODE_INTEL_HOME\{entrypoint}"#);
    if !command.contains(&entrypoint_reference) {
        errors.push(format!(
            "{integration_id} command does not invoke its entrypoint: {command}"
        ));
    }
    if command != operation.command_template {
        errors.push(format!(
            "{integration_id} command drifts from {}/{} provider command: {command}",
            operation.provider, operation.operation
        ));
    }
}

fn orchestration_manifest() -> std::result::Result<(PathBuf, PathBuf), String> {
    if let Ok(explicit) = env::var("CODE_INTEL_INTEGRATIONS_MANIFEST") {
        let path = PathBuf::from(explicit);
        let path = if path.is_absolute() {
            path
        } else {
            env::current_dir()
                .map_err(|error| format!("cannot resolve current directory: {error}"))?
                .join(path)
        };
        return manifest_candidate(path).ok_or_else(|| {
            "CODE_INTEL_INTEGRATIONS_MANIFEST does not identify a readable integrations manifest"
                .to_string()
        });
    }

    if let Ok(home) = env::var("CODE_INTEL_HOME") {
        return manifest_candidate(
            PathBuf::from(home)
                .join("orchestration")
                .join("integrations.json"),
        )
        .ok_or_else(|| {
            "CODE_INTEL_HOME does not contain orchestration/integrations.json".to_string()
        });
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            for ancestor in parent.ancestors() {
                if let Some(found) =
                    manifest_candidate(ancestor.join("orchestration").join("integrations.json"))
                {
                    return Ok(found);
                }
            }
        }
    }

    let dev_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if let Some(found) =
        manifest_candidate(dev_root.join("orchestration").join("integrations.json"))
    {
        return Ok(found);
    }

    let cwd =
        env::current_dir().map_err(|error| format!("cannot resolve current directory: {error}"))?;
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join("orchestration").join("integrations.json");
        if is_safe_cwd_manifest(&candidate) {
            return manifest_candidate(candidate)
                .ok_or_else(|| "validated cwd manifest became unavailable".to_string());
        }
    }
    Err("trusted orchestration manifest not found; set CODE_INTEL_HOME or CODE_INTEL_INTEGRATIONS_MANIFEST".to_string())
}

fn manifest_candidate(path: PathBuf) -> Option<(PathBuf, PathBuf)> {
    if !path.is_file() {
        return None;
    }
    let orchestration_dir = path.parent()?;
    if !orchestration_dir
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("orchestration")
    {
        return None;
    }
    let root = orchestration_dir.parent()?.to_path_buf();
    Some((path, root))
}

fn is_safe_cwd_manifest(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
        return false;
    };
    value.pointer("/policy/name").and_then(Value::as_str)
        == Some("code-intel-integration-orchestration")
        && value
            .get("integrations")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("id").and_then(Value::as_str) == Some("runtime.code-intel")
                })
            })
}

pub fn invoke(options: &Options<'_>) -> Result<Value> {
    let operation = find_required(options)?;
    let repo_input = options
        .repo
        .ok_or("provider invoke requires --repo <path>")?;
    let repo = repo_input.canonicalize()?;

    match (operation.provider, operation.operation) {
        ("understand", "graph") => {
            invoke_understand(&repo, options.language, options.full, options.write)
        }
        ("understand", "graph_full") => invoke_understand(&repo, options.language, true, true),
        ("repowise", "status") => invoke_repowise(&repo, "status", &["--no-workspace"]),
        ("repowise", "index") => invoke_repowise_index(&repo),
        (provider, operation) => Err(format!(
            "provider invoke not implemented for {provider}/{operation}; use provider plan for compatibility command"
        )
        .into()),
    }
}

fn invoke_understand(repo: &Path, language: &str, full: bool, write: bool) -> Result<Value> {
    let graph = graph::generate(repo, language, full, write)?;
    Ok(json!({
        "ok": true,
        "schema": "code-intel-provider-api.v1",
        "provider": "understand",
        "operation": if full { "graph_full" } else { "graph" },
        "artifact": graph::graph_path(repo),
        "result": graph
    }))
}

fn invoke_repowise_index(repo: &Path) -> Result<Value> {
    let state_path = repo.join(".repowise").join("state.json");
    let db_path = repo.join(".repowise").join("wiki.db");
    if state_path.exists() || db_path.exists() {
        let update = invoke_repowise(repo, "update", &["--no-workspace", "--index-only"])?;
        let stderr = update
            .get("stderrTail")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if update.get("ok").and_then(|value| value.as_bool()) == Some(false)
            && stderr.contains("No previous sync found")
        {
            return invoke_repowise(
                repo,
                "init",
                &[
                    "--index-only",
                    "--no-agents",
                    "--no-codex",
                    "--no-distill-hook",
                ],
            );
        }
        Ok(update)
    } else {
        invoke_repowise(
            repo,
            "init",
            &[
                "--index-only",
                "--no-agents",
                "--no-codex",
                "--no-distill-hook",
            ],
        )
    }
}

fn invoke_repowise(repo: &Path, subcommand: &str, args: &[&str]) -> Result<Value> {
    let repo_cli = cli_path(repo);
    let mut child = Command::new("repowise")
        .arg(subcommand)
        .args(args)
        .arg(&repo_cli)
        .current_dir(&repo_cli)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(b"n\n")?;
    }

    let output = child.wait_with_output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let ok = output.status.success();

    Ok(json!({
        "ok": ok,
        "schema": "code-intel-provider-api.v1",
        "provider": "repowise",
        "operation": if subcommand == "update" || subcommand == "init" { "index" } else { subcommand },
        "classification": "legacy_diagnostic_or_rollback",
        "evidence": false,
        "factPromotionEligible": false,
        "exitCode": output.status.code().unwrap_or(-1),
        "artifact": repo_cli.join(".repowise").join("wiki.db"),
        "stdoutTail": tail(&stdout, 80),
        "stderrTail": tail(&stderr, 80)
    }))
}

fn cli_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(stripped);
    }
    path.to_path_buf()
}

fn find_required<'a>(options: &Options<'a>) -> Result<&'static ProviderOperation> {
    let provider = options
        .provider
        .ok_or("provider action requires --provider")?;
    let operation = options
        .operation
        .ok_or("provider action requires --operation")?;
    find(provider, operation)
        .ok_or_else(|| format!("unknown provider operation: {provider}/{operation}").into())
}

pub fn find(provider: &str, operation: &str) -> Option<&'static ProviderOperation> {
    OPERATIONS
        .iter()
        .find(|item| item.provider == provider && item.operation == operation)
}

pub fn operation_json(operation: &ProviderOperation) -> Value {
    json!({
        "provider": operation.provider,
        "operation": operation.operation,
        "stage": operation.stage,
        "protocol": operation.protocol,
        "method": operation.method,
        "route": operation.route,
        "commandTemplate": operation.command_template,
        "artifact": operation.artifact,
        "required": operation.required,
        "status": operation.status,
        "sourceSpec": operation.source_spec,
        "notes": operation.notes
    })
}

pub fn render_command(operation: &ProviderOperation, repo: Option<&Path>) -> String {
    match repo {
        Some(repo) if operation.command_template.contains("'<repo-path>'") => operation
            .command_template
            .replace("'<repo-path>'", &powershell_literal(repo)),
        Some(repo) => operation
            .command_template
            .replace("<repo-path>", &repo.to_string_lossy()),
        None => operation.command_template.to_string(),
    }
}

fn powershell_literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

fn print_human(value: &Value) {
    if let Some(operations) = value.get("operations").and_then(|value| value.as_array()) {
        for operation in operations {
            println!(
                "{}:{} {} {} -> {}",
                operation["provider"].as_str().unwrap_or(""),
                operation["operation"].as_str().unwrap_or(""),
                operation["method"].as_str().unwrap_or(""),
                operation["route"].as_str().unwrap_or(""),
                operation["commandTemplate"].as_str().unwrap_or("")
            );
        }
        return;
    }
    println!("{value}");
}

fn tail(value: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = value.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn provider_registry_validates() {
        let result = validate();
        assert_eq!(result["ok"].as_bool(), Some(true));
        assert!(result["operations"].as_u64().unwrap_or(0) >= 6);
    }

    #[test]
    fn repowise_and_understand_share_schema() {
        let repowise = operation_json(find("repowise", "index").unwrap());
        let understand = operation_json(find("understand", "graph").unwrap());

        for key in [
            "provider",
            "operation",
            "protocol",
            "route",
            "commandTemplate",
            "artifact",
            "sourceSpec",
        ] {
            assert!(repowise.get(key).is_some(), "repowise missing {key}");
            assert!(understand.get(key).is_some(), "understand missing {key}");
        }
    }

    #[test]
    fn codenexus_registry_uses_real_adapter_and_deprecates_legacy_alias() {
        let canonical = find("codenexus", "lite").expect("canonical CodeNexus provider");
        assert_eq!(
            canonical.command_template,
            r#"pwsh -NoProfile -File "$env:CODE_INTEL_HOME\Invoke-CodeNexusLite.ps1" -RepoPath '<repo-path>'"#
        );
        assert!(!canonical.required);
        assert_eq!(canonical.status, "compatibility");
        assert!(canonical.notes.contains("non-blocking"));

        let legacy = find("repowise", "lite").expect("legacy Repowise alias");
        assert!(!legacy.required);
        assert_eq!(legacy.status, "compatibility");
        assert!(legacy.notes.to_ascii_lowercase().contains("deprecated"));
    }

    #[test]
    fn codenexus_manifest_command_drift_is_rejected() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = env::temp_dir().join(format!("code-intel-provider-drift-{stamp}"));
        fs::create_dir_all(&root).expect("fixture root");
        fs::write(root.join("Invoke-CodeNexusLite.ps1"), "# fixture\n")
            .expect("fixture entrypoint");
        let manifest = json!({
            "integrations": [{
                "id": "localization.codenexus-lite",
                "required": false,
                "kind": "compatibility-adapter",
                "entrypoint": "Invoke-CodeNexusLite.ps1",
                "commands": {"compat": "target/debug/code-nexus-lite.exe codenexus::lite"}
            }]
        });
        let mut errors = Vec::new();
        validate_codenexus_integration(
            &manifest,
            &root,
            "localization.codenexus-lite",
            find("codenexus", "lite").unwrap(),
            false,
            &mut errors,
        );
        fs::remove_dir_all(&root).expect("fixture cleanup");

        assert!(errors
            .iter()
            .any(|error| error.contains("command does not invoke its entrypoint")));
        assert!(errors
            .iter()
            .any(|error| error.contains("command drifts from codenexus/lite")));
    }

    #[test]
    fn codenexus_adapter_registry_rename_is_rejected() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = env::temp_dir().join(format!("code-intel-codenexus-adapter-drift-{stamp}"));
        fs::create_dir_all(root.join("crates/code-intel-cli/src")).expect("fixture source root");
        fs::create_dir_all(root.join("orchestration/schemas")).expect("fixture schema root");
        fs::create_dir_all(root.join("docs")).expect("fixture docs root");
        fs::write(
            root.join("crates/code-intel-cli/src/providers.rs"),
            "// fixture\n",
        )
        .expect("fixture entrypoint");
        for path in [
            "orchestration/schemas/code-intel-codenexus-port.v1.schema.json",
            "orchestration/schemas/code-intel-codenexus-route-result.v1.schema.json",
            "orchestration/schemas/code-intel-evidence-provider-port.v1.schema.json",
            "orchestration/schemas/code-intel-evidence-admissibility-result.v1.schema.json",
        ] {
            fs::write(root.join(path), "{}\n").expect("fixture schema");
        }
        fs::write(
            root.join("docs/codenexus-provider-adapter.md"),
            "# fixture\n",
        )
        .expect("fixture docs");

        let operation = find("codenexus", "adapt").expect("codenexus adapter operation");
        let manifest = json!({
            "integrations": [{
                "id": "provider.codenexus-adapt-drift",
                "kind": "internal-adapter",
                "required": operation.required,
                "entrypoint": "crates/code-intel-cli/src/providers.rs",
                "commands": {"adapt": operation.command_template},
                "artifactContract": [
                    "orchestration/schemas/code-intel-codenexus-port.v1.schema.json",
                    "orchestration/schemas/code-intel-codenexus-route-result.v1.schema.json",
                    "orchestration/schemas/code-intel-evidence-provider-port.v1.schema.json",
                    "orchestration/schemas/code-intel-evidence-admissibility-result.v1.schema.json"
                ]
            }]
        });
        let mut errors = Vec::new();
        validate_codenexus_adapter_integration(&manifest, &root, &mut errors);
        fs::remove_dir_all(&root).expect("fixture cleanup");

        assert!(
            errors
                .iter()
                .any(|error| error
                    == "provider binding missing integration: provider.codenexus-adapt")
        );
    }

    #[test]
    fn codenexus_plan_quotes_repo_paths_for_powershell() {
        let command = render_command(
            find("codenexus", "lite").unwrap(),
            Some(Path::new(r"D:\work repo\O'Brien")),
        );
        assert_eq!(
            command,
            r#"pwsh -NoProfile -File "$env:CODE_INTEL_HOME\Invoke-CodeNexusLite.ps1" -RepoPath 'D:\work repo\O''Brien'"#
        );
    }
}
