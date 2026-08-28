use super::*;

fn scan_structured_fixture(
    coupling: f64,
    complex: i64,
    god_files: i64,
    max_complexity: i64,
    cycles: i64,
    quality: i64,
) -> Value {
    json!({
        "quality_signal": quality,
        "coupling_score": coupling,
        "complex_fn_count": complex,
        "god_file_count": god_files,
        "max_complexity": max_complexity,
        "cycle_count": cycles,
    })
}

fn baseline_metrics_fixture(
    coupling: f64,
    complex: i64,
    god_files: i64,
    max_complexity: i64,
    cycles: i64,
    quality: i64,
) -> Value {
    scan_structured_fixture(
        coupling,
        complex,
        god_files,
        max_complexity,
        cycles,
        quality,
    )
}

#[test]
fn quality_signal_section_computes_total_delta_from_current_and_baseline() {
    let current = scan_structured_fixture(4.0, 1, 0, 12, 0, 9200);
    let baseline = baseline_metrics_fixture(6.0, 2, 1, 14, 0, 9000);
    let health = json!({"bottleneck": "coupling"});
    let mut diagnostics = Vec::new();
    let signal = quality_signal_section(
        Some(&current),
        Some(&health),
        Some(&baseline),
        &mut diagnostics,
    );
    assert_eq!(signal["total"]["current"], 9200);
    assert_eq!(signal["total"]["baseline"], 9000);
    assert_eq!(signal["total"]["delta"], 200);
    assert_eq!(signal["bottleneck"], "coupling");
    assert_eq!(signal["formulaVersion"], "legacy_proxy_v0");

    let root_causes = signal["rootCauses"].as_array().expect("root causes array");
    assert_eq!(root_causes.len(), 5);
    let coupling_cause = root_causes
        .iter()
        .find(|entry| entry["id"] == "coupling")
        .expect("coupling root cause present");
    assert_eq!(coupling_cause["raw"]["current"], 4.0);
    assert_eq!(coupling_cause["raw"]["baseline"], 6.0);
    assert_eq!(coupling_cause["raw"]["delta"], -2.0);
    assert!(coupling_cause["score"].is_null());
    assert_eq!(coupling_cause["scoreStatus"], "pending_upstream_formula");
}

#[test]
fn quality_signal_section_is_honest_when_scan_or_baseline_missing() {
    let mut diagnostics = Vec::new();
    let signal = quality_signal_section(None, None, None, &mut diagnostics);
    assert!(signal["total"]["current"].is_null());
    assert!(signal["total"]["baseline"].is_null());
    assert!(signal["total"]["delta"].is_null());
    assert!(signal["bottleneck"].is_null());
    let root_causes = signal["rootCauses"].as_array().unwrap();
    for cause in root_causes {
        assert!(cause["raw"]["current"].is_null());
        assert!(cause["raw"]["baseline"].is_null());
        assert!(cause["score"].is_null());
    }
    assert!(!diagnostics.is_empty());
}

#[test]
fn quality_signal_section_accepts_integral_float_totals_and_diagnoses_unusable_values() {
    let mut current = scan_structured_fixture(4.0, 1, 0, 12, 0, 9200);
    current["quality_signal"] = json!(9200.0);
    let mut baseline = baseline_metrics_fixture(6.0, 2, 1, 14, 0, 9000);
    baseline["quality_signal"] = json!(9000.0);
    let mut diagnostics = Vec::new();
    let signal = quality_signal_section(Some(&current), None, Some(&baseline), &mut diagnostics);
    assert_eq!(signal["total"]["current"], 9200);
    assert_eq!(signal["total"]["baseline"], 9000);
    assert_eq!(signal["total"]["delta"], 200);
    assert!(!diagnostics
        .iter()
        .any(|item| item.contains("quality_signal")));

    current["quality_signal"] = json!(9200.5);
    baseline["quality_signal"] = json!("9000");
    let mut diagnostics = Vec::new();
    let signal = quality_signal_section(Some(&current), None, Some(&baseline), &mut diagnostics);
    assert!(signal["total"]["current"].is_null());
    assert!(signal["total"]["baseline"].is_null());
    assert!(signal["total"]["delta"].is_null());
    assert!(diagnostics.iter().any(|item| item.contains("sentrux.scan")));
    assert!(diagnostics
        .iter()
        .any(|item| item.contains(BASELINE_RELATIVE_PATH)));
}

