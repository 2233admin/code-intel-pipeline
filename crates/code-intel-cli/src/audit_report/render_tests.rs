use super::*;
use std::{fs, path::Path};

fn fixture_bytes() -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audit/audit-report.v1.example.json"),
    )
    .unwrap()
}

#[test]
fn summary_reflects_findings_total_and_severity_counts() {
    let report = AuditReport::parse(&fixture_bytes()).unwrap();
    let summary = report.summary("audit-report.json");
    assert_eq!(summary.status, AuditSummaryStatus::Present);
    assert_eq!(summary.artifact.as_deref(), Some("audit-report.json"));
    assert_eq!(summary.findings_total, Some(1));
    let by_severity = summary.by_severity.unwrap();
    assert_eq!(by_severity.medium, Some(1));
    assert_eq!(by_severity.critical, None);
}

#[test]
fn render_markdown_section_lists_score_coverage_and_top_findings() {
    let report = AuditReport::parse(&fixture_bytes()).unwrap();
    let markdown = render_markdown_section(&report);
    assert!(markdown.contains("## Audit"));
    assert!(markdown.contains("| security |"));
    assert!(markdown.contains("### Coverage Matrix"));
    assert!(markdown.contains("medium | security-001 |"));
}

#[test]
fn render_markdown_section_escapes_pipes_and_newlines_in_free_text() {
    let mut value: Value = serde_json::from_slice(&fixture_bytes()).unwrap();
    value["score_dashboard"]["entries"][0]["justification"] = json!("a|b\nc");
    let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let markdown = render_markdown_section(&report);
    assert!(
        markdown.contains("| security | 7.0 | a\\|b c |"),
        "{markdown}"
    );
    // The embedded "|" and "\n" must not add or split table rows: exactly
    // one line still starts this department's score row (the coverage
    // matrix has its own, differently-shaped "| security |" row, so
    // match on the score column too to identify this one specifically).
    let matching_lines = markdown
        .lines()
        .filter(|line| line.starts_with("| security | 7.0 |"))
        .count();
    assert_eq!(matching_lines, 1);
}
