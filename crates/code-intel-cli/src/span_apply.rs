//! `edit.span-apply` — the span-addressed write primitive (#96 item 1, and
//! the write half of charter gate G4 in #139).
//!
//! The waste this exists to remove: to change one digit inside one
//! identifier, an old/new-string editor makes the model regenerate the whole
//! line — often with extra context lines dragged in just to make the old
//! string unique. Deciding *what* to change is cheap; *expressing* it is
//! expensive. The index already holds verified spans and byte digests, so the
//! agent can issue an instruction (`this span becomes those bytes`) instead of
//! producing bytes.
//!
//! Two independent guards stand between an instruction and a wrong-place
//! edit, and each fails loudly rather than degrading:
//!
//! 1. The snapshot lease (`snapshot::begin_consumption`) refuses when the
//!    tree is not the tree the request describes.
//! 2. The per-span digest refuses when the bytes *at those coordinates* are
//!    not the bytes the caller hashed. This is the guard that matters when an
//!    index has drifted: coordinates rot silently, digests do not.
//!
//! A refusal is not an error string. It publishes the same artifact a success
//! publishes, with `applied:false`, the expected and found digest for every
//! span, and a bounded literal of what was actually there — and the envelope's
//! `observedEffects` omits `repo_mutation`, which is the machine-checkable
//! proof that nothing was written.
//!
//! Deliberate limitations, so nobody mistakes this for more than it is:
//!
//! - One file per call. A file is replaced atomically (write sibling, then
//!   rename), so a call either lands whole or not at all. A plan that spans
//!   several files is `edit.ast-grep-apply`'s job, which verifies every file
//!   before writing any of them.
//! - Every span is resolved against the same pre-edit bytes and applied
//!   back-to-front, because sequential single-span calls on one line are
//!   broken by construction: the first replacement shifts every later column.
//! - A span digest proves *these bytes*, not *this meaning*. If a refactor
//!   moved an identical identifier to the same coordinates, the digest still
//!   matches. Narrow spans keep that window small; the snapshot lease closes
//!   the rest for callers who bind to a run.
//!
//! The address arithmetic, the path policy and the write itself live in
//! `span_patch`, shared with the plan and plan-apply stages: see that module
//! for why agreeing there is not optional.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::span_patch::{self, LineIndex, SpanAddress};
use super::{publish_named, snapshot_adapter_error, AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;
use crate::capability::sha256_hex;
use crate::snapshot;

const MAX_SPANS: usize = 256;
const MAX_REPLACEMENT_BYTES: usize = 64 * 1024;

struct SpanEdit {
    address: SpanAddress,
    expected_sha256: String,
    replacement: String,
}

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "edit.span-apply does not accept input artifacts".into(),
        ));
    }
    // The envelope compares declared against observed effects *after* an
    // adapter returns, which would catch an undeclared mutation only once the
    // bytes were already on disk. Checking the request's own effect policy
    // here is what turns it into a gate: a caller who did not ask for
    // `repo_mutation` gets a refusal instead of an audit finding.
    if !request["effectPolicy"]["allowedEffects"]
        .as_array()
        .is_some_and(|effects| effects.iter().any(|effect| effect == "repo_mutation"))
    {
        return Err(AdapterError::Contract(
            "edit.span-apply writes repository bytes and requires the repo_mutation effect; request effectPolicy does not allow it".into(),
        ));
    }
    let options = request["options"]
        .as_object()
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options
        .keys()
        .any(|key| !matches!(key.as_str(), "repoPath" | "path" | "spans"))
    {
        return Err(AdapterError::InvalidOptions(
            "edit.span-apply accepts only repoPath/path/spans".into(),
        ));
    }
    let repo = options
        .get("repoPath")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(Path::new)
        .ok_or_else(|| AdapterError::InvalidOptions("options.repoPath must be non-empty".into()))?;
    if !repo.is_dir() {
        return Err(AdapterError::InvalidOptions(format!(
            "repoPath is not a directory: {}",
            repo.display()
        )));
    }
    let relative = options
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| AdapterError::InvalidOptions("options.path must be a string".into()))?;
    let relative = span_patch::normalize_relative(relative, true)?;
    let spans = parse_spans(options.get("spans"))?;
    let target = span_patch::resolve_target(repo, &relative, &request["snapshot"])?;

    let lease =
        snapshot::begin_consumption(repo, &request["snapshot"]).map_err(snapshot_adapter_error)?;
    let before =
        fs::read(&target).map_err(|error| AdapterError::Io(format!("read {relative}: {error}")))?;
    let index = LineIndex::build(&before);

    let mut rows = Vec::new();
    let mut resolved = Vec::new();
    let mut refusal: Option<String> = None;
    for span in &spans {
        match index.resolve(&span.address) {
            Err(reason) => {
                refusal.get_or_insert(format!("span {}: {reason}", span.address.label()));
                rows.push(json!({
                    "span": span.address.label(),
                    "expectedSha256": span.expected_sha256,
                    "resolved": false,
                    "verdict": "out_of_bounds",
                    "diagnostic": reason,
                }));
            }
            Ok((start, end)) => {
                let found = &before[start..end];
                let found_sha256 = sha256_hex(found);
                let matched = found_sha256 == span.expected_sha256;
                if !matched {
                    refusal.get_or_insert(format!(
                        "span {}: expected sha256 {} but found {} ({} byte(s){})",
                        span.address.label(),
                        span.expected_sha256,
                        found_sha256,
                        found.len(),
                        span_patch::quoted_evidence(found),
                    ));
                }
                rows.push(json!({
                    "span": span.address.label(),
                    "byteRange": {"start": start, "end": end},
                    "expectedSha256": span.expected_sha256,
                    "foundSha256": found_sha256,
                    "foundBytes": found.len(),
                    "foundText": span_patch::evidence_text(found),
                    "replacementSha256": sha256_hex(span.replacement.as_bytes()),
                    "replacementBytes": span.replacement.len(),
                    "resolved": true,
                    "verdict": if matched { "match" } else { "digest_mismatch" },
                    "diagnostic": Value::Null,
                }));
                resolved.push((start, end, span.replacement.as_str()));
            }
        }
    }

    let mut artifact = json!({
        "schema": "code-intel-span-edit-result.v1",
        "capability": "edit.span-apply",
        "snapshotIdentity": request["snapshot"]["identity"],
        "path": relative,
        "applied": false,
        "refusal": refusal,
        "file": {
            "bytesBefore": before.len(),
            "sha256Before": sha256_hex(&before),
            "bytesAfter": Value::Null,
            "sha256After": Value::Null,
        },
        "spans": rows,
    });

    if let Some(diagnostic) = refusal {
        // Nothing was written, so the tree must still be the tree the lease
        // bound. Verifying that here means a refusal can never be mistaken
        // for "we looked at a different file than the one we reported".
        lease.verify_after(repo).map_err(snapshot_adapter_error)?;
        return publish(out, artifact, false, Some(diagnostic));
    }

    let after = span_patch::rewrite(&before, &resolved);
    // The mutation lands before the artifact is published: an artifact that
    // claims `applied:true` before the rename could outlive a failed write,
    // and a false success is worse than a publication failure that names the
    // mutation it is reporting on.
    span_patch::atomic_replace(&target, &after).map_err(AdapterError::Io)?;
    let after_sha256 = sha256_hex(&after);
    artifact["applied"] = json!(true);
    artifact["file"]["bytesAfter"] = json!(after.len());
    artifact["file"]["sha256After"] = json!(after_sha256);
    // If publication fails now, the repository has already changed. Say so in
    // the diagnostic and name the resulting digest: a caller who only saw
    // "publish failed" would have no way to tell an aborted call from a
    // completed edit whose receipt went missing.
    publish(out, artifact, true, None).map_err(|error| match error {
        AdapterError::Io(message) => AdapterError::Io(format!(
            "{message}; {relative} was already rewritten and is now sha256 {after_sha256}"
        )),
        other => other,
    })
}

