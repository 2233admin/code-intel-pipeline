//! #301's wall-clock backstop for `weco run`. Unlike the DAG's generic node
//! timeout (`node_timeout.rs`, hard-kill on expiry), weco is a tree search:
//! its best-so-far candidate lives in its own run state, and a hard kill
//! risks losing it before weco can persist it. So on expiry this asks weco
//! to stop itself first (`weco run stop <run-id>`) and only falls back to
//! `node_timeout`'s hard-kill if weco doesn't exit within a short grace
//! period afterward — never hangs forever either way.
//!
//! `extract_run_id`'s pattern is confirmed against weco-cli's own source
//! (#301 research), not a guess: it prints exactly `Run ID: {run_id}` at
//! the start of every run. Stdout is still read and scanned line-by-line as
//! it streams (not after the process exits), so a run id printed before the
//! timeout fires is available in time to use for the graceful stop.

use std::io::{BufRead, BufReader};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
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

/// A reader thread's output, shared with the main thread via a mutex rather
/// than only handed back through `JoinHandle::join`. A hard-killed process
/// can leave a descendant (e.g. a shell's already-forked child) holding the
/// stdout/stderr pipe open well past the point the process we tracked has
/// exited -- `node_timeout.rs`'s own doc comment calls this out for exactly
/// the same `sh -c "..."` shape these fixtures and weco itself both use.
/// Joining the reader thread in that case blocks until the *descendant*
/// exits, not the process we killed, so the hard-kill path reads a snapshot
/// of this shared buffer instead of joining.
type SharedOutput = Arc<Mutex<String>>;

fn snapshot(output: &SharedOutput) -> String {
    output
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn spawn_line_reader(
    pipe: impl std::io::Read + Send + 'static,
    output: SharedOutput,
    run_id_tx: Option<std::sync::mpsc::Sender<String>>,
) -> thread::JoinHandle<std::io::Result<()>> {
    thread::spawn(move || -> std::io::Result<()> {
        let reader = BufReader::new(pipe);
        for line in reader.lines() {
            let line = line?;
            if let Some(tx) = &run_id_tx {
                if let Some(run_id) = extract_run_id(&line) {
                    let _ = tx.send(run_id);
                }
            }
            let mut buffer = output
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            buffer.push_str(&line);
            buffer.push('\n');
        }
        Ok(())
    })
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
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| WecoRunError::Io(std::io::Error::other("child stderr is unavailable")))?;

    let stdout_buffer: SharedOutput = Arc::new(Mutex::new(String::new()));
    let stderr_buffer: SharedOutput = Arc::new(Mutex::new(String::new()));
    let (run_id_tx, run_id_rx) = std::sync::mpsc::channel::<String>();
    let stdout_reader = spawn_line_reader(stdout_pipe, stdout_buffer.clone(), Some(run_id_tx));
    let stderr_reader = spawn_line_reader(stderr_pipe, stderr_buffer.clone(), None);

    let mut observed_run_id: Option<String> = None;
    let mut graceful_stop_invoked = false;
    let mut timed_out = false;

    loop {
        while let Ok(run_id) = run_id_rx.try_recv() {
            observed_run_id = Some(run_id);
        }
        if let Some(status) = child.try_wait().map_err(WecoRunError::Io)? {
            // A clean exit means EOF really did close both pipes, so the
            // reader threads are finishing (or already have) on their own --
            // safe to join before snapshotting, so the buffers are complete.
            join_best_effort(stdout_reader);
            join_best_effort(stderr_reader);
            return Ok(WecoRunOutcome {
                stdout: snapshot(&stdout_buffer),
                stderr: snapshot(&stderr_buffer),
                exit_status: Some(status),
                timed_out,
                graceful_stop_invoked,
                run_id: observed_run_id,
            });
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
                    // Exited on its own within the grace period (the
                    // graceful-stop path) -- same reasoning as the clean-exit
                    // case above: safe to join before snapshotting.
                    join_best_effort(stdout_reader);
                    join_best_effort(stderr_reader);
                    return Ok(WecoRunOutcome {
                        stdout: snapshot(&stdout_buffer),
                        stderr: snapshot(&stderr_buffer),
                        exit_status: Some(status),
                        timed_out,
                        graceful_stop_invoked,
                        run_id: observed_run_id,
                    });
                }
                thread::sleep(Duration::from_millis(5));
            }
            crate::node_timeout::terminate_process_tree(&mut child);
            let status = child.wait().ok();
            // Deliberately does not join the reader threads: a descendant of
            // the just-killed process may still hold the pipe open (see the
            // module doc comment), which would block here until that
            // descendant exits on its own -- exactly the hang this backstop
            // exists to prevent. The threads are left running in the
            // background (harmless; they exit whenever their pipe finally
            // closes) and this returns a snapshot of whatever they'd
            // captured up to now instead.
            return Ok(WecoRunOutcome {
                stdout: snapshot(&stdout_buffer),
                stderr: snapshot(&stderr_buffer),
                exit_status: status,
                timed_out,
                graceful_stop_invoked,
                run_id: observed_run_id,
            });
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// A reader thread panicking or erroring mid-read doesn't invalidate a
/// result the process itself already exited cleanly for -- the shared
/// buffer just stays whatever was captured up to that point.
fn join_best_effort(reader: thread::JoinHandle<std::io::Result<()>>) {
    let _ = reader.join();
}

#[cfg(test)]
#[path = "weco_process_tests.rs"]
mod tests;
