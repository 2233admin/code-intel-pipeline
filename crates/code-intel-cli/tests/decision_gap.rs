#[path = "../src/decision_gap.rs"]
mod decision_gap;

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> Value {
    serde_json::from_slice(
        &fs::read(
            root()
                .join("tests/fixtures/decision-gap")
                .join(format!("{name}.json")),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn unresolved_risk_acceptance_blocks_only_publication() {
    let rules =
        decision_gap::load_rule_table(&root().join("orchestration/decision-gap-rules.v1.json"))
            .unwrap();
    let result = decision_gap::detect(&fixture("risk-acceptance"), &rules).unwrap();

    assert_eq!(
        result["schema"],
        "code-intel-decision-gap-detection-result.v1"
    );
    assert_eq!(result["gaps"].as_array().unwrap().len(), 1);
    let gap = &result["gaps"][0];
    assert_eq!(gap["kind"], "risk_acceptance");
    assert_eq!(gap["blockedDecision"], "publish release with residual risk");
    assert_eq!(gap["recommendedAnswer"]["kind"], "proposal");
    assert_eq!(gap["affectedBranches"], json!(["publication"]));
    assert_eq!(gap["authorityRequired"], true);

    assert_eq!(result["branches"][0]["branchId"], "inventory");
    assert_eq!(result["branches"][0]["status"], "completed");
    assert_eq!(result["branches"][1]["branchId"], "publication");
    assert_eq!(result["branches"][1]["status"], "blocked_decision_gap");
    assert_eq!(result["answersRecorded"], false);
    assert_eq!(result["authorityEvents"], json!([]));
    assert_eq!(result["adoptionDecisions"], json!([]));
    assert_eq!(result["committedEngineeringPlans"], json!([]));
}

#[test]
fn missing_fact_is_discovery_work_not_a_decision_gap() {
    let rules =
        decision_gap::load_rule_table(&root().join("orchestration/decision-gap-rules.v1.json"))
            .unwrap();
    let result = decision_gap::detect(&fixture("missing-fact"), &rules).unwrap();

    assert_eq!(result["gaps"], json!([]));
    assert_eq!(
        result["factDiscovery"][0]["blockerId"],
        "fact-release-owner"
    );
    assert_eq!(
        result["factDiscovery"][0]["missingFactIds"],
        json!(["fact-owner"])
    );
    assert_eq!(result["branches"][0]["status"], "fact_discovery_required");
}

#[test]
fn multiple_gaps_and_branches_have_stable_order() {
    let rules =
        decision_gap::load_rule_table(&root().join("orchestration/decision-gap-rules.v1.json"))
            .unwrap();
    let mut request = fixture("risk-acceptance");
    let second = json!({
        "id": "gap-priority-a",
        "kind": "priority",
        "blockedDecision": "choose the first migration branch",
        "discoverableFactsChecked": [{"factId": "fact-dependencies", "status": "resolved"}],
        "missingFactIds": [],
        "options": [
            {"id": "zeta", "label": "Migrate publication", "consequence": "publication changes first"},
            {"id": "alpha", "label": "Migrate inventory", "consequence": "inventory changes first"}
        ],
        "recommendedOptionId": "alpha",
        "recommendationRationale": "reduce dependency fan-out",
        "affectedBranches": ["inventory"]
    });
    request["branches"][0]["blockers"] = json!([second]);
    request["branches"].as_array_mut().unwrap().reverse();

    let first = decision_gap::detect(&request, &rules).unwrap();
    let second = decision_gap::detect(&request, &rules).unwrap();

    assert_eq!(first, second);
    assert_eq!(first["gaps"][0]["id"], "gap-priority-a");
    assert_eq!(first["gaps"][1]["id"], "gap-risk-release");
    assert_eq!(first["gaps"][0]["options"][0]["id"], "alpha");
    assert_eq!(first["branches"][0]["branchId"], "inventory");
    assert_eq!(first["branches"][1]["branchId"], "publication");
}

#[test]
fn duplicate_branch_and_blocker_ids_are_rejected() {
    let rules =
        decision_gap::load_rule_table(&root().join("orchestration/decision-gap-rules.v1.json"))
            .unwrap();
    let mut duplicate_branch = fixture("risk-acceptance");
    duplicate_branch["branches"][1]["branchId"] = json!("inventory");
    assert!(decision_gap::detect(&duplicate_branch, &rules)
        .unwrap_err()
        .to_string()
        .contains("duplicate branchId"));

    let mut duplicate_blocker = fixture("risk-acceptance");
    let blocker = duplicate_blocker["branches"][1]["blockers"][0].clone();
    duplicate_blocker["branches"][0]["blockers"] = json!([blocker]);
    assert!(decision_gap::detect(&duplicate_blocker, &rules)
        .unwrap_err()
        .to_string()
        .contains("duplicate blocker id"));
}

#[test]
fn unknown_affected_branch_is_rejected() {
    let rules =
        decision_gap::load_rule_table(&root().join("orchestration/decision-gap-rules.v1.json"))
            .unwrap();
    let mut request = fixture("risk-acceptance");
    request["branches"][1]["blockers"][0]["affectedBranches"] = json!(["unknown"]);

    assert!(decision_gap::detect(&request, &rules)
        .unwrap_err()
        .to_string()
        .contains("unknown branch"));
}

#[test]
fn resolved_missing_fact_blocker_is_rejected_instead_of_blocking_work() {
    let rules =
        decision_gap::load_rule_table(&root().join("orchestration/decision-gap-rules.v1.json"))
            .unwrap();
    let mut request = fixture("missing-fact");
    request["branches"][0]["blockers"][0]["discoverableFactsChecked"][0]["status"] =
        json!("resolved");
    request["branches"][0]["blockers"][0]["missingFactIds"] = json!([]);

    assert!(decision_gap::detect(&request, &rules)
        .unwrap_err()
        .to_string()
        .contains("must name an unresolved fact"));
}

#[test]
fn unknown_kind_with_missing_fact_is_rejected() {
    let rules =
        decision_gap::load_rule_table(&root().join("orchestration/decision-gap-rules.v1.json"))
            .unwrap();
    let mut request = fixture("missing-fact");
    request["branches"][0]["blockers"][0]["kind"] = json!("invented_kind");

    assert!(decision_gap::detect(&request, &rules)
        .unwrap_err()
        .to_string()
        .contains("unknown blocker kind"));
}

#[test]
fn detector_has_no_interactive_or_decision_authority_surface() {
    let source =
        fs::read_to_string(root().join("crates/code-intel-cli/src/decision_gap.rs")).unwrap();
    for forbidden in [
        "std::io::stdin",
        "read_line(",
        "evaluate_batch(",
        "authority::",
        "answersRecorded\": true",
    ] {
        assert!(
            !source.contains(forbidden),
            "found forbidden surface: {forbidden}"
        );
    }
}