fn publish(
    out: &Path,
    artifact: Value,
    mutated: bool,
    diagnostic: Option<String>,
) -> Result<AdapterOutput, AdapterError> {
    let bytes = serde_json::to_vec(&artifact)
        .map_err(|error| AdapterError::Internal(format!("serialize span edit result: {error}")))?;
    publish_named(out, "span-apply-result.json", &bytes, |_| Ok(()))?;
    let mut observed_effects = vec!["repo_read".to_string(), "local_write".to_string()];
    if mutated {
        observed_effects.push("repo_mutation".to_string());
    }
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-span-edit-result.v1".into(),
            artifact_type: "edit.span-apply-result".into(),
            relative_path: "span-apply-result.json".into(),
            bytes,
        }],
        observed_effects,
        domain_verdict: if diagnostic.is_some() {
            AdapterDomainVerdict::Fail
        } else {
            AdapterDomainVerdict::Pass
        },
        domain_failure: diagnostic,
    })
}

fn parse_spans(value: Option<&Value>) -> Result<Vec<SpanEdit>, AdapterError> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| AdapterError::InvalidOptions("options.spans must be an array".into()))?;
    if items.is_empty() || items.len() > MAX_SPANS {
        return Err(AdapterError::InvalidOptions(format!(
            "options.spans must contain 1..={MAX_SPANS} entries"
        )));
    }
    let mut spans = items
        .iter()
        .map(parse_span)
        .collect::<Result<Vec<_>, _>>()?;
    let mut replacement_bytes = 0usize;
    for span in &spans {
        replacement_bytes += span.replacement.len();
    }
    if replacement_bytes > MAX_REPLACEMENT_BYTES {
        return Err(AdapterError::InvalidOptions(format!(
            "options.spans replacement text exceeds {MAX_REPLACEMENT_BYTES} bytes"
        )));
    }
    // Sorting here rather than demanding sorted input keeps the contract
    // caller-friendly; the disjointness check below is what actually matters,
    // because two spans that overlap have no single well-defined result.
    spans.sort_by_key(|span| span.address.start());
    for pair in spans.windows(2) {
        if pair[1].address.start() < pair[0].address.end() {
            return Err(AdapterError::InvalidOptions(format!(
                "options.spans overlap: {} and {}",
                pair[0].address.label(),
                pair[1].address.label()
            )));
        }
    }
    Ok(spans)
}

