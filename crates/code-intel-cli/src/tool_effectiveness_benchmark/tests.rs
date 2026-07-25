use super::*;

fn run(id: &str, condition: &str, repetition: u64, ranked: Value, diagnosis: Value) -> Value {
    let snapshot_identity = "c".repeat(64);
    let treatment_digest = match condition {
        "C0" => "5".repeat(64),
        "C1" => "6".repeat(64),
        _ => "7".repeat(64),
    };
    let experiment = json!({
        "model":"fixture-model",
        "reasoning":"fixture-reasoning",
        "promptDigest":"1".repeat(64),
        "budgetDigest":"2".repeat(64),
        "permissionProfileDigest":"3".repeat(64),
        "toolSnapshotDigest":"4".repeat(64),
        "seed":repetition
    });
    let profile = json!({
        "snapshotIdentity":snapshot_identity,
        "model":"fixture-model",
        "reasoning":"fixture-reasoning",
        "promptDigest":"1".repeat(64),
        "budgetDigest":"2".repeat(64),
        "permissionProfileDigest":"3".repeat(64),
        "toolSnapshotDigest":"4".repeat(64),
        "seed":repetition
    });
    json!({
        "runId":id,
        "caseId":"case-1",
        "condition":condition,
        "repetition":repetition,
        "status":"completed",
        "snapshotIdentity":snapshot_identity,
        "experiment":experiment,
        "experimentProfileDigest":sha256_hex(&serde_json::to_vec(&profile).unwrap()),
        "treatmentDigest":treatment_digest,
        "contextDigest":"b".repeat(64),
        "rankedEntities":ranked,
        "diagnosisEntities":diagnosis,
        "attestationRef":{
            "schema":"code-intel-artifact-ref.v1",
            "artifactSchema":"code-intel-external-attestation.v1",
            "type":"benchmark.external-attestation",
            "path":format!("attestations/{id}.json"),
            "sha256":"e".repeat(64),
            "consumedSnapshotIdentity":"c".repeat(64)
        },
        "unsupportedClaims":0,
        "claimCount":1,
        "wallTimeMs":100,
        "inputTokens":10,
        "outputTokens":10,
        "toolCalls":1,
        "artifactBytes":100
    })
}

#[test]
fn paired_baseline_reports_uplift_without_composite_score() {
    let root = "repo://fixture/src/root.rs";
    let noise = "repo://fixture/src/noise.rs";
    let corpus = json!({
        "schema":"code-intel-tool-effectiveness-corpus.v1",
        "corpusId":"fixture-v1",
        "topK":5,
        "repetitions":2,
        "verifierIdentity":"fixture-verifier-v1",
        "treatments":{
            "C0":"5".repeat(64),
            "C1":"6".repeat(64),
            "Cfull":"7".repeat(64)
        },
        "cases":[{
            "id":"case-1",
            "snapshotIdentity":"c".repeat(64),
            "relevantEntities":[root],
            "rootCauseEntities":[root]
        }]
    });
    let mut values = Vec::new();
    let mut attestations = BTreeMap::new();
    for repetition in 1..=2 {
        values.push(run(
            &format!("c0-{repetition}"),
            "C0",
            repetition,
            json!([noise]),
            json!([]),
        ));
        attestations.insert(format!("c0-{repetition}"), false);
        values.push(run(
            &format!("c1-{repetition}"),
            "C1",
            repetition,
            json!([noise, root]),
            json!([root]),
        ));
        attestations.insert(format!("c1-{repetition}"), true);
        values.push(run(
            &format!("full-{repetition}"),
            "Cfull",
            repetition,
            json!([root]),
            json!([root]),
        ));
        attestations.insert(format!("full-{repetition}"), true);
    }
    let runs = json!({
        "schema":"code-intel-tool-effectiveness-runs.v1",
        "corpusId":"fixture-v1",
        "runs":values
    });
    let report = evaluate(&corpus, &runs, &attestations).unwrap();
    assert_eq!(report["verdict"], "baseline_recorded");
    assert_eq!(report["method"]["compositeScore"], false);
    assert_eq!(
        report["pairedComparisons"][0]["rootCauseRecallAtKDelta"],
        1.0
    );
    assert_eq!(
        report["pairedComparisons"][1]["meanReciprocalRankDelta"],
        1.0
    );

    let mut insufficient = runs;
    insufficient["runs"][1]["status"] = json!("unavailable");
    let report = evaluate(&corpus, &insufficient, &attestations).unwrap();
    assert_eq!(report["verdict"], "insufficient_evidence");
}

