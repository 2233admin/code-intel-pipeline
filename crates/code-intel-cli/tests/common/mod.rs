//! Shared test support: spawning the CLI without the developer's environment
//! leaking into it.
//!
//! `src/env_contract.rs` is included directly rather than copied, so the list
//! of variables this module clears is the same list the binary declares.

#[path = "../../src/env_contract.rs"]
pub mod env_contract;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The CLI under test, with every pipeline-owned variable removed.
///
/// Tests used to build `Command::new(env!("CARGO_BIN_EXE_code-intel"))`
/// directly, which inherits the whole ambient environment. Any variable in
/// [`env_contract::PIPELINE_VARS`] that the developer happened to export then
/// redirected the child at their real installation — a root, a manifest or a
/// binary — and the default branch went untested. Worse, it only reproduced
/// for people whose shell exported it, which is why `CODE_INTEL_HOME` took two
/// separate investigations to pin down.
///
/// Ambient variables (`PATH`, `HOME`, …) are deliberately left alone: a child
/// with no `PATH` cannot find `git`, and tests that care set them explicitly.
///
/// A test that is genuinely exercising one of these variables sets it back
/// with `.env(...)` after calling this — the point is that it becomes a
/// property of the test rather than of the shell.
pub fn cli() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_code-intel"));
    for name in env_contract::PIPELINE_VARS {
        command.env_remove(name);
    }
    command
}

/// [`cli`] with the working directory set.
pub fn cli_in(dir: &Path) -> Command {
    let mut command = cli();
    command.current_dir(dir);
    command
}

/// Values that must not change the CLI's behaviour when they leak in from the
/// surrounding shell. Used by the hermeticity regression test to prove the
/// clearing above actually holds.
pub fn hostile_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("CODE_INTEL_ARTIFACT_ROOT", "/nonexistent/hostile/artifacts"),
        ("CODE_INTEL_BIN", "/nonexistent/hostile/bin"),
        ("CODE_INTEL_DATA_ROOT", "/nonexistent/hostile/data"),
        ("CODE_INTEL_HOME", "/nonexistent/hostile/home"),
        (
            "CODE_INTEL_INTEGRATIONS_MANIFEST",
            "/nonexistent/hostile/manifest.json",
        ),
        ("CODE_INTEL_TEST_RG_EXTRA_PATH", "hostile/extra.rs"),
        ("SENTRUX_AUTO_PRO", "1"),
    ]
}

/// Same-process uniqueness rests on `SEQUENCE`: the pid is constant for the
/// whole process, and a clock reading alone can repeat when sibling test
/// threads allocate in the same tick (same reasoning as
/// `src/test_support.rs`, #231/#352). The clock is kept so leftovers from a
/// killed run stay sortable by age.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A private scratch directory under the system temp dir, removed on drop.
///
/// Named `code-intel-test-{label}-{pid}-{nanos}-{sequence}` and always
/// freshly created, so concurrent test binaries (or two worktrees running the
/// suite at once) never share a directory.
#[allow(dead_code)]
pub struct TempDir(PathBuf);

#[allow(dead_code)]
impl TempDir {
    pub fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "code-intel-test-{label}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create private temporary directory");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Write `body` to `dir/name`, executable, and return the path.
///
/// On Unix a child `sh` writes the file and sets its mode, so this process
/// never holds a writable descriptor on a file it is about to execute. A
/// sibling test thread forking while we held one would leak it into its child
/// until that child's `execve`, and running the script in that window fails
/// with ETXTBSY (rust-lang/rust#114554, #485).
#[allow(dead_code)]
pub fn write_executable(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    write_executable_file(&path, body);
    path
}

#[cfg(not(windows))]
fn write_executable_file(path: &Path, body: &str) {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn fixture writer");
    child
        .stdin
        .take()
        .expect("fixture writer stdin")
        .write_all(body.as_bytes())
        .expect("feed fixture writer");
    let status = child.wait().expect("wait for fixture writer");
    assert!(status.success(), "fixture writer failed: {status}");
}

#[cfg(windows)]
fn write_executable_file(path: &Path, body: &str) {
    fs::write(path, body).expect("write fixture");
}

#[test]
fn temp_dir_allocations_are_distinct_and_exist() {
    let first = TempDir::new("common-unit");
    let second = TempDir::new("common-unit");
    assert_ne!(first.path(), second.path());
    assert!(first.path().is_dir());
    assert!(second.path().is_dir());
}

#[test]
fn temp_dir_is_removed_on_drop() {
    let dir = TempDir::new("common-unit");
    let path = dir.path().to_path_buf();
    drop(dir);
    assert!(!path.exists());
}
