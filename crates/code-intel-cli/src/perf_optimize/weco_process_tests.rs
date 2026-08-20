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
        "if exist \"%~dp0stop-requested.marker\" exit /b 0\r\nping -n 1 -w 20 127.0.0.1 >nul\r\ngoto loop\r\n"
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
    assert_eq!(outcome.run_id.as_deref(), Some("fixture-run-id-789"));
    assert!(outcome.exit_status.is_some_and(|status| status.success()));

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_timed_out_run_that_honors_the_graceful_stop_exits_without_a_hard_kill() {
    let dir = temp_dir("graceful");
    let weco_binary = write_fixture(
        &dir,
        if cfg!(windows) { "weco.cmd" } else { "weco.sh" },
        "fixture-run-id-123",
        true,
    );

    let started = Instant::now();
    let outcome = run_weco_with_wall_clock_backstop(
        run_command(&weco_binary),
        Duration::from_millis(50),
        &weco_binary,
        Duration::from_millis(500),
    )
    .expect("weco run resolves after graceful stop");
    let elapsed = started.elapsed();

    assert!(outcome.timed_out);
    assert!(outcome.graceful_stop_invoked);
    assert_eq!(outcome.run_id.as_deref(), Some("fixture-run-id-123"));
    // Grace period was generous (500ms) but the fixture checks its marker
    // every 20ms, so it should exit almost immediately after the stop
    // request lands -- well under the full grace period.
    assert!(
        elapsed < Duration::from_millis(400),
        "graceful exit took too long: {elapsed:?}"
    );

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

    let started = Instant::now();
    let outcome = run_weco_with_wall_clock_backstop(
        run_command(&weco_binary),
        Duration::from_millis(50),
        &weco_binary,
        Duration::from_millis(100),
    )
    .expect("weco run resolves via hard-kill fallback");
    let elapsed = started.elapsed();

    assert!(outcome.timed_out);
    assert!(
        outcome.graceful_stop_invoked,
        "stop should still have been attempted"
    );
    assert_eq!(outcome.run_id.as_deref(), Some("fixture-run-id-456"));
    // Never hangs forever: timeout (50ms) + grace period (100ms) + a small
    // margin for the hard-kill itself, well short of the fixture's own
    // 60-second sleep.
    assert!(
        elapsed < Duration::from_secs(5),
        "hard-kill fallback took too long: {elapsed:?}"
    );

    fs::remove_dir_all(&dir).ok();
}
