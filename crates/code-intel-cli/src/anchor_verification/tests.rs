use super::*;

#[test]
fn verified_round_trips_through_json() {
    let state = AnchorState::Verified;
    assert_eq!(AnchorState::from_json(&state.to_json()).unwrap(), state);
}

#[test]
fn approximate_round_trips_through_json() {
    let state = AnchorState::Approximate { resolved_line: 42 };
    assert_eq!(AnchorState::from_json(&state.to_json()).unwrap(), state);
}

#[test]
fn dropped_round_trips_through_json() {
    let state = AnchorState::Dropped {
        reason: "symbol \"foo\" no longer resolves in a.rs".to_string(),
    };
    assert_eq!(AnchorState::from_json(&state.to_json()).unwrap(), state);
}

#[test]
fn verified_without_state_is_rejected() {
    assert!(AnchorState::from_json(&json!({})).is_err());
}

#[test]
fn dropped_without_reason_is_rejected() {
    let forged = json!({ "state": "dropped" });
    assert!(AnchorState::from_json(&forged).is_err());
}

#[test]
fn approximate_without_resolved_line_is_rejected() {
    let forged = json!({ "state": "approximate" });
    assert!(AnchorState::from_json(&forged).is_err());
}

/// The forgery that actually matters, mirroring G1's own key test: a real
/// `Dropped` value already has a genuine `reason` -- touching only
/// `state` to relabel it `"verified"` must be caught by the leftover
/// `reason` key, since `"verified"` alone needs nothing else.
#[test]
fn dropped_relabeled_verified_by_state_only_is_rejected() {
    let dropped = AnchorState::Dropped {
        reason: "file not found in repository: a.rs".to_string(),
    };
    let mut forged = dropped.to_json();
    assert!(
        AnchorState::from_json(&forged).is_ok(),
        "sanity: the real value must itself parse"
    );
    forged["state"] = json!("verified");
    assert!(
        AnchorState::from_json(&forged).is_err(),
        "a verified claim forged from a dropped one by touching only state, \
         leaving its real reason intact, must be rejected: {forged}"
    );
}

/// Same forgery, the other direction: a real `Approximate` relabeled
/// `"verified"` leaves `resolvedLine` behind.
#[test]
fn approximate_relabeled_verified_by_state_only_is_rejected() {
    let approximate = AnchorState::Approximate { resolved_line: 7 };
    let mut forged = approximate.to_json();
    forged["state"] = json!("verified");
    assert!(AnchorState::from_json(&forged).is_err());
}

