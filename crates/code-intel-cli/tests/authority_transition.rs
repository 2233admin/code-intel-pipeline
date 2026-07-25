#[path = "../src/authority.rs"]
mod authority;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

fn branch(id: &str, actor: &str, from: &str, to: &str, event: Option<Value>) -> Value {
    let mut transition = json!({
        "to":to,
        "outputId":format!("output-{id}"),
        "evidenceIds":["evidence-1"]
    });
    if let Some(event) = event {
        transition["authorityEvent"] = event;
    }
    json!({
        "branchId":id,
        "source":{"kind":actor,"id":format!("source-{id}")},
        "current":{"kind":from,"id":format!("current-{id}")},
        "transition":transition
    })
}

fn approved_event(id: &str) -> Value {
    json!({
        "schema":"code-intel-authority-event.v1",
        "id":id,
        "decision":"approved",
        "approver":{"id":"eng-manager-1","role":"engineering_manager"},
        "evidenceIds":["evidence-1"],
        "issuedAt":1_700_000_000u64,
        "expiresAt":1_700_000_300u64
    })
}

fn repository_signed_event(id: &str) -> Value {
    let mut event = approved_event(id);
    event["approver"] = json!({"id":"code-intel-maintainers","role":"repository_governance"});
    let digest = authority::authority_event_digest(&event).unwrap();
    event["attestation"] = json!({
        "scheme":"repository-governed-sha256-v1",
        "digest":digest
    });
    event
}

fn request(branches: Vec<Value>) -> Value {
    json!({
        "schema":"code-intel-authority-transition-batch.v1",
        "evaluatedAt":1_700_000_100u64,
        "knownEvidenceIds":["evidence-1","evidence-2"],
        "consumedAuthorityEventIds":[],
        "branches":branches
    })
}

fn evaluate(branches: Vec<Value>) -> Value {
    authority::evaluate_batch(&request(branches)).unwrap()
}

#[test]
fn every_declared_edge_is_enforced_and_approved_authority_edges_pass() {
    let cases = [
        ("observed_evidence", "engineering_fact", None),
        ("engineering_fact", "derived_engineering_model", None),
        ("derived_engineering_model", "proposal", None),
        (
            "proposal",
            "adoption_decision",
            Some(approved_event("event-adopt")),
        ),
        (
            "adoption_decision",
            "committed_engineering_plan",
            Some(approved_event("event-commit")),
        ),
        (
            "proposal",
            "committed_engineering_plan",
            Some(approved_event("event-direct-commit")),
        ),
    ];
    for (index, (from, to, event)) in cases.into_iter().enumerate() {
        let actor = if event.is_some() {
            "human"
        } else {
            "deterministic_pipeline"
        };
        let result = evaluate(vec![branch(
            &format!("edge-{index}"),
            actor,
            from,
            to,
            event,
        )]);
        assert_eq!(
            result["branches"][0]["status"], "accepted",
            "{from}->{to}: {result}"
        );
        if result["branches"][0]["authorityEventId"].is_null() {
            assert_eq!(
                result["branches"][0]["effectiveAuthority"],
                "deterministic_policy"
            );
        } else {
            assert_eq!(
                result["branches"][0]["effectiveAuthority"],
                "authority_event"
            );
        }
    }
}

#[test]
fn all_kind_pairs_match_the_checked_policy_edge_table() {
    let policy = authority::policy_document();
    let kinds = policy["artifactKinds"].as_array().unwrap();
    let edges = policy["edges"].as_array().unwrap();
    for from in kinds {
        for to in kinds {
            let declared = edges
                .iter()
                .find(|edge| edge["from"] == *from && edge["to"] == *to);
            let event = declared
                .and_then(|edge| edge["authorityEventRequired"].as_bool())
                .filter(|required| *required)
                .map(|_| {
                    approved_event(&format!(
                        "event-{}-{}",
                        from.as_str().unwrap(),
                        to.as_str().unwrap()
                    ))
                });
            let result = evaluate(vec![branch(
                "matrix",
                "deterministic_pipeline",
                from.as_str().unwrap(),
                to.as_str().unwrap(),
                event,
            )]);
            assert_eq!(
                result["branches"][0]["status"] == "accepted",
                declared.is_some(),
                "{} -> {}: {result}",
                from,
                to
            );
        }
    }
}

