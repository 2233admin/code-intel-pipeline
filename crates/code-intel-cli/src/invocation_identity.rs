//! Anti-replay invocation identity for verdict-producing commands.
//!
//! Agent hosts increasingly wrap their shell in token-optimizing command
//! proxies that may replay a cached capture when the same command line runs
//! twice (#197). Exit code and stdout are replayed with it, so a verdict
//! command cannot prove liveness through its verdict alone. The
//! counter-measure is one stderr line whose bytes necessarily differ on
//! every invocation: a replayed capture exposes itself by a repeated `id=`
//! and a stale `at=` clock.
//!
//! Named "invocation identity", not "run identity": a manifest `runIdentity`
//! is digest-derived and deterministically stable for the same content, while
//! this line is nondeterministic by design — the two must never be conflated.
//!
//! stderr carries the line so no stdout JSON contract changes shape, and
//! the line never enters manifests, reports, or digests, so artifact
//! determinism contracts are unaffected. Doctor stays silent here: its
//! envelope contract asserts an empty stderr, so its identity has to ride
//! inside the envelope itself (tracked in #197).

use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::capability::rfc3339_now;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Writes the identity line straight to the process stderr stream, matching
/// the direct-emission family of the compatibility routes that call it.
pub(crate) fn emit(command_label: &str) {
    eprintln!("{}", line(command_label));
}

fn line(command_label: &str) -> String {
    format!(
        "invocation-identity: command={command_label} id={} at={}",
        unique_id(),
        rfc3339_now()
    )
}

/// Wall-clock nanoseconds, process id, and an in-process sequence number.
/// Uniqueness is the only requirement — replay detection needs two
/// invocations to differ, not cryptographic unforgeability.
fn unique_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let pid = process::id();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{pid:x}-{sequence:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_carries_command_id_and_utc_clock() {
        let line = line("audit");
        assert!(
            line.starts_with("invocation-identity: command=audit id="),
            "{line}"
        );
        let at = line.split(" at=").nth(1).expect("at field");
        assert_eq!(at.len(), "2026-08-06T00:00:00Z".len(), "{line}");
        assert!(at.ends_with('Z'), "{line}");
    }

    #[test]
    fn consecutive_lines_never_repeat() {
        assert_ne!(line("run-execute"), line("run-execute"));
    }
}
