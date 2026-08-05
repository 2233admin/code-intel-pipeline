//! Gate over every digest an internalization record declares about itself.
//!
//! The reason this is one sweep rather than a per-record assertion: before it,
//! 84 declared pins were spread across 35 records and exactly one — the
//! ast-grep conformance digest — had a test watching it. Editing 37 test files
//! went 31 stale pins deep with a green suite, because the gate was scoped to
//! the pins someone had happened to write an assertion for rather than to the
//! invariant "a declared digest matches the file it names".
//!
//! The check runs through `repin` itself, so the tool that resyncs a pin and
//! the gate that fails on a stale one are the same code path. A gate that
//! reimplemented the audit could pass while `repin --write` left the tree
//! inconsistent — which is the shape of the bug this is here to prevent.

mod common;

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn repin_reports_a_consistent_tree() {
    let output = common::cli()
        .args(["repin", "--repo"])
        .arg(repo_root())
        .output()
        .expect("run repin");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let offenders: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("STALE") || line.contains("UNRESOLVED"))
        .collect();

    assert!(
        output.status.success(),
        "repin reports the tree inconsistent (exit {:?}).\n{}\n\
         Stale declared pins are fixed by `code-intel repin --repo . --write`; \
         UNRESOLVED ones need a human to say what the record now claims.\n{}",
        output.status.code(),
        offenders.join("\n"),
        String::from_utf8_lossy(&output.stderr),
    );
}

// There is deliberately no `--write` test here. `--write` mutates the
// repository it is pointed at, and the only tree that carries real records is
// this one — so such a test would rewrite `orchestration/internalization/`
// underneath every other test in the run. The fixpoint property it would check
// is enforced inside `repin` instead: after writing it re-audits and exits 65
// if any declared pin is still stale, so a `--write` that leaves work behind
// fails loudly at the point of the write rather than silently reporting
// success.