#[test]
fn recommender_direct_commit_requires_explicit_approved_authority_event() {
    let rejected = evaluate(vec![branch(
        "reject",
        "recommender",
        "proposal",
        "committed_engineering_plan",
        None,
    )]);
    assert_eq!(rejected["branches"][0]["status"], "rejected");
    let accepted = evaluate(vec![branch(
        "accept",
        "recommender",
        "proposal",
        "committed_engineering_plan",
        Some(approved_event("event-approved")),
    )]);
    assert_eq!(accepted["branches"][0]["status"], "accepted");
    assert_eq!(
        accepted["branches"][0]["effectiveAuthority"],
        "authority_event"
    );
    assert_eq!(
        accepted["branches"][0]["authorityEvent"]["approver"]["id"],
        "eng-manager-1"
    );
    assert_eq!(
        accepted["consumedAuthorityEventIds"],
        json!(["event-approved"])
    );
}

#[test]
fn llm_provider_and_recommender_cannot_create_facts_models_or_unapproved_commitments() {
    for actor in ["llm", "provider", "recommender"] {
        for to in ["engineering_fact", "derived_engineering_model"] {
            let result = evaluate(vec![branch(actor, actor, "observed_evidence", to, None)]);
            assert_eq!(result["branches"][0]["status"], "rejected", "{actor}->{to}");
        }
        for (from, to) in [
            ("proposal", "adoption_decision"),
            ("proposal", "committed_engineering_plan"),
        ] {
            let result = evaluate(vec![branch(actor, actor, from, to, None)]);
            assert_eq!(result["branches"][0]["status"], "rejected", "{actor}->{to}");
        }
    }
}

#[test]
fn replay_missing_approver_unknown_evidence_expired_and_duplicate_event_fail_closed() {
    let mut replay = request(vec![branch(
        "replay",
        "human",
        "proposal",
        "adoption_decision",
        Some(approved_event("event-used")),
    )]);
    replay["consumedAuthorityEventIds"] = json!(["event-used"]);
    assert_eq!(
        authority::evaluate_batch(&replay).unwrap()["branches"][0]["status"],
        "rejected"
    );

    let mut missing = approved_event("event-missing");
    missing["approver"]["id"] = json!("");
    assert_eq!(
        evaluate(vec![branch(
            "missing",
            "human",
            "proposal",
            "adoption_decision",
            Some(missing)
        )])["branches"][0]["status"],
        "rejected"
    );

    let mut unknown = branch(
        "unknown",
        "human",
        "proposal",
        "adoption_decision",
        Some(approved_event("event-unknown")),
    );
    unknown["transition"]["evidenceIds"] = json!(["not-known"]);
    assert_eq!(evaluate(vec![unknown])["branches"][0]["status"], "rejected");

    let mut expired = approved_event("event-expired");
    expired["expiresAt"] = json!(1_700_000_099u64);
    assert_eq!(
        evaluate(vec![branch(
            "expired",
            "human",
            "proposal",
            "adoption_decision",
            Some(expired)
        )])["branches"][0]["status"],
        "rejected"
    );

    let duplicate = approved_event("event-duplicate");
    let result = evaluate(vec![
        branch(
            "duplicate-a",
            "human",
            "proposal",
            "adoption_decision",
            Some(duplicate.clone()),
        ),
        branch(
            "duplicate-b",
            "human",
            "proposal",
            "adoption_decision",
            Some(duplicate),
        ),
    ]);
    assert!(result["branches"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["status"] == "rejected"));
}

