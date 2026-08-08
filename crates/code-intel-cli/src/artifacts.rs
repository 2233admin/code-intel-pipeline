use serde_json::Value;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::run_commit;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The environment variable that names the artifact root when no flag does.
pub(crate) const ARTIFACT_ROOT_ENV: &str = "CODE_INTEL_ARTIFACT_ROOT";

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

#[derive(Debug)]
struct ReportArtifact {
    schema: String,
    artifact_type: String,
    path: PathBuf,
}

impl ReportArtifact {
    fn to_json(&self) -> Value {
        serde_json::json!({
            "schema": self.schema,
            "type": self.artifact_type,
            "path": self.path,
        })
    }
}

pub(crate) fn resume(repo: &Path, artifact_root: Option<&Path>, json: bool) -> Result<()> {
    let repo = absolute_existing_dir(repo)?;
    let artifact_root = resolve_artifact_root(artifact_root)?;
    let repo_name = repo
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("repo path has no final directory name")?;
    let repo_artifacts = artifact_root.join(repo_name);
    let artifact_dir = latest_run_dir(&repo_artifacts)?;
    let report_path = artifact_dir.join("report.json");
    if !report_path.is_file() {
        return Err(format!(
            "resume cannot find {}; this committed run uses manifest Artifact Refs; use `code-intel report --repo {} --artifact-root {}`",
            report_path.display(),
            repo.display(),
            artifact_root.display()
        )
        .into());
    }
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
    if json {
        print_resume_json(&summary)?;
    } else {
        print_resume_text(&summary);
    }
    Ok(())
}

