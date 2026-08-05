//! A completeness claim for one evidence-producing surface (a scan, a
//! query, an enumeration) that a downstream reader can check rather than
//! just trust. Gate G1 of the project charter (issue #139, tracked in
//! #138): "the honesty bit is computed, not asserted."
//!
//! The motivating failure (#133): `code-intel repin --write --json`
//! printed `"clean": true, "filesChanged": 0` while digests the scan never
//! actually reached (`operationTrace[].source.sha256`, embedded
//! `evidenceIds` suffixes) sat stale. Nothing in that report distinguished
//! "I looked everywhere and it's clean" from "I looked nowhere" -- both
//! render as the same three JSON tokens. A downstream agent reading
//! silence as "fine" then writes code on a false premise.
//!
//! [`EvidenceOutcome::Complete`] closes that gap the only way a type
//! system can: there is no constructor for it that does not take an
//! [`EvidenceScope`]. You cannot write `EvidenceOutcome::Complete` in Rust
//! without a scope value already in hand, any more than you can write
//! `Some` without a payload.
//!
//! That alone cannot stop a hand-edited (or hand-written-by-a-different-
//! code-path) JSON artifact from claiming `"status":"complete"` with no
//! `"scope"` object at all -- which is exactly the shape #133's
//! `clean:true` took: a bare assertion, no backing data, nothing to check
//! it against. So [`EvidenceOutcome::from_json`] enforces the same rule at
//! the deserialization boundary: a `"complete"` (or `"partial"`) status
//! whose `scope` is missing, or whose scope is missing a required field,
//! is a parse error, not a value with an empty scope silently filled in.
//!
//! A missing-scope check alone is not enough, though: the minimal, most
//! realistic forgery is not "delete everything and write `complete`" but
//! "flip the one field that says the status" -- exactly what #133's own
//! bug did (one stale field, everything else untouched). Take a real
//! `Partial` value, which already carries a genuine, fully-populated
//! `scope`, and change only its `status` to `"complete"`: the scope is
//! right there and well-formed, so a bare "does `scope` exist and have its
//! fields?" check would accept it. What gives it away is the leftover
//! `partialReason` (and `detail`/`paths`) fields a real `Complete` never
//! has. `from_json` therefore also requires each status's JSON object to
//! carry *exactly* its expected key set -- no leftovers from a different
//! status -- the same discipline `content_contract::require_exact_keys`
//! applies elsewhere in this crate.
//! See the reverse test in `crates/code-intel-cli/tests/repin.rs`
//! (`forged_complete_claim_without_scope_is_rejected`), which forges both
//! ways: a `not-computed` report relabeled `complete` (missing `scope`
//! entirely), and a `partial` report relabeled `complete` by touching only
//! `status` (present, well-formed `scope`, but disqualifying leftovers).
//!
//! `repin.rs` is this type's first (and, as of this change, only) real
//! consumer -- see its module docs for how `RepinReport` derives both its
//! JSON `scanCoverage` field and its `clean`/exit-code verdict from the
//! same stored [`EvidenceOutcome`] value, so the two can never read
//! different data (the failure mode the charter calls out separately: an
//! interval computed in one structure, a pass/fail decision read from
//! another, and the two never meeting).

use serde_json::{json, Value};

/// The completeness state of one evidence surface. `Complete` is the only
/// variant that lets a reader trust silence as "checked, and it's fine";
/// the other two exist so a surface never has to lie its way into looking
/// like `Complete` when it isn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EvidenceOutcome {
    /// The scan surface was fully enumerated and everything in it was
    /// inspected. Carries the [`EvidenceScope`] it covers -- constructing
    /// this variant without one is not possible in Rust, since it is a
    /// required field, not an `Option`.
    Complete(EvidenceScope),
    /// The surface was only partly covered. `reason` says why (truncation,
    /// a cap, an input that could not be read); `scope` says what *was*
    /// covered despite that, so a reader can still tell how much ground
    /// was actually inspected.
    Partial {
        reason: PartialReason,
        scope: EvidenceScope,
    },
    /// The surface was never evaluated. `reason` says why it was skipped
    /// (not attempted, or attempted over a vacuous / degenerate input).
    NotComputed { reason: String },
}