#[test]
fn v1_events_remain_compatible_and_repository_attestation_rejects_every_bound_tamper() {
    let known = BTreeSet::from(["evidence-1".to_string(), "evidence-2".to_string()]);
    let required = BTreeSet::from(["evidence-1".to_string()]);
    let consumed = BTreeSet::new();

    let legacy = approved_event("legacy-v1");
    assert!(authority::validate_authority_event(
        &legacy,
        1_700_000_100,
        &known,
        &required,
        &consumed
    )
    .is_ok());
    assert!(authority::validate_signed_authority_event(
        &legacy,
        1_700_000_100,
        &known,
        &required,
        &consumed
    )
    .unwrap_err()
    .contains("attestation is required"));

    let signed = repository_signed_event("signed-v1");
    assert!(authority::validate_signed_authority_event(
        &signed,
        1_700_000_100,
        &known,
        &required,
        &consumed
    )
    .is_ok());

    for (label, mut tampered) in [
        ("actor", signed.clone()),
        ("evidence", signed.clone()),
        ("expiry", signed.clone()),
        ("digest", signed.clone()),
    ] {
        match label {
            "actor" => tampered["approver"]["id"] = json!("untrusted-maintainer"),
            "evidence" => tampered["evidenceIds"] = json!(["evidence-1", "evidence-2"]),
            "expiry" => tampered["expiresAt"] = json!(1_700_000_301u64),
            "digest" => tampered["attestation"]["digest"] = json!("0".repeat(64)),
            _ => unreachable!(),
        }
        assert!(
            authority::validate_signed_authority_event(
                &tampered,
                1_700_000_100,
                &known,
                &required,
                &consumed,
            )
            .is_err(),
            "{label} tamper must fail"
        );
    }
}

#[test]
fn consumed_authority_events_survive_unprotected_batches_and_block_later_replay() {
    let first = evaluate(vec![branch(
        "protected",
        "human",
        "proposal",
        "adoption_decision",
        Some(approved_event("event-sequence")),
    )]);
    assert_eq!(
        first["consumedAuthorityEventIds"],
        json!(["event-sequence"])
    );

    let mut unprotected = request(vec![branch(
        "unprotected",
        "deterministic_pipeline",
        "observed_evidence",
        "engineering_fact",
        None,
    )]);
    unprotected["consumedAuthorityEventIds"] = first["consumedAuthorityEventIds"].clone();
    let second = authority::evaluate_batch(&unprotected).unwrap();
    assert_eq!(second["branches"][0]["status"], "accepted");
    assert_eq!(
        second["consumedAuthorityEventIds"],
        json!(["event-sequence"]),
        "an unprotected batch must not erase replay history"
    );

    let mut replay = request(vec![branch(
        "replay-after-gap",
        "human",
        "proposal",
        "adoption_decision",
        Some(approved_event("event-sequence")),
    )]);
    replay["consumedAuthorityEventIds"] = second["consumedAuthorityEventIds"].clone();
    let third = authority::evaluate_batch(&replay).unwrap();
    assert_eq!(third["branches"][0]["status"], "rejected");
    assert_eq!(
        third["consumedAuthorityEventIds"],
        json!(["event-sequence"])
    );
}

#[test]
fn rejected_transition_preserves_unrelated_analysis_branch() {
    let result = evaluate(vec![
        branch(
            "bad",
            "recommender",
            "proposal",
            "committed_engineering_plan",
            None,
        ),
        branch(
            "good",
            "deterministic_pipeline",
            "observed_evidence",
            "engineering_fact",
            None,
        ),
    ]);
    assert_eq!(result["branches"][0]["status"], "rejected");
    assert!(result["branches"][0]["outputId"].is_null());
    assert_eq!(result["branches"][1]["status"], "accepted");
    assert_eq!(result["branches"][1]["outputId"], "output-good");
}

#[test]
fn policy_has_no_product_priority_or_tool_specific_actor() {
    let policy = authority::policy_document();
    let text = serde_json::to_string(&policy).unwrap().to_ascii_lowercase();
    for forbidden in [
        "priority",
        "roadmap",
        "repowise",
        "codenexus",
        "sentrux",
        "openspec",
    ] {
        assert!(
            !text.contains(forbidden),
            "forbidden policy authority: {forbidden}"
        );
    }
    let actors = policy["actorKinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actors,
        BTreeSet::from([
            "deterministic_pipeline",
            "human",
            "llm",
            "provider",
            "recommender"
        ])
    );
}

#[test]
fn checked_policy_and_closed_schema_match_runtime_contract() {
    let root = option_env!("CODE_INTEL_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let checked: Value = serde_json::from_slice(
        &fs::read(root.join("orchestration/authority-transition-policy.v1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(checked, authority::policy_document());
    let schema: Value = serde_json::from_slice(
        &fs::read(
            root.join("orchestration/schemas/code-intel-authority-transition.v1.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["$id"], "code-intel-authority-transition.v1");
    assert_eq!(schema["$defs"]["batch"]["additionalProperties"], false);
    assert_eq!(schema["$defs"]["event"]["additionalProperties"], false);
    assert_eq!(schema["$defs"]["result"]["additionalProperties"], false);
}
