use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use serde_json::Value;

mod admissibility;
mod artifact_index;
mod artifact_ref;
mod artifacts;
mod authority;
mod capability;
mod capability_inventory;
mod change_impact;
mod codenexus_adapter;
mod committed_evidence;
mod compatibility_retirement_ticket;
mod dag_coordinator;
mod dag_run;
mod decision_port;
mod decision_record;
mod evidence_query;
mod file_boundary;
mod graph;
mod method_catalog;
mod model_channels;
mod orchestration;
mod ponytail_gate;
mod project_orientation_benchmark;
mod providers;
mod routes;
mod run_commit;
mod runtime_ci_evidence;
mod sentrux;
mod sentrux_analysis;
mod session_evidence;
mod snapshot;
mod stable_artifact;
mod staged_artifact;
mod survival_scan;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Default)]
struct Args {
    command: String,
    repo: Option<PathBuf>,
    report: Option<PathBuf>,
    artifact_root: Option<PathBuf>,
    steps: Option<PathBuf>,
    failures: Option<PathBuf>,
    out: Option<PathBuf>,
    manifest: Option<PathBuf>,
    capability: Option<String>,
    provider: Option<String>,
    operation: Option<String>,
    action: String,
    mode: String,
    language: String,
    write: bool,
    full: bool,
    json: bool,
}

#[cfg(test)]
mod sentrux_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sentrux_normalize_and_debt_register_classify_known_and_worsened_debt() {
        let report = json!({
            "steps": [
                {
                    "name": "sentrux check",
                    "status": "failed",
                    "output": "run-code-intel.ps1:Get-CodeEvidenceSymbols (cc=412)",
                    "error": ""
                },
                {
                    "name": "sentrux gate",
                    "status": "failed",
                    "output": "Quality:      3508 -> 4316\nCycles:       0 → 0\nComplex functions increased: 7 → 12",
                    "error": ""
                }
            ]
        });

        let failures = normalize_sentrux_failures(&report);
        assert_eq!(failures["schema"], "code-intel-sentrux-failures.v1");
        assert_eq!(failures["status"], "failed");
        assert_eq!(
            failures["primary"]["target"]["symbol"],
            "Get-CodeEvidenceSymbols"
        );

        let debt = build_sentrux_debt_register(&failures, "D:/repo");
        assert_eq!(debt["schema"], "code-intel-sentrux-debt-register.v1");
        assert_eq!(debt["summary"]["knownDebt"], 1);
        assert_eq!(debt["summary"]["newDebt"], 0);
        assert_eq!(debt["summary"]["worsenedDebt"], 2);
        assert_eq!(debt["summary"]["informational"], 1);
        assert_eq!(debt["summary"]["blocking"], 2);
    }

    #[test]
    fn sentrux_debt_register_keeps_aggregate_max_cc_informational() {
        let report = json!({
            "steps": [
                {
                    "name": "sentrux check",
                    "status": "failed",
                    "output": "max_cc exceeded: threshold 70, actual 311",
                    "error": ""
                }
            ]
        });

        let failures = normalize_sentrux_failures(&report);
        assert_eq!(failures["primary"]["target"]["status"], "unresolved");
        let debt = build_sentrux_debt_register(&failures, "D:/repo");
        assert_eq!(debt["summary"]["informational"], 1);
        assert_eq!(debt["summary"]["blocking"], 0);
    }

    #[test]
    fn sentrux_debt_register_blocks_unknown_named_max_cc() {
        let report = json!({
            "steps": [
                {
                    "name": "sentrux check",
                    "status": "failed",
                    "output": "other.ps1:New-BigFunction (cc=101)",
                    "error": ""
                }
            ]
        });

        let failures = normalize_sentrux_failures(&report);
        let debt = build_sentrux_debt_register(&failures, "D:/repo");
        assert_eq!(debt["summary"]["newDebt"], 1);
        assert_eq!(debt["summary"]["blocking"], 1);
    }

    #[test]
    fn raw_routes_preserve_specific_dispatch_precedence_and_argument_offsets() {
        let decision_record = vec!["decision".into(), "record".into(), "--store".into()];
        let decision_default = vec!["decision".into(), "request-response".into()];
        let artifact_index = vec!["artifact".into(), "index".into(), "--repo".into()];

        let record_route = resolve_raw_route(&decision_record).expect("decision record route");
        let default_route = resolve_raw_route(&decision_default).expect("decision default route");
        let artifact_route = resolve_raw_route(&artifact_index).expect("artifact index route");

        assert_eq!(record_route.subcommand, Some("record"));
        assert_eq!(record_route.argument_offset, 1);
        assert_eq!(default_route.subcommand, None);
        assert_eq!(default_route.argument_offset, 1);
        assert_eq!(artifact_route.subcommand, Some("index"));
        assert_eq!(artifact_route.argument_offset, 1);
    }

    #[test]
    fn legacy_parser_commands_are_not_intercepted_by_raw_routes() {
        let doctor = vec!["doctor".into(), "--json".into()];
        let provider_list = vec!["provider".into(), "--action".into(), "List".into()];

        assert!(resolve_raw_route(&doctor).is_none());
        assert!(resolve_raw_route(&provider_list).is_none());
    }
}

#[derive(Debug)]
struct ResumeSummary {
    repo: PathBuf,
    artifact_dir: PathBuf,
    report_path: PathBuf,
    summary_path: Option<PathBuf>,
    understanding_path: Option<PathBuf>,
    hospital_path: Option<PathBuf>,
    hospital_markdown: Option<PathBuf>,
    github_research_path: Option<PathBuf>,
    github_research_markdown: Option<PathBuf>,
    pipeline_failed: i64,
    pipeline_manual_required: i64,
    provider_quota: i64,
    local_tool_error: i64,
    graph_missing: i64,
    sentrux_fail: i64,
    hospital_status: String,
    hospital_disposition: String,
    hospital_next_protocol: String,
    hospital_current_state: String,
    hospital_primary_diagnosis: String,
    research_status: String,
    research_required: bool,
}

fn main() {
    let raw: Vec<String> = env::args().skip(1).collect();
    if let Some(exit_code) = dispatch_raw_command(&raw) {
        process::exit(exit_code);
    }
    if let Err(err) = run() {
        eprintln!("error: {err}");
        process::exit(1);
    }
}

type RawRunner = fn(&[String]) -> i32;

struct RawRoute {
    command: &'static str,
    subcommand: Option<&'static str>,
    argument_offset: usize,
    runner: RawRunner,
}

