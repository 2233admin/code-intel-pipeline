#[path = "../src/method_catalog.rs"]
mod method_catalog;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "code-intel-method-catalog-{label}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn root() -> PathBuf {
    option_env!("CODE_INTEL_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn seed_documents() -> (Value, Vec<(String, Value)>) {
    let methods = root().join("orchestration/methods");
    let index: Value =
        serde_json::from_slice(&fs::read(methods.join("catalog.v1.json")).unwrap()).unwrap();
    let cards = index["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let path = entry["path"].as_str().unwrap().to_string();
            let card = serde_json::from_slice(&fs::read(methods.join(&path)).unwrap()).unwrap();
            (path, card)
        })
        .collect();
    (index, cards)
}

#[test]
fn seed_catalog_loads_all_nine_methods_in_stable_order() {
    let catalog = method_catalog::load_catalog(&root().join("orchestration/methods")).unwrap();
    let ids = catalog
        .cards()
        .iter()
        .map(|card| card["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "contract-testing",
            "critical-path-pert",
            "fault-tree-analysis",
            "fmea",
            "pdca",
            "root-cause-analysis",
            "spc",
            "strangler-migration",
            "value-stream-queue-delay",
        ]
    );
}

#[test]
fn every_card_has_the_frozen_engineering_fields_without_execution_claims() {
    let catalog = method_catalog::load_catalog(&root().join("orchestration/methods")).unwrap();
    for card in catalog.cards() {
        for field in [
            "problemSignals",
            "requiredEvidence",
            "assumptions",
            "deterministicSteps",
            "outputs",
            "confidenceRules",
            "contraindications",
            "implementationPorts",
        ] {
            assert!(
                !card[field].as_array().unwrap().is_empty(),
                "{}:{field}",
                card["id"]
            );
        }
        assert_eq!(
            card["executionPolicy"],
            "catalog_only_no_selection_or_execution"
        );
        assert!(!serde_json::to_string(card)
            .unwrap()
            .to_ascii_lowercase()
            .contains("method executed"));
    }
}

#[test]
fn missing_contraindications_or_confidence_rules_fail_closed() {
    let (index, cards) = seed_documents();
    for missing in ["contraindications", "confidenceRules"] {
        let mut candidate = cards.clone();
        candidate[0].1.as_object_mut().unwrap().remove(missing);
        let error = method_catalog::validate_documents(&index, &candidate).unwrap_err();
        assert!(error.to_string().contains(missing), "{error}");
    }
}

#[test]
fn unknown_fields_fail_closed_at_catalog_card_and_nested_levels() {
    let (index, cards) = seed_documents();
    let mut bad_index = index.clone();
    bad_index["automaticSelection"] = json!(true);
    assert!(method_catalog::validate_documents(&bad_index, &cards).is_err());

    let mut bad_card = cards.clone();
    bad_card[0].1["selected"] = json!(true);
    assert!(method_catalog::validate_documents(&index, &bad_card).is_err());

    let mut bad_nested = cards.clone();
    bad_nested[0].1["cost"]["score"] = json!(99);
    assert!(method_catalog::validate_documents(&index, &bad_nested).is_err());
}

#[test]
fn duplicate_ids_unsorted_index_and_unknown_method_references_are_rejected() {
    let (index, cards) = seed_documents();

    let mut duplicate = cards.clone();
    duplicate[1].1["id"] = duplicate[0].1["id"].clone();
    assert!(method_catalog::validate_documents(&index, &duplicate).is_err());

    let mut unsorted = index.clone();
    unsorted["cards"].as_array_mut().unwrap().swap(0, 1);
    assert!(method_catalog::validate_documents(&unsorted, &cards).is_err());

    let mut unknown = cards.clone();
    unknown[0].1["relatedMethodIds"] = json!(["not-a-method"]);
    assert!(method_catalog::validate_documents(&index, &unknown).is_err());
}

#[test]
fn unversioned_paths_and_unregistered_nested_cards_fail_closed() {
    let (index, cards) = seed_documents();
    let mut unversioned_index = index.clone();
    let mut unversioned_cards = cards.clone();
    unversioned_index["cards"][0]["path"] = json!("cards/unversioned.json");
    unversioned_cards[0].0 = "cards/unversioned.json".to_string();
    assert!(method_catalog::validate_documents(&unversioned_index, &unversioned_cards).is_err());

    let temp = TempTree::new("nested-card");
    let source = root().join("orchestration/methods");
    let target = temp.0.join("methods");
    fs::create_dir_all(target.join("cards/rogue")).unwrap();
    fs::copy(
        source.join("catalog.v1.json"),
        target.join("catalog.v1.json"),
    )
    .unwrap();
    for entry in fs::read_dir(source.join("cards")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), target.join("cards").join(entry.file_name())).unwrap();
    }
    fs::write(target.join("cards/rogue/unlisted.json"), b"{}\n").unwrap();
    let error = method_catalog::load_catalog(&target).unwrap_err();
    assert!(error.to_string().contains("unregistered non-card entry"));
}

#[test]
fn step_references_are_closed_over_declared_evidence_steps_and_outputs() {
    let (index, cards) = seed_documents();
    let mut unknown_evidence = cards.clone();
    unknown_evidence[0].1["deterministicSteps"][0]["requires"] = json!(["evidence:not-declared"]);
    assert!(method_catalog::validate_documents(&index, &unknown_evidence).is_err());

    let mut forward_step = cards.clone();
    let second_id = forward_step[0].1["deterministicSteps"][1]["id"]
        .as_str()
        .unwrap()
        .to_string();
    forward_step[0].1["deterministicSteps"][0]["requires"] = json!([format!("step:{second_id}")]);
    assert!(method_catalog::validate_documents(&index, &forward_step).is_err());

    let mut unknown_output = cards.clone();
    unknown_output[0].1["deterministicSteps"][0]["produces"] = json!(["not-declared"]);
    assert!(method_catalog::validate_documents(&index, &unknown_output).is_err());
}

#[test]
fn seed_cards_remain_distinct_and_preserve_method_specific_preconditions() {
    let catalog = method_catalog::load_catalog(&root().join("orchestration/methods")).unwrap();
    let distinct_names = catalog
        .cards()
        .iter()
        .map(|card| card["name"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(distinct_names.len(), 9);
    let required = catalog
        .cards()
        .iter()
        .map(|card| {
            (
                card["id"].as_str().unwrap(),
                card["requiredEvidence"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|entry| entry["id"].as_str().unwrap())
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert!(required["critical-path-pert"].contains("activity-duration-estimates"));
    assert!(required["spc"].contains("time-ordered-measurements"));
    assert!(required["fault-tree-analysis"].contains("top-event-definition"));
    assert!(required["contract-testing"].contains("consumer-provider-contracts"));
}

#[test]
fn checked_schema_is_versioned_closed_and_requires_the_frozen_fields() {
    let schema: Value = serde_json::from_slice(
        &fs::read(root().join("orchestration/schemas/code-intel-method-card.v1.schema.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["$id"], "code-intel-method-card.v1");
    assert_eq!(schema["$defs"]["card"]["additionalProperties"], false);
    assert_eq!(schema["$defs"]["cost"]["additionalProperties"], false);
    let required = schema["$defs"]["card"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    for field in [
        "contraindications",
        "confidenceRules",
        "implementationPorts",
    ] {
        assert!(required.contains(field));
    }
}
