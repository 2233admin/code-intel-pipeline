use super::*;
use serde_json::json;
use std::{fs, path::Path};

fn fixture_bytes() -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audit/audit-report.v1.example.json"),
    )
    .unwrap()
}

fn fixture_value() -> Value {
    serde_json::from_slice(&fixture_bytes()).unwrap()
}

#[test]
fn rejects_unknown_top_level_field() {
    let mut value = fixture_value();
    value["bogus"] = json!(true);
    let error = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert!(error.contains("unrecognized field"), "{error}");
}

#[test]
fn rejects_finding_with_no_evidence_entries() {
    let mut value = fixture_value();
    value["findings"][0]["evidence"] = json!([]);
    let error = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert!(error.contains("at least one entry"), "{error}");
}

#[test]
fn finding_id_pattern_accepts_and_rejects_expected_shapes() {
    assert!(is_finding_id("security-001"));
    assert!(is_finding_id("ai-safety-042"));
    assert!(!is_finding_id("security-1"));
    assert!(!is_finding_id("Security-001"));
    assert!(!is_finding_id("-001"));
    assert!(!is_finding_id("security001"));
    assert!(!is_finding_id("abc-12x4"));
}

#[test]
fn finding_id_pattern_rejects_multibyte_utf8_without_panicking() {
    // Regression test: `value.len()` counts bytes, not chars, so a naive
    // `str::split_at(value.len() - 4)` panics on non-ASCII input whose
    // byte length lands the split inside a multi-byte character. "日日"
    // is 6 bytes (two 3-byte characters); this must return `false`
    // cleanly rather than panicking.
    assert!(!is_finding_id("日日"));
}
