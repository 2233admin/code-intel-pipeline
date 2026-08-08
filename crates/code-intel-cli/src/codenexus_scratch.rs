//! Scratch-directory and process-timeout helpers for the CodeNexus-lite
//! builtin admission route (`builtin_provider_evidence::codenexus_admission`).
//! Split out of that file to keep it under the sentrux `god_files` line-count
//! gate (`loc>800`) -- this module has no admission/evidence logic of its
//! own, purely process/filesystem plumbing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::AdapterError;

static CODENEXUS_SCRATCH_NONCE: AtomicU64 = AtomicU64::new(0);

/// Bounds how long the CodeNexus-lite facade may run. Without this, a hung
/// `pwsh` process (network stall inside the script, a pathologically large
/// git history) would block `evidence.codenexus` -- and the whole DAG run --
/// forever. `status: "unavailable"` is already a first-class admitted
/// outcome, so timing out just routes into that existing branch rather than
/// adding a new failure mode.
pub(super) const CODENEXUS_LITE_TIMEOUT: Duration = Duration::from_secs(120);

/// A scratch directory for the CodeNexus-lite facade's own working files,
/// separate from `out`: `out` must not exist until after collection (see
/// `graph_admission`/`sentrux_admission`'s `fs::create_dir(out)` ordering),
/// but the facade needs somewhere to write `codenexus-context.json` before
/// that point.
pub(super) fn create_codenexus_scratch_dir() -> Result<PathBuf, AdapterError> {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AdapterError::Io(format!("read clock for scratch dir: {error}")))?
        .as_nanos();
    for _ in 0..64 {
        let nonce = CODENEXUS_SCRATCH_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "code-intel-codenexus-lite-{}-{epoch}-{nonce}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(AdapterError::Io(format!(
                    "create CodeNexus-lite scratch directory: {error}"
                )))
            }
        }
    }
    Err(AdapterError::Io(
        "CodeNexus-lite scratch directory name space exhausted".into(),
    ))
}

/// Removes the CodeNexus-lite scratch directory on every exit path,
/// including the early `?` returns between collection and publication
/// (snapshot verification, clock reads, identity extraction) that a plain
/// end-of-function cleanup would miss.
pub(super) struct ScratchDir(pub(super) PathBuf);

impl ScratchDir {
    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs `command` to completion or kills it after `timeout`, whichever comes
/// first. Stdio is discarded rather than piped: the facade communicates its
/// result through `-OutputPath`, not stdout/stderr, and piping without
/// draining while polling `try_wait` would risk a full-pipe deadlock.
pub(super) fn run_with_timeout(mut command: Command, timeout: Duration) -> std::io::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Output {
                status,
                stdout: Vec::new(),
                stderr: Vec::new(),
            });
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("process exceeded {timeout:?} timeout"),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
