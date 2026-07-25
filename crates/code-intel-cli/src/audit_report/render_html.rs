//! `--operation render --format html`: one self-contained HTML document
//! rendered directly from the parsed, validated `AuditReport` model — never
//! from a raw template string. There is therefore no placeholder-substitution
//! failure mode to lint for separately (see `docs/audit-report.md`): every
//! value that reaches the page went through `escape_html`, the single
//! escaping helper below, and nothing here does string-splicing of report
//! text into markup ahead of that call.
//!
//! No external CSS, JS, fonts, or images, and no network references of any
//! kind — the `<style>` block is inlined and hand-written so the report
//! opens correctly from a `file://` path with no other resource to fetch.

use super::{
    enums::Severity,
    model::{AuditReport, EvidenceRef, Finding, Scope},
    render::format_score,
};

const STYLE: &str = r#"
body { font-family: -apple-system, "Segoe UI", Roboto, Arial, sans-serif; line-height: 1.5; margin: 0; padding: 2rem; max-width: 960px; margin-inline: auto; color: #1a1a1a; background: #ffffff; }
* { box-sizing: border-box; }
h1, h2, h3, h4, h5 { line-height: 1.25; }
h1 { font-size: 1.6rem; margin-bottom: 0.25rem; }
h2 { font-size: 1.3rem; margin-top: 2rem; border-bottom: 2px solid #ddd; padding-bottom: 0.25rem; }
h3 { font-size: 1.1rem; margin-top: 1.5rem; }
h4 { font-size: 1rem; margin-bottom: 0.25rem; }
h5 { font-size: 0.85rem; text-transform: uppercase; letter-spacing: 0.03em; color: #555; margin: 0.75rem 0 0.15rem; }
header { border-bottom: 1px solid #ddd; padding-bottom: 1rem; }
table { border-collapse: collapse; width: 100%; margin: 0.75rem 0 1.25rem; font-size: 0.92rem; }
th, td { border: 1px solid #ccc; padding: 0.4rem 0.6rem; text-align: left; vertical-align: top; }
th { background: #f2f2f2; }
code { background: #f2f2f2; padding: 0.1rem 0.3rem; border-radius: 3px; font-size: 0.9em; }
.finding { border: 1px solid #ddd; border-left-width: 6px; border-radius: 4px; padding: 0.75rem 1rem; margin: 0.75rem 0; }
.finding-meta { display: flex; flex-wrap: wrap; gap: 0.25rem 1.5rem; margin: 0.35rem 0; padding: 0; }
.finding-meta dt { font-weight: 600; margin: 0; }
.finding-meta dt::after { content: ":"; }
.finding-meta dd { margin: 0 0 0 0.35rem; }
.evidence-list { padding-left: 1.25rem; }
.redacted-notice { font-weight: 700; color: #7a0000; background: #fdeaea; border: 1px solid #f2b8b8; padding: 0.4rem 0.6rem; border-radius: 4px; display: inline-block; }
.severity-critical { border-left-color: #8b0000; }
.severity-high { border-left-color: #d9534f; }
.severity-medium { border-left-color: #e0a800; }
.severity-low { border-left-color: #5b8def; }
.severity-info { border-left-color: #6c757d; }
td.severity-critical, td.severity-high, td.severity-medium, td.severity-low, td.severity-info { font-weight: 600; }
h3.severity-heading { padding: 0.15rem 0.5rem; border-radius: 4px; display: inline-block; }
h3.severity-critical { background: #f8d7da; }
h3.severity-high { background: #fde2cf; }
h3.severity-medium { background: #fff3cd; }
h3.severity-low { background: #dbe9ff; }
h3.severity-info { background: #e2e3e5; }
@media (prefers-color-scheme: dark) {
  body { background: #121212; color: #e6e6e6; }
  h2 { border-bottom-color: #444; }
  h5 { color: #aaa; }
  th, td { border-color: #444; }
  th { background: #1e1e1e; }
  code { background: #1e1e1e; }
  .finding { border-color: #444; }
  .redacted-notice { background: #3a1111; color: #ffb3b3; border-color: #6b1f1f; }
}
"#;

const SEVERITIES: [Severity; 5] = [
    Severity::Critical,
    Severity::High,
    Severity::Medium,
    Severity::Low,
    Severity::Info,
];

/// The single HTML-escaping helper: every interpolated value in this module
/// passes through here, whether it is department-authored free text (title,
/// problem, evidence note, …) or this module's own enum wire strings — the
/// latter never contain special characters, but escaping them too keeps the
/// rule exceptionless rather than "escape it unless you're sure".
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// The full self-contained document: header, score dashboard, coverage
/// matrix, findings grouped by severity (critical to info), and a fix-order
/// section — in that order.
pub(crate) fn render_html_document(report: &AuditReport) -> String {
    let mut html = String::from("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("<meta charset=\"utf-8\">\n");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    html.push_str(&format!(
        "<title>Audit Report: {}</title>\n",
        escape_html(&report.repo)
    ));
    html.push_str("<style>");
    html.push_str(STYLE);
    html.push_str("</style>\n</head>\n<body>\n");
    html.push_str(&render_header(report));
    html.push_str(&render_score_dashboard(report));
    html.push_str(&render_coverage_matrix(report));
    html.push_str(&render_findings_by_severity(report));
    html.push_str(&render_fix_order(report));
    html.push_str("</body>\n</html>\n");
    html
}

/// Repo name, overall score, rubric version, and (when present) the scope
/// line. `rubric_version` is hardcoded to the literal "v1": the schema pins
/// it to `const: "v1"` and `AuditReport::parse` already rejects any other
/// value, so the model has nothing else to store there.
fn render_header(report: &AuditReport) -> String {
    let mut header = String::from("<header>\n");
    header.push_str(&format!(
        "<h1>Audit Report: {}</h1>\n",
        escape_html(&report.repo)
    ));
    header.push_str(&format!(
        "<p>Overall score: <strong>{}</strong> &middot; Rubric version: <strong>v1</strong></p>\n",
        escape_html(&format_score(report.score_dashboard.overall))
    ));
    if let Some(scope) = &report.scope {
        header.push_str(&render_scope_line(scope));
    }
    header.push_str("</header>\n");
    header
}

fn render_scope_line(scope: &Scope) -> String {
    let mut line = format!(
        "<p class=\"scope-line\">Scope: <strong>{}</strong>",
        escape_html(scope.kind.as_str())
    );
    if let Some(since) = &scope.since {
        line.push_str(&format!(" since <code>{}</code>", escape_html(since)));
    }
    if !scope.files.is_empty() {
        let files = scope
            .files
            .iter()
            .map(|file| escape_html(file))
            .collect::<Vec<_>>()
            .join(", ");
        line.push_str(&format!(
            " &mdash; {} file(s): <code>{files}</code>",
            scope.files.len()
        ));
    }
    line.push_str("</p>\n");
    line
}

fn render_score_dashboard(report: &AuditReport) -> String {
    let mut section = String::from(
        "<section id=\"score-dashboard\">\n<h2>Score Dashboard</h2>\n<table>\n\
         <thead><tr><th>Department</th><th>Score</th><th>Justification</th></tr></thead>\n<tbody>\n",
    );
    for entry in &report.score_dashboard.entries {
        section.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            escape_html(&entry.department),
            escape_html(&format_score(entry.score)),
            escape_html(&entry.justification)
        ));
    }
    section.push_str("</tbody>\n</table>\n</section>\n");
    section
}

fn render_coverage_matrix(report: &AuditReport) -> String {
    let mut section = String::from(
        "<section id=\"coverage-matrix\">\n<h2>Coverage Matrix</h2>\n<table>\n\
         <thead><tr><th>Department</th><th>Coverage</th><th>Inspected Evidence</th><th>Exclusions</th></tr></thead>\n<tbody>\n",
    );
    for row in &report.coverage_matrix {
        let coverage = escape_html(row.coverage.as_str());
        section.push_str(&format!(
            "<tr><td>{}</td><td class=\"severity-{coverage}\">{coverage}</td><td>{}</td><td>{}</td></tr>\n",
            escape_html(&row.department),
            escape_html(&row.inspected_evidence.join(", ")),
            escape_html(&row.exclusions.join(", "))
        ));
    }
    section.push_str("</tbody>\n</table>\n</section>\n");
    section
}

/// Shared total order for both the "grouped by severity" and "fix order"
/// sections, so the two sections agree on ordering wherever they overlap:
/// severity rank first (critical .. info), then department, then id.
fn sorted_findings(report: &AuditReport) -> Vec<&Finding> {
    let mut findings = report.findings.iter().collect::<Vec<_>>();
    findings.sort_by(|a, b| {
        a.severity
            .rank()
            .cmp(&b.severity.rank())
            .then_with(|| a.department.cmp(&b.department))
            .then_with(|| a.id.cmp(&b.id))
    });
    findings
}

fn severity_title(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "Critical",
        Severity::High => "High",
        Severity::Medium => "Medium",
        Severity::Low => "Low",
        Severity::Info => "Info",
    }
}

fn render_findings_by_severity(report: &AuditReport) -> String {
    let findings = sorted_findings(report);
    let mut section = String::from("<section id=\"findings\">\n<h2>Findings</h2>\n");
    if findings.is_empty() {
        section.push_str("<p>No findings recorded.</p>\n</section>\n");
        return section;
    }
    for severity in SEVERITIES {
        let bucket = findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .collect::<Vec<_>>();
        if bucket.is_empty() {
            continue;
        }
        section.push_str(&format!(
            "<h3 class=\"severity-heading severity-{sev}\">{title} ({count})</h3>\n",
            sev = severity.as_str(),
            title = severity_title(severity),
            count = bucket.len()
        ));
        for finding in bucket {
            section.push_str(&render_finding_block(finding));
        }
    }
    section.push_str("</section>\n");
    section
}

/// One finding's full contract: id, title, severity, confidence, status,
/// affected area, evidence, problem, failure scenario, minimal fix,
/// long-term fix when present, regression test, estimated effort — plus a
/// visible marker when `redacted` is true.
fn render_finding_block(finding: &Finding) -> String {
    let id = escape_html(&finding.id);
    let mut block = format!(
        "<article class=\"finding severity-{}\" id=\"{id}\">\n<h4>{id} &mdash; {}</h4>\n",
        finding.severity.as_str(),
        escape_html(&finding.title)
    );
    block.push_str("<dl class=\"finding-meta\">\n");
    block.push_str(&format!(
        "<dt>Severity</dt><dd>{}</dd><dt>Confidence</dt><dd>{}</dd><dt>Status</dt><dd>{}</dd><dt>Estimated effort</dt><dd>{}</dd>\n",
        escape_html(finding.severity.as_str()),
        escape_html(finding.confidence.as_str()),
        escape_html(finding.status.as_str()),
        escape_html(finding.estimated_effort.as_str())
    ));
    if let Some(area) = &finding.affected_area {
        block.push_str(&format!(
            "<dt>Affected area</dt><dd>{}</dd>\n",
            escape_html(area)
        ));
    }
    block.push_str("</dl>\n");
    if finding.redacted {
        block.push_str(
            "<p class=\"redacted-notice\">REDACTED &mdash; this finding's evidence has been withheld from the report.</p>\n",
        );
    }
    block.push_str("<h5>Evidence</h5>\n<ul class=\"evidence-list\">\n");
    for entry in &finding.evidence {
        block.push_str(&render_evidence_entry(entry));
    }
    block.push_str("</ul>\n");
    for (label, text) in [
        ("Problem", &finding.problem),
        ("Failure scenario", &finding.failure_scenario),
        ("Minimal fix", &finding.minimal_fix),
    ] {
        block.push_str(&format!("<h5>{label}</h5>\n<p>{}</p>\n", escape_html(text)));
    }
    if let Some(long_term) = &finding.long_term_fix {
        block.push_str(&format!(
            "<h5>Long-term fix</h5>\n<p>{}</p>\n",
            escape_html(long_term)
        ));
    }
    block.push_str(&format!(
        "<h5>Regression test</h5>\n<p>{}</p>\n",
        escape_html(&finding.regression_test)
    ));
    block.push_str("</article>\n");
    block
}

/// One evidence entry: path with line range when present, modality/source,
/// note.
fn render_evidence_entry(entry: &EvidenceRef) -> String {
    let mut item = format!(
        "<li><span class=\"evidence-kind\">{}</span> ",
        escape_html(entry.kind.as_str())
    );
    if let Some(path) = &entry.path {
        item.push_str(&format!("<code>{}</code>", escape_html(path)));
        if let Some(start) = entry.line_start {
            let end = entry.line_end.unwrap_or(start);
            item.push_str(&format!(" (lines {start}-{end})"));
        }
        item.push(' ');
    }
    if let Some(modality) = entry.modality {
        item.push_str(&format!("[{}] ", escape_html(modality.as_str())));
    }
    item.push_str(&format!("&mdash; {}", escape_html(&entry.source)));
    if let Some(note) = &entry.note {
        item.push_str(&format!(": {}", escape_html(note)));
    }
    item.push_str("</li>\n");
    item
}

/// Findings ordered by severity then department (the same total order as
/// `render_findings_by_severity`, unbucketed) — a compact work queue that
/// links back to each finding's full detail above.
fn render_fix_order(report: &AuditReport) -> String {
    let findings = sorted_findings(report);
    let mut section = String::from(
        "<section id=\"fix-order\">\n<h2>Fix Order</h2>\n<p>Findings ordered by severity, then department.</p>\n",
    );
    if findings.is_empty() {
        section.push_str("<p>No findings recorded.</p>\n</section>\n");
        return section;
    }
    section.push_str(
        "<table>\n<thead><tr><th>#</th><th>ID</th><th>Severity</th><th>Department</th><th>Title</th></tr></thead>\n<tbody>\n",
    );
    for (index, finding) in findings.iter().enumerate() {
        let id = escape_html(&finding.id);
        let severity = escape_html(finding.severity.as_str());
        section.push_str(&format!(
            "<tr><td>{order}</td><td><a href=\"#{id}\">{id}</a></td><td class=\"severity-{severity}\">{severity}</td><td>{dept}</td><td>{title}</td></tr>\n",
            order = index + 1,
            dept = escape_html(&finding.department),
            title = escape_html(&finding.title)
        ));
    }
    section.push_str("</tbody>\n</table>\n</section>\n");
    section
}

#[cfg(test)]
#[path = "render_html_tests.rs"]
mod tests;
