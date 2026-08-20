use super::*;
use std::fs;
use std::path::PathBuf;

fn temp_dir() -> PathBuf {
    let unique = format!(
        "code-intel-denoise-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn parse_metric_line_reads_the_last_colon_pair() {
    assert_eq!(
        parse_metric_line("latency_ms: 123.45"),
        Some(("latency_ms".to_string(), 123.45))
    );
}

#[test]
fn parse_metric_line_splits_on_the_last_colon_so_a_colon_in_the_name_survives() {
    assert_eq!(
        parse_metric_line("namespace::latency_ms: 7"),
        Some(("namespace::latency_ms".to_string(), 7.0))
    );
}

#[test]
fn parse_metric_line_trims_whitespace_on_both_sides() {
    assert_eq!(
        parse_metric_line("  latency_ms  :   7.5  "),
        Some(("latency_ms".to_string(), 7.5))
    );
}

#[test]
fn parse_metric_line_rejects_a_non_numeric_value() {
    assert_eq!(parse_metric_line("latency_ms: not-a-number"), None);
}

#[test]
fn parse_metric_line_rejects_a_line_with_no_colon() {
    assert_eq!(parse_metric_line("no colon here 7"), None);
}

#[test]
fn parse_metric_line_rejects_an_empty_name() {
    assert_eq!(parse_metric_line(": 7"), None);
}

#[test]
fn median_of_odd_count_is_the_middle_value() {
    assert_eq!(median(&[10.0, 12.0, 15.0, 20.0, 100.0]), 15.0);
}

#[test]
fn median_of_even_count_averages_the_two_middle_values() {
    assert_eq!(median(&[10.0, 20.0, 30.0, 40.0]), 25.0);
}

#[test]
fn median_of_a_single_value_is_itself() {
    assert_eq!(median(&[42.0]), 42.0);
}

#[test]
fn zero_samples_requested_is_rejected_before_spawning_anything() {
    let error = run_denoised("echo unused", 0).unwrap_err();
    assert_eq!(error, DenoiseError::ZeroSamplesRequested);
}

#[cfg(windows)]
fn write_outlier_once_fixture(dir: &std::path::Path) -> String {
    let marker = dir.join("used-once.marker");
    let script = dir.join("noisy_eval.cmd");
    fs::write(
        &script,
        format!(
            "@echo off\r\nif exist \"{marker}\" (\r\n  echo latency_ms: 12\r\n) else (\r\n  echo. > \"{marker}\"\r\n  echo latency_ms: 100\r\n)\r\n",
            marker = marker.display()
        ),
    )
    .unwrap();
    script.display().to_string()
}

#[cfg(not(windows))]
fn write_outlier_once_fixture(dir: &std::path::Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let marker = dir.join("used-once.marker");
    let script = dir.join("noisy_eval.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ -f \"{marker}\" ]; then\n  echo \"latency_ms: 12\"\nelse\n  touch \"{marker}\"\n  echo \"latency_ms: 100\"\nfi\n",
            marker = marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script.display().to_string()
}

#[test]
fn denoising_a_noisy_eval_command_is_more_stable_than_a_single_sample() {
    let dir = temp_dir();
    let command = write_outlier_once_fixture(&dir);

    let result = run_denoised(&command, 5).expect("denoised run succeeds");

    assert_eq!(result.metric_name, "latency_ms");
    // First sample (what a single unwrapped invocation would have reported)
    // is the outlier; the wrapped, denoised result is not.
    assert_eq!(result.samples[0], 100.0);
    assert_eq!(result.samples[1..], [12.0, 12.0, 12.0, 12.0]);
    assert_eq!(result.median, 12.0);
    assert_ne!(
        result.median, result.samples[0],
        "denoised median must not equal the noisy single-sample reading"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_nonzero_exit_on_any_attempt_fails_the_whole_run() {
    let error = run_denoised("exit 3", 2).unwrap_err();
    match error {
        DenoiseError::NonZeroExit {
            attempt, exit_code, ..
        } => {
            assert_eq!(attempt, 1);
            assert_eq!(exit_code, Some(3));
        }
        other => panic!("expected NonZeroExit, got {other:?}"),
    }
}

#[test]
fn unparsable_output_fails_the_whole_run() {
    let command = if cfg!(windows) {
        "echo not a metric line"
    } else {
        "echo 'not a metric line'"
    };
    let error = run_denoised(command, 2).unwrap_err();
    match error {
        DenoiseError::Unparsable { attempt, .. } => assert_eq!(attempt, 1),
        other => panic!("expected Unparsable, got {other:?}"),
    }
}

#[cfg(windows)]
fn write_flip_metric_name_fixture(dir: &std::path::Path) -> String {
    let marker = dir.join("flip.marker");
    let script = dir.join("flip.cmd");
    fs::write(
        &script,
        format!(
            "@echo off\r\nif exist \"{marker}\" (\r\n  echo other_metric: 1\r\n) else (\r\n  echo. > \"{marker}\"\r\n  echo latency_ms: 1\r\n)\r\n",
            marker = marker.display()
        ),
    )
    .unwrap();
    script.display().to_string()
}

#[cfg(not(windows))]
fn write_flip_metric_name_fixture(dir: &std::path::Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let marker = dir.join("flip.marker");
    let script = dir.join("flip.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ -f \"{marker}\" ]; then\n  echo \"other_metric: 1\"\nelse\n  touch \"{marker}\"\n  echo \"latency_ms: 1\"\nfi\n",
            marker = marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script.display().to_string()
}

#[test]
fn an_inconsistent_metric_name_between_attempts_fails_the_run() {
    let dir = temp_dir();
    let command = write_flip_metric_name_fixture(&dir);

    let error = run_denoised(&command, 2).unwrap_err();
    match error {
        DenoiseError::InconsistentMetricName {
            first,
            attempt,
            found,
        } => {
            assert_eq!(first, "latency_ms");
            assert_eq!(attempt, 2);
            assert_eq!(found, "other_metric");
        }
        other => panic!("expected InconsistentMetricName, got {other:?}"),
    }
    fs::remove_dir_all(&dir).ok();
}