#[test]
fn quality_signal_section_nulls_and_diagnoses_extreme_delta_overflow() {
    let cases = [
        (
            9_223_372_036_854_774_784.0,
            -9_223_372_036_854_775_808.0,
            9_223_372_036_854_774_784_i64,
            i64::MIN,
        ),
        (
            -9_223_372_036_854_775_808.0,
            9_223_372_036_854_774_784.0,
            i64::MIN,
            9_223_372_036_854_774_784_i64,
        ),
    ];

    for (current_total, baseline_total, expected_current, expected_baseline) in cases {
        let mut current = scan_structured_fixture(4.0, 1, 0, 12, 0, 0);
        current["quality_signal"] = json!(current_total);
        let mut baseline = baseline_metrics_fixture(6.0, 2, 1, 14, 0, 0);
        baseline["quality_signal"] = json!(baseline_total);
        let mut diagnostics = Vec::new();

        let signal =
            quality_signal_section(Some(&current), None, Some(&baseline), &mut diagnostics);

        assert_eq!(signal["total"]["current"], expected_current);
        assert_eq!(signal["total"]["baseline"], expected_baseline);
        assert!(signal["total"]["delta"].is_null());
        let overflow_diagnostics = diagnostics
            .iter()
            .filter(|item| item.contains("Quality Signal delta overflow"))
            .collect::<Vec<_>>();
        assert_eq!(overflow_diagnostics.len(), 1);
        assert!(overflow_diagnostics[0].contains("delta is unavailable"));
    }
}

#[test]
fn normalize_bottleneck_id_maps_engine_ids_and_drops_none() {
    assert_eq!(normalize_bottleneck_id("god_files"), Some("godFiles"));
    assert_eq!(normalize_bottleneck_id("complexity"), Some("complexity"));
    assert_eq!(normalize_bottleneck_id("coupling"), Some("coupling"));
    assert_eq!(normalize_bottleneck_id("none"), None);
    assert_eq!(normalize_bottleneck_id("unknown_future_value"), None);
}

/// Issue #386's forward-compatibility contract with sibling #385: once a
/// `sentrux.scan` payload carries the upstream `root_causes.<id>` shape
/// (#385's documented output), this module must consume it verbatim --
/// real `score` values, not `null`/`pending_upstream_formula` -- without
/// any change to this module's own code.
#[test]
fn root_causes_section_prefers_the_upstream_shape_once_385_lands() {
    let scan_structured = json!({
        "quality_signal": 8800,
        "formula_version": "sentrux-upstream-v1",
        "root_causes": {
            "modularity": {"raw": 0.62, "score": 0.75},
            "acyclicity": {"raw": 2.0, "score": 0.33},
            "depth": {"raw": 5.0, "score": 0.61},
            "equality": {"raw": 0.2, "score": 0.8},
            "redundancy": {"raw": 0.1, "score": 0.9},
        },
    });
    let mut diagnostics = Vec::new();
    let (root_causes, formula_version) =
        root_causes_section(Some(&scan_structured), None, &mut diagnostics);
    assert_eq!(formula_version, "sentrux-upstream-v1");
    let entries = root_causes.as_array().expect("array");
    assert_eq!(entries.len(), 5);
    let modularity = entries
        .iter()
        .find(|entry| entry["id"] == "modularity")
        .expect("modularity present");
    assert_eq!(modularity["raw"]["current"], 0.62);
    assert_eq!(modularity["score"], 0.75);
    assert_eq!(modularity["scoreStatus"], "upstream");
    assert!(diagnostics.is_empty());
}

#[test]
fn root_causes_section_falls_back_to_legacy_proxy_when_upstream_shape_is_partial() {
    // Only 3 of 5 upstream ids present -- must not be treated as the
    // upstream shape (a partial read would silently misreport 2 root
    // causes as entirely absent instead of falling back honestly).
    let scan_structured = json!({
        "quality_signal": 9000,
        "coupling_score": 3.0,
        "root_causes": {
            "modularity": {"raw": 0.5, "score": 0.5},
            "acyclicity": {"raw": 1.0, "score": 0.5},
        },
    });
    let mut diagnostics = Vec::new();
    let (root_causes, formula_version) =
        root_causes_section(Some(&scan_structured), None, &mut diagnostics);
    assert_eq!(formula_version, "legacy_proxy_v0");
    assert!(!diagnostics.is_empty());
    let entries = root_causes.as_array().unwrap();
    assert!(entries.iter().any(|entry| entry["id"] == "coupling"));
    assert!(entries.iter().all(|entry| entry["score"].is_null()));
}

