use super::*;

fn valid_v2() -> Value {
    let options = Options {
        repo: std::env::current_dir().unwrap(),
        intents: BTreeSet::from(["plan".to_string()]),
        capabilities: BTreeSet::from(["delta-governance".to_string()]),
        preferred: None,
        override_reason: None,
        compatibility_auto: false,
    };
    evaluate(&options, &load_catalog().unwrap(), None).unwrap()
}

fn authority_event() -> Value {
    let mut event = json!({
        "schema":"code-intel-authority-event.v1",
        "id":"authority-workflow-openspec",
        "decision":"approved",
        "approver":{"id":"repository-owner","role":"maintainer"},
        "evidenceIds":["workflow-adapter:openspec"],
        "issuedAt":1,
        "expiresAt":4_000_000_000u64,
        "attestation":{"scheme":"repository-governed-sha256-v1","digest":""}
    });
    reattest(&mut event);
    event
}

fn reattest(event: &mut Value) {
    event["attestation"]["digest"] =
        json!(crate::artifact_ref::content_contract::authority_event_digest(event).unwrap());
}

#[test]
fn v2_artifact_validator_rejects_nested_schema_violations() {
    let valid = valid_v2();
    validate_v2(&valid).unwrap();

    let mut invalid = Vec::new();

    let mut value = valid.clone();
    value["recommendation"] = json!(0);
    invalid.push(value);

    let mut value = valid.clone();
    value["evidence"][0] = json!({"kind":"","value":"evidence"});
    invalid.push(value);

    let mut value = valid.clone();
    value["confidence"] = json!("certain");
    invalid.push(value);

    let mut value = valid.clone();
    value["alternatives"][0]["score"] = Value::Null;
    invalid.push(value);

    let mut value = valid.clone();
    value["alternatives"][0]["capabilities"] = json!("delta-governance");
    invalid.push(value);

    let mut value = valid.clone();
    value["alternatives"][0]["entryActions"][0]["invocations"]["codex"] = json!(1);
    invalid.push(value);

    let mut value = valid.clone();
    value["provenance"]["sourceVersions"][0] = json!({
        "uri":"https://example.invalid/substitute",
        "version":"1.8.0",
        "revision":"d57889664cab4f2f061d236ec3ff82a5578701bb",
        "license":"MIT"
    });
    invalid.push(value);

    let mut value = valid.clone();
    value["manualOverride"] = json!(0);
    invalid.push(value);

    let mut value = valid;
    value["handoffs"] =
        json!([{"intent":"ship","availability":"available","missingCapability":""}]);
    invalid.push(value);

    for (index, value) in invalid.into_iter().enumerate() {
        assert!(
            validate_v2(&value).is_err(),
            "nested invalid case {index} crossed the artifact boundary"
        );
    }
}

#[test]
fn authority_event_validator_matches_closed_public_schema() {
    let valid = authority_event();
    validate_authority_event_bytes(&serde_json::to_vec(&valid).unwrap()).unwrap();

    let mut invalid = Vec::new();

    let mut value = valid.clone();
    value["evidenceIds"] = json!(["workflow-adapter:openspec", "workflow-adapter:openspec"]);
    reattest(&mut value);
    invalid.push(value);

    let mut value = valid.clone();
    value["evidenceIds"] = json!([""]);
    reattest(&mut value);
    invalid.push(value);

    let mut value = valid.clone();
    value["approver"]["extra"] = json!(true);
    reattest(&mut value);
    invalid.push(value);

    let mut value = valid.clone();
    value["issuedAt"] = json!(1.5);
    reattest(&mut value);
    invalid.push(value);

    let mut value = valid.clone();
    value["expiresAt"] = json!(-1);
    reattest(&mut value);
    invalid.push(value);

    let mut value = valid;
    value["attestation"]["extra"] = json!(true);
    reattest(&mut value);
    invalid.push(value);

    for (index, value) in invalid.into_iter().enumerate() {
        assert!(
            validate_authority_event_bytes(&serde_json::to_vec(&value).unwrap()).is_err(),
            "authority invalid case {index} crossed the artifact boundary"
        );
    }
}
