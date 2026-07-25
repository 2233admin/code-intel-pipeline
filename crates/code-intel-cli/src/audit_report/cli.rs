use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{enums::DepartmentRunStatus, model::AuditReport, registry::DepartmentRegistry};

/// `code-intel audit --operation validate --repo <root> --report <path>` or
/// `code-intel audit --operation render --report <path>`. Mirrors the
/// least-invasive `RAW_ROUTES` pattern already used by `change_impact`,
/// `decision_record`, and friends: this module owns its own tiny argument
/// parser and prints its own JSON, so `main.rs` only ever gains one table
/// entry (see the `"audit"` route).
pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match parse(raw) {
        Ok(Operation::Validate { repo, report }) => match validate(&repo, &report) {
            Ok(summary) => {
                println!("{summary}");
                0
            }
            Err(message) => fail(&message),
        },
        Ok(Operation::Render { report }) => match render(&report) {
            Ok(markdown) => {
                println!("{markdown}");
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
    Validate { repo: PathBuf, report: PathBuf },
    Render { report: PathBuf },
}

fn parse(raw: &[String]) -> Result<Operation, String> {
    let mut operation = None;
    let mut repo = None;
    let mut report = None;
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
            _ => return Err(format!("unknown audit argument: {flag}")),
        }
        index += 2;
    }
    let report = report.map(PathBuf::from).ok_or("--report is required")?;
    match operation.as_deref() {
        Some("validate") => Ok(Operation::Validate {
            repo: repo
                .map(PathBuf::from)
                .ok_or("--operation validate requires --repo")?,
            report,
        }),
        Some("render") => Ok(Operation::Render { report }),
        Some(other) => Err(format!(
            "unknown --operation \"{other}\" (expected validate or render)"
        )),
        None => Err("--operation is required (validate or render)".to_string()),
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
/// on-disk department registry, then check the report against it.
fn validate(repo: &Path, report_path: &Path) -> Result<Value, String> {
    let bytes = read_report(report_path)?;
    let report = AuditReport::parse(&bytes)?;
    let registry = DepartmentRegistry::load(repo)?;
    registry.validate(repo)?;
    validate_report(&report, &registry)
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

/// Registry-independent: `render_markdown_section` only reads the parsed
/// report, so rendering never needs `--repo`.
fn render(report_path: &Path) -> Result<String, String> {
    let bytes = read_report(report_path)?;
    let report = AuditReport::parse(&bytes)?;
    Ok(super::render_markdown_section(&report))
}

fn read_report(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("read report {}: {error}", path.display()))
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
