#[path = "../src/assistance_discovery.rs"]
mod assistance_discovery;

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn request() -> Value {
    json!({
        "schema": "code-intel-assistance-discovery-request.v1",
        "gap": {
            "schema": "code-intel-engineering-capability-gap.v1",
            "id": "gap-schema-validation",
            "capability": "validate versioned JSON contracts offline",
            "description": "The runtime needs deterministic offline contract checks.",
            "constraints": ["no network", "no new runtime service"],
            "evidenceRefs": ["artifact:gap-analysis"]
        },
        "candidates": [
            {
                "id": "internal-contract-atom",
                "kind": "internal_atom",
                "name": "contract.validate",
                "fit": {"status": "strong", "basis": "already validates adjacent envelopes"},
                "license": {"status": "not_applicable", "basis": "repository-owned code"},
                "security": {"status": "acceptable", "basis": "offline and bounded input"},
                "integration": {"effort": "low", "basis": "shared Rust module"},
                "reversibility": {"status": "high", "basis": "adapter can be removed"},
                "evidenceRefs": ["artifact:internal-inventory"]
            },
            {
                "id": "method-contract-testing",
                "kind": "established_method",
                "name": "contract testing",
                "fit": {"status": "strong", "basis": "matches the boundary problem"},
                "license": {"status": "not_applicable", "basis": "engineering method"},
                "security": {"status": "acceptable", "basis": "no executable dependency"},
                "integration": {"effort": "low", "basis": "method card already exists"},
                "reversibility": {"status": "high", "basis": "advisory method selection"},
                "evidenceRefs": ["method:contract-testing"]
            },
            {
                "id": "external-validator",
                "kind": "external_tool",
                "name": "candidate validator",
                "fit": {"status": "unknown", "basis": "not yet proven against fixtures"},
                "license": {"status": "review_required", "basis": "license evidence absent"},
                "security": {"status": "review_required", "basis": "supply-chain review absent"},
                "integration": {"effort": "unknown", "basis": "adapter not designed"},
                "reversibility": {"status": "unknown", "basis": "migration surface unknown"},
                "evidenceRefs": ["artifact:external-candidate-note"]
            },
            {
                "id": "docs-json-schema",
                "kind": "documentation",
                "name": "JSON Schema reference",
                "fit": {"status": "partial", "basis": "documents semantics but does not execute"},
                "license": {"status": "acceptable", "basis": "reference use only"},
                "security": {"status": "acceptable", "basis": "non-executable reference"},
                "integration": {"effort": "low", "basis": "link from implementation notes"},
                "reversibility": {"status": "high", "basis": "documentation reference"},
                "evidenceRefs": ["doc:json-schema"]
            }
        ]
    })
}

#[test]
fn named_gap_produces_comparable_proposal_dossiers_without_authority() {
    let result = assistance_discovery::discover(&request()).unwrap();
    assert_eq!(
        result["schema"],
        "code-intel-assistance-discovery-result.v1"
    );
    assert_eq!(result["gapId"], "gap-schema-validation");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["dossiers"].as_array().unwrap().len(), 4);
    assert_eq!(result["dossiers"][0]["kind"], "internal_atom");
    assert_eq!(result["dossiers"][1]["kind"], "established_method");
    assert_eq!(result["dossiers"][2]["kind"], "external_tool");
    assert_eq!(result["dossiers"][3]["kind"], "documentation");
    assert_eq!(result["proposalOnly"], true);
    assert_eq!(result["effects"], json!([]));
    assert_eq!(result["adoptionDecisions"], json!([]));
    assert_eq!(result["committedEngineeringPlans"], json!([]));
}

#[test]
fn unnamed_gap_and_popularity_only_candidates_are_rejected() {
    let mut unnamed = request();
    unnamed["gap"]["id"] = json!("");
    assert!(assistance_discovery::discover(&unnamed).is_err());

    let mut popularity = request();
    popularity["candidates"][2]["fit"]["basis"] = json!("popular on GitHub");
    popularity["candidates"][2]["evidenceRefs"] = json!(["metric:stars"]);
    assert!(assistance_discovery::discover(&popularity)
        .unwrap_err()
        .to_string()
        .contains("popularity"));
}

#[test]
fn duplicate_candidates_and_unknown_fields_fail_closed() {
    let mut duplicate = request();
    duplicate["candidates"][1]["id"] = json!("internal-contract-atom");
    assert!(assistance_discovery::discover(&duplicate).is_err());

    let mut extra = request();
    extra["install"] = json!(true);
    assert!(assistance_discovery::discover(&extra).is_err());
}

#[test]
fn core_has_no_install_network_or_authority_write_surface() {
    let source =
        fs::read_to_string(root().join("crates/code-intel-cli/src/assistance_discovery.rs"))
            .unwrap();
    for forbidden in [
        "std::process::Command",
        "std::net::",
        "fs::write",
        "authority::evaluate",
    ] {
        assert!(
            !source.contains(forbidden),
            "found forbidden surface: {forbidden}"
        );
    }
}

#[test]
fn schema_rating_enums_and_semantic_identity_match_runtime_policy() {
    let dossier: Value = serde_json::from_slice(
        &fs::read(
            root().join("orchestration/schemas/code-intel-assistance-dossier.v1.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        dossier["$defs"]["fit"]["properties"]["status"]["enum"],
        json!(["strong", "partial", "weak", "unknown"])
    );
    assert_eq!(
        dossier["$defs"]["security"]["properties"]["status"]["enum"],
        json!(["acceptable", "review_required", "unacceptable", "unknown"])
    );
    let request_schema: Value =
        serde_json::from_slice(
            &fs::read(root().join(
                "orchestration/schemas/code-intel-assistance-discovery-request.v1.schema.json",
            ))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        request_schema["properties"]["candidates"]["uniqueItems"],
        true
    );
    assert_eq!(
        request_schema["properties"]["candidates"]["x-code-intel-uniqueBy"],
        "id"
    );
}