/// Over-8KB round-trip: a `sentrux.scan` `structuredData` payload well
/// past the old 8KB preview cap (#383) must project into the same
/// `qualitySignal`/`rootCauses` values as a small one -- nothing in this
/// module's own JSON traversal silently drops or truncates a large
/// payload.
#[test]
fn quality_signal_section_round_trips_a_scan_payload_over_8kb() {
    let mut scan_structured = scan_structured_fixture(5.5, 3, 1, 20, 2, 8700);
    scan_structured["godFiles"] = json!(["src/huge_module.rs"]);
    scan_structured["filler"] = json!("z".repeat(12 * 1024));
    let bytes = serde_json::to_vec(&scan_structured).unwrap();
    assert!(
        bytes.len() > 8 * 1024,
        "fixture must exceed the 8KB preview cap"
    );

    let baseline = baseline_metrics_fixture(4.0, 2, 0, 16, 0, 9100);
    let mut diagnostics = Vec::new();
    let signal = quality_signal_section(
        Some(&scan_structured),
        None,
        Some(&baseline),
        &mut diagnostics,
    );
    assert_eq!(signal["total"]["current"], 8700);
    assert_eq!(signal["total"]["baseline"], 9100);
    assert_eq!(signal["total"]["delta"], -400);
    let god_files_cause = signal["rootCauses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "godFiles")
        .expect("godFiles root cause present");
    assert_eq!(god_files_cause["raw"]["current"], 1.0);
}

#[test]
fn violation_findings_classifies_ratchet_regressions_and_rule_violations_separately() {
    let check_reference =
        json!({"path": "sentrux-capability-sentrux-check.json", "sha256": "a".repeat(64)});
    let check_payload = json!({
        "capabilityId": "sentrux.check",
        "outputs": {"command": {"violations": [
            {"rule": "max_cc", "message": "max_cc exceeded: 30 > 20", "targets": ["src/big.rs"]},
        ]}},
    });
    let gate_reference =
        json!({"path": "sentrux-capability-sentrux-gate.json", "sha256": "b".repeat(64)});
    let gate_payload = json!({
        "capabilityId": "sentrux.gate",
        "outputs": {"command": {"violations": [
            {"rule": "quality_degraded", "message": "Quality: 9000 -> 8800", "targets": []},
        ]}},
    });

    let check_findings =
        violation_findings(Some((&check_reference, &check_payload)), "sentrux.check");
    assert_eq!(check_findings.len(), 1);
    assert_eq!(check_findings[0]["kind"], "rule_violation");
    assert_eq!(check_findings[0]["severityNormalized"], "medium");
    assert_eq!(check_findings[0]["targets"], json!(["src/big.rs"]));

    let gate_findings = violation_findings(Some((&gate_reference, &gate_payload)), "sentrux.gate");
    assert_eq!(gate_findings.len(), 1);
    assert_eq!(gate_findings[0]["kind"], "baseline_regression");
    assert_eq!(gate_findings[0]["severityNormalized"], "high");

    assert!(violation_findings(None, "sentrux.check").is_empty());
}

#[test]
fn finding_fingerprint_is_stable_across_message_text_but_sensitive_to_identity() {
    let targets = vec!["src/a.rs".to_string()];
    let first = finding_fingerprint("rule_violation", "sentrux.check", "max_cc", &targets);
    // Same rule/capability/targets, different message-equivalent inputs
    // (message itself is not a fingerprint input) -- must match.
    let second = finding_fingerprint("rule_violation", "sentrux.check", "max_cc", &targets);
    assert_eq!(first, second);

    // Sorted-targets independence: order must not change the identity.
    let reordered = vec!["src/b.rs".to_string(), "src/a.rs".to_string()];
    let forward = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
    assert_eq!(
        finding_fingerprint("rule_violation", "sentrux.check", "max_cc", &reordered),
        finding_fingerprint("rule_violation", "sentrux.check", "max_cc", &forward)
    );

    // Different rule -> different fingerprint.
    let different_rule =
        finding_fingerprint("rule_violation", "sentrux.check", "no_god_files", &targets);
    assert_ne!(first, different_rule);

    // Different kind -> different fingerprint even for the same rule name.
    let different_kind =
        finding_fingerprint("baseline_regression", "sentrux.check", "max_cc", &targets);
    assert_ne!(first, different_kind);

    assert!(first.starts_with("sha256:"));
}

#[test]
fn violation_kind_defaults_unknown_rules_to_rule_violation_not_dropped() {
    assert_eq!(violation_kind("quality_degraded"), "baseline_regression");
    assert_eq!(violation_kind("coupling_increased"), "baseline_regression");
    assert_eq!(violation_kind("cycles_increased"), "baseline_regression");
    assert_eq!(violation_kind("god_files_increased"), "baseline_regression");
    assert_eq!(violation_kind("max_cc"), "rule_violation");
    assert_eq!(
        violation_kind("some_future_rule_this_table_does_not_know"),
        "rule_violation"
    );
}

#[test]
fn root_cause_finding_only_fires_when_a_bottleneck_is_present() {
    let signal_with_bottleneck = json!({"bottleneck": "godFiles"});
    let finding = root_cause_finding(&signal_with_bottleneck, Some(json!({"path": "x"})))
        .expect("finding present");
    assert_eq!(finding["kind"], "root_cause_diagnostic");
    assert_eq!(finding["rule"], "godFiles");

    let signal_without_bottleneck = json!({"bottleneck": Value::Null});
    assert!(root_cause_finding(&signal_without_bottleneck, None).is_none());
}

#[test]
fn finding_counts_tallies_by_kind_and_severity() {
    let findings = vec![
        json!({"kind": "rule_violation", "severityNormalized": "medium"}),
        json!({"kind": "rule_violation", "severityNormalized": "medium"}),
        json!({"kind": "baseline_regression", "severityNormalized": "high"}),
    ];
    let counts = finding_counts(&findings);
    assert_eq!(counts["total"], 3);
    assert_eq!(counts["byKind"]["rule_violation"], 2);
    assert_eq!(counts["byKind"]["baseline_regression"], 1);
    assert_eq!(counts["bySeverity"]["medium"], 2);
    assert_eq!(counts["bySeverity"]["high"], 1);
}

#[test]
fn read_baseline_metrics_is_honest_when_the_file_is_absent_or_malformed() {
    let temp = std::env::temp_dir().join(format!(
        "code-intel-386-baseline-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).expect("temp dir");

    let mut diagnostics = Vec::new();
    assert!(read_baseline_metrics(&temp, &mut diagnostics).is_none());
    assert!(!diagnostics.is_empty());

    std::fs::create_dir_all(temp.join(".sentrux")).unwrap();
    std::fs::write(temp.join(BASELINE_RELATIVE_PATH), b"not json").unwrap();
    let mut diagnostics = Vec::new();
    assert!(read_baseline_metrics(&temp, &mut diagnostics).is_none());
    assert!(!diagnostics.is_empty());

    std::fs::write(
        temp.join(BASELINE_RELATIVE_PATH),
        serde_json::to_vec(&json!({"metrics": {"quality_signal": 9000}})).unwrap(),
    )
    .unwrap();
    let mut diagnostics = Vec::new();
    let metrics = read_baseline_metrics(&temp, &mut diagnostics).expect("valid baseline");
    assert_eq!(metrics["quality_signal"], 9000);
    assert!(diagnostics.is_empty());

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn build_refuses_to_bind_a_projection_to_the_wrong_commit() {
    let temp = std::env::temp_dir().join(format!(
        "code-intel-386-repo-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).expect("temp dir");
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&temp)
        .status();
    if init.is_err() || !init.unwrap().success() {
        let _ = std::fs::remove_dir_all(&temp);
        return; // git unavailable in this environment; skip.
    }
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&temp)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&temp)
        .status();
    std::fs::write(temp.join("README.md"), b"fixture\n").unwrap();
    let _ = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .status();
    let _ = std::process::Command::new("git")
        .args(["commit", "-q", "-m", "fixture"])
        .current_dir(&temp)
        .status();

    let evidence = CommittedEvidence {
        entry: json!({"repo": "fixture", "run": "run-1", "snapshotIdentity": "sha256:deadbeef"}),
        refs: Vec::new(),
        verified: Vec::new(),
        run_root: temp.clone(),
    };
    let request = ProjectionRequest {
        evidence: &evidence,
        repo_path: &temp,
        commit: "0000000000000000000000000000000000000000",
        base_ref: Some("origin/main"),
        correlation: OrcaCorrelation {
            run_id: None,
            task_id: None,
            dispatch_id: None,
            pr_number: None,
        },
    };
    let error = build(&request).expect_err("wrong commit must refuse deterministically");
    assert!(matches!(error, ProjectionError::Contract(_)));

    let _ = std::fs::remove_dir_all(&temp);
}
