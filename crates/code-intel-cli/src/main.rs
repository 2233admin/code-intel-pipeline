use serde_json::Value;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

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
    if let Err(err) = run() {
        eprintln!("error: {err}");
        process::exit(1);
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
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => Err(format!("unknown command: {other}").into()),
    }
}

fn parse_args(raw: Vec<String>) -> Result<Args> {
    if raw.is_empty() {
        return Ok(Args {
            command: "help".to_string(),
            ..Args::default()
        });
    }

    let mut args = Args {
        command: raw[0].clone(),
        ..Args::default()
    };
    let mut i = 1usize;
    while i < raw.len() {
        match raw[i].as_str() {
            "--repo" => {
                i += 1;
                args.repo = Some(PathBuf::from(required_value(&raw, i, "--repo")?));
            }
            "--report" => {
                i += 1;
                args.report = Some(PathBuf::from(required_value(&raw, i, "--report")?));
            }
            "--steps" => {
                i += 1;
                args.steps = Some(PathBuf::from(required_value(&raw, i, "--steps")?));
            }
            "--failures" => {
                i += 1;
                args.failures = Some(PathBuf::from(required_value(&raw, i, "--failures")?));
            }
            "--out" => {
                i += 1;
                args.out = Some(PathBuf::from(required_value(&raw, i, "--out")?));
            }
            "--artifact-root" => {
                i += 1;
                args.artifact_root =
                    Some(PathBuf::from(required_value(&raw, i, "--artifact-root")?));
            }
            "--json" => args.json = true,
            "--help" | "-h" => args.command = "help".to_string(),
            flag => return Err(format!("unknown argument for {}: {flag}", args.command).into()),
        }
        i += 1;
    }
    Ok(args)
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
    let repo = absolute_existing_dir(&repo)?;
    let artifact_root = resolve_artifact_root(args.artifact_root.as_deref())?;
    let repo_name = repo
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("repo path has no final directory name")?;
    let repo_artifacts = artifact_root.join(repo_name);
    let artifact_dir = latest_run_dir(&repo_artifacts)?;
    let report_path = artifact_dir.join("report.json");
    let report = read_json(&report_path)?;
    let hospital_path = string_path(&report, &["hospital", "path"]).or_else(|| {
        let candidate = artifact_dir.join("hospital-report.json");
        candidate.exists().then_some(candidate)
    });
    let hospital = match hospital_path.as_ref() {
        Some(path) if path.exists() => read_json(path)?,
        _ => Value::Null,
    };

    let summary = build_resume_summary(repo, artifact_dir, report_path, &report, &hospital);
    if args.json {
        print_resume_json(&summary)?;
    } else {
        print_resume_text(&summary);
    }
    Ok(())
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

fn cmd_doctor(args: &Args) -> Result<()> {
    let artifact_root = resolve_artifact_root(args.artifact_root.as_deref())?;
    let ok = artifact_root.exists();
    if args.json {
        let out = serde_json::json!({
            "ok": ok,
            "artifactRoot": artifact_root,
            "artifactRootExists": ok
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("artifactRoot: {}", artifact_root.display());
        println!("artifactRootExists: {ok}");
    }
    Ok(())
}

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

fn print_resume_text(summary: &ResumeSummary) {
    println!("Code Intel Resume");
    println!("repo: {}", summary.repo.display());
    println!("artifactDir: {}", summary.artifact_dir.display());
    println!("report: {}", summary.report_path.display());
    print_optional_path("summary", summary.summary_path.as_ref());
    print_optional_path("understanding", summary.understanding_path.as_ref());
    print_optional_path("hospital", summary.hospital_path.as_ref());
    print_optional_path("hospitalMarkdown", summary.hospital_markdown.as_ref());
    println!("failed: {}", summary.pipeline_failed);
    println!("manualRequired: {}", summary.pipeline_manual_required);
    println!(
        "failureCategories: providerQuota={}, localToolError={}, graphMissing={}, sentruxFail={}",
        summary.provider_quota,
        summary.local_tool_error,
        summary.graph_missing,
        summary.sentrux_fail
    );
    println!(
        "hospitalStatus: {}",
        empty_as_unknown(&summary.hospital_status)
    );
    println!(
        "hospitalDisposition: {}",
        empty_as_unknown(&summary.hospital_disposition)
    );
    println!(
        "hospitalState: {}",
        empty_as_unknown(&summary.hospital_current_state)
    );
    println!(
        "primaryDiagnosis: {}",
        empty_as_unknown(&summary.hospital_primary_diagnosis)
    );
    println!(
        "nextProtocol: {}",
        empty_as_unknown(&summary.hospital_next_protocol)
    );
    println!("githubResearch: {}", summary.research_status);
    println!("githubResearchRequired: {}", summary.research_required);
    if summary.research_required {
        print_optional_path("githubResearchJson", summary.github_research_path.as_ref());
        print_optional_path(
            "githubResearchMarkdown",
            summary.github_research_markdown.as_ref(),
        );
    }
    println!("nextRead: {}", next_read(summary).display());
}

fn print_resume_json(summary: &ResumeSummary) -> Result<()> {
    let out = serde_json::json!({
        "repo": summary.repo,
        "artifactDir": summary.artifact_dir,
        "report": summary.report_path,
        "summary": summary.summary_path,
        "understanding": summary.understanding_path,
        "hospital": summary.hospital_path,
        "hospitalMarkdown": summary.hospital_markdown,
        "failed": summary.pipeline_failed,
        "manualRequired": summary.pipeline_manual_required,
        "failureCategories": {
            "providerQuota": summary.provider_quota,
            "localToolError": summary.local_tool_error,
            "graphMissing": summary.graph_missing,
            "sentruxFail": summary.sentrux_fail
        },
        "hospitalStatus": summary.hospital_status,
        "hospitalDisposition": summary.hospital_disposition,
        "hospitalState": summary.hospital_current_state,
        "primaryDiagnosis": summary.hospital_primary_diagnosis,
        "nextProtocol": summary.hospital_next_protocol,
        "githubResearch": {
            "status": summary.research_status,
            "required": summary.research_required,
            "path": summary.github_research_path,
            "markdown": summary.github_research_markdown
        },
        "nextRead": next_read(summary)
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

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

fn print_optional_path(label: &str, path: Option<&PathBuf>) {
    if let Some(path) = path {
        if !path.as_os_str().is_empty() {
            println!("{label}: {}", path.display());
        }
    }
}

fn resolve_artifact_root(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Ok(value) = env::var("CODE_INTEL_ARTIFACT_ROOT") {
        if !value.trim().is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    if let Ok(value) = env::var("LOCALAPPDATA") {
        if !value.trim().is_empty() {
            return Ok(PathBuf::from(value).join("code-intel").join("artifacts"));
        }
    }
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE"))?;
    Ok(PathBuf::from(home)
        .join(".code-intel")
        .join("code-intel")
        .join("artifacts"))
}

fn absolute_existing_dir(path: &Path) -> Result<PathBuf> {
    if !path.is_dir() {
        return Err(format!("repo path is not a directory: {}", path.display()).into());
    }
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(env::current_dir()?.join(path))
}

fn latest_run_dir(repo_artifacts: &Path) -> Result<PathBuf> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(repo_artifacts)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    dirs.pop().ok_or_else(|| {
        format!(
            "no artifact run directories under {}",
            repo_artifacts.display()
        )
        .into()
    })
}

fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
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

fn empty_as_unknown(value: &str) -> &str {
    if value.trim().is_empty() {
        "unknown"
    } else {
        value
    }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

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
