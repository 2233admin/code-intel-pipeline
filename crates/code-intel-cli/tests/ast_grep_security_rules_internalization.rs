//! `orchestration/internalization/ast-grep-security-rules.json` records the
//! concept-absorption boundary for the bundled ast-grep security rule
//! library: reimplement rung, no upstream code vendored or executed. See
//! `orchestration/internalization/mattpocock-skills.json` for the sibling
//! "reimplement" record this one follows the shape of.
#[path = "../src/authority.rs"]
mod authority;
#[path = "../src/content_contract.rs"]
mod content_contract;
#[path = "../src/internalization_record.rs"]
mod internalization_record;

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn ast_grep_security_rules_record_stays_optional_and_research_only() {
    let record: Value = serde_json::from_slice(
        &fs::read(root().join("orchestration/internalization/ast-grep-security-rules.json"))
            .unwrap(),
    )
    .unwrap();
    let evidence = internalization_record::record_evidence_ids(&record).unwrap();
    let gaps = evidence
        .iter()
        .filter(|id| id.starts_with("gap:"))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        !gaps.is_empty(),
        "expected at least one open gap (third-party CWE mapping review)"
    );
    let admitted = evidence
        .into_iter()
        .filter(|id| !id.starts_with("gap:"))
        .collect::<Vec<_>>();
    let evaluated_at = record["provenance"]["recordedAt"].as_u64().unwrap();
    let evaluation =
        internalization_record::evaluate_record(&record, evaluated_at, &admitted, &[]).unwrap();

    // an open gap keeps production off; research (this session's own use of
    // the bundled rules through scan.ast-grep-security) stays allowed.
    assert_eq!(evaluation["researchAllowed"], true);
    assert_eq!(evaluation["productionEnabled"], false);
    assert_eq!(evaluation["consumedAuthorityEventId"], Value::Null);
    assert_eq!(record["lifecycle"]["authorityEvent"], Value::Null);
    assert_eq!(record["adoption"]["rung"], "reimplement");

    let registry: Value =
        serde_json::from_slice(&fs::read(root().join("orchestration/integrations.json")).unwrap())
            .unwrap();
    let integration = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "scan.ast-grep-security")
        .unwrap();
    assert_eq!(integration["required"], false);
    assert_eq!(integration["stage"], "localization");
    assert_eq!(
        integration["capabilityDeclaration"]["allowedEffects"],
        serde_json::json!(["repo_read", "local_write", "process_spawn"])
    );
}

#[test]
fn every_owned_rule_file_pin_matches_its_current_bytes() {
    let record: Value = serde_json::from_slice(
        &fs::read(root().join("orchestration/internalization/ast-grep-security-rules.json"))
            .unwrap(),
    )
    .unwrap();
    for modification in record["ownedModifications"].as_array().unwrap() {
        let Some(expected) = modification["sha256"].as_str() else {
            continue;
        };
        let path = modification["path"].as_str().unwrap();
        let bytes = fs::read(root().join(path)).unwrap_or_else(|error| {
            panic!("owned modification {path} is unreadable: {error}");
        });
        let actual = content_contract::sha256_hex(&bytes);
        assert_eq!(
            actual, expected,
            "{path} pin is stale; rerun `code-intel repin --write`"
        );
    }
}

