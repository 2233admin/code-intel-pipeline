//! Shared test-only fixture helpers (#231, #352).
//!
//! `#231` diagnosed why this crate accumulates duplicate small helpers
//! instead of sharing one implementation: modules are wired together with
//! `#[path = "..."] mod x;` re-inclusion, so pulling a shared helper into a
//! `#[path]`-mounted file taxes every test binary that mounts it -- and its
//! transitive dependency chain. `#352` fixed the four instances of this
//! crate's `unique_temp_dir`-style helper that back production code or a
//! `#[path]`-mounted file (`audit_report::temp_report`,
//! `mcp_serve::handlers`, `project_context`, `session_evidence`); those keep
//! their own implementations because a shared version there would need the
//! `module_path!()` owner-token disambiguation those call sites require.
//!
//! This module is the other half of `#352`'s deferred scope: the seven
//! `unique_temp_dir` reimplementations that back ordinary `#[cfg(test)] mod
//! tests` blocks compiled exactly once, as a plain `mod x;` reachable only
//! from `cargo test -p code-intel --bin code-intel`. None of those files are
//! `#[path]`-mounted into any other test binary, so the `#[path]` tax that
//! blocks sharing code in the `#352` cases does not apply here -- one
//! canonical, tested helper can replace all seven copies without adding
//! compile or dead-code surface to unrelated test binaries.
//!
//! No `module_path!()` owner token: that component exists solely to
//! disambiguate two copies of the *same* file mounted at different parent
//! modules within one process (see `temp_report.rs`), a scenario that
//! cannot occur for a helper that is itself compiled exactly once.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir_path(pid: u32, nonce: u128, sequence: u64, name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("code-intel-test-{name}-{pid}-{nonce}-{sequence}"))
}

/// #175 / #178 / #231 / #352: pid alone does not make this unique -- it is
/// constant across every call made by one process, so within-process
/// uniqueness rests entirely on `SEQUENCE`. A raw clock reading is not a
/// substitute: two calls issued in quick succession by the same process (or
/// by sibling test threads racing each other, which `cargo test`'s default
/// runner does routinely) can land in the same clock tick, and pid+clock
/// alone then hands out the identical path twice -- exactly the collision
/// `#352` traced to one duplicated test file's leftover directory. The
/// clock reading is kept only so leftover directories from a killed run
/// stay sortable by age; `SEQUENCE` is what actually buys uniqueness.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn allocate_unique_temp_dir_path(name: &str, pid: u32, clock: impl FnOnce() -> u128) -> PathBuf {
    let nonce = clock();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    temp_dir_path(pid, nonce, sequence, name)
}

/// Build a filesystem path guaranteed unique per call, for test fixtures
/// that need a scratch directory. `name` should describe the fixture's
/// purpose so a leftover directory (e.g. from a killed test run) stays
/// identifiable during manual cleanup.
pub(crate) fn unique_temp_dir(name: &str) -> PathBuf {
    allocate_unique_temp_dir_path(name, std::process::id(), || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    })
}

#[test]
fn allocate_unique_temp_dir_path_differs_even_when_the_clock_repeats() {
    // #352-style regression: with pid and name both held constant and the
    // clock stubbed to return the same reading on every call, the only
    // remaining source of uniqueness is `SEQUENCE`. Without it incrementing,
    // this assertion is red -- see the PR description for the red-run output
    // captured by temporarily disabling the `fetch_add`.
    let fixed_clock = || 0xC352_u128;
    let first = allocate_unique_temp_dir_path("fixture", 4242, fixed_clock);
    let second = allocate_unique_temp_dir_path("fixture", 4242, fixed_clock);
    assert_ne!(
        first, second,
        "allocator must stay unique even when the clock reading repeats"
    );
}

#[test]
fn unique_temp_dir_differs_across_repeated_calls() {
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..256 {
        let dir = unique_temp_dir("burst");
        assert!(
            seen.insert(dir.clone()),
            "unique_temp_dir returned a duplicate inside one burst: {}",
            dir.display()
        );
    }
}
