use super::*;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn cli_parse_requires_repo_target_metric_and_goal() {
    assert!(Cli::parse(&args(&[])).is_err());
    assert!(Cli::parse(&args(&["--repo", "."])).is_err());
    assert!(Cli::parse(&args(&["--repo", ".", "--target", "fixture"])).is_err());
    assert!(Cli::parse(&args(&[
        "--repo",
        ".",
        "--target",
        "fixture",
        "--metric",
        "latency_ms"
    ]))
    .is_err());
}

#[test]
fn cli_parse_succeeds_with_the_required_flags_and_sensible_defaults() {
    let cli = Cli::parse(&args(&[
        "--repo",
        ".",
        "--target",
        "fixture::target",
        "--metric",
        "latency_ms",
        "--goal",
        "minimize",
    ]))
    .unwrap();
    assert_eq!(cli.target, "fixture::target");
    assert_eq!(cli.metric, "latency_ms");
    assert_eq!(cli.goal, Goal::Minimize);
    assert!(cli.eval_command.is_none());
    assert_eq!(
        cli.min_improvement_percent,
        report::DEFAULT_MIN_IMPROVEMENT_PERCENT
    );
    assert_eq!(cli.seconds_per_step, steps::DEFAULT_SECONDS_PER_STEP);
    assert_eq!(cli.denoise_n, denoise::DEFAULT_DENOISE_N);
    assert_eq!(cli.grace_period_seconds, DEFAULT_GRACE_PERIOD_SECONDS);
}

#[test]
fn cli_parse_rejects_an_invalid_goal() {
    let error = Cli::parse(&args(&[
        "--repo", ".", "--target", "t", "--metric", "m", "--goal", "bogus",
    ]))
    .unwrap_err();
    assert!(error.contains("--goal"));
}

#[test]
fn cli_parse_accepts_a_percent_suffix_on_min_improvement() {
    let cli = Cli::parse(&args(&[
        "--repo",
        ".",
        "--target",
        "t",
        "--metric",
        "m",
        "--goal",
        "maximize",
        "--min-improvement",
        "2%",
    ]))
    .unwrap();
    assert_eq!(cli.min_improvement_percent, 2.0);
}

#[test]
fn cli_parse_rejects_an_unknown_flag() {
    let error = Cli::parse(&args(&["--bogus"])).unwrap_err();
    assert!(error.contains("unknown perf-optimize run argument"));
}

#[test]
fn no_eval_command_reports_unavailable_without_touching_weco() {
    let exit_code = execute(
        &Cli::parse(&args(&[
            "--repo", ".", "--target", "t", "--metric", "m", "--goal", "maximize",
        ]))
        .unwrap(),
    );
    assert_eq!(exit_code, 69);
}

#[test]
fn a_wall_clock_budget_too_small_for_one_step_fails_before_touching_weco() {
    let cli = Cli::parse(&args(&[
        "--repo",
        ".",
        "--target",
        "t",
        "--metric",
        "m",
        "--goal",
        "maximize",
        "--eval-command",
        "echo unused",
        "--budget-wall-clock",
        "5",
        "--seconds-per-step",
        "30",
        // Point weco detection at an empty directory so this exercises the
        // budget check specifically, not a real BYOK/weco probe -- if
        // budget passed silently, the next thing hit would be
        // weco-unavailable (69), not the budget failure (76) asserted below.
        "--doctor-tool-path-prefix",
        std::env::temp_dir().to_str().unwrap(),
    ]))
    .unwrap();
    let exit_code = execute(&cli);
    assert_eq!(exit_code, 76);
}

#[test]
fn extremum_maximize_picks_the_largest_value() {
    assert_eq!(extremum(&[1.0, 5.0, 3.0], Goal::Maximize), Some(5.0));
}

#[test]
fn extremum_minimize_picks_the_smallest_value() {
    assert_eq!(extremum(&[5.0, 1.0, 3.0], Goal::Minimize), Some(1.0));
}

#[test]
fn extremum_of_no_readings_is_none() {
    assert_eq!(extremum(&[], Goal::Maximize), None);
}

#[test]
fn shell_quote_escapes_embedded_quotes_and_backslashes() {
    assert_eq!(shell_quote("plain"), "\"plain\"");
    assert_eq!(shell_quote("has space"), "\"has space\"");
    assert_eq!(shell_quote("has\"quote"), "\"has\\\"quote\"");
    assert_eq!(shell_quote("back\\slash"), "\"back\\\\slash\"");
}

#[test]
fn parse_percent_strips_a_trailing_percent_sign() {
    assert_eq!(parse_percent("5"), Ok(5.0));
    assert_eq!(parse_percent("5%"), Ok(5.0));
    assert!(parse_percent("not-a-number").is_err());
}