/// What a `Complete` (or `Partial`) claim actually covers: which artifact
/// types were consumed, which paths were enumerated and inspected, and
/// which paths were deliberately left out of the surface (as opposed to
/// paths that failed to load -- that gap belongs in
/// `PartialReason::UnreadableInput` instead, because an intentional
/// exclusion is not a coverage failure).
///
/// Deliberately does not derive `Default`: an all-empty scope should
/// require writing out every field by hand, not a single `::default()`
/// call, so a vacuous scope never appears by accident (or by someone
/// reaching for the path of least resistance under review pressure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidenceScope {
    pub(crate) artifact_types: Vec<String>,
    pub(crate) scanned_paths: Vec<String>,
    pub(crate) excluded_prefixes: Vec<String>,
}

/// Why an [`EvidenceOutcome::Partial`] is partial. Every variant carries
/// its own detail rather than a bare tag, so `"partial, no reason given"`
/// is not representable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PartialReason {
    /// The surface was cut off partway through (a result stream, a walk
    /// that stopped early) rather than reaching a natural end.
    Truncated(String),
    /// A configured limit was hit before the surface was fully covered.
    CapHit(String),
    /// One or more inputs inside the surface could not be read at all
    /// (too large, not valid text, I/O error); the paths that failed.
    UnreadableInput(Vec<String>),
}

impl EvidenceOutcome {
    pub(crate) fn is_complete(&self) -> bool {
        matches!(self, EvidenceOutcome::Complete(_))
    }

    /// A short human-readable summary, for text-mode output that needs to
    /// say *why* a surface isn't `Complete` without dumping the full
    /// `scanCoverage` JSON object.
    pub(crate) fn describe(&self) -> String {
        match self {
            EvidenceOutcome::Complete(scope) => {
                format!("complete ({} path(s) scanned)", scope.scanned_paths.len())
            }
            EvidenceOutcome::Partial { reason, scope } => format!(
                "partial ({}; {} path(s) scanned)",
                reason.describe(),
                scope.scanned_paths.len()
            ),
            EvidenceOutcome::NotComputed { reason } => format!("not-computed ({reason})"),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        match self {
            EvidenceOutcome::Complete(scope) => json!({
                "status": "complete",
                "scope": scope.to_json(),
            }),
            EvidenceOutcome::Partial { reason, scope } => {
                let mut value = reason.to_json();
                let object = value
                    .as_object_mut()
                    .expect("reason.to_json() is an object");
                object.insert("status".to_string(), json!("partial"));
                object.insert("scope".to_string(), scope.to_json());
                value
            }
            EvidenceOutcome::NotComputed { reason } => json!({
                "status": "not-computed",
                "reason": reason,
            }),
        }
    }

    /// The inverse of [`Self::to_json`], and the enforcement point for
    /// G1's exit condition. A `"status":"complete"` (or `"partial"`)
    /// object that lacks a `"scope"`, or whose scope is missing a
    /// required field, is rejected here rather than deserialized with an
    /// empty scope silently substituted -- an empty scope would satisfy
    /// the shape check while carrying none of the honesty a real scope
    /// carries, which is precisely the gap this type exists to close.
    pub(crate) fn from_json(value: &Value) -> Result<Self, String> {
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .ok_or("evidence outcome missing \"status\"")?;
        match status {
            "complete" => {
                require_exact_keys(
                    value,
                    &["status", "scope"],
                    "a \"complete\" evidence outcome",
                )?;
                let scope = require_object(value, "scope")?;
                Ok(EvidenceOutcome::Complete(EvidenceScope::from_json(scope)?))
            }
            "partial" => {
                // The reason must be parsed *before* the key-shape check so
                // the check knows which reason-specific key
                // (`detail`/`paths`) is expected -- but parsing alone is not
                // the gate here: a `Partial`'s object is a strict superset
                // of a `Complete`'s (it has `scope` plus `partialReason` and
                // its detail), so relabeling `status` to `"complete"` while
                // leaving everything else in place must be caught by *this*
                // status's own exact-key check, not by falling through to
                // reuse the `"complete"` arm above.
                //
                // The expected key set is derived from `reason.to_json()`
                // itself -- not a hand-maintained list keyed on the
                // `PartialReason` variant -- so it can never drift out of
                // sync with what `PartialReason::to_json` actually emits.
                // A hand-maintained parallel table (e.g. matching on the
                // variant to decide "detail" vs "paths") would still be
                // exhaustive over the enum and compile fine even if a
                // future edit to one variant's `to_json` went unmirrored
                // there -- exhaustiveness catches a missing *variant*, not
                // a drifted *shape*.
                let reason = PartialReason::from_json(value)?;
                let reason_json = reason.to_json();
                let reason_keys = reason_json
                    .as_object()
                    .expect("PartialReason::to_json() is an object");
                let mut expected_keys: Vec<&str> = vec!["status", "scope"];
                expected_keys.extend(reason_keys.keys().map(String::as_str));
                require_exact_keys(value, &expected_keys, "a \"partial\" evidence outcome")?;
                let scope = require_object(value, "scope")?;
                Ok(EvidenceOutcome::Partial {
                    reason,
                    scope: EvidenceScope::from_json(scope)?,
                })
            }
            "not-computed" => {
                require_exact_keys(
                    value,
                    &["status", "reason"],
                    "a \"not-computed\" evidence outcome",
                )?;
                let reason = value
                    .get("reason")
                    .and_then(Value::as_str)
                    .ok_or("\"not-computed\" requires a \"reason\"")?;
                Ok(EvidenceOutcome::NotComputed {
                    reason: reason.to_string(),
                })
            }
            other => Err(format!("unknown evidence outcome status: {other}")),
        }
    }
}

impl EvidenceScope {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "artifactTypes": self.artifact_types,
            "scannedPaths": self.scanned_paths,
            "excludedPrefixes": self.excluded_prefixes,
        })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        require_exact_keys(
            value,
            &["artifactTypes", "scannedPaths", "excludedPrefixes"],
            "an evidence scope",
        )?;
        Ok(EvidenceScope {
            artifact_types: required_string_array(value, "artifactTypes")?,
            scanned_paths: required_string_array(value, "scannedPaths")?,
            excluded_prefixes: required_string_array(value, "excludedPrefixes")?,
        })
    }
}

