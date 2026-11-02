#[path = "../src/adapter_contract.rs"]
mod adapter_contract;
#[path = "../src/artifact_ref.rs"]
mod artifact_ref;
#[path = "../src/audit_report/mod.rs"]
mod audit_report;
#[path = "../src/capability.rs"]
mod capability;
#[path = "../src/capability_inventory.rs"]
mod capability_inventory;
mod common;
#[path = "../src/snapshot.rs"]
mod snapshot;
#[path = "../src/stable_artifact.rs"]
mod stable_artifact;

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../orchestration/schemas")
}

/// The registry's own payload validator for a contract, so these tests judge
/// payloads with exactly what production uses.
fn payload_validator(schema: &str, artifact_type: &str) -> fn(&[u8]) -> Result<(), String> {
    artifact_ref::registered_contract(&json!({"artifactSchema": schema, "type": artifact_type}))
        .expect("contract is registered")
        .validate_payload
}

fn schema_validator(id: &str) -> jsonschema::Validator {
    let text = fs::read_to_string(schemas_dir().join(format!("{id}.schema.json")))
        .expect("schema is published");
    let schema: Value = serde_json::from_str(&text).expect("schema is JSON");
    assert_eq!(schema["$id"], id);
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    jsonschema::validator_for(&schema).expect("schema compiles")
}

/// The validator accepts `good`, the schema accepts it too.
fn assert_accepted_by_both(
    validate: fn(&[u8]) -> Result<(), String>,
    validator: &jsonschema::Validator,
    good: &Value,
) {
    let bytes = serde_json::to_vec(good).unwrap();
    assert_eq!(
        validate(&bytes),
        Ok(()),
        "validator rejects the fixture {good}"
    );
    assert!(validator.is_valid(good), "schema rejects {good}");
}

/// Neither the validator nor the schema accepts `bad`.
fn assert_rejected_by_both(
    validate: fn(&[u8]) -> Result<(), String>,
    validator: &jsonschema::Validator,
    bad: &Value,
) {
    let bytes = serde_json::to_vec(bad).unwrap();
    assert!(validate(&bytes).is_err(), "validator accepts {bad}");
    assert!(!validator.is_valid(bad), "schema accepts {bad}");
}

/// Shared negative cases: extra top-level key, missing `schema`, wrong `schema`.
fn assert_envelope_rejections(
    validate: fn(&[u8]) -> Result<(), String>,
    validator: &jsonschema::Validator,
    good: &Value,
) {
    let mut extra = good.clone();
    extra["unexpected"] = json!(true);
    assert_rejected_by_both(validate, validator, &extra);
    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("schema");
    assert_rejected_by_both(validate, validator, &missing);
    let mut wrong = good.clone();
    wrong["schema"] = json!("code-intel-other.v1");
    assert_rejected_by_both(validate, validator, &wrong);
}

#[test]
fn surgery_plan_schema_agrees_with_validator() {
    let validator = schema_validator("code-intel-surgery-plan.v1");
    let good = json!({
        "schema": "code-intel-surgery-plan.v1",
        "status": "planned",
        "admission": {"anything": 1},
        "primary_target": {},
        "operating_plan": [1, "two"],
        "verification": [],
        "discharge_criteria": [null]
    });
    assert_accepted_by_both(
        payload_validator("code-intel-surgery-plan.v1", "diagnosis.surgery-plan"),
        &validator,
        &good,
    );
    let mut not_required = good.clone();
    not_required["status"] = json!("not_required");
    assert_accepted_by_both(
        payload_validator("code-intel-surgery-plan.v1", "diagnosis.surgery-plan"),
        &validator,
        &not_required,
    );
    assert_envelope_rejections(
        payload_validator("code-intel-surgery-plan.v1", "diagnosis.surgery-plan"),
        &validator,
        &good,
    );
    let mut bad_status = good.clone();
    bad_status["status"] = json!("done");
    assert_rejected_by_both(
        payload_validator("code-intel-surgery-plan.v1", "diagnosis.surgery-plan"),
        &validator,
        &bad_status,
    );
}

#[test]
fn method_catalog_schema_agrees_with_validator() {
    let validator = schema_validator("code-intel-method-catalog.v1");
    let text = fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../orchestration/methods/catalog.v1.json"),
    )
    .expect("the shipped catalog is readable");
    let good: Value = serde_json::from_str(&text).unwrap();
    assert_accepted_by_both(
        payload_validator("code-intel-method-catalog.v1", "method.catalog"),
        &validator,
        &good,
    );
    assert_envelope_rejections(
        payload_validator("code-intel-method-catalog.v1", "method.catalog"),
        &validator,
        &good,
    );
    let mut newline_id = good.clone();
    newline_id["cards"] = json!([{"id": "a\nb", "path": "cards/a\nb.v1.json"}]);
    assert_accepted_by_both(
        payload_validator("code-intel-method-catalog.v1", "method.catalog"),
        &validator,
        &newline_id,
    );
    let mut empty = good.clone();
    empty["cards"] = json!([]);
    assert_rejected_by_both(
        payload_validator("code-intel-method-catalog.v1", "method.catalog"),
        &validator,
        &empty,
    );
}

#[test]
fn anchor_verification_schema_agrees_with_validator() {
    let validator = schema_validator("code-intel-anchor-verification.v1");
    let good = json!({
        "schema": "code-intel-anchor-verification.v1",
        "counts": {"verified": 2, "approximate": 1, "dropped": 1},
        "sources": [
            {
                "artifactType": "a.type",
                "artifactPath": "a.json",
                "anchorKind": "file",
                "anchors": [
                    {"path": "src/a.rs", "state": "verified"},
                    {"path": "src/b.rs", "state": "dropped", "reason": "gone"}
                ]
            },
            {
                "artifactType": "b.type",
                "artifactPath": "b.json",
                "anchorKind": "symbol",
                "anchors": [
                    {"file": "src/a.rs", "name": "f", "claimedLine": 3, "state": "verified"},
                    {"file": "src/a.rs", "name": "g", "claimedLine": 9, "state": "approximate", "resolvedLine": 11, "extra": true}
                ]
            }
        ]
    });
    assert_accepted_by_both(
        payload_validator("code-intel-anchor-verification.v1", "verification.anchors"),
        &validator,
        &good,
    );
    assert_envelope_rejections(
        payload_validator("code-intel-anchor-verification.v1", "verification.anchors"),
        &validator,
        &good,
    );
    let mut bad_kind = good.clone();
    bad_kind["sources"][0]["anchorKind"] = json!("line");
    assert_rejected_by_both(
        payload_validator("code-intel-anchor-verification.v1", "verification.anchors"),
        &validator,
        &bad_kind,
    );
}