const RAW_ROUTES: &[RawRoute] = &[
    RawRoute {
        command: "compatibility",
        subcommand: Some("retirement-ticket"),
        argument_offset: 2,
        runner: compatibility_retirement_ticket::run_raw,
    },
    RawRoute {
        command: "provider",
        subcommand: Some("repowise-adapt"),
        argument_offset: 2,
        runner: providers::run_repowise_adapt_raw,
    },
    RawRoute {
        command: "provider",
        subcommand: Some("graph-adapt"),
        argument_offset: 2,
        runner: providers::run_graph_adapt_raw,
    },
    RawRoute {
        command: "provider",
        subcommand: Some("sentrux-adapt"),
        argument_offset: 2,
        runner: providers::run_sentrux_adapt_raw,
    },
    RawRoute {
        command: "provider",
        subcommand: Some("session-adapt"),
        argument_offset: 2,
        runner: session_evidence::run_raw,
    },
    RawRoute {
        command: "provider",
        subcommand: Some("codenexus-adapt"),
        argument_offset: 2,
        runner: providers::run_codenexus_adapt_raw,
    },
    RawRoute {
        command: "provider",
        subcommand: Some("file-boundary"),
        argument_offset: 2,
        runner: run_file_boundary_raw,
    },
    RawRoute {
        command: "provider",
        subcommand: Some("runtime-ci-evidence"),
        argument_offset: 2,
        runner: run_runtime_ci_raw,
    },
    RawRoute {
        command: "repository",
        subcommand: Some("survival-scan"),
        argument_offset: 2,
        runner: survival_scan::run_raw,
    },
    RawRoute {
        command: "artifact",
        subcommand: Some("index"),
        argument_offset: 1,
        runner: artifact_index::run_raw,
    },
    RawRoute {
        command: "artifact",
        subcommand: Some("query"),
        argument_offset: 1,
        runner: evidence_query::run_raw,
    },
    RawRoute {
        command: "change",
        subcommand: Some("impact"),
        argument_offset: 1,
        runner: change_impact::run_raw,
    },
    RawRoute {
        command: "decision",
        subcommand: Some("record"),
        argument_offset: 1,
        runner: decision_record::run_raw,
    },
    RawRoute {
        command: "decision",
        subcommand: Some("replay"),
        argument_offset: 1,
        runner: decision_record::run_raw,
    },
    RawRoute {
        command: "run",
        subcommand: Some("commit"),
        argument_offset: 1,
        runner: run_commit::run_raw,
    },
    RawRoute {
        command: "capability",
        subcommand: None,
        argument_offset: 1,
        runner: capability::run_raw,
    },
    RawRoute {
        command: "model",
        subcommand: None,
        argument_offset: 1,
        runner: model_channels::run_raw,
    },
    RawRoute {
        command: "benchmark",
        subcommand: None,
        argument_offset: 1,
        runner: project_orientation_benchmark::run_raw,
    },
    RawRoute {
        command: "snapshot",
        subcommand: None,
        argument_offset: 1,
        runner: snapshot::run_raw,
    },
    RawRoute {
        command: "evidence",
        subcommand: None,
        argument_offset: 1,
        runner: admissibility::run_raw,
    },
    RawRoute {
        command: "decision",
        subcommand: None,
        argument_offset: 1,
        runner: decision_port::run_raw,
    },
    RawRoute {
        command: "run",
        subcommand: None,
        argument_offset: 1,
        runner: dag_run::run_raw,
    },
    RawRoute {
        command: "governance",
        subcommand: None,
        argument_offset: 1,
        runner: ponytail_gate::run_raw,
    },
];

fn dispatch_raw_command(raw: &[String]) -> Option<i32> {
    let route = resolve_raw_route(raw)?;
    Some((route.runner)(&raw[route.argument_offset..]))
}

fn resolve_raw_route(raw: &[String]) -> Option<&'static RawRoute> {
    let command = raw.first()?;
    RAW_ROUTES.iter().find(|route| {
        route.command == command
            && route.subcommand.map_or(true, |subcommand| {
                raw.get(1).map(String::as_str) == Some(subcommand)
            })
    })
}

fn raw_option(raw: &[String], name: &str) -> std::result::Result<PathBuf, String> {
    let positions = raw
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (value == name).then_some(index))
        .collect::<Vec<_>>();
    if positions.len() != 1 {
        return Err(format!("{name} must appear exactly once"));
    }
    raw.get(positions[0] + 1)
        .filter(|value| !value.starts_with("--"))
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} requires a value"))
}

fn write_provider_result(out: &Path, value: &Value) -> std::result::Result<(), String> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create output directory: {error}"))?;
    }
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("serialize output: {error}"))?;
    fs::write(out, bytes).map_err(|error| format!("write output: {error}"))
}

fn run_file_boundary_raw(raw: &[String]) -> i32 {
    let result = (|| -> std::result::Result<(), String> {
        if raw.len() != 4 {
            return Err("file-boundary requires --request <path> --out <path>".into());
        }
        let request = raw_option(raw, "--request")?;
        let out = raw_option(raw, "--out")?;
        let bytes = fs::read(request).map_err(|error| format!("read request: {error}"))?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| "file boundary request must be UTF-8 JSON")?;
        capability::reject_duplicate_json_keys(text)?;
        let value: Value =
            serde_json::from_str(text).map_err(|error| format!("parse request: {error}"))?;
        write_provider_result(&out, &file_boundary::resolve(&value)?)
    })();
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            65
        }
    }
}

fn run_runtime_ci_raw(raw: &[String]) -> i32 {
    let result = (|| -> std::result::Result<(), String> {
        if raw.len() != 6 {
            return Err(
                "runtime-ci-evidence requires --artifact-root <path> --request <path> --out <path>"
                    .into(),
            );
        }
        let artifact_root = raw_option(raw, "--artifact-root")?;
        let request = raw_option(raw, "--request")?;
        let out = raw_option(raw, "--out")?;
        let bytes = fs::read(request).map_err(|error| format!("read request: {error}"))?;
        let value = runtime_ci_evidence::parse_request_bytes(&bytes)?;
        write_provider_result(
            &out,
            &runtime_ci_evidence::ingest_request(&artifact_root, &value)?,
        )
    })();
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            65
        }
    }
}

fn run() -> Result<()> {
    let args = parse_args(env::args().skip(1).collect())?;
    match args.command.as_str() {
        "resume" => cmd_resume(&args),
        "classify" => cmd_classify(&args),
        "doctor" => cmd_doctor(&args),
        "sentrux-normalize" => cmd_sentrux_normalize(&args),
        "sentrux-debt-register" => cmd_sentrux_debt_register(&args),
        "graph" | "understand" => cmd_graph(&args),
        "orchestrate" | "orchestration" => cmd_orchestrate(&args),
        "provider" | "providers" => cmd_provider(&args),
        "route" | "routes" => cmd_route(&args),
        "sentrux" => cmd_sentrux(&args),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => Err(format!("unknown command: {other}").into()),
    }
}

