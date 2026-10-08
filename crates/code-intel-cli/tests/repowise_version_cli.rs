mod common;

use serde_json::Value;

fn version_decision(report: &str, minimum: Option<&str>) -> Value {
    let mut command = common::cli();
    command.args(["repowise-version", "--reported", report]);
    if let Some(minimum) = minimum {
        command.args(["--minimum", minimum]);
    }
    let output = command.output().expect("run version decision CLI");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).expect("version decision JSON")
}

#[test]
fn repowise_floor_uses_complete_python_version_ordering() {
    for (version, minimum, expected) in [
        ("0.55.0rc1", "0.55.0", false),
        ("0.55.0.post1", "0.55.0", true),
        ("0.55.0.dev1", "0.55.0", false),
        ("0.55.0+vendor.1", "0.55.0", true),
        ("1!0.1", "0.55.0", true),
        ("0.55", "0.55.0", true),
        ("0.55.0.0", "0.55.0", true),
        ("0.54.99", "0.55.0", false),
        ("0.56.0rc1", "0.55.0", true),
        ("0.55.0.post1.dev1", "0.55.0.post1", false),
    ] {
        let report = format!("repowise, version {version}");
        let result = version_decision(&report, Some(minimum));
        assert_eq!(result["meetsMinimum"], expected, "{version} >= {minimum}");
        assert_eq!(
            result["status"],
            if expected {
                "accepted"
            } else {
                "below_minimum"
            }
        );
    }
}

#[test]
fn version_reports_preserve_suffixes_and_ignore_unrelated_banner_numbers() {
    let result = version_decision(
        "DeprecationWarning setuptools 3.11.0\nrepowise, version 0.55.0.post1\n",
        None,
    );
    assert_eq!(result["version"], "0.55.0.post1");
    assert_eq!(result["status"], "parsed");
    assert_eq!(result["meetsMinimum"], Value::Null);
}

#[test]
fn unreadable_or_conflicting_reports_do_not_forge_a_version() {
    for report in [
        "warning 3.11.0",
        "repowise, version 0.55.0garbage",
        "repowise, version 0.55.0 trailing junk",
        "repowise, version 0.55.0\nrepowise, version 0.56.0",
    ] {
        let result = version_decision(report, Some("0.55.0"));
        assert_eq!(result["version"], Value::Null, "{report}");
        assert_eq!(result["status"], "unknown");
        assert_eq!(result["meetsMinimum"], Value::Null);
    }
}

#[test]
fn malformed_minimum_and_unknown_or_duplicate_options_are_usage_errors() {
    for arguments in [
        vec![
            "--reported",
            "repowise, version 0.55.0",
            "--minimum",
            "invalid",
        ],
        vec!["--reported", "repowise, version 0.55.0", "--write"],
        vec![
            "--reported",
            "repowise, version 0.55.0",
            "--reported",
            "repowise, version 0.56.0",
        ],
    ] {
        let output = common::cli()
            .arg("repowise-version")
            .args(arguments)
            .output()
            .expect("run invalid version decision CLI");
        assert_eq!(output.status.code(), Some(64));
        assert!(output.stdout.is_empty());
    }
}
