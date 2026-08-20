use super::*;

#[test]
fn goal_parses_maximize_and_minimize_only() {
    assert_eq!(Goal::parse("maximize"), Ok(Goal::Maximize));
    assert_eq!(Goal::parse("minimize"), Ok(Goal::Minimize));
    assert!(Goal::parse("optimize").is_err());
}

#[test]
fn maximize_improvement_is_positive_when_candidate_is_higher() {
    // baseline 100 -> candidate 120 is a 20% improvement to maximize.
    assert_eq!(improvement(100.0, 120.0, Goal::Maximize), 20.0);
}

#[test]
fn maximize_improvement_is_negative_when_candidate_is_lower() {
    assert_eq!(improvement(100.0, 90.0, Goal::Maximize), -10.0);
}

#[test]
fn minimize_improvement_is_positive_when_candidate_is_lower() {
    // baseline 100ms -> candidate 80ms is a 20% improvement to minimize.
    assert_eq!(improvement(100.0, 80.0, Goal::Minimize), 20.0);
}

#[test]
fn minimize_improvement_is_negative_when_candidate_is_higher() {
    assert_eq!(improvement(100.0, 110.0, Goal::Minimize), -10.0);
}

#[test]
fn zero_baseline_that_does_not_change_reports_zero_improvement() {
    assert_eq!(improvement(0.0, 0.0, Goal::Maximize), 0.0);
}

#[test]
fn zero_baseline_that_improves_reports_positive_infinity() {
    assert!(improvement(0.0, 5.0, Goal::Maximize).is_infinite());
    assert!(improvement(0.0, 5.0, Goal::Maximize) > 0.0);
}

#[test]
fn meets_threshold_is_true_at_exactly_the_minimum() {
    assert!(meets_threshold(5.0, 5.0));
    assert!(!meets_threshold(4.999, 5.0));
}

#[test]
fn report_with_no_candidate_reports_null_fields_and_unmet_threshold() {
    let report = build_report(
        "fixture::target",
        "latency_ms",
        Goal::Minimize,
        100.0,
        None,
        None,
        5.0,
        0,
        10,
        Some(StoppedBy::BudgetExhausted),
    );
    assert_eq!(report["bestCandidate"], Value::Null);
    assert_eq!(report["metThreshold"], false);
    assert_eq!(report["stoppedBy"], "budget_exhausted");
    assert_eq!(report["stepsRun"], 0);
    assert_eq!(report["stepsPlanned"], 10);
}

#[test]
fn report_below_threshold_distinguishes_steps_exhausted_from_budget_exhausted() {
    let exhausted = build_report(
        "fixture::target",
        "latency_ms",
        Goal::Minimize,
        100.0,
        Some(99.0),
        None,
        5.0,
        10,
        10,
        Some(StoppedBy::StepsExhausted),
    );
    assert_eq!(exhausted["metThreshold"], false);
    assert_eq!(exhausted["stoppedBy"], "steps_exhausted");

    let cut_short = build_report(
        "fixture::target",
        "latency_ms",
        Goal::Minimize,
        100.0,
        Some(99.0),
        None,
        5.0,
        4,
        10,
        Some(StoppedBy::BudgetExhausted),
    );
    assert_eq!(cut_short["metThreshold"], false);
    assert_eq!(cut_short["stoppedBy"], "budget_exhausted");
}

#[test]
fn report_above_threshold_omits_stopped_by_since_the_search_succeeded() {
    let report = build_report(
        "fixture::target",
        "latency_ms",
        Goal::Minimize,
        100.0,
        Some(80.0),
        Some("--- a/fixture.rs\n+++ b/fixture.rs\n"),
        5.0,
        3,
        10,
        Some(StoppedBy::StepsExhausted),
    );
    assert_eq!(report["metThreshold"], true);
    assert_eq!(report["improvementPercent"], 20.0);
    assert_eq!(report["stoppedBy"], Value::Null);
    assert!(report["bestCandidateDiff"]
        .as_str()
        .unwrap()
        .contains("fixture.rs"));
}

#[test]
fn report_always_carries_the_declared_schema() {
    let report = build_report(
        "fixture::target",
        "latency_ms",
        Goal::Minimize,
        100.0,
        None,
        None,
        5.0,
        0,
        10,
        None,
    );
    assert_eq!(report["schema"], "code-intel-perf-optimize-run.v1");
}