pub(crate) fn report(repo: &Path, artifact_root: Option<&Path>, json: bool) -> Result<()> {
    let repo = absolute_existing_dir(repo)?;
    let artifact_root = resolve_artifact_root(artifact_root)?;
    let repo_name = repo
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("repo path has no final directory name")?;
    let repo_artifacts = artifact_root.join(repo_name);
    let (run_root, manifest) = latest_committed_run(&repo_artifacts)?;
    let hospital = required_report_artifact(&run_root, &manifest, "diagnosis.hospital")?;
    let hospital_markdown =
        required_report_artifact(&run_root, &manifest, "diagnosis.hospital-view")?;
    let hospital_text = fs::read_to_string(&hospital_markdown.path)?;
    let agent_code_slice_ranking =
        optional_report_artifact(&run_root, &manifest, "code_evidence.agent_slice")?;

    let out = serde_json::json!({
        "schema": "code-intel-report.v1",
        "repo": repo,
        "run": run_root.file_name().and_then(|name| name.to_str()),
        "hospital": hospital.to_json(),
        "hospitalMarkdown": hospital_markdown.to_json(),
        "agentCodeSliceRanking": agent_code_slice_ranking.as_ref().map(ReportArtifact::to_json),
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("Code Intel Report");
        println!("repo: {}", repo.display());
        println!("run: {}", out["run"].as_str().unwrap_or(""));
        println!("hospital: {}", hospital.path.display());
        println!("hospitalMarkdown: {}", hospital_markdown.path.display());
        if let Some(ranking) = &agent_code_slice_ranking {
            println!("agentCodeSliceRanking: {}", ranking.path.display());
        }
        println!();
        println!("--- hospital.md ---");
        print!("{hospital_text}");
        if !hospital_text.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

fn required_report_artifact(
    run_root: &Path,
    manifest: &Value,
    artifact_type: &str,
) -> Result<ReportArtifact> {
    optional_report_artifact(run_root, manifest, artifact_type)?.ok_or_else(|| {
        format!(
            "committed run {} has no `{artifact_type}` Artifact Ref",
            run_root.display()
        )
        .into()
    })
}

fn optional_report_artifact(
    run_root: &Path,
    manifest: &Value,
    artifact_type: &str,
) -> Result<Option<ReportArtifact>> {
    let reference = manifest["nodes"]
        .as_object()
        .into_iter()
        .flat_map(|nodes| nodes.values())
        .flat_map(|node| node["artifacts"].as_array().into_iter().flatten())
        .find(|reference| reference["type"].as_str() == Some(artifact_type));
    let Some(reference) = reference else {
        return Ok(None);
    };
    let path = reference["path"].as_str().ok_or_else(|| {
        format!(
            "Artifact Ref `{artifact_type}` has no path in {}",
            run_root.display()
        )
    })?;
    let path = run_root.join(path);
    if !path.is_file() {
        return Err(format!(
            "Artifact Ref `{artifact_type}` points to missing file {}",
            path.display()
        )
        .into());
    }
    Ok(Some(ReportArtifact {
        schema: reference["artifactSchema"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        artifact_type: artifact_type.to_string(),
        path,
    }))
}

fn latest_committed_run(repo_artifacts: &Path) -> Result<(PathBuf, Value)> {
    if !repo_artifacts.is_dir() {
        return Err(format!(
            "no artifact directory for repository under {}",
            repo_artifacts.display()
        )
        .into());
    }
    let mut dirs = Vec::new();
    for entry in fs::read_dir(repo_artifacts)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    let mut rejection = None;
    for path in dirs.iter().rev() {
        match run_commit::validate_committed_run(path) {
            Ok((_marker, manifest)) => return Ok((path.clone(), manifest)),
            Err(error) => rejection = Some(format!("{}: {error}", path.display())),
        }
    }
    Err(format!(
        "no committed A07 run under {}; latest candidate rejected: {}",
        repo_artifacts.display(),
        rejection.unwrap_or_else(|| "no run directories found".to_string())
    )
    .into())
}

pub(crate) fn classify(report_path: &Path, json: bool) -> Result<()> {
    let report = read_json(report_path)?;
    let provider_quota = int_at(&report, &["summary", "failureCategories", "providerQuota"]);
    let local_tool_error = int_at(&report, &["summary", "failureCategories", "localToolError"]);
    let graph_missing = int_at(&report, &["summary", "failureCategories", "graphMissing"]);
    let sentrux_fail = int_at(&report, &["summary", "failureCategories", "sentruxFail"]);
    let research_required = provider_quota > 0 || local_tool_error > 0 || sentrux_fail > 0;
    if json {
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

pub(crate) fn doctor(artifact_root: Option<&Path>, json: bool) -> Result<()> {
    let artifact_root = resolve_artifact_root(artifact_root)?;
    let ok = artifact_root.exists();
    if json {
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

pub(crate) fn resolve_artifact_root(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Ok(value) = env::var(ARTIFACT_ROOT_ENV) {
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

/// Composes a DAG staging directory the way `resume` and `artifact index` read
/// runs back: `<artifact root>/<repo name>/<run>`. The PowerShell launcher owned
/// this composition until it was retired, and the failure it existed to prevent
/// is repeating the repository name inside the path, which strands a run where
/// no reader looks. Only the repository directory is created here; the run
/// directory is left absent because `dag_run::execute_dag` claims it
/// exclusively and must keep failing when it already exists.
pub(crate) fn compose_dag_staging_dir(
    artifact_root: Option<&Path>,
    repo: &Path,
) -> Result<PathBuf> {
    let repo = absolute_existing_dir(repo)?;
    let artifact_root = resolve_artifact_root(artifact_root)?;
    let repo_name = repo
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("repo path has no final directory name")?;
    let repo_artifacts = artifact_root.join(repo_name);
    fs::create_dir_all(&repo_artifacts)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the unix epoch: {error}"))?;
    let stem = utc_run_stamp(now.as_secs());
    let mut nonce = u64::from(now.subsec_nanos());
    for _ in 0..64 {
        let candidate = repo_artifacts.join(format!("{stem}.dag-staging-{nonce:012x}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
        nonce = nonce.wrapping_add(1);
    }
    Err(format!(
        "no free DAG staging directory under {}",
        repo_artifacts.display()
    )
    .into())
}

/// `yyyyMMdd-HHmmss` in UTC, by Howard Hinnant's `civil_from_days`. Nothing
/// parses this stem back, but `latest_run_dir` sorts run directories by name,
/// so a run written today must still sort after one the launcher wrote.
fn utc_run_stamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let second_of_day = seconds % 86_400;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_position + 2) / 5 + 1;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        second_of_day / 3_600,
        (second_of_day % 3_600) / 60,
        second_of_day % 60
    )
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

#[cfg(test)]
#[path = "artifacts_tests.rs"]
mod tests;
