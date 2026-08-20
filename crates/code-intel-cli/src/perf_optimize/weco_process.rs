//! #301's wall-clock backstop for `weco run`. Unlike the DAG's generic node
//! timeout (`node_timeout.rs`, hard-kill on expiry), weco is a tree search:
//! its best-so-far candidate lives in its own run state, and a hard kill
//! risks losing it before weco can persist it. So on expiry this asks weco
//! to stop itself first (`weco run stop <run-id>`) and only falls back to
//! `node_timeout`'s hard-kill if weco doesn't exit within a short grace
//! period afterward — never hangs forever either way.
//!
//! **Needs verification against a real `weco` install**: `extract_run_id`
//! assumes weco prints a `run id: <id>` line (case/separator-insensitive)
//! early in its stdout, since the local weco CLI has no other documented way
//! to learn the run id `weco run stop` needs. Stdout is read and scanned
//! line-by-line as it streams (not after the process exits), so a run id
//! printed before the timeout fires is available in time to use. If real
//! weco output doesn't match this shape, the graceful-stop path silently
//! degrades to "never had a run id" and falls straight to the hard-kill
//! fallback below — still correct, just not graceful, and worth confirming
//! before this ships.

use std::io::{BufRead, BufReader};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) enum WecoRunError {
    Io(std::io::Error),
}

#[derive(Debug)]
pub(crate) struct WecoRunOutcome {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) exit_status: Option<ExitStatus>,
    pub(crate) timed_out: bool,
    pub(crate) graceful_stop_invoked: bool,
    pub(crate) run_id: Option<String>,
}

/// Scans a line for a `run id`/`run-id`/`run_id`/`runid` marker (any
/// separator, case-insensitive) followed by `:`, and returns the first
/// whitespace-delimited token after it.
pub(crate) fn extract_run_id(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    for marker in ["run id", "run-id", "run_id", "runid"] {
        let Some(marker_pos) = lower.find(marker) else {
            continue;
        };
        let after_marker = &line[marker_pos + marker.len()..];
        let Some(colon_pos) = after_marker.find(':') else {
            continue;
        };
        let candidate = after_marker[colon_pos + 1..].trim();
        if let Some(id) = candidate.split_whitespace().next() {
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Runs `command` (a fully-configured `weco run ...` invocation) to
/// completion, or until `wall_clock_timeout` elapses. On timeout, attempts
/// `<weco_binary> run stop <run-id>` if a run id was ever seen on stdout,
/// waits `grace_period` for `command`'s process to exit on its own, and
/// hard-kills it (via `node_timeout::terminate_process_tree`) if it hasn't.
pub(crate) fn run_weco_with_wall_clock_backstop(
    mut command: Command,
    wall_clock_timeout: Duration,
    weco_binary: &std::path::Path,
    grace_period: Duration,
) -> Result<WecoRunOutcome, WecoRunError> {
    let started = Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(WecoRunError::Io)?;

    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| WecoRunError::Io(std::io::Error::other("child stdout is unavailable")))?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| WecoRunError::Io(std::io::Error::other("child stderr is unavailable")))?;

    let (run_id_tx, run_id_rx) = mpsc::channel::<String>();
    let stdout_reader = thread::spawn(move || -> std::io::Result<String> {
        let mut full_text = String::new();
        let reader = BufReader::new(stdout_pipe);
        for line in reader.lines() {
            let line = line?;
            if let Some(run_id) = extract_run_id(&line) {
                let _ = run_id_tx.send(run_id);
            }
            full_text.push_str(&line);
            full_text.push('\n');
        }
        Ok(full_text)
    });
    let stderr_reader = thread::spawn(move || -> std::io::Result<String> {
        use std::io::Read;
        let mut bytes = Vec::new();
        stderr_pipe
            .read_to_end(&mut bytes)
            .map(|_| String::from_utf8_lossy(&bytes).into_owned())
    });

    let mut observed_run_id: Option<String> = None;
    let mut graceful_stop_invoked = false;
    let mut timed_out = false;

    loop {
        while let Ok(run_id) = run_id_rx.try_recv() {
            observed_run_id = Some(run_id);
        }
        if let Some(status) = child.try_wait().map_err(WecoRunError::Io)? {
            return finish(
                stdout_reader,
                stderr_reader,
                Some(status),
                timed_out,
                graceful_stop_invoked,
                observed_run_id,
            );
        }
        if started.elapsed() >= wall_clock_timeout {
            timed_out = true;
            if let Some(run_id) = &observed_run_id {
                let stop_status = Command::new(weco_binary)
                    .args(["run", "stop", run_id])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                graceful_stop_invoked = stop_status.is_ok();
            }
            let stop_deadline = Instant::now() + grace_period;
            while Instant::now() < stop_deadline {
                if let Some(status) = child.try_wait().map_err(WecoRunError::Io)? {
                    return finish(
                        stdout_reader,
                        stderr_reader,
                        Some(status),
                        timed_out,
                        graceful_stop_invoked,
                        observed_run_id,
                    );
                }
                thread::sleep(Duration::from_millis(5));
            }
            crate::node_timeout::terminate_process_tree(&mut child);
            let status = child.wait().ok();
            return finish(
                stdout_reader,
                stderr_reader,
                status,
                timed_out,
                graceful_stop_invoked,
                observed_run_id,
            );
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn finish(
    stdout_reader: thread::JoinHandle<std::io::Result<String>>,
    stderr_reader: thread::JoinHandle<std::io::Result<String>>,
    exit_status: Option<ExitStatus>,
    timed_out: bool,
    graceful_stop_invoked: bool,
    run_id: Option<String>,
) -> Result<WecoRunOutcome, WecoRunError> {
    let stdout = stdout_reader
        .join()
        .map_err(|_| WecoRunError::Io(std::io::Error::other("stdout reader panicked")))?
        .map_err(WecoRunError::Io)?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| WecoRunError::Io(std::io::Error::other("stderr reader panicked")))?
        .map_err(WecoRunError::Io)?;
    Ok(WecoRunOutcome {
        stdout,
        stderr,
        exit_status,
        timed_out,
        graceful_stop_invoked,
        run_id,
    })
}

#[cfg(test)]
#[path = "weco_process_tests.rs"]
mod tests;
