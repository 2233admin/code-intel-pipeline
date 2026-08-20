use super::*;

#[test]
fn parse_requires_command() {
    let error = parse(&[]).unwrap_err();
    assert!(error.contains("--command is required"));
}

#[test]
fn parse_defaults_n_to_five() {
    let cli = parse(&["--command".to_string(), "echo hi".to_string()]).unwrap();
    assert_eq!(cli.command, "echo hi");
    assert_eq!(cli.n, 5);
}

#[test]
fn parse_accepts_an_explicit_n() {
    let cli = parse(&[
        "--command".to_string(),
        "echo hi".to_string(),
        "--n".to_string(),
        "3".to_string(),
    ])
    .unwrap();
    assert_eq!(cli.n, 3);
}

#[test]
fn parse_rejects_a_non_numeric_n() {
    let error = parse(&[
        "--command".to_string(),
        "echo hi".to_string(),
        "--n".to_string(),
        "nope".to_string(),
    ])
    .unwrap_err();
    assert!(error.contains("--n must be a positive integer"));
}

#[test]
fn parse_rejects_unknown_arguments() {
    let error = parse(&["--bogus".to_string()]).unwrap_err();
    assert!(error.contains("unknown perf-optimize denoise-eval argument"));
}

#[test]
fn run_raw_prints_the_denoised_metric_line_and_exits_zero() {
    let command = if cfg!(windows) {
        "echo latency_ms: 7"
    } else {
        "echo 'latency_ms: 7'"
    };
    let exit_code = run_raw(&[
        "--command".to_string(),
        command.to_string(),
        "--n".to_string(),
        "1".to_string(),
    ]);
    assert_eq!(exit_code, 0);
}

#[test]
fn run_raw_exits_sixty_four_on_a_usage_error() {
    let exit_code = run_raw(&[]);
    assert_eq!(exit_code, 64);
}

#[test]
fn run_raw_exits_sixty_five_when_the_eval_command_cannot_be_denoised() {
    let command = if cfg!(windows) { "exit 9" } else { "exit 9" };
    let exit_code = run_raw(&["--command".to_string(), command.to_string()]);
    assert_eq!(exit_code, 65);
}