impl PartialReason {
    fn to_json(&self) -> Value {
        match self {
            PartialReason::Truncated(detail) => json!({
                "partialReason": "truncated",
                "detail": detail,
            }),
            PartialReason::CapHit(detail) => json!({
                "partialReason": "cap_hit",
                "detail": detail,
            }),
            PartialReason::UnreadableInput(paths) => json!({
                "partialReason": "unreadable_input",
                "paths": paths,
            }),
        }
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let reason = value
            .get("partialReason")
            .and_then(Value::as_str)
            .ok_or("\"partial\" requires a \"partialReason\"")?;
        match reason {
            "truncated" => Ok(PartialReason::Truncated(required_string(value, "detail")?)),
            "cap_hit" => Ok(PartialReason::CapHit(required_string(value, "detail")?)),
            "unreadable_input" => Ok(PartialReason::UnreadableInput(required_string_array(
                value, "paths",
            )?)),
            other => Err(format!("unknown partial reason: {other}")),
        }
    }

    fn describe(&self) -> String {
        match self {
            PartialReason::Truncated(detail) => format!("truncated: {detail}"),
            PartialReason::CapHit(detail) => format!("cap hit: {detail}"),
            PartialReason::UnreadableInput(paths) => {
                format!("unreadable input: {}", paths.join(", "))
            }
        }
    }
}

/// Rejects any object whose key set is not *exactly* `keys` -- both a
/// missing required key and an unexpected extra one are errors. The extra-
/// key half is what catches a status-only forge: relabeling a real
/// `Partial`'s `status` to `"complete"` leaves its `partialReason` (and
/// `detail`/`paths`) sitting in the object, which this rejects even though
/// every key `"complete"` itself requires is present and well-formed.
fn require_exact_keys(value: &Value, keys: &[&str], context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))?;
    let actual: std::collections::BTreeSet<&str> = object.keys().map(String::as_str).collect();
    let expected: std::collections::BTreeSet<&str> = keys.iter().copied().collect();
    if actual == expected {
        return Ok(());
    }
    let unexpected: Vec<&str> = actual.difference(&expected).copied().collect();
    let missing: Vec<&str> = expected.difference(&actual).copied().collect();
    Err(format!(
        "{context} has the wrong fields (unexpected: {unexpected:?}, missing: {missing:?})"
    ))
}

fn require_object<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    let found = value
        .get(key)
        .ok_or_else(|| format!("\"{key}\" is required"))?;
    if found.is_object() {
        Ok(found)
    } else {
        Err(format!("\"{key}\" must be an object"))
    }
}

fn required_string(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("\"{key}\" must be a string"))
}