#[test]
fn verify_file_anchor_is_verified_when_file_exists() {
    let root = crate::test_support::unique_temp_dir("file-verified");
    fs::create_dir_all(&root).expect("fixture root");
    fs::write(root.join("present.rs"), b"fn present() {}").expect("fixture file");

    assert_eq!(
        verify_file_anchor(&root, "present.rs"),
        AnchorState::Verified
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn verify_file_anchor_is_dropped_when_file_missing() {
    let root = crate::test_support::unique_temp_dir("file-dropped");
    fs::create_dir_all(&root).expect("fixture root");

    assert!(matches!(
        verify_file_anchor(&root, "gone.rs"),
        AnchorState::Dropped { .. }
    ));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn verify_file_anchor_is_dropped_when_path_escapes_repository() {
    let root = crate::test_support::unique_temp_dir("file-escape");
    fs::create_dir_all(&root).expect("fixture root");

    assert!(matches!(
        verify_file_anchor(&root, "../outside.rs"),
        AnchorState::Dropped { .. }
    ));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn verify_symbol_anchor_is_verified_at_the_exact_claimed_line() {
    let root = crate::test_support::unique_temp_dir("symbol-verified");
    fs::create_dir_all(&root).expect("fixture root");
    fs::write(&root.join("lib.rs"), "fn alpha() {}\nfn beta() {}\n").expect("fixture file");

    assert_eq!(
        verify_symbol_anchor(&root, "lib.rs", "beta", 2),
        AnchorState::Verified
    );

    fs::remove_dir_all(&root).ok();
}

/// The acceptance-criterion scenario: a product is "frozen" (its anchor
/// claims a symbol at a line), then the target file changes underneath
/// it (the symbol moves to another line). The gate must degrade the
/// anchor to `Approximate` with the corrected line, not keep reporting
/// the stale claim as verified.
#[test]
fn verify_symbol_anchor_degrades_to_approximate_after_the_symbol_moves() {
    let root = crate::test_support::unique_temp_dir("symbol-drifted");
    fs::create_dir_all(&root).expect("fixture root");
    let target = root.join("lib.rs");
    fs::write(&target, "fn alpha() {}\nfn beta() {}\n").expect("fixture file (frozen state)");

    // The anchor was frozen claiming "beta" at line 2.
    let frozen = verify_symbol_anchor(&root, "lib.rs", "beta", 2);
    assert_eq!(frozen, AnchorState::Verified, "sanity: frozen claim holds");

    // The target file changes: a line is inserted above "beta", pushing
    // it from line 2 to line 3.
    fs::write(&target, "fn alpha() {}\nfn inserted() {}\nfn beta() {}\n")
        .expect("mutate fixture file after freeze");

    let after_change = verify_symbol_anchor(&root, "lib.rs", "beta", 2);
    assert_eq!(
        after_change,
        AnchorState::Approximate { resolved_line: 3 },
        "the stale line-2 claim must degrade to approximate, corrected to line 3"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn verify_symbol_anchor_is_dropped_when_the_symbol_is_removed() {
    let root = crate::test_support::unique_temp_dir("symbol-removed");
    fs::create_dir_all(&root).expect("fixture root");
    let target = root.join("lib.rs");
    fs::write(&target, "fn alpha() {}\nfn beta() {}\n").expect("fixture file (frozen state)");

    assert_eq!(
        verify_symbol_anchor(&root, "lib.rs", "beta", 2),
        AnchorState::Verified,
        "sanity: frozen claim holds"
    );

    // "beta" is deleted outright rather than moved.
    fs::write(&target, "fn alpha() {}\n").expect("remove symbol after freeze");

    assert!(matches!(
        verify_symbol_anchor(&root, "lib.rs", "beta", 2),
        AnchorState::Dropped { .. }
    ));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn verify_symbol_anchor_is_dropped_when_the_file_itself_is_deleted() {
    let root = crate::test_support::unique_temp_dir("file-deleted-after-freeze");
    fs::create_dir_all(&root).expect("fixture root");
    let target = root.join("lib.rs");
    fs::write(&target, "fn alpha() {}\n").expect("fixture file (frozen state)");

    assert_eq!(
        verify_symbol_anchor(&root, "lib.rs", "alpha", 1),
        AnchorState::Verified,
        "sanity: frozen claim holds"
    );

    fs::remove_file(&target).expect("delete target file after freeze");

    assert!(matches!(
        verify_symbol_anchor(&root, "lib.rs", "alpha", 1),
        AnchorState::Dropped { .. }
    ));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn anchor_counts_records_each_state_exhaustively() {
    let mut counts = AnchorCounts::default();
    counts.record(&AnchorState::Verified);
    counts.record(&AnchorState::Verified);
    counts.record(&AnchorState::Approximate { resolved_line: 3 });
    counts.record(&AnchorState::Dropped {
        reason: "gone".to_string(),
    });
    assert_eq!(
        counts,
        AnchorCounts {
            verified: 2,
            approximate: 1,
            dropped: 1,
        }
    );
    assert_eq!(
        counts.to_json(),
        json!({"verified":2,"approximate":1,"dropped":1})
    );
}

#[test]
fn verify_and_report_produces_all_three_states_from_one_manifest() {
    let root = crate::test_support::unique_temp_dir("verify-and-report");
    fs::create_dir_all(&root).expect("fixture root");
    fs::write(&root.join("present.rs"), "fn kept() {}\n").expect("fixture file");
    // "gone.rs" is referenced by the fixture artifacts below but never
    // created, so it resolves to Dropped.

    let node_dir = root.join("evidence.native-code");
    fs::create_dir_all(&node_dir).expect("fixture node dir");
    let ranking = json!({
        "schema":"agent-code-slice-ranking.v1",
        "strategy":"native-evidence-default",
        "files":[
            {"path":"present.rs","language":"rust","score":1,"reasons":["inventory"],"symbols":Value::Null,"imports":Value::Null},
            {"path":"gone.rs","language":"rust","score":1,"reasons":["inventory"],"symbols":Value::Null,"imports":Value::Null},
        ],
    });
    fs::write(
        node_dir.join("ranking.json"),
        serde_json::to_vec(&ranking).unwrap(),
    )
    .expect("write fixture ranking.json");

    let manifest = json!({
        "schema":"code-intel-run-manifest.v1",
        "runIdentity":"fixture",
        "snapshotIdentity":"fixture",
        "outcome":"completed",
        "nodes":{
            "evidence.native-code":{
                "status":"succeeded",
                "verdict":"pass",
                "artifacts":[{
                    "schema":"code-intel-artifact-ref.v1",
                    "artifactSchema":"agent-code-slice-ranking.v1",
                    "type":"code_evidence.agent_slice",
                    "path":"evidence.native-code/ranking.json",
                    "sha256":"0".repeat(64),
                    "consumedSnapshotIdentity":"fixture",
                }],
            },
        },
    });

    let (report, counts) = verify_and_report(&root, &root, &manifest).expect("gate runs");
    assert_eq!(counts.verified, 1);
    assert_eq!(counts.dropped, 1);
    assert_eq!(counts.approximate, 0);
    assert_eq!(report["schema"], ANCHOR_VERIFICATION_SCHEMA);
    assert_eq!(report["counts"], counts.to_json());
    assert_eq!(report["sources"].as_array().unwrap().len(), 1);

    fs::remove_dir_all(&root).ok();
}