#[test]
fn changed_experiment_profile_fails_closed() {
    let root = "repo://fixture/src/root.rs";
    let corpus = json!({
        "schema":"code-intel-tool-effectiveness-corpus.v1",
        "corpusId":"fixture-v1",
        "topK":5,
        "repetitions":1,
        "verifierIdentity":"fixture-verifier-v1",
        "treatments":{
            "C0":"5".repeat(64),
            "C1":"6".repeat(64),
            "Cfull":"7".repeat(64)
        },
        "cases":[{
            "id":"case-1",
            "snapshotIdentity":"c".repeat(64),
            "relevantEntities":[root],
            "rootCauseEntities":[root]
        }]
    });
    let mut values = vec![
        run("c0", "C0", 1, json!([]), json!([])),
        run("c1", "C1", 1, json!([root]), json!([root])),
        run("full", "Cfull", 1, json!([root]), json!([root])),
    ];
    values[2]["experiment"]["seed"] = json!(2);
    let changed_profile = json!({
        "snapshotIdentity":"c".repeat(64),
        "model":"fixture-model",
        "reasoning":"fixture-reasoning",
        "promptDigest":"1".repeat(64),
        "budgetDigest":"2".repeat(64),
        "permissionProfileDigest":"3".repeat(64),
        "toolSnapshotDigest":"4".repeat(64),
        "seed":2
    });
    values[2]["experimentProfileDigest"] =
        json!(sha256_hex(&serde_json::to_vec(&changed_profile).unwrap()));
    let attestations = BTreeMap::from([
        ("c0".to_string(), false),
        ("c1".to_string(), true),
        ("full".to_string(), true),
    ]);
    let error = evaluate(
        &corpus,
        &json!({
            "schema":"code-intel-tool-effectiveness-runs.v1",
            "corpusId":"fixture-v1",
            "runs":values
        }),
        &attestations,
    )
    .unwrap_err();
    assert!(error.contains("changed the experiment profile"));
}

#[test]
fn duplicate_ranked_entities_fail_closed() {
    let root = "repo://fixture/src/root.rs";
    let corpus = json!({
        "schema":"code-intel-tool-effectiveness-corpus.v1",
        "corpusId":"fixture-v1",
        "topK":5,
        "repetitions":1,
        "verifierIdentity":"fixture-verifier-v1",
        "treatments":{
            "C0":"5".repeat(64),
            "C1":"6".repeat(64),
            "Cfull":"7".repeat(64)
        },
        "cases":[{
            "id":"case-1",
            "snapshotIdentity":"c".repeat(64),
            "relevantEntities":[root],
            "rootCauseEntities":[root]
        }]
    });
    let values = vec![
        run("c0", "C0", 1, json!([root, root]), json!([root])),
        run("c1", "C1", 1, json!([root]), json!([root])),
        run("full", "Cfull", 1, json!([root]), json!([root])),
    ];
    let attestations = BTreeMap::from([
        ("c0".to_string(), true),
        ("c1".to_string(), true),
        ("full".to_string(), true),
    ]);
    let error = evaluate(
        &corpus,
        &json!({
            "schema":"code-intel-tool-effectiveness-runs.v1",
            "corpusId":"fixture-v1",
            "runs":values
        }),
        &attestations,
    )
    .unwrap_err();
    assert!(error.contains("rankedEntities contains duplicates"));
}

#[test]
fn external_attestation_is_digest_and_snapshot_bound() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "code-intel-tool-attestation-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let snapshot = "c".repeat(64);
    let payload = serde_json::to_vec(&json!({
        "schema":"code-intel-external-attestation.v1",
        "runId":"run-1",
        "caseId":"case-1",
        "snapshotIdentity":snapshot,
        "condition":"C0",
        "treatmentDigest":"5".repeat(64),
        "contextDigest":"b".repeat(64),
        "experimentProfileDigest":"a".repeat(64),
        "passed":true,
        "verifierIdentity":"fixture-verifier-v1",
        "method":"frozen_evidence_replay",
        "evidenceRefs":[format!("artifact://sha256/{}", "8".repeat(64))]
    }))
    .unwrap();
    fs::write(root.join("attestation.json"), &payload).unwrap();
    let corpus = json!({
        "schema":"code-intel-tool-effectiveness-corpus.v1",
        "corpusId":"fixture-v1",
        "topK":5,
        "repetitions":1,
        "verifierIdentity":"fixture-verifier-v1",
        "treatments":{
            "C0":"5".repeat(64),
            "C1":"6".repeat(64),
            "Cfull":"7".repeat(64)
        },
        "cases":[{
            "id":"case-1",
            "snapshotIdentity":"c".repeat(64),
            "relevantEntities":["repo://fixture/src/root.rs"],
            "rootCauseEntities":["repo://fixture/src/root.rs"]
        }]
    });
    let runs = json!({
        "schema":"code-intel-tool-effectiveness-runs.v1",
        "corpusId":"fixture-v1",
        "runs":[{
            "runId":"run-1",
            "caseId":"case-1",
            "condition":"C0",
            "treatmentDigest":"5".repeat(64),
            "contextDigest":"b".repeat(64),
            "experimentProfileDigest":"a".repeat(64),
            "attestationRef":{
                "schema":"code-intel-artifact-ref.v1",
                "artifactSchema":"code-intel-external-attestation.v1",
                "type":"benchmark.external-attestation",
                "path":"attestation.json",
                "sha256":sha256_hex(&payload),
                "consumedSnapshotIdentity":"c".repeat(64)
            }
        }]
    });
    let verified = verify_attestations(&corpus, &runs, &root).unwrap();
    assert_eq!(verified["run-1"], true);

    let mut forged = runs;
    forged["runs"][0]["attestationRef"]["sha256"] = json!("f".repeat(64));
    let error = verify_attestations(&corpus, &forged, &root).unwrap_err();
    assert!(error.contains("SHA-256 mismatch"));
    fs::remove_dir_all(root).unwrap();
}