fn parse_span(value: &Value) -> Result<SpanEdit, AdapterError> {
    let object = value.as_object().ok_or_else(|| {
        AdapterError::InvalidOptions("options.spans[] entries must be objects".into())
    })?;
    if object.len() != 6
        || ![
            "startLine",
            "startColumn",
            "endLine",
            "endColumn",
            "expectedSha256",
            "replacement",
        ]
        .iter()
        .all(|key| object.contains_key(*key))
    {
        return Err(AdapterError::InvalidOptions(
            "options.spans[] requires exactly startLine/startColumn/endLine/endColumn/expectedSha256/replacement".into(),
        ));
    }
    let position = |key: &str| -> Result<usize, AdapterError> {
        object[key]
            .as_u64()
            .filter(|value| *value >= 1 && *value <= u32::MAX as u64)
            .map(|value| value as usize)
            .ok_or_else(|| {
                AdapterError::InvalidOptions(format!(
                    "options.spans[].{key} must be a 1-based integer"
                ))
            })
    };
    let expected_sha256 = object["expectedSha256"]
        .as_str()
        .filter(|value| crate::capability::is_digest(value))
        .ok_or_else(|| {
            AdapterError::InvalidOptions(
                "options.spans[].expectedSha256 must be a 64-character lowercase sha256".into(),
            )
        })?
        .to_string();
    let replacement = object["replacement"]
        .as_str()
        .ok_or_else(|| {
            AdapterError::InvalidOptions("options.spans[].replacement must be a string".into())
        })?
        .to_string();
    let address = SpanAddress {
        start_line: position("startLine")?,
        start_column: position("startColumn")?,
        end_line: position("endLine")?,
        end_column: position("endColumn")?,
    };
    if address.start() >= address.end() {
        return Err(AdapterError::InvalidOptions(format!(
            "options.spans[] end must be after start: {}",
            address.label()
        )));
    }
    Ok(SpanEdit {
        address,
        expected_sha256,
        replacement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_digest_binds_this_adapter() {
        let registry: Value =
            serde_json::from_slice(include_bytes!("../../../orchestration/integrations.json"))
                .unwrap();
        let integration = registry["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "edit.span-apply")
            .unwrap();
        assert_eq!(
            integration["capabilityDeclaration"]["implementation"]["toolchainDigests"],
            json!([
                sha256_hex(include_bytes!("span_apply.rs")),
                sha256_hex(include_bytes!("span_patch.rs"))
            ])
        );
        assert_eq!(
            integration["capabilityDeclaration"]["allowedEffects"],
            json!(["repo_read", "local_write", "repo_mutation"])
        );
    }

    #[test]
    fn overlapping_spans_are_refused_because_the_result_is_undefined() {
        let entry = |start: u64, end: u64| {
            json!({
                "startLine":1,"startColumn":start,"endLine":1,"endColumn":end,
                "expectedSha256":"0".repeat(64),"replacement":"x"
            })
        };
        assert!(parse_spans(Some(&json!([entry(1, 5), entry(4, 8)]))).is_err());
        // Touching but disjoint spans are legitimate.
        assert!(parse_spans(Some(&json!([entry(4, 8), entry(1, 4)]))).is_ok());
    }
}
