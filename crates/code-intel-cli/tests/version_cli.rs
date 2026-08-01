//! Contract tests for `code-intel --version`.
//!
//! This surface exists because a binary that cannot report its own version
//! cannot be audited. `%LOCALAPPDATA%/code-intel/bin/repo.json` records where
//! the installer ran from, not what it produced, so a machine running a stale
//! build is indistinguishable from a current one — a v0.6.0 binary answering
//! `snapshot identity` in 10.9s looked exactly like the 0.28s build until the
//! two were timed side by side.
//!
//! What is pinned here is what the installer's version gate parses: a version
//! token on stdout, exit 0, and a stable `--json` schema id. The gate treats an
//! unreadable version as "unknown", never as "matches", so an empty or
//! non-zero-exit answer is a contract break, not a soft failure.

use std::process::Command;

fn run(args: &[&str]) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(args)
        .output()
        .expect("run code-intel");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn version_flag_reports_the_crate_version_and_exits_zero() {
    let (stdout, stderr, code) = run(&["--version"]);

    assert_eq!(code, 0, "--version must exit 0, stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        format!("code-intel {}", env!("CARGO_PKG_VERSION")),
        "installer parses this line; the shape is the contract"
    );
}

#[test]
fn short_version_flag_matches_long_flag() {
    let (long, _, long_code) = run(&["--version"]);
    let (short, _, short_code) = run(&["-V"]);

    assert_eq!(long_code, 0);
    assert_eq!(short_code, 0);
    assert_eq!(
        long.trim(),
        short.trim(),
        "-V is an alias, not a second format"
    );
}

#[test]
fn json_version_carries_a_stable_schema_id() {
    let (stdout, stderr, code) = run(&["--version", "--json"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("--version --json must emit one JSON document");
    assert_eq!(parsed["schema"], "code-intel-version.v1");
    assert_eq!(parsed["name"], "code-intel");
    assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn version_token_is_parseable_by_the_installer_regex() {
    // Get-ToolVersion in legacy/install-code-intel-pipeline.ps1 matches
    // `\d+\.\d+\.\d+(?:[-.][0-9A-Za-z.]+)?` against this output. A version that
    // does not match reads as "unknown" and the pin gate cannot fire, so the
    // shape is asserted here rather than only in PowerShell.
    let (stdout, _, _) = run(&["--version"]);
    let token = stdout
        .trim()
        .rsplit(' ')
        .next()
        .expect("version token")
        .to_string();

    let mut chars = token.chars();
    let leading_digit = chars.next().is_some_and(|c| c.is_ascii_digit());
    assert!(
        leading_digit,
        "version token must start with a digit: {token}"
    );
    assert_eq!(
        token.split('.').count().min(3),
        3,
        "version token needs at least major.minor.patch: {token}"
    );
}

#[test]
fn version_is_answered_before_route_dispatch() {
    // `--version` starts with `-`, so it is neither a primary invocation nor a
    // raw route. Without the early dispatch it falls through to `run()` and
    // exits as an unknown command — which is exactly what shipped until now.
    let (stdout, _, code) = run(&["--version", "--repo", "does-not-exist"]);

    assert_eq!(
        code, 0,
        "--version must not be evaluated against unrelated arguments"
    );
    assert!(
        stdout.trim().starts_with("code-intel "),
        "got: {}",
        stdout.trim()
    );
}