fn parse_args(raw: Vec<String>) -> Result<Args> {
    if raw.is_empty() {
        return Ok(help_args());
    }

    let mut args = command_args(raw[0].clone());
    let mut i = 1usize;
    while i < raw.len() {
        i += parse_next_arg(&raw, i, &mut args)?;
    }
    Ok(args)
}

fn help_args() -> Args {
    Args {
        command: "help".to_string(),
        ..Args::default()
    }
}

fn command_args(command: String) -> Args {
    Args {
        command,
        action: "Validate".to_string(),
        mode: "normal".to_string(),
        language: "zh".to_string(),
        ..Args::default()
    }
}

fn parse_next_arg(raw: &[String], index: usize, args: &mut Args) -> Result<usize> {
    let token = raw[index].as_str();
    if set_path_arg(raw, index, args, token)? {
        return Ok(2);
    }
    if set_string_arg(raw, index, args, token)? {
        return Ok(2);
    }
    if set_switch_arg(args, token) {
        return Ok(1);
    }
    if set_sentrux_positional(args, token) {
        return Ok(1);
    }
    Err(format!("unknown argument for {}: {token}", args.command).into())
}

fn set_path_arg(raw: &[String], index: usize, args: &mut Args, flag: &str) -> Result<bool> {
    if flag == "--repo" {
        args.repo = Some(path_value(raw, index, flag)?);
        return Ok(true);
    }
    if flag == "--report" {
        args.report = Some(path_value(raw, index, flag)?);
        return Ok(true);
    }
    if flag == "--steps" {
        args.steps = Some(path_value(raw, index, flag)?);
        return Ok(true);
    }
    if flag == "--failures" {
        args.failures = Some(path_value(raw, index, flag)?);
        return Ok(true);
    }
    if flag == "--out" {
        args.out = Some(path_value(raw, index, flag)?);
        return Ok(true);
    }
    if flag == "--artifact-root" {
        args.artifact_root = Some(path_value(raw, index, flag)?);
        return Ok(true);
    }
    if flag == "--manifest" {
        args.manifest = Some(path_value(raw, index, flag)?);
        return Ok(true);
    }
    Ok(false)
}

fn set_string_arg(raw: &[String], index: usize, args: &mut Args, flag: &str) -> Result<bool> {
    if flag == "--capability" {
        args.capability = Some(required_value(raw, index + 1, flag)?);
        return Ok(true);
    }
    if flag == "--provider" {
        args.provider = Some(required_value(raw, index + 1, flag)?);
        return Ok(true);
    }
    if flag == "--operation" {
        args.operation = Some(required_value(raw, index + 1, flag)?);
        return Ok(true);
    }
    if flag == "--action" {
        args.action = required_value(raw, index + 1, flag)?;
        return Ok(true);
    }
    if flag == "--mode" {
        args.mode = required_value(raw, index + 1, flag)?;
        return Ok(true);
    }
    if flag == "--language" {
        args.language = required_value(raw, index + 1, flag)?;
        return Ok(true);
    }
    Ok(false)
}

fn set_switch_arg(args: &mut Args, flag: &str) -> bool {
    match flag {
        "--write" => args.write = true,
        "--full" => args.full = true,
        "--json" => args.json = true,
        "--help" | "-h" => args.command = "help".to_string(),
        _ => return false,
    }
    true
}

fn set_sentrux_positional(args: &mut Args, value: &str) -> bool {
    if args.command != "sentrux" {
        return false;
    }
    if args.operation.is_none() {
        args.operation = Some(value.to_string());
        return true;
    }
    if args.repo.is_none() {
        args.repo = Some(PathBuf::from(value));
        return true;
    }
    false
}

