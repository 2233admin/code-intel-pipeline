use super::*;
use serde_json::{json, Value};
use std::{fs, path::Path};

fn fixture_value() -> Value {
    serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audit/audit-report.v1.example.json"),
        )
        .unwrap(),
    )
    .unwrap()
}

fn render(value: &Value) -> String {
    let report = AuditReport::parse(&serde_json::to_vec(value).unwrap()).unwrap();
    render_html_document(&report)
}

#[test]
fn escape_html_escapes_all_five_special_characters() {
    assert_eq!(
        escape_html(r#"a & b < c > d " e ' f"#),
        "a &amp; b &lt; c &gt; d &quot; e &#39; f"
    );
}

#[test]
fn renders_a_complete_self_contained_document_with_sections_in_order() {
    let html = render(&fixture_value());
    assert!(html.starts_with("<!doctype html>"), "{html}");
    assert!(!html.contains("http://"), "{html}");
    assert!(!html.contains("https://"), "{html}");
    assert!(!html.contains("<script"), "{html}");
    assert!(!html.contains("<link "), "{html}");

    let header_pos = html.find("<header>").unwrap();
    let dashboard_pos = html.find("Score Dashboard").unwrap();
    let coverage_pos = html.find("Coverage Matrix").unwrap();
    let findings_pos = html.find("id=\"findings\"").unwrap();
    let fix_order_pos = html.find("id=\"fix-order\"").unwrap();
    assert!(header_pos < dashboard_pos);
    assert!(dashboard_pos < coverage_pos);
    assert!(coverage_pos < findings_pos);
    assert!(findings_pos < fix_order_pos);
}

/// The required regression test: a finding title carrying a live `<script>`
/// tag must appear only in its escaped form, never as markup the browser
/// would execute.
#[test]
fn escapes_a_finding_title_containing_a_script_tag() {
    let mut value = fixture_value();
    value["findings"][0]["title"] = json!("<script>alert(1)</script>");
    let html = render(&value);
    assert!(!html.contains("<script>alert(1)</script>"), "{html}");
    assert!(
        html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "{html}"
    );
}

#[test]
fn redacted_finding_shows_a_visible_marker() {
    let mut value = fixture_value();
    value["findings"][0]["redacted"] = json!(true);
    let html = render(&value);
    assert!(html.contains("REDACTED"), "{html}");
    assert!(html.contains("redacted-notice"), "{html}");
}

#[test]
fn scope_line_is_absent_without_a_scope_block_and_present_with_one() {
    let base = fixture_value();
    let without_scope = render(&base);
    assert!(!without_scope.contains("scope-line"), "{without_scope}");

    let mut with_scope = base;
    with_scope["scope"] = json!({
        "kind": "diff",
        "since": "HEAD~3",
        "files": ["crates/code-intel-cli/src/capability_inventory.rs"]
    });
    let html = render(&with_scope);
    assert!(html.contains("scope-line"), "{html}");
    assert!(html.contains("HEAD~3"), "{html}");
}

#[test]
fn evidence_entry_renders_path_with_line_range() {
    let html = render(&fixture_value());
    assert!(html.contains("capability_inventory.rs"), "{html}");
    assert!(html.contains("(lines 156-165)"), "{html}");
}

#[test]
fn fix_order_links_to_the_finding_anchor() {
    let html = render(&fixture_value());
    assert!(html.contains("href=\"#security-001\""), "{html}");
    assert!(html.contains("id=\"security-001\""), "{html}");
}

#[test]
fn findings_grouped_by_severity_shows_a_heading_with_count() {
    let html = render(&fixture_value());
    assert!(html.contains("Medium (1)"), "{html}");
}

#[test]
fn renders_cleanly_with_zero_findings() {
    let mut value = fixture_value();
    value["findings"] = json!([]);
    let html = render(&value);
    assert!(html.contains("No findings recorded."), "{html}");
}
