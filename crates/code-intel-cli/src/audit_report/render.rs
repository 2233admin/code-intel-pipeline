use serde_json::{json, Map, Value};

use super::{enums::Severity, model::AuditReport};

impl AuditReport {
    /// The compact block embedded in `hospital-report.json.audit`. The rich
    /// score/coverage/findings detail lives in `render_markdown_section`
    /// instead, since the hospital schema's `audit` property is a pointer
    /// plus counters, not the full report.
    pub(crate) fn summary(&self, artifact: &str) -> AuditSummary {
        let findings_total = self.findings.len() as u64;
        let by_severity = if findings_total == 0 {
            None
        } else {
            let count_of = |severity: Severity| {
                let count = self
                    .findings
                    .iter()
                    .filter(|finding| finding.severity == severity)
                    .count() as u64;
                (count > 0).then_some(count)
            };
            Some(BySeverity {
                critical: count_of(Severity::Critical),
                high: count_of(Severity::High),
                medium: count_of(Severity::Medium),
                low: count_of(Severity::Low),
                info: count_of(Severity::Info),
            })
        };
        AuditSummary {
            status: AuditSummaryStatus::Present,
            artifact: Some(artifact.to_string()),
            overall: self.score_dashboard.overall,
            findings_total: Some(findings_total),
            by_severity,
        }
    }
}

// ---------------------------------------------------------------------
// The compact `hospital-report.json.audit` block.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditSummaryStatus {
    Absent,
    Present,
}

impl AuditSummaryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Present => "present",
        }
    }
}

pub(crate) struct BySeverity {
    pub(crate) critical: Option<u64>,
    pub(crate) high: Option<u64>,
    pub(crate) medium: Option<u64>,
    pub(crate) low: Option<u64>,
    pub(crate) info: Option<u64>,
}

impl BySeverity {
    fn to_value(&self) -> Value {
        let mut object = Map::new();
        for (key, count) in [
            ("critical", self.critical),
            ("high", self.high),
            ("medium", self.medium),
            ("low", self.low),
            ("info", self.info),
        ] {
            if let Some(count) = count {
                object.insert(key.to_string(), Value::from(count));
            }
        }
        Value::Object(object)
    }
}

pub(crate) struct AuditSummary {
    pub(crate) status: AuditSummaryStatus,
    pub(crate) artifact: Option<String>,
    pub(crate) overall: Option<f64>,
    pub(crate) findings_total: Option<u64>,
    pub(crate) by_severity: Option<BySeverity>,
}

impl AuditSummary {
    pub(crate) fn to_value(&self) -> Value {
        json!({
            "status": self.status.as_str(),
            "artifact": self.artifact,
            "overall": self.overall,
            "findings_total": self.findings_total,
            "by_severity": self.by_severity.as_ref().map(BySeverity::to_value),
        })
    }
}

pub(super) fn format_score(score: Option<f64>) -> String {
    match score {
        Some(value) => format!("{value:.1}"),
        None => "n/a".to_string(),
    }
}

/// Makes free text safe to interpolate into a markdown table cell (or the
/// top-findings list, which uses the same `|`-separated shape): collapses
/// embedded line breaks to a single space so one report field can never
/// split a row across multiple lines, and escapes literal `|` so it can't be
/// mistaken for a column separator. Applied to department-authored free
/// text (justifications, finding titles, evidence/exclusion lists) — not to
/// this module's own enum wire strings or validated ids, which cannot
/// contain either character.
fn escape_table_cell(text: &str) -> String {
    text.replace("\r\n", " ")
        .replace('\n', " ")
        .replace('|', "\\|")
}

/// The rich `## Audit` section for `hospital.md`: score table, coverage
/// matrix, and up to 10 findings sorted by severity (most severe first).
pub(crate) fn render_markdown_section(report: &AuditReport) -> String {
    let mut section = format!(
        "\n## Audit\n\nOverall: {}\n\n### Score Dashboard\n\n| Department | Score | Justification |\n| --- | --- | --- |\n",
        format_score(report.score_dashboard.overall)
    );
    for entry in &report.score_dashboard.entries {
        section.push_str(&format!(
            "| {} | {} | {} |\n",
            entry.department,
            format_score(entry.score),
            escape_table_cell(&entry.justification)
        ));
    }
    section.push_str(
        "\n### Coverage Matrix\n\n| Department | Coverage | Inspected Evidence | Exclusions |\n| --- | --- | --- | --- |\n",
    );
    for row in &report.coverage_matrix {
        section.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            row.department,
            row.coverage.as_str(),
            escape_table_cell(&row.inspected_evidence.join(", ")),
            escape_table_cell(&row.exclusions.join(", "))
        ));
    }
    section.push_str("\n### Top Findings\n\n");
    let mut findings = report.findings.iter().collect::<Vec<_>>();
    findings.sort_by_key(|finding| finding.severity.rank());
    if findings.is_empty() {
        section.push_str("- none\n");
    } else {
        for finding in findings.into_iter().take(10) {
            section.push_str(&format!(
                "- {} | {} | {}\n",
                finding.severity.as_str(),
                finding.id,
                escape_table_cell(&finding.title)
            ));
        }
    }
    section
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