fn path_value(raw: &[String], index: usize, flag: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(required_value(raw, index + 1, flag)?))
}
fn required_value(raw: &[String], index: usize, flag: &str) -> Result<String> {
    raw.get(index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value").into())
}

fn cmd_resume(args: &Args) -> Result<()> {
    let repo = args
        .repo
        .as_ref()
        .ok_or("resume requires --repo <path>")?
        .to_path_buf();
    artifacts::resume(&repo, args.artifact_root.as_deref(), args.json)
}

fn cmd_classify(args: &Args) -> Result<()> {
    let report_path = args
        .report
        .as_ref()
        .ok_or("classify requires --report <path>")?;
    let report = read_json(report_path)?;
    let policy = classify_report_policy(&report);
    if args.json {
        let out = serde_json::json!({
            "report": report_path,
            "failureCategories": {
                "providerQuota": policy.provider_quota,
                "localToolError": policy.local_tool_error,
                "graphMissing": policy.graph_missing,
                "sentruxFail": policy.sentrux_fail
            },
            "effectiveFailureCategories": {
                "providerQuota": policy.effective_provider_quota,
                "localToolError": policy.effective_local_tool_error,
                "graphMissing": policy.effective_graph_missing,
                "sentruxFail": policy.effective_sentrux_fail
            },
            "blockingSentruxDebt": policy.blocking_sentrux_debt,
            "knownSentruxDebt": policy.known_sentrux_debt,
            "knownDebtOnly": policy.known_debt_only,
            "pipelineBlocking": policy.pipeline_blocking,
            "githubResearchRequired": policy.github_research_required,
            "nextProtocol": policy.next_protocol,
            "exitCode": policy.exit_code
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("Report: {}", report_path.display());
        println!("providerQuota={}", policy.provider_quota);
        println!("localToolError={}", policy.local_tool_error);
        println!("graphMissing={}", policy.graph_missing);
        println!("sentruxFail={}", policy.sentrux_fail);
        println!("effectiveSentruxFail={}", policy.effective_sentrux_fail);
        println!("blockingSentruxDebt={}", policy.blocking_sentrux_debt);
        println!("knownSentruxDebt={}", policy.known_sentrux_debt);
        println!("knownDebtOnly={}", policy.known_debt_only);
        println!("pipelineBlocking={}", policy.pipeline_blocking);
        println!("githubResearchRequired={}", policy.github_research_required);
        println!("nextProtocol={}", policy.next_protocol);
        println!("exitCode={}", policy.exit_code);
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ClassifyPolicy {
    provider_quota: i64,
    local_tool_error: i64,
    graph_missing: i64,
    sentrux_fail: i64,
    effective_provider_quota: i64,
    effective_local_tool_error: i64,
    effective_graph_missing: i64,
    effective_sentrux_fail: i64,
    blocking_sentrux_debt: i64,
    known_sentrux_debt: i64,
    known_debt_only: bool,
    pipeline_blocking: bool,
    github_research_required: bool,
    next_protocol: String,
    exit_code: i64,
}

fn classify_report_policy(report: &Value) -> ClassifyPolicy {
    let provider_quota = int_at(report, &["summary", "failureCategories", "providerQuota"]);
    let local_tool_error = int_at(report, &["summary", "failureCategories", "localToolError"]);
    let graph_missing = int_at(report, &["summary", "failureCategories", "graphMissing"]);
    let sentrux_fail = int_at(report, &["summary", "failureCategories", "sentruxFail"]);

    let effective_provider_quota = effective_category(report, "providerQuota", provider_quota);
    let effective_local_tool_error = effective_category(report, "localToolError", local_tool_error);
    let effective_graph_missing = effective_category(report, "graphMissing", graph_missing);
    let effective_sentrux_fail = effective_category(report, "sentruxFail", sentrux_fail);
    let blocking_sentrux_debt = int_at(report, &["summary", "blockingSentruxDebt"]);
    let known_sentrux_debt = int_at(report, &["summary", "knownSentruxDebt"]);

    let pipeline_blocking = effective_provider_quota > 0
        || effective_local_tool_error > 0
        || effective_sentrux_fail > 0;
    let github_research_required = pipeline_blocking;
    let known_debt_only = sentrux_fail > 0
        && effective_sentrux_fail == 0
        && blocking_sentrux_debt == 0
        && known_sentrux_debt > 0;
    let next_protocol = string_at(report, &["hospital", "nextProtocol"])
        .or_else(|| string_at(report, &["hospital", "next_protocol"]))
        .unwrap_or_else(|| {
            if github_research_required {
                "github_solution_research".to_string()
            } else if effective_graph_missing > 0 {
                "understanding".to_string()
            } else {
                "understanding".to_string()
            }
        });
    let exit_code = if pipeline_blocking { 1 } else { 0 };

    ClassifyPolicy {
        provider_quota,
        local_tool_error,
        graph_missing,
        sentrux_fail,
        effective_provider_quota,
        effective_local_tool_error,
        effective_graph_missing,
        effective_sentrux_fail,
        blocking_sentrux_debt,
        known_sentrux_debt,
        known_debt_only,
        pipeline_blocking,
        github_research_required,
        next_protocol,
        exit_code,
    }
}

fn effective_category(report: &Value, name: &str, fallback: i64) -> i64 {
    let value = int_at(report, &["summary", "effectiveFailureCategories", name]);
    if value > 0
        || report
            .get("summary")
            .and_then(|summary| summary.get("effectiveFailureCategories"))
            .is_some()
    {
        value
    } else {
        fallback
    }
}

fn cmd_sentrux_normalize(args: &Args) -> Result<()> {
    let steps_path = args
        .steps
        .as_ref()
        .or(args.report.as_ref())
        .ok_or("sentrux-normalize requires --steps <report-or-steps-json>")?;
    let input = read_json(steps_path)?;
    let artifact = normalize_sentrux_failures(&input);
    write_or_print_json(args.out.as_ref(), &artifact)
}

fn cmd_sentrux_debt_register(args: &Args) -> Result<()> {
    let failures_path = args
        .failures
        .as_ref()
        .ok_or("sentrux-debt-register requires --failures <sentrux-failures.json>")?;
    let failures = read_json(failures_path)?;
    let repo = args
        .repo
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let artifact = build_sentrux_debt_register(&failures, &repo);
    write_or_print_json(args.out.as_ref(), &artifact)
}

fn normalize_sentrux_failures(input: &Value) -> Value {
    let steps: Vec<Value> = match input.get("steps").and_then(Value::as_array) {
        Some(steps) => steps.clone(),
        None => input.as_array().cloned().unwrap_or_default(),
    };
    let mut records = Vec::new();
    let mut parser_errors = Vec::new();

    for step in steps
        .iter()
        .filter(|step| step_name(step).starts_with("sentrux"))
    {
        let name = step_name(step);
        let status = string_at(step, &["status"]).unwrap_or_default();
        let text = step_text(step);
        if status != "failed" && status != "manual_required" {
            continue;
        }

        if name == "sentrux check" {
            if let Some((file, symbol, cc)) = parse_named_max_cc(&text) {
                records.push(serde_json::json!({
                    "id": format!("check:max_cc:{file}:{symbol}"),
                    "kind": "max_cc",
                    "source": "sentrux check",
                    "source_step": "sentrux check",
                    "provenance": "stdout",
                    "raw_output_path": "report.json#/steps/sentrux check/output",
                    "stdout_excerpt": bounded_excerpt(&text),
                    "parsed_at": generated_at(),
                    "target": {
                        "status": "resolved",
                        "file": file,
                        "symbol": symbol
                    },
                    "metric": "max_cc",
                    "value": cc,
                    "threshold": 70
                }));
            } else if let Some(cc) = parse_aggregate_max_cc(&text) {
                records.push(serde_json::json!({
                    "id": "check:max_cc:unresolved",
                    "kind": "max_cc",
                    "source": "sentrux check",
                    "source_step": "sentrux check",
                    "provenance": "stdout",
                    "raw_output_path": "report.json#/steps/sentrux check/output",
                    "stdout_excerpt": bounded_excerpt(&text),
                    "parsed_at": generated_at(),
                    "target": { "status": "unresolved", "file": "", "symbol": "" },
                    "metric": "max_cc",
                    "value": cc,
                    "threshold": 70
                }));
            } else {
                parser_errors
                    .push("sentrux check failed but stdout did not match known max_cc formats.");
            }
        } else if name.starts_with("sentrux gate") {
            let gate_records = parse_gate_records(&text);
            if gate_records.is_empty() && status == "manual_required" {
                records.push(serde_json::json!({
                    "id": "gate:manual_required",
                    "kind": "manual_required",
                    "source": "sentrux gate",
                    "source_step": "sentrux gate",
                    "provenance": "stdout",
                    "raw_output_path": "report.json#/steps/sentrux gate/output",
                    "stdout_excerpt": bounded_excerpt(&text),
                    "parsed_at": generated_at(),
                    "target": { "status": "not_applicable", "file": "", "symbol": "" }
                }));
            } else if gate_records.is_empty() {
                parser_errors.push(
                    "sentrux gate failed but stdout did not match known gate regression formats.",
                );
            } else {
                records.extend(gate_records);
            }
        }
    }

    let skipped = steps.iter().any(|step| {
        step_name(step).starts_with("sentrux")
            && string_at(step, &["status"]).as_deref() == Some("skipped")
    });
    let manual = steps.iter().any(|step| {
        step_name(step).starts_with("sentrux")
            && string_at(step, &["status"]).as_deref() == Some("manual_required")
    });
    let status = if manual {
        "manual_required"
    } else if !records.is_empty() && parser_errors.is_empty() {
        "failed"
    } else if !records.is_empty() {
        "partial"
    } else if !parser_errors.is_empty() {
        "unparsed"
    } else if skipped {
        "skipped"
    } else {
        "not_run"
    };

    let primary = records
        .iter()
        .find(|record| record.get("source").and_then(Value::as_str) == Some("sentrux check"))
        .cloned()
        .unwrap_or(Value::Null);
    let gate = records
        .iter()
        .find(|record| record.get("source").and_then(Value::as_str) == Some("sentrux gate"))
        .cloned()
        .unwrap_or(Value::Null);

    serde_json::json!({
        "schema": "code-intel-sentrux-failures.v1",
        "status": status,
        "generatedAt": generated_at(),
        "primary": primary,
        "gate": gate,
        "records": records,
        "conflicts": [],
        "parser": {
            "status": if parser_errors.is_empty() { "ok" } else { "partial" },
            "notes": [],
            "errors": parser_errors,
            "enrichment": { "hotspots": "", "fileDetails": "" }
        }
    })
}

fn build_sentrux_debt_register(failures: &Value, repo: &str) -> Value {
    let mut entries = Vec::new();
    if let Some(records) = failures.get("records").and_then(Value::as_array) {
        for record in records {
            let (classification, reason) = classify_sentrux_record(record);
            let blocking = classification == "new_debt" || classification == "worsened_debt";
            entries.push(serde_json::json!({
                "id": string_at(record, &["id"]).unwrap_or_default(),
                "classification": classification,
                "blocking": blocking,
                "reason": reason,
                "firstSeen": generated_at(),
                "source": string_at(record, &["source"]).unwrap_or_default(),
                "kind": string_at(record, &["kind"]).unwrap_or_default(),
                "value": record.get("value").cloned().unwrap_or(Value::Null),
                "threshold": record.get("threshold").cloned().unwrap_or(Value::Null),
                "before": record.get("before").cloned().unwrap_or(Value::Null),
                "after": record.get("after").cloned().unwrap_or(Value::Null),
                "target": record.get("target").cloned().unwrap_or_else(|| serde_json::json!({
                    "status": "not_applicable",
                    "file": "",
                    "symbol": ""
                }))
            }));
        }
    }

    if entries.is_empty() {
        let status = string_at(failures, &["status"]).unwrap_or_else(|| "not_run".to_string());
        if ["manual_required", "skipped", "unparsed", "not_run"].contains(&status.as_str()) {
            entries.push(serde_json::json!({
                "id": "",
                "classification": "informational",
                "blocking": false,
                "reason": format!("Sentrux status '{status}' does not represent actionable structural debt."),
                "firstSeen": generated_at(),
                "source": "",
                "kind": "",
                "value": Value::Null,
                "threshold": Value::Null,
                "before": Value::Null,
                "after": Value::Null,
                "target": { "status": "not_applicable", "file": "", "symbol": "" }
            }));
        }
    }

    let count = |name: &str| {
        entries
            .iter()
            .filter(|entry| entry.get("classification").and_then(Value::as_str) == Some(name))
            .count()
    };
    let blocking = entries
        .iter()
        .filter(|entry| {
            entry
                .get("blocking")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();

    serde_json::json!({
        "schema": "code-intel-sentrux-debt-register.v1",
        "generatedAt": generated_at(),
        "repoPath": repo,
        "source": "sentrux-failures.json",
        "policy": {
            "knownDebtBlocks": false,
            "blockingClassifications": ["new_debt", "worsened_debt"],
            "informationalClassifications": ["informational"]
        },
        "summary": {
            "knownDebt": count("known_debt"),
            "newDebt": count("new_debt"),
            "worsenedDebt": count("worsened_debt"),
            "informational": count("informational"),
            "blocking": blocking
        },
        "entries": entries
    })
}

fn classify_sentrux_record(record: &Value) -> (&'static str, &'static str) {
    let source = string_at(record, &["source"]).unwrap_or_default();
    let kind = string_at(record, &["kind"]).unwrap_or_default();
    let target_status = string_at(record, &["target", "status"]).unwrap_or_default();
    let target_file = string_at(record, &["target", "file"]).unwrap_or_default();
    let target_symbol = string_at(record, &["target", "symbol"]).unwrap_or_default();

    if ["manual_required", "skipped", "unparsed"].contains(&kind.as_str())
        || target_status == "not_applicable"
    {
        return (
            "informational",
            "Sentrux record is not an actionable structural debt target.",
        );
    }

    if source == "sentrux check"
        && kind == "max_cc"
        && target_status == "resolved"
        && target_file == "run-code-intel.ps1"
        && target_symbol == "Get-CodeEvidenceSymbols"
    {
        return (
            "known_debt",
            "Current pipeline historical max_cc debt; tracked but not blocking understanding artifacts.",
        );
    }

    if source == "sentrux check" && kind == "max_cc" && target_status == "unresolved" {
        return (
            "informational",
            "Aggregate max_cc output has no authoritative symbol target; do not invent a debt owner.",
        );
    }

    let before = record.get("before").and_then(Value::as_i64);
    let after = record.get("after").and_then(Value::as_i64);
    if source == "sentrux gate" {
        if let (Some(before), Some(after)) = (before, after) {
            if after > before {
                return (
                    "worsened_debt",
                    "Sentrux gate reports a structural metric increased in this run.",
                );
            }
            return (
                "informational",
                "Sentrux gate metric did not increase in this run.",
            );
        }
    }

    if source == "sentrux gate" || source == "sentrux check" {
        return (
            "new_debt",
            "Sentrux reported a structural failure not matched by known historical debt policy.",
        );
    }

    (
        "informational",
        "Sentrux status is informational for blocking policy.",
    )
}

fn step_name(step: &Value) -> String {
    string_at(step, &["name"]).unwrap_or_default()
}

fn step_text(step: &Value) -> String {
    let output = string_at(step, &["output"]).unwrap_or_default();
    let error = string_at(step, &["error"]).unwrap_or_default();
    format!("{output}\n{error}").trim().to_string()
}

fn parse_named_max_cc(text: &str) -> Option<(String, String, i64)> {
    for token in text.split_whitespace() {
        if let Some((left, right)) = token.split_once(":") {
            if !left.contains('.') {
                continue;
            }
            let symbol = right.trim_end_matches(',');
            let tail = text.split(symbol).nth(1).unwrap_or_default();
            if let Some(cc) = parse_cc_value(tail) {
                return Some((left.to_string(), symbol.to_string(), cc));
            }
        }
    }
    None
}

fn parse_aggregate_max_cc(text: &str) -> Option<i64> {
    let lower = text.to_ascii_lowercase();
    if !lower.contains("max_cc") && !lower.contains("max cc") && !lower.contains("cyclomatic") {
        return None;
    }
    parse_last_i64(text)
}

fn parse_cc_value(text: &str) -> Option<i64> {
    let start = text.find("cc=")?;
    let after = &text[start + 3..];
    let digits: String = after.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn parse_last_i64(text: &str) -> Option<i64> {
    let mut current = String::new();
    let mut last = None;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            last = current.parse::<i64>().ok();
            current.clear();
        }
    }
    if !current.is_empty() {
        last = current.parse::<i64>().ok();
    }
    last
}

fn parse_gate_records(text: &str) -> Vec<Value> {
    let mut records = Vec::new();
    for line in text.lines() {
        let Some((label, before, after)) = parse_gate_line(line) else {
            continue;
        };
        let kind = label.to_ascii_lowercase().replace(' ', "_");
        records.push(serde_json::json!({
            "id": format!("gate:{kind}"),
            "kind": kind,
            "source": "sentrux gate",
            "source_step": "sentrux gate",
            "provenance": "stdout",
            "raw_output_path": "report.json#/steps/sentrux gate/output",
            "stdout_excerpt": bounded_excerpt(text),
            "parsed_at": generated_at(),
            "target": { "status": "aggregate", "file": "", "symbol": "" },
            "before": before,
            "after": after
        }));
    }
    records
}

fn parse_gate_line(line: &str) -> Option<(String, i64, i64)> {
    let clean = line.trim().trim_start_matches('✗').trim();
    let (label, rest) = clean.split_once(':')?;
    let label = label.trim();
    if !["God files", "Cycles", "Quality"].contains(&label)
        && !label.starts_with("Complex functions")
        && !label.starts_with("Coupling")
    {
        return None;
    }
    let label = if label.starts_with("Complex functions") {
        "Complex functions"
    } else {
        label
    };
    let arrow = if rest.contains("->") { "->" } else { "→" };
    let (before, after) = rest.split_once(arrow)?;
    Some((
        label.to_string(),
        parse_last_i64(before)?,
        parse_last_i64(after)?,
    ))
}

fn bounded_excerpt(text: &str) -> String {
    let trimmed = text.trim();
    trimmed.chars().take(500).collect()
}

fn generated_at() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn write_or_print_json(path: Option<&PathBuf>, value: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    if let Some(path) = path {
        fs::write(path, text)?;
    } else {
        println!("{text}");
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str().map(ToString::to_string)
}

fn int_at(value: &Value, path: &[&str]) -> i64 {
    let mut current = value;
    for segment in path {
        current = match current.get(*segment) {
            Some(value) => value,
            None => return 0,
        };
    }
    current.as_i64().unwrap_or(0)
}

fn bool_at(value: &Value, path: &[&str]) -> bool {
    let mut current = value;
    for segment in path {
        current = match current.get(*segment) {
            Some(value) => value,
            None => return false,
        };
    }
    current.as_bool().unwrap_or(false)
}

fn existing_path(dir: &Path, file_name: &str) -> Option<PathBuf> {
    let path = dir.join(file_name);
    path.exists().then_some(path)
}

fn string_path(value: &Value, path: &[&str]) -> Option<PathBuf> {
    string_at(value, path).and_then(|s| {
        if s.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(s))
        }
    })
}

fn string_first(values: &[&Value], paths: &[&[&str]]) -> String {
    for value in values {
        for path in paths {
            if let Some(text) = string_at(value, path) {
                if !text.trim().is_empty() {
                    return text;
                }
            }
        }
    }
    String::new()
}

#[cfg(test)]
fn build_resume_summary(
    repo: PathBuf,
    artifact_dir: PathBuf,
    report_path: PathBuf,
    report: &Value,
    hospital: &Value,
) -> ResumeSummary {
    ResumeSummary {
        repo,
        artifact_dir: artifact_dir.clone(),
        report_path,
        summary_path: existing_path(&artifact_dir, "summary.md"),
        understanding_path: existing_path(&artifact_dir, "understanding.md"),
        hospital_path: string_path(report, &["hospital", "path"]),
        hospital_markdown: string_path(report, &["hospital", "markdown"]),
        github_research_path: string_path(report, &["githubResearch", "path"]),
        github_research_markdown: string_path(report, &["githubResearch", "markdown"]),
        pipeline_failed: int_at(report, &["summary", "failed"]),
        pipeline_manual_required: int_at(report, &["summary", "manualRequired"]),
        provider_quota: int_at(report, &["summary", "failureCategories", "providerQuota"]),
        local_tool_error: int_at(report, &["summary", "failureCategories", "localToolError"]),
        graph_missing: int_at(report, &["summary", "failureCategories", "graphMissing"]),
        sentrux_fail: int_at(report, &["summary", "failureCategories", "sentruxFail"]),
        hospital_status: string_first(
            &[hospital, report],
            &[&["triage", "status"], &["hospital", "status"]],
        ),
        hospital_disposition: string_first(
            &[hospital, report],
            &[&["triage", "disposition"], &["hospital", "disposition"]],
        ),
        hospital_next_protocol: string_first(
            &[hospital, report],
            &[&["triage", "next_protocol"], &["hospital", "nextProtocol"]],
        ),
        hospital_current_state: string_first(
            &[hospital, report],
            &[
                &["state_machine", "current_state"],
                &["hospital", "currentState"],
            ],
        ),
        hospital_primary_diagnosis: string_first(
            &[hospital, report],
            &[
                &["triage", "primary_diagnosis"],
                &["hospital", "primaryDiagnosis"],
            ],
        ),
        research_status: string_at(report, &["githubResearch", "status"])
            .or_else(|| string_at(hospital, &["triage", "research_status"]))
            .unwrap_or_else(|| "not_applicable".to_string()),
        research_required: bool_at(report, &["githubResearch", "required"])
            || bool_at(hospital, &["triage", "research_required"]),
    }
}

#[cfg(test)]
fn next_read(summary: &ResumeSummary) -> PathBuf {
    if summary.research_required {
        if let Some(path) = &summary.github_research_markdown {
            if !path.as_os_str().is_empty() {
                return path.clone();
            }
        }
    }
    match summary.hospital_next_protocol.as_str() {
        "surgery_plan" => summary.artifact_dir.join("surgery-plan.md"),
        "github_solution_research" => summary
            .github_research_markdown
            .clone()
            .unwrap_or_else(|| summary.artifact_dir.join("github-solution-research.md")),
        _ => summary
            .understanding_path
            .clone()
            .or_else(|| summary.hospital_markdown.clone())
            .unwrap_or_else(|| summary.report_path.clone()),
    }
}

fn cmd_doctor(args: &Args) -> Result<()> {
    artifacts::doctor(args.artifact_root.as_deref(), args.json)
}

fn cmd_graph(args: &Args) -> Result<()> {
    let repo = args.repo.as_ref().ok_or("graph requires --repo <path>")?;
    graph::run(&graph::Options {
        repo,
        language: &args.language,
        full: args.full,
        write: args.write,
        json: args.json,
    })
}

fn cmd_orchestrate(args: &Args) -> Result<()> {
    orchestration::run(&orchestration::Options {
        action: &args.action,
        mode: &args.mode,
        manifest: args.manifest.as_deref(),
        capability: args.capability.as_deref(),
        repo: args.repo.as_deref(),
        json: args.json,
    })
}

fn cmd_provider(args: &Args) -> Result<()> {
    providers::run(&providers::Options {
        action: &args.action,
        provider: args.provider.as_deref(),
        operation: args.operation.as_deref(),
        repo: args.repo.as_deref(),
        language: &args.language,
        full: args.full,
        write: args.write || args.operation.as_deref().unwrap_or("") == "graph",
        json: args.json,
    })
}

fn cmd_route(args: &Args) -> Result<()> {
    routes::run(&routes::Options {
        action: &args.action,
        provider: args.provider.as_deref(),
        operation: args.operation.as_deref(),
        repo: args.repo.as_deref(),
        json: args.json,
    })
}

fn cmd_sentrux(args: &Args) -> Result<()> {
    sentrux::run(&sentrux::Options {
        operation: args.operation.as_deref(),
        repo: args.repo.as_deref(),
    })
}

fn print_help() {
    println!("code-intel <command> [options]");
    println!();
    println!("Commands:");
    println!("  resume --repo <path> [--artifact-root <path>] [--json]");
    println!("  classify --report <path> [--json]");
    println!("  sentrux-normalize --steps <report.json> [--out <sentrux-failures.json>]");
    println!("  sentrux-debt-register --failures <sentrux-failures.json> [--repo <path>] [--out <sentrux-debt-register.json>]");
    println!("  doctor [--artifact-root <path>] [--json]");
    println!("  graph --repo <path> [--language zh] [--full] [--write] [--json]");
    println!("  provider [--action List|Plan|Validate|Invoke] [--provider repowise|understand] [--operation <name>] [--repo <path>] [--language zh] [--write] [--json]");
    println!("  provider repowise-adapt --request <native.json|-> --artifact-root <directory> --evaluated-at <unix-seconds> --max-age-seconds <seconds>");
    println!("  provider graph-adapt --request <native.json|-> --artifact-root <directory> --evaluated-at <unix-seconds> --max-age-seconds <seconds>");
    println!("  provider sentrux-adapt --request <native.json|-> --artifact-root <directory> --evaluated-at <unix-seconds> --max-age-seconds <seconds>");
    println!("  provider session-adapt --repo <repo> --trace <mindwalk-trace.json> [--hotspots <sentrux-hotspots-or-dsm.json>] [--out <session-evidence.json>] [--working-tree-policy head_only|explicit_overlay]");
    println!("  provider file-boundary --request <request.json> --out <result.json>");
    println!("  provider runtime-ci-evidence --artifact-root <directory> --request <request.json> --out <summary.json>");
    println!("  route [--action List|Plan|Validate] [--provider repowise|understand] [--operation <name>] [--repo <path>] [--json]");
    println!("  sentrux <dsm|scan|health|check|gate|check_rules|gate_save> <path>");
    println!("  capability exec <id> --request <request.json|-> --out <staging-dir> [--artifact-root <directory>] [--manifest <integrations.json>]");
    println!("  model inventory-validate --request <inventory.json> [--out <validated.json>]");
    println!("  model route --request <routing-request.json> [--out <routing-result.json>]");
    println!("  snapshot identity --repo <root> --working-tree-policy <head_only|explicit_overlay> [--scope <relative-path>]...");
    println!("  evidence validate --request <request.json> --artifact-root <directory>");
    println!("  artifact index --artifact-root <root> [--output <index.json>] [--operation rebuild|incremental] [--existing <index.json>]");
    println!("  artifact query --artifact-root <root> --repo <name> [--repo-path <path>] [--artifact-schema <schema>] [--type <artifact-type>] [--contains <text>] [--limit <1..100>]");
    println!("  change impact --artifact-root <root> --repo <name> --repo-path <checkout> --changed <relative-path> [--changed <relative-path>]...");
    println!("  decision request-response --request <request.json|-> [--response <response.json>|--cancel <cancellation.json>] --now <unix-seconds> --branch <branch-id>...");
    println!("  decision record --resolution <resolution.json> --store <record-directory>");
    println!("  decision replay --query <query.json> --store <record-directory>");
    println!("  run dag-coordinate --repo <repo-root> --out <run-staging-directory> [--manifest <integrations.json>] [--max-concurrency <n>] [--session-evidence <session-evidence.json>]");
    println!("  run commit --source-root <A09-artifact-root> --authority-root <publication-root> --manifest-ref <artifact-ref.json> --final-name <name>");
    println!("  governance ponytail-gate --request <request.json|->");
    println!("  orchestrate [--action Validate|List|Plan] [--repo <path>] [--mode lite|normal|full] [--capability <name>] [--manifest <path>] [--json]");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn cli_args(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn parse_args_defaults_empty_input_to_help() {
        let args = parse_args(Vec::new()).expect("empty CLI should parse");

        assert_eq!(args.command, "help");
    }

    #[test]
    fn parse_args_preserves_graph_options() {
        let args = parse_args(cli_args(&[
            "graph",
            "--repo",
            "D:/repo",
            "--language",
            "en",
            "--write",
            "--full",
            "--json",
        ]))
        .expect("graph CLI should parse");

        assert_eq!(args.command, "graph");
        assert_eq!(args.repo, Some(PathBuf::from("D:/repo")));
        assert_eq!(args.language, "en");
        assert!(args.write);
        assert!(args.full);
        assert!(args.json);
    }

    #[test]
    fn parse_args_preserves_provider_options() {
        let args = parse_args(cli_args(&[
            "provider",
            "--action",
            "Invoke",
            "--provider",
            "understand",
            "--operation",
            "graph",
            "--repo",
            "D:/repo",
            "--language",
            "zh",
            "--write",
            "--json",
        ]))
        .expect("provider CLI should parse");

        assert_eq!(args.command, "provider");
        assert_eq!(args.action, "Invoke");
        assert_eq!(args.provider.as_deref(), Some("understand"));
        assert_eq!(args.operation.as_deref(), Some("graph"));
        assert_eq!(args.repo, Some(PathBuf::from("D:/repo")));
        assert_eq!(args.language, "zh");
        assert!(args.write);
        assert!(args.json);
    }

    #[test]
    fn parse_args_preserves_sentrux_positional_operation_and_repo() {
        let args = parse_args(cli_args(&["sentrux", "check_rules", "D:/repo"]))
            .expect("sentrux positional CLI should parse");

        assert_eq!(args.command, "sentrux");
        assert_eq!(args.operation.as_deref(), Some("check_rules"));
        assert_eq!(args.repo, Some(PathBuf::from("D:/repo")));
    }

    #[test]
    fn parse_args_rejects_unknown_argument() {
        let err = parse_args(cli_args(&["graph", "--bogus"]))
            .expect_err("unknown flag should fail")
            .to_string();

        assert!(err.contains("unknown argument for graph: --bogus"));
    }

    #[test]
    fn parse_args_rejects_missing_flag_value() {
        let err = parse_args(cli_args(&["graph", "--repo"]))
            .expect_err("missing flag value should fail")
            .to_string();

        assert!(err.contains("--repo requires a value"));
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        env::temp_dir().join(format!("code-intel-{name}-{stamp}"))
    }

    fn touch(path: &Path, text: &str) {
        fs::write(path, text).expect("fixture file should be writable");
    }

    fn summary_for(report: Value, hospital: Value, artifact_dir: &Path) -> ResumeSummary {
        build_resume_summary(
            PathBuf::from("D:/work/quant-system"),
            artifact_dir.to_path_buf(),
            artifact_dir.join("report.json"),
            &report,
            &hospital,
        )
    }

    #[test]
    fn resume_contract_routes_graph_missing_to_understanding() {
        let dir = unique_temp_dir("graph-missing");
        fs::create_dir_all(&dir).expect("fixture dir should be created");
        touch(&dir.join("summary.md"), "# Summary");
        touch(&dir.join("understanding.md"), "# Understanding");

        let hospital_path = dir.join("hospital-report.json");
        let hospital_markdown = dir.join("hospital.md");
        let report = json!({
            "hospital": {
                "path": hospital_path,
                "markdown": hospital_markdown
            },
            "githubResearch": {
                "status": "not_applicable",
                "required": false,
                "path": "",
                "markdown": ""
            },
            "summary": {
                "failed": 0,
                "manualRequired": 1,
                "failureCategories": {
                    "providerQuota": 0,
                    "localToolError": 0,
                    "graphMissing": 1,
                    "sentruxFail": 0
                }
            }
        });
        let hospital = json!({
            "triage": {
                "status": "amber",
                "disposition": "admit",
                "primary_diagnosis": "architecture graph missing",
                "next_protocol": "diagnose",
                "research_status": "not_applicable",
                "research_required": false
            },
            "state_machine": {
                "current_state": "diagnose"
            }
        });

        let summary = summary_for(report, hospital, &dir);

        assert_eq!(summary.graph_missing, 1);
        assert_eq!(summary.hospital_next_protocol, "diagnose");
        assert!(!summary.research_required);
        assert_eq!(next_read(&summary), dir.join("understanding.md"));
    }

    #[test]
    fn resume_contract_prioritizes_github_research_when_required() {
        let dir = unique_temp_dir("research-required");
        fs::create_dir_all(&dir).expect("fixture dir should be created");
        touch(&dir.join("understanding.md"), "# Understanding");
        let research_markdown = dir.join("github-solution-research.md");
        touch(&research_markdown, "# GitHub Solution Research");

        let report = json!({
            "hospital": {
                "path": dir.join("hospital-report.json"),
                "markdown": dir.join("hospital.md")
            },
            "githubResearch": {
                "status": "manual_required",
                "required": true,
                "path": dir.join("github-solution-research.json"),
                "markdown": research_markdown
            },
            "summary": {
                "failed": 1,
                "manualRequired": 1,
                "failureCategories": {
                    "providerQuota": 0,
                    "localToolError": 0,
                    "graphMissing": 0,
                    "sentruxFail": 1
                }
            }
        });
        let hospital = json!({
            "triage": {
                "status": "red",
                "disposition": "admit",
                "primary_diagnosis": "architecture gate failure",
                "next_protocol": "github_solution_research",
                "research_status": "manual_required",
                "research_required": true
            },
            "state_machine": {
                "current_state": "triage"
            }
        });

        let summary = summary_for(report, hospital, &dir);

        assert_eq!(summary.sentrux_fail, 1);
        assert!(summary.research_required);
        assert_eq!(summary.hospital_next_protocol, "github_solution_research");
        assert_eq!(next_read(&summary), research_markdown);
    }

    #[test]
    fn classify_contract_requires_research_for_upstream_or_tool_blockers() {
        let report = json!({
            "summary": {
                "failureCategories": {
                    "providerQuota": 1,
                    "localToolError": 0,
                    "graphMissing": 1,
                    "sentruxFail": 0
                }
            }
        });

        let provider_quota = int_at(&report, &["summary", "failureCategories", "providerQuota"]);
        let local_tool_error = int_at(&report, &["summary", "failureCategories", "localToolError"]);
        let sentrux_fail = int_at(&report, &["summary", "failureCategories", "sentruxFail"]);

        assert!(provider_quota > 0 || local_tool_error > 0 || sentrux_fail > 0);
    }

    #[test]
    fn classify_policy_does_not_block_known_sentrux_debt() {
        let report = json!({
            "summary": {
                "failureCategories": {
                    "providerQuota": 0,
                    "localToolError": 0,
                    "graphMissing": 1,
                    "sentruxFail": 1
                },
                "effectiveFailureCategories": {
                    "providerQuota": 0,
                    "localToolError": 0,
                    "graphMissing": 1,
                    "sentruxFail": 0
                },
                "blockingSentruxDebt": 0,
                "knownSentruxDebt": 1
            }
        });

        let policy = classify_report_policy(&report);

        assert!(policy.known_debt_only);
        assert!(!policy.pipeline_blocking);
        assert!(!policy.github_research_required);
        assert_eq!(policy.exit_code, 0);
    }

    #[test]
    fn classify_policy_blocks_effective_sentrux_failure() {
        let report = json!({
            "summary": {
                "failureCategories": {
                    "providerQuota": 0,
                    "localToolError": 0,
                    "graphMissing": 0,
                    "sentruxFail": 1
                },
                "effectiveFailureCategories": {
                    "providerQuota": 0,
                    "localToolError": 0,
                    "graphMissing": 0,
                    "sentruxFail": 1
                },
                "blockingSentruxDebt": 1,
                "knownSentruxDebt": 0
            }
        });

        let policy = classify_report_policy(&report);

        assert!(!policy.known_debt_only);
        assert!(policy.pipeline_blocking);
        assert!(policy.github_research_required);
        assert_eq!(policy.next_protocol, "github_solution_research");
        assert_eq!(policy.exit_code, 1);
    }
}