fn required_string_array(value: &Value, key: &str) -> Result<Vec<String>, String> {
    value
        .get(key)
        .ok_or_else(|| format!("scope missing required field \"{key}\""))?
        .as_array()
        .ok_or_else(|| format!("scope field \"{key}\" must be an array"))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("scope field \"{key}\" must contain only strings"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(paths: &[&str]) -> EvidenceScope {
        EvidenceScope {
            artifact_types: vec!["tracked-text-file".to_string()],
            scanned_paths: paths.iter().map(|p| p.to_string()).collect(),
            excluded_prefixes: vec![],
        }
    }

    #[test]
    fn complete_round_trips_through_json() {
        let outcome = EvidenceOutcome::Complete(scope(&["a.rs", "b.rs"]));
        let round_tripped = EvidenceOutcome::from_json(&outcome.to_json()).unwrap();
        assert_eq!(outcome, round_tripped);
    }

    #[test]
    fn partial_round_trips_through_json() {
        let outcome = EvidenceOutcome::Partial {
            reason: PartialReason::UnreadableInput(vec!["big.json".to_string()]),
            scope: scope(&["a.rs"]),
        };
        let round_tripped = EvidenceOutcome::from_json(&outcome.to_json()).unwrap();
        assert_eq!(outcome, round_tripped);
    }

    #[test]
    fn not_computed_round_trips_through_json() {
        let outcome = EvidenceOutcome::NotComputed {
            reason: "scan surface empty".to_string(),
        };
        let round_tripped = EvidenceOutcome::from_json(&outcome.to_json()).unwrap();
        assert_eq!(outcome, round_tripped);
    }

    /// The core G1 assertion at the unit level: a `"complete"` claim with
    /// no `scope` key at all must fail to parse, not silently become a
    /// `Complete` with an empty scope.
    #[test]
    fn complete_without_scope_is_rejected() {
        let forged = json!({ "status": "complete" });
        assert!(EvidenceOutcome::from_json(&forged).is_err());
    }

    /// A `scope` object that is present but missing a required field (the
    /// shape a careless partial refactor of a writer might produce) must
    /// also be rejected, not defaulted.
    #[test]
    fn complete_with_incomplete_scope_is_rejected() {
        let forged = json!({
            "status": "complete",
            "scope": { "artifactTypes": ["tracked-text-file"] },
        });
        assert!(EvidenceOutcome::from_json(&forged).is_err());
    }

    #[test]
    fn partial_without_reason_is_rejected() {
        let forged = json!({
            "status": "partial",
            "scope": {
                "artifactTypes": [],
                "scannedPaths": [],
                "excludedPrefixes": [],
            },
        });
        assert!(EvidenceOutcome::from_json(&forged).is_err());
    }

    #[test]
    fn not_computed_without_reason_is_rejected() {
        let forged = json!({ "status": "not-computed" });
        assert!(EvidenceOutcome::from_json(&forged).is_err());
    }

    /// The forgery that actually matters: a real `Partial` already has a
    /// genuine, fully-populated `scope` -- so touching only `status` is not
    /// caught by "is `scope` present and complete?" at all. It must be
    /// caught by the leftover `partialReason`/`paths` fields a real
    /// `Complete` never has.
    #[test]
    fn partial_relabeled_complete_by_status_only_is_rejected() {
        let partial = EvidenceOutcome::Partial {
            reason: PartialReason::UnreadableInput(vec!["big.json".to_string()]),
            scope: scope(&["a.rs"]),
        };
        let mut forged = partial.to_json();
        assert!(
            EvidenceOutcome::from_json(&forged).is_ok(),
            "sanity: the real value must itself parse"
        );
        forged["status"] = json!("complete");
        assert!(
            EvidenceOutcome::from_json(&forged).is_err(),
            "a complete claim forged from a partial by touching only status, \
             leaving its real scope and partialReason/paths intact, must be rejected: {forged}"
        );
    }

    /// The inverse direction, for completeness: a real `Complete`'s object
    /// is missing the keys `"partial"` requires, so relabeling the other
    /// way must fail too.
    #[test]
    fn complete_relabeled_partial_by_status_only_is_rejected() {
        let complete = EvidenceOutcome::Complete(scope(&["a.rs"]));
        let mut forged = complete.to_json();
        forged["status"] = json!("partial");
        assert!(EvidenceOutcome::from_json(&forged).is_err());
    }
}
