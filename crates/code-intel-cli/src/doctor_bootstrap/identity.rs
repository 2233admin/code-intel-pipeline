//! Anti-replay invocation identity for the doctor CLI surface (#197).
//!
//! Attached only where the CLI prints: the DAG doctor node consumes
//! `observe()` directly and its artifact digests must stay replay-stable, so
//! the identity never enters the observation builder. Kept inside
//! `doctor_bootstrap` instead of calling `crate::invocation_identity`: the
//! parent module is `#[path]`-included as an independent compilation root by
//! the doctor-adapter copies in integration tests (see the graphProvider
//! comment in `observe`), where `crate::` references outside the included set
//! do not resolve.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Inserts `invocationIdentity` into the printed observation. The `nonce` is
/// the caller-supplied cache-buster echoed back to prove the value reached a
/// live process instead of a cached capture.
pub(super) fn attach(observation: &mut Value, nonce: Option<String>) {
    if let Some(fields) = observation.as_object_mut() {
        fields.insert(
            "invocationIdentity".into(),
            json!({
                "id": invocation_id(),
                "nonce": nonce.unwrap_or_default(),
            }),
        );
    }
}

/// The human-rendering counterpart of the JSON field.
pub(super) fn human_line(observation: &Value) -> String {
    format!(
        "Invocation identity: {} nonce={}",
        observation["invocationIdentity"]["id"]
            .as_str()
            .unwrap_or(""),
        observation["invocationIdentity"]["nonce"]
            .as_str()
            .unwrap_or("")
    )
}

/// Wall-clock nanoseconds, process id, and an in-process sequence — two
/// honest invocations can never repeat it. Uniqueness is the only
/// requirement; replay detection needs no unforgeability.
fn invocation_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{:x}-{sequence:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_id_never_repeats_within_a_process() {
        assert_ne!(invocation_id(), invocation_id());
    }

    #[test]
    fn attach_echoes_the_nonce_and_human_line_mirrors_it() {
        let mut observation = json!({"ok": true});
        attach(&mut observation, Some("night-7".into()));
        assert_eq!(observation["invocationIdentity"]["nonce"], json!("night-7"));
        let line = human_line(&observation);
        assert!(line.starts_with("Invocation identity: "), "{line}");
        assert!(line.ends_with(" nonce=night-7"), "{line}");
    }
}
