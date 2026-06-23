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
    json: bool,
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
    let provider_quota = int_at(&report, &["summary", "failureCategories", "providerQuota"]);
    let local_tool_error = int_at(&report, &["summary", "failureCategories", "localToolError"]);
    let graph_missing = int_at(&report, &["summary", "failureCategories", "graphMissing"]);
    let sentrux_fail = int_at(&report, &["summary", "failureCategories", "sentruxFail"]);
    let research_required = provider_quota > 0 || local_tool_error > 0 || sentrux_fail > 0;
    if args.json {
        let out = serde_json::json!({
            "report": report_path,
            "failureCategories": {
                "providerQuota": provider_quota,
                "localToolError": local_tool_error,
                "graphMissing": graph_missing,
                "sentruxFail": sentrux_fail
            },
            "githubResearchRequired": research_required
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("Report: {}", report_path.display());
        println!("providerQuota={provider_quota}");
        println!("localToolError={local_tool_error}");
        println!("graphMissing={graph_missing}");
        println!("sentruxFail={sentrux_fail}");
        println!("githubResearchRequired={research_required}");
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
}
