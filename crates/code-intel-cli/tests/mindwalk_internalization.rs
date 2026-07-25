#[path = "../src/authority.rs"]
mod authority;
#[path = "../src/internalization_record.rs"]
mod internalization_record;

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn mindwalk_adapter_record_stays_optional_and_research_only() {
    let record: Value = serde_json::from_slice(
        &fs::read(root().join("orchestration/internalization/mindwalk.json")).unwrap(),
    )
    .unwrap();
    let evidence = internalization_record::record_evidence_ids(&record).unwrap();
    let admitted = evidence
        .into_iter()
        .filter(|id| !id.starts_with("gap:"))
        .collect::<Vec<_>>();
    let evaluated_at = record["provenance"]["recordedAt"].as_u64().unwrap();
    let evaluation =
        internalization_record::evaluate_record(&record, evaluated_at, &admitted, &[]).unwrap();

    assert_eq!(evaluation["researchAllowed"], true);
    assert_eq!(evaluation["productionEnabled"], false);
    assert_eq!(evaluation["consumedAuthorityEventId"], Value::Null);
    assert_eq!(record["lifecycle"]["authorityEvent"], Value::Null);

    let registry: Value =
        serde_json::from_slice(&fs::read(root().join("orchestration/integrations.json")).unwrap())
            .unwrap();
    let integration = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "provider.session-adapt")
        .unwrap();
    assert_eq!(integration["required"], false);
    assert_eq!(integration["stage"], "verification");
    assert_eq!(
        record["operationTrace"][0]["command"],
        integration["commands"]["adapt"]
    );
}
