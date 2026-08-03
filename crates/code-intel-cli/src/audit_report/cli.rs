use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{enums::DepartmentRunStatus, model::AuditReport, registry::DepartmentRegistry};

/// `code-intel audit --operation validate --repo <root> --report <path>` or
/// `code-intel audit --operation render --repo <root> --report <path>`.
/// Rendering requires `--repo` and runs the same fail-closed validate
/// pipeline before producing output (ai-safety-003, issue #34): a report
/// that does not pass registry/evidence/shape validation must never be
/// turned into human-facing markdown or HTML. The private command catalog
/// registers the typed `audit` route while this module owns its small
/// compatibility parser and output, matching the other Phase-2 adapters.
pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match parse(raw) {
        Ok(Operation::Validate { repo, report }) => match validate(&repo, &report) {
            Ok(summary) => {
                println!("{summary}");
                0
            }
            Err(message) => fail(&message),
        },
        Ok(Operation::Render {
            repo,
            report,
            format,
        }) => match render(&repo, &report, format) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(message) => fail(&message),
        },
        Ok(Operation::Scope { repo, since }) => match scope(&repo, &since) {
            Ok(value) => {
                println!("{value}");
                0
            }
            Err(message) => fail(&message),
        },
        Err(message) => fail(&message),
    }
}

fn fail(message: &str) -> i32 {
    println!("{}", json!({"ok": false, "error": message}));
    65
}

#[derive(Debug)]
enum Operation {
    Validate {
        repo: PathBuf,
        report: PathBuf,
    },
    Render {
        repo: PathBuf,
        report: PathBuf,
        format: RenderFormat,
    },
    Scope {
        repo: PathBuf,
        since: String,
    },
}

/// `--format` on `--operation render`; defaults to `Markdown` when absent so
/// the existing `--operation render --report <path>` invocation (no
/// `--format`) keeps behaving exactly as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderFormat {
    Markdown,
    Html,
}

impl RenderFormat {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "markdown" => Some(Self::Markdown),
            "html" => Some(Self::Html),
            _ => None,
        }
    }
}

