use super::{legacy_session_script, LegacySessionTemp};
use std::process::{Command, ExitStatus, Stdio};

pub(super) struct LegacySessionOutput {
    pid: u32,
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

pub(super) fn invoke_legacy_session(
    tree: &LegacySessionTemp,
    operation: &str,
    session_id: &str,
) -> LegacySessionOutput {
    let path = std::env::join_paths(
        std::iter::once(tree.fake_bin()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )),
    )
    .expect("compose hermetic PATH");
    // The agent tool pins the session gate to the repository's lite core and
    // honors SENTRUX_CORE_EXE as the explicit override (issue #182), so the
    // fake CLI is injected through that seam; the PATH prepend stays for the
    // last-resort `sentrux` lookup.
    #[cfg(windows)]
    let fake_cli = tree.fake_bin().join("sentrux.cmd");
    #[cfg(not(windows))]
    let fake_cli = tree.fake_bin().join("sentrux");
    let child = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(legacy_session_script())
        .arg(operation)
        .arg(tree.repo())
        .args(["-SessionId", session_id])
        .env("PATH", path)
        .env("SENTRUX_CORE_EXE", &fake_cli)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn real legacy session gate");
    let pid = child.id();
    let output = child
        .wait_with_output()
        .expect("wait for real legacy session gate");
    LegacySessionOutput {
        pid,
        status: output.status,
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn legacy_session_failure_message(
    exit_code: Option<i32>,
    signal: Option<i32>,
    pid: u32,
    operation: &str,
    stdout: &[u8],
    stderr: &[u8],
) -> Option<String> {
    if exit_code == Some(0) {
        return None;
    }
    let termination = match (exit_code, signal) {
        (Some(code), _) => format!("exited with code {code}"),
        (None, Some(signal)) => format!("was terminated by signal {signal}"),
        (None, None) => "ended without a numeric exit code".to_owned(),
    };
    Some(format!(
        "{operation} subprocess {termination} (pid {pid}); stdout={}; stderr={}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    ))
}

fn legacy_session_signal(status: &ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

pub(super) fn assert_legacy_session_success(output: &LegacySessionOutput, operation: &str) {
    if let Some(message) = legacy_session_failure_message(
        output.status.code(),
        legacy_session_signal(&output.status),
        output.pid,
        operation,
        &output.stdout,
        &output.stderr,
    ) {
        panic!("{message}");
    }
}

pub(super) fn parse_legacy_session_json(output: &LegacySessionOutput) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "session gate must emit JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn signal_terminated_legacy_session_reports_signal_and_pid() {
    let message = legacy_session_failure_message(
        None,
        Some(9),
        4242,
        "session_end",
        b"partial stdout",
        b"partial stderr",
    )
    .expect("signal termination must be reported as a process failure");

    assert!(message.contains("session_end subprocess was terminated by signal 9 (pid 4242)"));
    assert!(message.contains("stdout=partial stdout"));
    assert!(message.contains("stderr=partial stderr"));
}

#[test]
fn nonzero_legacy_session_exit_remains_distinct_from_signal_termination() {
    let message = legacy_session_failure_message(
        Some(17),
        None,
        4242,
        "session_end",
        b"partial stdout",
        b"partial stderr",
    )
    .expect("a nonzero exit must be reported as a process failure");

    assert!(message.contains("session_end subprocess exited with code 17 (pid 4242)"));
    assert!(!message.contains("terminated by signal"));
}
