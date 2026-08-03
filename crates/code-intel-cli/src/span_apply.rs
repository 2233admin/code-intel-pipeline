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
//!   rename), so a call either lands whole or not at all. Multi-file edits are
//!   the caller's sequencing problem and are *not* atomic across files.
//! - Every span is resolved against the same pre-edit bytes and applied
//!   back-to-front, because sequential single-span calls on one line are
//!   broken by construction: the first replacement shifts every later column.
//! - A span digest proves *these bytes*, not *this meaning*. If a refactor
//!   moved an identical identifier to the same coordinates, the digest still
//!   matches. Narrow spans keep that window small; the snapshot lease closes
//!   the rest for callers who bind to a run.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::structured_edit::{normalize_relative, within_scope};
use super::{publish_named, snapshot_adapter_error, AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;
use crate::capability::sha256_hex;
use crate::snapshot;

const MAX_SPANS: usize = 256;
const MAX_REPLACEMENT_BYTES: usize = 64 * 1024;
/// Refusal evidence quotes what was actually found. Bounded so a mis-addressed
/// span covering half a file cannot turn one refusal into a megabyte artifact.
const MAX_EVIDENCE_BYTES: usize = 256;

struct SpanEdit {
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    expected_sha256: String,
    replacement: String,
}

impl SpanEdit {
    fn start(&self) -> (usize, usize) {
        (self.start_line, self.start_column)
    }

    fn end(&self) -> (usize, usize) {
        (self.end_line, self.end_column)
    }

    fn address(&self) -> String {
        format!(
            "{}:{}-{}:{}",
            self.start_line, self.start_column, self.end_line, self.end_column
        )
    }
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
    let relative = normalize_relative(relative, true)?;
    let spans = parse_spans(options.get("spans"))?;
    let target = resolve_target(repo, &relative, &request["snapshot"])?;

    let lease =
        snapshot::begin_consumption(repo, &request["snapshot"]).map_err(snapshot_adapter_error)?;
    let before =
        fs::read(&target).map_err(|error| AdapterError::Io(format!("read {relative}: {error}")))?;
    let bounds = line_bounds(&before);

    let mut rows = Vec::new();
    let mut resolved = Vec::new();
    let mut refusal: Option<String> = None;
    for span in &spans {
        match resolve_span(&bounds, span) {
            Err(reason) => {
                refusal.get_or_insert(format!("span {}: {reason}", span.address()));
                rows.push(json!({
                    "span": span.address(),
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
                        span.address(),
                        span.expected_sha256,
                        found_sha256,
                        found.len(),
                        quoted_evidence(found),
                    ));
                }
                rows.push(json!({
                    "span": span.address(),
                    "byteRange": {"start": start, "end": end},
                    "expectedSha256": span.expected_sha256,
                    "foundSha256": found_sha256,
                    "foundBytes": found.len(),
                    "foundText": evidence_text(found),
                    "replacementSha256": sha256_hex(span.replacement.as_bytes()),
                    "replacementBytes": span.replacement.len(),
                    "resolved": true,
                    "verdict": if matched { "match" } else { "digest_mismatch" },
                    "diagnostic": Value::Null,
                }));
                resolved.push((start, end));
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

    let after = rewrite(&before, &spans, &resolved);
    // The mutation lands before the artifact is published: an artifact that
    // claims `applied:true` before the rename could outlive a failed write,
    // and a false success is worse than a publication failure that names the
    // mutation it is reporting on.
    atomic_replace(&target, &after).map_err(AdapterError::Io)?;
    artifact["applied"] = json!(true);
    artifact["file"]["bytesAfter"] = json!(after.len());
    artifact["file"]["sha256After"] = json!(sha256_hex(&after));
    publish(out, artifact, true, None)
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

/// Resolve the edit target and refuse every path shape that could escape the
/// repository or the requested snapshot scope. Symlinks are refused outright:
/// writing through one is a write to wherever it points, which no scope check
/// upstream of the rename can constrain.
fn resolve_target(repo: &Path, relative: &str, snapshot: &Value) -> Result<PathBuf, AdapterError> {
    let scopes = snapshot["scope"]
        .as_array()
        .ok_or_else(|| AdapterError::Contract("snapshot scope must be an array".into()))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| {
                    AdapterError::Contract("snapshot scope item must be a string".into())
                })
                .and_then(|value| normalize_relative(value, false))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !scopes.iter().any(|scope| within_scope(relative, scope)) {
        return Err(AdapterError::Contract(format!(
            "edit path is outside the requested snapshot scope: {relative}"
        )));
    }
    let canonical_repo = fs::canonicalize(repo)
        .map_err(|error| AdapterError::Io(format!("resolve repoPath: {error}")))?;
    let target = canonical_repo.join(Path::new(relative));
    let metadata = fs::symlink_metadata(&target)
        .map_err(|error| AdapterError::InvalidOptions(format!("inspect {relative}: {error}")))?;
    if metadata.file_type().is_symlink() {
        return Err(AdapterError::Contract(format!(
            "edit path is a symbolic link and is refused: {relative}"
        )));
    }
    if !metadata.is_file() {
        return Err(AdapterError::InvalidOptions(format!(
            "edit path is not a regular file: {relative}"
        )));
    }
    let canonical_target = fs::canonicalize(&target)
        .map_err(|error| AdapterError::Io(format!("resolve {relative}: {error}")))?;
    if !canonical_target.starts_with(&canonical_repo) {
        return Err(AdapterError::Contract(format!(
            "edit path escapes repository: {relative}"
        )));
    }
    Ok(canonical_target)
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
    spans.sort_by_key(SpanEdit::start);
    for pair in spans.windows(2) {
        if pair[1].start() < pair[0].end() {
            return Err(AdapterError::InvalidOptions(format!(
                "options.spans overlap: {} and {}",
                pair[0].address(),
                pair[1].address()
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
    let span = SpanEdit {
        start_line: position("startLine")?,
        start_column: position("startColumn")?,
        end_line: position("endLine")?,
        end_column: position("endColumn")?,
        expected_sha256,
        replacement,
    };
    if span.start() >= span.end() {
        return Err(AdapterError::InvalidOptions(format!(
            "options.spans[] end must be after start: {}",
            span.address()
        )));
    }
    Ok(span)
}

/// Byte range of every line's *content*, terminator excluded.
///
/// Lines are 1-based and columns are 1-based byte offsets into that content,
/// with the end column exclusive — so `12:21-12:33` addresses twelve bytes on
/// line 12 and can never swallow the newline that ends it. A file that ends
/// with a terminator has exactly as many lines as it has terminators; the
/// empty remainder after the final `\n` is not an addressable line.
fn line_bounds(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut bounds = Vec::new();
    let mut start = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            let mut end = index;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            bounds.push((start, end));
            start = index + 1;
        }
    }
    if start < bytes.len() {
        bounds.push((start, bytes.len()));
    }
    bounds
}

fn resolve_span(bounds: &[(usize, usize)], span: &SpanEdit) -> Result<(usize, usize), String> {
    let offset = |line: usize, column: usize| -> Result<usize, String> {
        let (start, end) = *bounds.get(line - 1).ok_or_else(|| {
            format!(
                "line {line} is beyond the file, which has {} line(s)",
                bounds.len()
            )
        })?;
        let offset = start + column - 1;
        if offset > end {
            return Err(format!(
                "column {column} is beyond line {line}, which holds {} byte(s)",
                end - start
            ));
        }
        Ok(offset)
    };
    Ok((
        offset(span.start_line, span.start_column)?,
        offset(span.end_line, span.end_column)?,
    ))
}

/// Substitute back-to-front so every span keeps the offsets it was resolved
/// at against the pre-edit bytes.
fn rewrite(before: &[u8], spans: &[SpanEdit], resolved: &[(usize, usize)]) -> Vec<u8> {
    let mut after = before.to_vec();
    for (span, (start, end)) in spans.iter().zip(resolved).rev() {
        after.splice(*start..*end, span.replacement.bytes());
    }
    after
}

/// Write a sibling temporary and rename over the target, so an interrupted
/// write can never leave the file truncated: either the rename lands the whole
/// new content or the original is untouched.
fn atomic_replace(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut name = target
        .file_name()
        .ok_or_else(|| format!("{} has no file name", target.display()))?
        .to_os_string();
    name.push(".span-apply-tmp");
    let temporary = target.with_file_name(name);
    if let Err(error) = fs::write(&temporary, bytes) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("write {}: {error}", temporary.display()));
    }
    if let Ok(metadata) = fs::metadata(target) {
        let _ = fs::set_permissions(&temporary, metadata.permissions());
    }
    if let Err(error) = fs::rename(&temporary, target) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("replace {}: {error}", target.display()));
    }
    Ok(())
}

fn evidence_text(found: &[u8]) -> Value {
    if found.len() > MAX_EVIDENCE_BYTES {
        return Value::Null;
    }
    match std::str::from_utf8(found) {
        Ok(text) => json!(text),
        Err(_) => Value::Null,
    }
}

fn quoted_evidence(found: &[u8]) -> String {
    match evidence_text(found) {
        Value::String(text) => format!(": {text:?}"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start_column: usize, end_column: usize, expected: &str) -> SpanEdit {
        SpanEdit {
            start_line: 1,
            start_column,
            end_line: 1,
            end_column,
            expected_sha256: expected.to_string(),
            replacement: String::new(),
        }
    }

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
            integration["capabilityDeclaration"]["implementation"]["toolchainDigests"][0],
            sha256_hex(include_bytes!("span_apply.rs"))
        );
        assert_eq!(
            integration["capabilityDeclaration"]["allowedEffects"],
            json!(["repo_read", "local_write", "repo_mutation"])
        );
    }

    #[test]
    fn columns_address_line_content_and_never_the_terminator() {
        for text in ["ab\ncd\n", "ab\r\ncd\r\n", "ab\ncd"] {
            let bounds = line_bounds(text.as_bytes());
            assert_eq!(bounds.len(), 2, "{text:?}");
            // Column 3 is the exclusive end of a two-byte line; column 4 is
            // past it and must not reach into the terminator.
            assert!(resolve_span(&bounds, &span(1, 3, "")).is_ok());
            assert!(resolve_span(&bounds, &span(1, 4, "")).is_err());
        }
    }

    #[test]
    fn a_span_beyond_the_file_is_reported_with_the_actual_line_count() {
        let bounds = line_bounds(b"one\n");
        let mut beyond = span(1, 2, "");
        beyond.start_line = 9;
        beyond.end_line = 9;
        let error = resolve_span(&bounds, &beyond).unwrap_err();
        assert!(error.contains("has 1 line(s)"), "{error}");
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

    #[test]
    fn back_to_front_rewriting_keeps_every_span_at_its_resolved_offsets() {
        let before = b"aa bb cc".to_vec();
        let spans = vec![
            SpanEdit {
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 3,
                expected_sha256: String::new(),
                replacement: "AAAAA".into(),
            },
            SpanEdit {
                start_line: 1,
                start_column: 7,
                end_line: 1,
                end_column: 9,
                expected_sha256: String::new(),
                replacement: "C".into(),
            },
        ];
        assert_eq!(rewrite(&before, &spans, &[(0, 2), (6, 8)]), b"AAAAA bb C");
    }
}
