use super::*;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn cli_parse_rejects_a_missing_subcommand() {
    assert!(Cli::parse(&args(&["--repo", "."])).is_err());
}

#[test]
fn cli_parse_requires_repo_and_out() {
    assert!(Cli::parse(&args(&["check"])).is_err());
    assert!(Cli::parse(&args(&["check", "--repo", "."])).is_err());
}

#[test]
fn cli_parse_rejects_fewer_than_two_runs() {
    let error = Cli::parse(&args(&[
        "check", "--repo", ".", "--out", ".", "--runs", "1",
    ]))
    .unwrap_err();
    assert!(error.contains("--runs must be at least 2"));
}

#[test]
fn cli_parse_rejects_an_unknown_flag() {
    assert!(Cli::parse(&args(&["check", "--repo", ".", "--bogus", "x"])).is_err());
}

#[test]
fn cli_parse_defaults_runs_to_three() {
    let cli = Cli::parse(&args(&["check", "--repo", ".", "--out", "."])).unwrap();
    assert_eq!(cli.runs, 3);
    assert_eq!(cli.max_concurrency, 2);
    assert_eq!(cli.profile, RunProfile::Default);
}

#[test]
fn strip_volatile_removes_every_listed_key_at_any_depth() {
    let mut value = json!({
        "generatedAt": "2026-08-06T00:00:00Z",
        "evidence": {
            "observedAt": 1700000000,
            "provenance": {"collectionId": "graph-internal-abc-1700000000"},
            "rules": [
                {"admissionIdentity": "sha", "kind": "boundary_dependency"}
            ]
        },
        "kept": "value"
    });
    strip_volatile(&mut value);
    assert_eq!(
        value,
        json!({
            "evidence": {
                "provenance": {},
                "rules": [{"kind": "boundary_dependency"}]
            },
            "kept": "value"
        })
    );
}

#[test]
fn json_diff_finds_nothing_for_equal_documents() {
    let a = json!({"a": 1, "b": [1, 2, {"c": "x"}]});
    let b = a.clone();
    assert!(json_diff(&a, &b, "").is_none());
}

#[test]
fn json_diff_reports_the_first_differing_leaf_path() {
    let a = json!({"triage": {"status": "green", "primary_diagnosis": "clean snapshot"}});
    let b = json!({"triage": {"status": "red", "primary_diagnosis": "clean snapshot"}});
    let diff = json_diff(&a, &b, "").expect("documents differ");
    assert_eq!(diff["path"], "/triage/status");
    assert_eq!(diff["from"], "green");
    assert_eq!(diff["to"], "red");
}

#[test]
fn json_diff_reports_array_length_mismatch() {
    let a = json!({"targets": ["a.rs"]});
    let b = json!({"targets": ["a.rs", "b.rs"]});
    let diff = json_diff(&a, &b, "").expect("documents differ");
    assert_eq!(diff["path"], "/targets.length");
}

fn snapshot(
    index: usize,
    outcome: &str,
    exit_code: i32,
    documents: &[(&str, Value)],
) -> RunSnapshot {
    RunSnapshot {
        index,
        outcome: outcome.to_string(),
        exit_code,
        documents: documents
            .iter()
            .map(|(key, value)| (key.to_string(), Comparable::Json(value.clone())))
            .collect(),
    }
}

#[test]
fn first_divergence_is_none_when_every_run_agrees() {
    let doc = json!({"triage": {"status": "green"}});
    let runs = vec![
        snapshot(
            1,
            "completed",
            0,
            &[("diagnosis.hospital/hospital-report.json", doc.clone())],
        ),
        snapshot(
            2,
            "completed",
            0,
            &[("diagnosis.hospital/hospital-report.json", doc.clone())],
        ),
        snapshot(
            3,
            "completed",
            0,
            &[("diagnosis.hospital/hospital-report.json", doc)],
        ),
    ];
    assert!(first_divergence(&runs).is_none());
}

#[test]
fn first_divergence_catches_a_run_outcome_mismatch_before_diffing_documents() {
    let doc = json!({"triage": {"status": "green"}});
    let runs = vec![
        snapshot(
            1,
            "completed",
            0,
            &[("diagnosis.hospital/hospital-report.json", doc.clone())],
        ),
        snapshot(
            2,
            "domain_failed",
            10,
            &[("diagnosis.hospital/hospital-report.json", doc)],
        ),
    ];
    let divergence = first_divergence(&runs).expect("outcomes differ");
    assert_eq!(divergence["kind"], "run_outcome");
}

#[test]
fn first_divergence_catches_a_document_present_in_only_one_run() {
    let runs = vec![
        snapshot(
            1,
            "completed",
            0,
            &[("evidence.graph/graph-admission.json", json!({}))],
        ),
        snapshot(2, "completed", 0, &[]),
    ];
    let divergence = first_divergence(&runs).expect("document missing in run 2");
    assert_eq!(divergence["kind"], "document_missing");
}

#[test]
fn first_divergence_checks_every_pair_not_just_adjacent_runs() {
    // Run 2 agrees with both neighbours pairwise-adjacently, but run 1
    // and run 3 disagree with each other -- only checking (1,2) and (2,3)
    // would miss it. PRD's `C(3,2)` wording is exactly this: every pair.
    let runs = vec![
        snapshot(
            1,
            "completed",
            0,
            &[("diagnosis.hospital/hospital-report.json", json!({"v": 1}))],
        ),
        snapshot(
            2,
            "completed",
            0,
            &[("diagnosis.hospital/hospital-report.json", json!({"v": 1}))],
        ),
        snapshot(
            3,
            "completed",
            0,
            &[("diagnosis.hospital/hospital-report.json", json!({"v": 2}))],
        ),
    ];
    let divergence = first_divergence(&runs).expect("run 1 and run 3 disagree");
    assert_eq!(divergence["runA"], 1);
    assert_eq!(divergence["runB"], 3);
}
