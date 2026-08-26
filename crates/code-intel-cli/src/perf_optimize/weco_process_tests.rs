use super::*;
use std::fs;
use std::path::{Path, PathBuf};

fn temp_dir(label: &str) -> PathBuf {
    let unique = format!(
        "code-intel-weco-process-test-{label}-{}-{}",
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
fn extract_run_id_reads_a_labelled_line_case_and_separator_insensitively() {
    assert_eq!(
        extract_run_id("Run ID: abc-123"),
        Some("abc-123".to_string())
    );
    assert_eq!(extract_run_id("run-id: xyz789"), Some("xyz789".to_string()));
    assert_eq!(extract_run_id("RUNID:zzz"), Some("zzz".to_string()));
    assert_eq!(extract_run_id("no run identifier here"), None);
}

#[cfg(windows)]
fn write_fixture(dir: &Path, name: &str, run_id: &str, responds_to_stop: bool) -> PathBuf {
    let script = dir.join(name);
    let loop_body = if responds_to_stop {
        // #349: was `ping -n 1 -w 20 127.0.0.1`. `-w` bounds how long ping
        // waits for a reply, not a guaranteed delay: 127.0.0.1 replies in
        // under 1ms, so `-w 20` never waited its intended 20ms (measured:
        // 10 iterations of the old line took ~74ms total). The loop was
        // spawning a fresh `ping.exe` as fast as the OS could create one --
        // a process-spawn storm that self-inflicted the contention this
        // test's flakiness was blamed on. `-n 2` with no `-w` uses ping's
        // own ~1.1s default inter-ping interval instead (measured), the
        // same technique the sibling branch below already uses correctly.
        "if exist \"%~dp0stop-requested.marker\" exit /b 0\r\nping -n 2 127.0.0.1 >nul\r\ngoto loop\r\n"
    } else {
        // `-w` bounds how long ping waits for a reply, not a guaranteed
        // delay: 127.0.0.1 replies instantly, so `-w 60000` returns almost
        // immediately. `-n 60` with no `-w` override sends 60 pings at
        // ping.exe's own ~1s default interval regardless of reply speed --
        // the same technique node_timeout.rs's own hanging-child test uses.
        "ping -n 60 127.0.0.1 >nul\r\n"
    };
    let body = format!(
        "@echo off\r\nif \"%2\"==\"stop\" (\r\n  echo. > \"%~dp0stop-requested.marker\"\r\n  exit /b 0\r\n)\r\necho Run ID: {run_id}\r\n:loop\r\n{loop_body}"
    );
    fs::write(&script, body).unwrap();
    script
}

#[cfg(not(windows))]
fn write_fixture(dir: &Path, name: &str, run_id: &str, responds_to_stop: bool) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join(name);
    let loop_body = if responds_to_stop {
        "i=0\nwhile [ $i -lt 300 ]; do\n  if [ -f \"$DIR/stop-requested.marker\" ]; then exit 0; fi\n  sleep 0.02\n  i=$((i+1))\ndone\n"
    } else {
        "sleep 60\n"
    };
    let body = format!(
        "#!/bin/sh\nDIR=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\nif [ \"$2\" = \"stop\" ]; then\n  touch \"$DIR/stop-requested.marker\"\n  exit 0\nfi\necho \"Run ID: {run_id}\"\n{loop_body}"
    );
    fs::write(&script, body).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[cfg(windows)]
fn write_fast_success_fixture(dir: &Path, run_id: &str) -> PathBuf {
    let script = dir.join("weco.cmd");
    fs::write(
        &script,
        format!("@echo off\r\necho Run ID: {run_id}\r\nexit /b 0\r\n"),
    )
    .unwrap();
    script
}

#[cfg(not(windows))]
fn write_fast_success_fixture(dir: &Path, run_id: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("weco.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\necho \"Run ID: {run_id}\"\nexit 0\n"),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn run_command(weco_binary: &Path) -> Command {
    let mut command = Command::new(weco_binary);
    command.args(["run", "go"]);
    command
}

#[test]
fn a_run_that_completes_within_budget_is_not_treated_as_timed_out() {
    let dir = temp_dir("fast-success");
    let weco_binary = write_fast_success_fixture(&dir, "fixture-run-id-789");

    let outcome = run_weco_with_wall_clock_backstop(
        run_command(&weco_binary),
        Duration::from_secs(5),
        &weco_binary,
        Duration::from_millis(200),
    )
    .expect("weco run succeeds");

    assert!(!outcome.timed_out);
    assert!(!outcome.graceful_stop_invoked);
    assert!(!outcome.hard_killed);
    assert_eq!(outcome.run_id.as_deref(), Some("fixture-run-id-789"));
    assert!(outcome.exit_status.is_some_and(|status| status.success()));

    fs::remove_dir_all(&dir).ok();
}

/// #349: two independent issues here. (1) The Windows fixture polled its
/// stop marker via `ping -n 1 -w 20`, which never actually waited its
/// intended 20ms (127.0.0.1 replies before the timeout is reached) -- a
/// process-spawn storm, fixed alongside this test by switching to `-n 2`
/// (ping's own ~1.1s default interval). (2) `wall_clock_timeout` (`50ms`
/// originally) left too little margin over the fixture's own
/// process-spawn-and-echo latency for `observed_run_id` to be set before
/// the timeout check fires under real concurrent-build contention (verified
/// against two genuinely concurrent full `cargo test -p code-intel` runs);
/// widened to `2s`. `grace_period` is platform-specific: Windows now polls
/// every ~1.1s and needs room for at least two cycles; Unix's `sleep 0.02`
/// fixture already polls every 20ms and needs none of that margin.
#[test]
fn a_timed_out_run_that_honors_the_graceful_stop_exits_without_a_hard_kill() {
    let dir = temp_dir("graceful");
    let weco_binary = write_fixture(
        &dir,
        if cfg!(windows) { "weco.cmd" } else { "weco.sh" },
        "fixture-run-id-123",
        true,
    );
    let grace_period = if cfg!(windows) {
        Duration::from_secs(3)
    } else {
        Duration::from_millis(500)
    };

    let outcome = run_weco_with_wall_clock_backstop(
        run_command(&weco_binary),
        Duration::from_secs(2),
        &weco_binary,
        grace_period,
    )
    .expect("weco run resolves after graceful stop");

    assert!(outcome.timed_out);
    assert!(outcome.graceful_stop_invoked);
    // `graceful_stop_invoked` only means the stop command was issued, not
    // that the child exited because of it rather than being hard-killed at
    // the end of `grace_period` -- `hard_killed` is the explicit bit for
    // that, asserted directly instead of inferred from elapsed time.
    assert!(
        !outcome.hard_killed,
        "expected a graceful exit, not a hard kill"
    );
    assert_eq!(outcome.run_id.as_deref(), Some("fixture-run-id-123"));

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_timed_out_run_that_ignores_the_graceful_stop_is_hard_killed_after_the_grace_period() {
    let dir = temp_dir("unresponsive");
    let weco_binary = write_fixture(
        &dir,
        if cfg!(windows) { "weco.cmd" } else { "weco.sh" },
        "fixture-run-id-456",
        false,
    );

    let outcome = run_weco_with_wall_clock_backstop(
        run_command(&weco_binary),
        Duration::from_secs(2),
        &weco_binary,
        Duration::from_millis(100),
    )
    .expect("weco run resolves via hard-kill fallback");

    assert!(outcome.timed_out);
    assert!(
        outcome.graceful_stop_invoked,
        "stop should still have been attempted"
    );
    // This fixture never responds to the stop request, so it must actually
    // be hard-killed, not merely have had the stop command issued at it.
    assert!(
        outcome.hard_killed,
        "expected a hard kill, not a graceful exit"
    );
    assert_eq!(outcome.run_id.as_deref(), Some("fixture-run-id-456"));

    fs::remove_dir_all(&dir).ok();
}

/// #349: a synthetic `TimeoutChild` -- no real process, no real clock, no
/// real sleep anywhere near it. `exits_after_polls` counts calls to
/// `try_wait`, not wall-clock time, so tests built on this are
/// deterministic by construction: the same inputs always produce the same
/// decision, on any machine, under any load.
struct MockChild {
    exits_after_polls: Option<u32>,
    poll_count: u32,
}

impl MockChild {
    fn never_exits() -> Self {
        Self {
            exits_after_polls: None,
            poll_count: 0,
        }
    }

    fn exits_after(polls: u32) -> Self {
        Self {
            exits_after_polls: Some(polls),
            poll_count: 0,
        }
    }
}

impl TimeoutChild for MockChild {
    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.poll_count += 1;
        match self.exits_after_polls {
            Some(threshold) if self.poll_count >= threshold => Ok(Some(synthetic_exit_status(0))),
            _ => Ok(None),
        }
    }
}

#[cfg(windows)]
fn synthetic_exit_status(code: u32) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(code)
}

#[cfg(not(windows))]
fn synthetic_exit_status(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(code)
}

#[test]
fn resolve_after_timeout_reports_a_graceful_exit_when_the_child_exits_before_the_deadline() {
    // #349: this is the actual decision logic the flaky real-process tests
    // above were trying (and repeatedly failing, under real CPU/build
    // contention) to exercise. Here it is fully deterministic: no real
    // process, no real clock, no real sleep. `MockChild` exits on its
    // second poll; `deadline_reached` never returns true because there is
    // no real grace period to race against -- the child simply wins first,
    // every single time, on every machine.
    let mut child = MockChild::exits_after(2);
    let mut issued_to: Option<String> = None;
    let (graceful_stop_invoked, hard_killed, exit_status) = resolve_after_timeout(
        &mut child,
        Some("run-123"),
        |run_id| {
            issued_to = Some(run_id.to_string());
            true
        },
        || false,
        || {},
    )
    .expect("resolve_after_timeout does not fail on a mock child");

    assert!(graceful_stop_invoked);
    assert!(!hard_killed);
    assert!(exit_status.is_some());
    assert_eq!(issued_to.as_deref(), Some("run-123"));
}

#[test]
fn resolve_after_timeout_hard_kills_when_the_child_never_exits() {
    let mut child = MockChild::never_exits();
    let mut deadline_checks = 0u32;
    let (graceful_stop_invoked, hard_killed, exit_status) = resolve_after_timeout(
        &mut child,
        Some("run-456"),
        |_| true,
        || {
            deadline_checks += 1;
            deadline_checks >= 3
        },
        || {},
    )
    .expect("resolve_after_timeout does not fail on a mock child");

    assert!(graceful_stop_invoked);
    assert!(hard_killed);
    assert!(exit_status.is_none());
}

#[test]
fn resolve_after_timeout_never_issues_a_stop_when_no_run_id_was_observed() {
    // A run whose id was never observed (the exact race #178/#349's
    // `wall_clock_timeout` widening was about) has nothing to name in a
    // `weco run stop <run-id>` call -- the decision logic must not attempt
    // one, and must go straight to "will hard-kill."
    let mut child = MockChild::never_exits();
    let mut stop_was_called = false;
    let (graceful_stop_invoked, hard_killed, exit_status) = resolve_after_timeout(
        &mut child,
        None,
        |_| {
            stop_was_called = true;
            true
        },
        || true,
        || {},
    )
    .expect("resolve_after_timeout does not fail on a mock child");

    assert!(!graceful_stop_invoked);
    assert!(hard_killed);
    assert!(exit_status.is_none());
    assert!(!stop_was_called, "no run id means no stop command to issue");
}
