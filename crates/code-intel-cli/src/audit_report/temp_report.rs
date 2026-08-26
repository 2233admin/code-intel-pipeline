//! Extracted from `cli_tests.rs` to keep that file under the god-file
//! threshold (#352 added ~80 lines here; folding them into the parent file
//! would have crossed `functions>25 && loc>400`). `TempReport` itself is
//! used throughout `cli_tests.rs`; only its allocator, the two regression
//! tests pinning that allocator's uniqueness contract, and their doc
//! commentary live here.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

use serde_json::Value;

pub(super) struct TempReport(PathBuf);

fn temp_report_path(owner: &str, pid: u32, nonce: u128, sequence: u64) -> PathBuf {
    env::temp_dir().join(format!(
        "code-intel-audit-cli-test-{owner}-{pid}-{nonce}-{sequence}.json"
    ))
}

/// #352: the previous allocator was `pid + a raw clock read`, nothing else.
/// Its whole job is "never hand out the same path twice," but pid is
/// constant within a process and the clock is the only other input, so a
/// clock-resolution collision (two calls landing in the same tick) was an
/// unconditional path collision. `clock` is a parameter, not a direct
/// `SystemTime::now()` call, so a test can force exactly that condition --
/// an identical reading on consecutive calls -- instead of hoping a real
/// collision happens to occur.
///
/// `SEQUENCE` alone is not enough to survive this crate's `#[path]`
/// topology (the same class of gap #178 closed for `tool_path.rs`): this
/// file is `#[path]`-mounted into roughly ten separate `cargo test`
/// integration binaries today, each with its own independent copy of this
/// `static` starting at 0. If `audit_report` is ever additionally
/// re-mounted a *second* time within one of those binaries under a
/// different parent module -- exactly what happened to `tool_path.rs` --
/// two copies in the same process, same pid, could still emit an identical
/// `(pid, nonce, sequence)` triple. `owner` (`module_path!()`, unique per
/// mount point) closes that gap; the clock reading itself is kept only so
/// leftover files stay sortable by age.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn allocate_unique_report_path(owner: &str, pid: u32, clock: impl FnOnce() -> u128) -> PathBuf {
    let nonce = clock();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    temp_report_path(owner, pid, nonce, sequence)
}

impl TempReport {
    pub(super) fn write(value: &Value) -> Self {
        let owner = module_path!().replace("::", "-");
        let path = allocate_unique_report_path(&owner, std::process::id(), || {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        });
        fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        Self(path)
    }

    pub(super) fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempReport {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn allocate_unique_report_path_differs_even_when_the_clock_repeats() {
    // #352 regression. Before `SEQUENCE` existed, this was red: pid and
    // owner are both constant across the two calls and the clock is stubbed
    // to return the exact same reading both times, so with pid+nonce alone
    // the two calls produced an identical path -- the same condition that
    // let one test's temp file silently become another test's temp file.
    // The allocator's contract is "never collide, even under a repeated
    // clock reading," not merely "format pid and nonce into a string."
    let fixed_clock = || 0xC352_u128;
    let first = allocate_unique_report_path("owner", 4242, fixed_clock);
    let second = allocate_unique_report_path("owner", 4242, fixed_clock);
    assert_ne!(
        first, second,
        "allocator must stay unique even when the clock reading repeats"
    );
}

#[test]
fn temp_report_path_partitions_by_owner_token() {
    // Mirrors #178: with pid, nonce, and sequence all held identical, two
    // different owner tokens (the disambiguator for two copies of this file
    // mounted at different parent modules within one process) must still
    // resolve to different paths. Deliberately does not go through
    // `allocate_unique_report_path` -- that would let the ever-incrementing
    // `SEQUENCE` make the paths differ for free without owner ever being
    // exercised.
    let path_a = temp_report_path("owner-a", 4242, 0xC352, 0);
    let path_b = temp_report_path("owner-b", 4242, 0xC352, 0);
    assert_ne!(
        path_a, path_b,
        "different owner tokens must not collide even with identical pid/nonce/sequence"
    );
}