fn parse(raw: &[String]) -> Result<Operation, String> {
    let mut operation = None;
    let mut repo = None;
    let mut report = None;
    let mut format_flag = None;
    let mut since = None;
    let mut index = 0;
    while index < raw.len() {
        let flag = raw[index].as_str();
        let value = raw
            .get(index + 1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires one value"))?;
        match flag {
            "--operation" => set_once(&mut operation, value, flag)?,
            "--repo" => set_once(&mut repo, value, flag)?,
            "--report" => set_once(&mut report, value, flag)?,
            "--format" => set_once(&mut format_flag, value, flag)?,
            "--since" => set_once(&mut since, value, flag)?,
            _ => return Err(format!("unknown audit argument: {flag}")),
        }
        index += 2;
    }
    match operation.as_deref() {
        Some("validate") => Ok(Operation::Validate {
            report: report.map(PathBuf::from).ok_or("--report is required")?,
            repo: repo
                .map(PathBuf::from)
                .ok_or("--operation validate requires --repo")?,
        }),
        Some("render") => {
            let format = match format_flag.as_deref() {
                None => RenderFormat::Markdown,
                Some(raw_format) => RenderFormat::parse(raw_format).ok_or_else(|| {
                    format!("unknown --format \"{raw_format}\" (expected markdown or html)")
                })?,
            };
            Ok(Operation::Render {
                report: report.map(PathBuf::from).ok_or("--report is required")?,
                repo: repo
                    .map(PathBuf::from)
                    .ok_or("--operation render requires --repo")?,
                format,
            })
        }
        Some("scope") => Ok(Operation::Scope {
            repo: repo
                .map(PathBuf::from)
                .ok_or("--operation scope requires --repo")?,
            since: since.ok_or("--operation scope requires --since")?,
        }),
        Some(other) => Err(format!(
            "unknown --operation \"{other}\" (expected validate, render, or scope)"
        )),
        None => Err("--operation is required (validate, render, or scope)".to_string()),
    }
}

fn set_once(slot: &mut Option<String>, value: &str, flag: &str) -> Result<(), String> {
    if slot.replace(value.to_string()).is_some() {
        Err(format!("duplicate {flag}"))
    } else {
        Ok(())
    }
}

/// The full validate pipeline the CLI runs against a real repository: read
/// the report file, parse it structurally, load and self-validate the
/// on-disk department registry, check the report against it, and — because
/// this path has the repository the report cites — ground every file evidence
/// entry in that tree. Shared with `render` (ai-safety-003, issue #34): the
/// two operations must run the identical pipeline so a report can never
/// reach human-facing output by a path that skipped a check `--operation
/// validate` would have caught.
fn validate_and_parse(repo: &Path, report_path: &Path) -> Result<(AuditReport, Value), String> {
    let bytes = read_report(report_path)?;
    let report = AuditReport::parse(&bytes)?;
    let registry = DepartmentRegistry::load(repo)?;
    registry.validate(repo)?;
    report.validate_evidence_grounding(repo)?;
    let summary = validate_report(&report, &registry)?;
    Ok((report, summary))
}

fn validate(repo: &Path, report_path: &Path) -> Result<Value, String> {
    let (_report, summary) = validate_and_parse(repo, report_path)?;
    Ok(summary)
}

/// The registry-agnostic half of `validate`: runs the fail-closed report
/// rules against whatever `DepartmentRegistry` it is given (real or
/// synthetic — see `cli_tests.rs`) and builds the compact success summary.
fn validate_report(report: &AuditReport, registry: &DepartmentRegistry) -> Result<Value, String> {
    report.validate(registry)?;
    Ok(json!({
        "ok": true,
        "findings_total": report.findings.len(),
        "overall": report.score_dashboard.overall,
        "departments_assessed": count_assessed(report),
    }))
}

fn count_assessed(report: &AuditReport) -> usize {
    report
        .departments
        .iter()
        .filter(|department| department.status == DepartmentRunStatus::Assessed)
        .count()
}

/// `--repo` is mandatory (ai-safety-003, issue #34): rendering runs the
/// exact same fail-closed pipeline `--operation validate` runs — registry
/// load and self-check, evidence grounding, report-shape rules — before
/// producing output. A report that fails validation returns `Err` here too,
/// so `render` can never turn an unvalidated or invalid report into
/// human-facing markdown or HTML.
fn render(repo: &Path, report_path: &Path, format: RenderFormat) -> Result<String, String> {
    let (report, _summary) = validate_and_parse(repo, report_path)?;
    Ok(match format {
        RenderFormat::Markdown => super::render_markdown_section(&report),
        RenderFormat::Html => super::render_html_document(&report),
    })
}

fn read_report(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("read report {}: {error}", path.display()))
}

/// `--operation scope --repo <root> --since <git-ref>`: the changed-file set
/// on HEAD since `since`'s merge base (`git diff --name-only
/// <since>...HEAD`, three-dot), printed as a ready-to-embed `scope` block.
/// Runs through `hardened_git::command` — `--repo` is a target repository an
/// operator pointed the CLI at, not one this process owns, and that is
/// exactly the threat model that wrapper disarms config-driven program
/// execution for. Filters out paths that no longer exist in the working
/// tree: a deleted file cannot carry file evidence. Never falls back to
/// "assume everything changed" — any git failure is a hard error.
fn scope(repo: &Path, since: &str) -> Result<Value, String> {
    let range = format!("{since}...HEAD");
    let output = hardened_git::command(repo)
        .arg("diff")
        .arg("--name-only")
        .arg(&range)
        .output()
        .map_err(|error| format!("failed to run git diff --name-only {range}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git diff --name-only {range} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("git diff output is not UTF-8: {error}"))?;
    let mut files = stdout
        .lines()
        .map(|line| line.replace('\\', "/"))
        .filter(|line| !line.is_empty())
        .filter(|relative| repo.join(relative).is_file())
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    Ok(json!({"kind": "diff", "since": since, "files": files}))
}

#[path = "../hardened_git.rs"]
mod hardened_git;

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
