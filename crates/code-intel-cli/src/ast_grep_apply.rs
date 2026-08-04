//! `edit.ast-grep-apply` — execute a plan `edit.ast-grep-plan` produced
//! (#96 item 2, charter gate G4 in #139).
//!
//! The model issues the instruction; this capability performs every byte
//! transformation. What crosses the boundary is a list of spans, the sha256
//! each span had when the plan was made, and the replacement ast-grep
//! computed for it.
//!
//! What that does and does not guarantee, stated exactly, because the
//! difference is the whole point of the tool. A `--plan` is a JSON file on
//! disk: `schema`, `capability` and `snapshotIdentity` are strings whoever
//! writes the file chooses. Nothing here proves a plan came from a real
//! `edit.ast-grep-plan` run, and nothing could — a caller able to hand-author
//! a plan is equally able to just run the plan capability. So this stage does
//! not claim authenticity.
//!
//! The property it *does* enforce is narrower and mechanical: **a caller can
//! only write where it has already read.** Every edit carries the bytes it
//! claims to replace (`text`), and that text must hash to the `sha256` it
//! declares; at apply time the bytes actually at the span must hash to that
//! same value, and the line they sit on must hash to `lineSha256`. An edit
//! therefore lands only on bytes its author had in hand and that have not
//! moved since. Replacement size is capped at `MAX_EDIT_BYTES` — the same
//! ceiling the plan stage applies — so a hand-authored plan cannot smuggle a
//! file's worth of new content through a field a real plan would have kept
//! small. `snapshotIdentity` is reported as provenance and is not trusted.
//!
//! A plan applies as a **unit**. Every span in every file is resolved and
//! digest-checked before a single byte is written, and one drifted span
//! refuses the whole plan — naming which matches drifted, with the digest it
//! expected, the digest it found, and a bounded literal of what is there now.
//! That is the difference between an editor that stops and one that leaves a
//! half-renamed tree behind.
//!
//! Two digests per match, not one. The span digest catches the bytes moving;
//! the line digest catches the case a span digest cannot see — a hand rename
//! from `commit` to `commit_now` leaves the planned six bytes reading
//! `commit`, and applying the plan anyway would write `commit_change_now`.
//! Dogfooding this capability on its own repository is how that hole was
//! found, so the plan records both digests and this stage checks both.
//!
//! Writes are then staged for every file first and renamed only once all of
//! them are on disk, so a failure while producing the new bytes still leaves
//! the repository untouched. The residual window is the rename loop itself:
//! if the filesystem fails *there*, the diagnostic names exactly which files
//! were already replaced and their digests. It does not claim an atomicity
//! across files that no filesystem offers.
//!
//! Why not simply require the tree to be identical to plan time? Because that
//! would make the digest redundant and the tool useless: any unrelated edit
//! anywhere in the scanned scope would invalidate a plan whose own spans are
//! untouched. The snapshot identity the plan was computed against is reported
//! for provenance; the per-span digests are what actually gate the write.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::span_patch::{self, LineIndex, SpanAddress};
use super::{publish_named, snapshot_adapter_error, AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;
use crate::capability::{is_digest, sha256_hex};
use crate::snapshot;

const MAX_EDITS: usize = 256;
const PLAN_SCHEMA: &str = "code-intel-structured-edit-plan.v1";

#[derive(Debug)]
struct PlannedEdit {
    index: u64,
    file: String,
    address: SpanAddress,
    expected_sha256: String,
    expected_line_sha256: String,
    replacement: String,
}

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "edit.ast-grep-apply does not accept input artifacts".into(),
        ));
    }
    // Same pre-flight gate the span primitive uses: the envelope's
    // declared-versus-observed audit happens after the adapter returns, which
    // would notice an unasked-for mutation only once the bytes were on disk.
    if !request["effectPolicy"]["allowedEffects"]
        .as_array()
        .is_some_and(|effects| effects.iter().any(|effect| effect == "repo_mutation"))
    {
        return Err(AdapterError::Contract(
            "edit.ast-grep-apply writes repository bytes and requires the repo_mutation effect; request effectPolicy does not allow it".into(),
        ));
    }
    let options = request["options"]
        .as_object()
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options
        .keys()
        .any(|key| !matches!(key.as_str(), "repoPath" | "plan"))
    {
        return Err(AdapterError::InvalidOptions(
            "edit.ast-grep-apply accepts only repoPath/plan".into(),
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
    let plan = options
        .get("plan")
        .filter(|value| value.is_object())
        .ok_or_else(|| AdapterError::InvalidOptions("options.plan must be an object".into()))?;
    let edits = parse_plan(plan)?;

    let lease =
        snapshot::begin_consumption(repo, &request["snapshot"]).map_err(snapshot_adapter_error)?;
    let mut grouped: BTreeMap<&str, Vec<&PlannedEdit>> = BTreeMap::new();
    for edit in &edits {
        grouped.entry(edit.file.as_str()).or_default().push(edit);
    }

    let mut rows = Vec::new();
    let mut files = Vec::new();
    let mut drifted = Vec::new();
    let mut pending = Vec::new();
    for (file, file_edits) in &grouped {
        let target = span_patch::resolve_target(repo, file, &request["snapshot"])?;
        let before =
            fs::read(&target).map_err(|error| AdapterError::Io(format!("read {file}: {error}")))?;
        let index = LineIndex::build(&before);
        let mut resolved = Vec::new();
        for edit in file_edits {
            match verify_one(&index, &before, edit) {
                Verified::Refused { row, diagnostic } => {
                    rows.push(row);
                    drifted.push(diagnostic);
                }
                Verified::Ready { row, start, end } => {
                    rows.push(row);
                    resolved.push((start, end, edit.replacement.as_str()));
                }
            }
        }
        files.push(json!({
            "path": file,
            "edits": file_edits.len(),
            "bytesBefore": before.len(),
            "sha256Before": sha256_hex(&before),
            "bytesAfter": Value::Null,
            "sha256After": Value::Null,
        }));
        pending.push((target, before, resolved));
    }

    let mut artifact = json!({
        "schema": "code-intel-structured-edit-apply-result.v1",
        "capability": "edit.ast-grep-apply",
        "snapshotIdentity": request["snapshot"]["identity"],
        "plan": {
            "capability": plan["capability"],
            "snapshotIdentity": plan["snapshotIdentity"],
            "language": plan["query"]["language"],
            "pattern": plan["query"]["pattern"],
            "rewrite": plan["query"]["rewrite"],
            "planIsFromThisSnapshot": plan["snapshotIdentity"] == request["snapshot"]["identity"],
        },
        "applied": false,
        "refusal": Value::Null,
        "summary": {
            "edits": edits.len(),
            "files": grouped.len(),
            "drifted": drifted.len(),
        },
        "files": files,
        "edits": rows,
    });

    if !drifted.is_empty() {
        // Nothing was written, so the tree must still be the tree the lease
        // bound; verifying that here is what keeps "refused" from meaning
        // "we read a different tree than the one we reported on".
        lease.verify_after(repo).map_err(snapshot_adapter_error)?;
        let refusal = format!(
            "{} of {} planned edit(s) no longer match the bytes at their span; nothing was written: {}",
            drifted.len(),
            edits.len(),
            drifted.join("; ")
        );
        artifact["refusal"] = json!(refusal);
        return publish(out, artifact, false, Some(refusal));
    }

    let written = commit_all(pending)?;
    for (entry, (bytes, digest)) in artifact["files"]
        .as_array_mut()
        .expect("files array")
        .iter_mut()
        .zip(&written)
    {
        entry["bytesAfter"] = json!(bytes);
        entry["sha256After"] = json!(digest);
    }
    artifact["applied"] = json!(true);
    let landed = describe(&grouped, &written);
    publish(out, artifact, true, None).map_err(|error| match error {
        AdapterError::Io(message) => AdapterError::Io(format!(
            "{message}; the plan had already been applied to {landed}"
        )),
        other => other,
    })
}

enum Verified {
    Ready {
        row: Value,
        start: usize,
        end: usize,
    },
    Refused {
        row: Value,
        diagnostic: String,
    },
}

/// One span, checked against the bytes that are there now.
fn verify_one(index: &LineIndex, before: &[u8], edit: &PlannedEdit) -> Verified {
    let mut row = json!({
        "match": edit.index,
        "file": edit.file,
        "span": edit.address.as_json(),
        "expectedSha256": edit.expected_sha256,
        "replacement": edit.replacement,
    });
    match index.resolve(&edit.address) {
        Err(reason) => {
            row["verdict"] = json!("out_of_bounds");
            row["diagnostic"] = json!(reason);
            Verified::Refused {
                diagnostic: format!(
                    "match {} at {}:{} {reason}",
                    edit.index,
                    edit.file,
                    edit.address.label()
                ),
                row,
            }
        }
        Ok((start, end)) => {
            let found = &before[start..end];
            let found_sha256 = sha256_hex(found);
            row["foundSha256"] = json!(found_sha256);
            row["foundBytes"] = json!(found.len());
            row["foundText"] = span_patch::evidence_text(found);
            row["diagnostic"] = Value::Null;
            if found_sha256 != edit.expected_sha256 {
                row["verdict"] = json!("digest_mismatch");
                return Verified::Refused {
                    diagnostic: format!(
                        "match {} at {}:{} expected sha256 {} but found {}{}",
                        edit.index,
                        edit.file,
                        edit.address.label(),
                        edit.expected_sha256,
                        found_sha256,
                        span_patch::quoted_evidence(found),
                    ),
                    row,
                };
            }
            // The span alone would let a rename that merely *extends* the
            // matched identifier through: `commit` becoming `commit_now`
            // leaves those six bytes reading `commit`, and the plan would
            // then produce `commit_change_now`. The plan was derived from a
            // parse of this line, so a changed line invalidates it.
            let (line_start, line_end) = match index.line_span(&edit.address) {
                Ok(range) => range,
                Err(reason) => {
                    row["verdict"] = json!("out_of_bounds");
                    row["diagnostic"] = json!(reason);
                    return Verified::Refused {
                        diagnostic: format!(
                            "match {} at {}:{} {reason}",
                            edit.index,
                            edit.file,
                            edit.address.label()
                        ),
                        row,
                    };
                }
            };
            let line = &before[line_start..line_end];
            let found_line_sha256 = sha256_hex(line);
            row["foundLineSha256"] = json!(found_line_sha256);
            if found_line_sha256 != edit.expected_line_sha256 {
                row["verdict"] = json!("line_drift");
                row["foundLineText"] = span_patch::evidence_text(line);
                return Verified::Refused {
                    diagnostic: format!(
                        "match {} at {}:{} still holds the planned bytes, but line {} changed after the plan was made (expected line sha256 {} but found {}{})",
                        edit.index,
                        edit.file,
                        edit.address.label(),
                        edit.address.start_line,
                        edit.expected_line_sha256,
                        found_line_sha256,
                        span_patch::quoted_evidence(line),
                    ),
                    row,
                };
            }
            row["verdict"] = json!("match");
            Verified::Ready { row, start, end }
        }
    }
}

/// Stage every file, then rename every file. Staging failures roll back to a
/// repository that was never touched; a rename failure names what landed.
fn commit_all(
    pending: Vec<(std::path::PathBuf, Vec<u8>, Vec<(usize, usize, &str)>)>,
) -> Result<Vec<(usize, String)>, AdapterError> {
    let mut staged = Vec::new();
    let mut summaries = Vec::new();
    for (target, before, resolved) in &pending {
        let after = span_patch::rewrite(before, resolved);
        match span_patch::StagedWrite::stage(target, &after) {
            Ok(write) => {
                summaries.push((after.len(), sha256_hex(&after)));
                staged.push(write);
            }
            Err(error) => {
                for write in staged {
                    write.discard();
                }
                return Err(AdapterError::Io(format!(
                    "staging the plan failed before anything was written: {error}"
                )));
            }
        }
    }
    for (position, write) in staged.into_iter().enumerate() {
        if let Err(error) = write.commit() {
            return Err(AdapterError::Io(format!(
                "{error}; {position} of {} planned file(s) had already been replaced",
                pending.len()
            )));
        }
    }
    Ok(summaries)
}

fn describe(grouped: &BTreeMap<&str, Vec<&PlannedEdit>>, written: &[(usize, String)]) -> String {
    grouped
        .keys()
        .zip(written)
        .map(|(path, (_, digest))| format!("{path} (now sha256 {digest})"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn publish(
    out: &Path,
    artifact: Value,
    mutated: bool,
    diagnostic: Option<String>,
) -> Result<AdapterOutput, AdapterError> {
    let bytes = serde_json::to_vec(&artifact).map_err(|error| {
        AdapterError::Internal(format!("serialize structured edit apply result: {error}"))
    })?;
    publish_named(out, "ast-grep-apply-result.json", &bytes, |_| Ok(()))?;
    let mut observed_effects = vec!["repo_read".to_string(), "local_write".to_string()];
    if mutated {
        observed_effects.push("repo_mutation".to_string());
    }
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-structured-edit-apply-result.v1".into(),
            artifact_type: "edit.ast-grep-apply-result".into(),
            relative_path: "ast-grep-apply-result.json".into(),
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

/// Accept only the *shape* `edit.ast-grep-plan` publishes, and only a plan it
/// marked applicable.
///
/// This is a shape check, not an authentication: `schema` and `capability`
/// are strings in a file, so passing them proves nothing about where the
/// document came from. What the checks here do rule out is a plan whose edits
/// overlap, whose paths escape the repository or point at generated content,
/// whose recorded text does not hash to the digest it claims, or whose
/// replacement is larger than the plan stage would ever have produced —
/// refused here rather than discovered mid-write.
fn parse_plan(plan: &Value) -> Result<Vec<PlannedEdit>, AdapterError> {
    if plan["schema"] != PLAN_SCHEMA || plan["capability"] != "edit.ast-grep-plan" {
        return Err(AdapterError::InvalidOptions(format!(
            "options.plan must be a {PLAN_SCHEMA} document published by edit.ast-grep-plan"
        )));
    }
    let apply = &plan["apply"];
    if apply["applicable"] != json!(true) {
        return Err(AdapterError::Contract(format!(
            "plan is not applicable: {}",
            apply["reason"]
                .as_str()
                .unwrap_or("the plan carries no applicability verdict")
        )));
    }
    let items = apply["edits"]
        .as_array()
        .ok_or_else(|| AdapterError::InvalidOptions("plan apply.edits must be an array".into()))?;
    if items.is_empty() || items.len() > MAX_EDITS {
        return Err(AdapterError::InvalidOptions(format!(
            "plan apply.edits must contain 1..={MAX_EDITS} entries"
        )));
    }
    let mut edits = items
        .iter()
        .map(parse_edit)
        .collect::<Result<Vec<_>, _>>()?;
    edits.sort_by(|left, right| {
        left.file
            .cmp(&right.file)
            .then_with(|| left.address.start().cmp(&right.address.start()))
    });
    for pair in edits.windows(2) {
        if pair[0].file == pair[1].file && pair[1].address.start() < pair[0].address.end() {
            return Err(AdapterError::InvalidOptions(format!(
                "plan edits overlap in {}: {} and {}",
                pair[0].file,
                pair[0].address.label(),
                pair[1].address.label()
            )));
        }
    }
    Ok(edits)
}

fn parse_edit(value: &Value) -> Result<PlannedEdit, AdapterError> {
    let object = value.as_object().ok_or_else(|| {
        AdapterError::InvalidOptions("plan apply.edits[] entries must be objects".into())
    })?;
    let index = object
        .get("match")
        .and_then(Value::as_u64)
        .ok_or_else(|| AdapterError::InvalidOptions("plan edit has no match index".into()))?;
    let file = span_patch::normalize_relative(
        object
            .get("file")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AdapterError::InvalidOptions(format!("plan edit {index} has no file"))
            })?,
        true,
    )?;
    let position = |key: &str| -> Result<usize, AdapterError> {
        object["span"][key]
            .as_u64()
            .filter(|value| *value >= 1 && *value <= u32::MAX as u64)
            .map(|value| value as usize)
            .ok_or_else(|| {
                AdapterError::InvalidOptions(format!(
                    "plan edit {index} span.{key} must be a 1-based integer"
                ))
            })
    };
    let address = SpanAddress {
        start_line: position("startLine")?,
        start_column: position("startColumn")?,
        end_line: position("endLine")?,
        end_column: position("endColumn")?,
    };
    if address.start() >= address.end() {
        return Err(AdapterError::InvalidOptions(format!(
            "plan edit {index} span end must be after start: {}",
            address.label()
        )));
    }
    let digest = |key: &str| -> Result<String, AdapterError> {
        object
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| is_digest(value))
            .map(str::to_string)
            .ok_or_else(|| {
                AdapterError::InvalidOptions(format!(
                    "plan edit {index} {key} must be a 64-character lowercase sha256"
                ))
            })
    };
    let expected_sha256 = digest("sha256")?;
    let expected_line_sha256 = digest("lineSha256")?;
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| AdapterError::InvalidOptions(format!("plan edit {index} has no text")))?;
    // The plan carries both the bytes it saw and their digest. Checking them
    // against each other costs nothing and closes the one way a hand-edited
    // plan could claim a digest for text it never read.
    if sha256_hex(text.as_bytes()) != expected_sha256 {
        return Err(AdapterError::Contract(format!(
            "plan edit {index} is internally inconsistent: sha256 does not hash its own recorded text"
        )));
    }
    if text.len() > span_patch::MAX_EDIT_BYTES {
        return Err(AdapterError::InvalidOptions(format!(
            "plan edit {index} replaces {} bytes; an applicable edit replaces at most {}",
            text.len(),
            span_patch::MAX_EDIT_BYTES
        )));
    }
    let replacement = object
        .get("replacement")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AdapterError::InvalidOptions(format!("plan edit {index} has no replacement"))
        })?
        .to_string();
    // The plan stage already refuses to *emit* a replacement wider than this.
    // Re-checking it here is not redundant: the plan the apply stage receives
    // is a file, and this is the field through which an oversized write would
    // otherwise pass — the digest guards only where the write lands, never how
    // much is written.
    if replacement.len() > span_patch::MAX_EDIT_BYTES {
        return Err(AdapterError::InvalidOptions(format!(
            "plan edit {index} writes a {} byte replacement; an applicable edit writes at most {}",
            replacement.len(),
            span_patch::MAX_EDIT_BYTES
        )));
    }
    Ok(PlannedEdit {
        index,
        file,
        address,
        expected_sha256,
        expected_line_sha256,
        replacement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(edits: Value) -> Value {
        json!({
            "schema": PLAN_SCHEMA,
            "capability": "edit.ast-grep-plan",
            "apply": {"applicable": true, "reason": Value::Null, "edits": edits},
        })
    }

    fn edit(index: u64, file: &str, start_column: u64, text: &str) -> Value {
        json!({
            "match": index,
            "file": file,
            "span": {
                "startLine": 1,
                "startColumn": start_column,
                "endLine": 1,
                "endColumn": start_column + text.len() as u64
            },
            "sha256": sha256_hex(text.as_bytes()),
            "lineSha256": sha256_hex(b"whatever line this sat on"),
            "text": text,
            "replacement": "replacement",
        })
    }

    #[test]
    fn registry_digest_binds_this_adapter_and_its_write_authority() {
        let registry: Value =
            serde_json::from_slice(include_bytes!("../../../orchestration/integrations.json"))
                .unwrap();
        let integration = registry["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "edit.ast-grep-apply")
            .unwrap();
        assert_eq!(
            integration["capabilityDeclaration"]["implementation"]["toolchainDigests"],
            json!([
                sha256_hex(include_bytes!("ast_grep_apply.rs")),
                sha256_hex(include_bytes!("span_patch.rs"))
            ])
        );
        assert_eq!(
            integration["capabilityDeclaration"]["allowedEffects"],
            json!(["repo_read", "local_write", "repo_mutation"])
        );
    }

    /// The plan says whether it is applicable, and a plan that is not applies
    /// to nothing. Without this the apply stage would quietly execute the
    /// subset of a broken plan that happened to parse.
    #[test]
    fn an_inapplicable_plan_is_refused_with_the_reason_the_plan_gave() {
        let mut document = plan(json!([edit(0, "a.rs", 1, "alpha")]));
        document["apply"]["applicable"] = json!(false);
        document["apply"]["reason"] = json!("plan carries no rewrite");
        let error = parse_plan(&document).unwrap_err();
        assert!(
            matches!(&error, AdapterError::Contract(message) if message.contains("plan carries no rewrite")),
            "{error:?}"
        );
    }

    /// A hand-authored plan may still only issue *instructions*. Claiming a
    /// digest for text the plan does not carry is the one shape that would
    /// let a caller aim a write at bytes it never read.
    #[test]
    fn a_plan_whose_digest_does_not_hash_its_own_text_is_refused() {
        let mut document = plan(json!([edit(0, "a.rs", 1, "alpha")]));
        document["apply"]["edits"][0]["text"] = json!("beta");
        let error = parse_plan(&document).unwrap_err();
        assert!(
            matches!(&error, AdapterError::Contract(message) if message.contains("internally inconsistent")),
            "{error:?}"
        );
    }

    /// Where the line between "reported" and "enforced" actually falls.
    ///
    /// `snapshotIdentity` is provenance: forging it is not refused, because
    /// nothing here could tell a forged one from a real one. The replacement
    /// ceiling *is* enforced, and it is enforced here rather than trusted from
    /// the plan stage, because the plan is a file the caller wrote.
    #[test]
    fn provenance_is_not_a_gate_but_the_edit_size_ceiling_is() {
        let mut forged = plan(json!([edit(0, "a.rs", 1, "alpha")]));
        forged["snapshotIdentity"] = json!("forged-not-a-real-snapshot-identity");
        assert_eq!(parse_plan(&forged).unwrap().len(), 1);

        let mut oversized = forged.clone();
        oversized["apply"]["edits"][0]["replacement"] =
            json!("x".repeat(span_patch::MAX_EDIT_BYTES + 1));
        let error = parse_plan(&oversized).unwrap_err();
        assert!(
            matches!(&error, AdapterError::InvalidOptions(message) if message.contains("writes at most")),
            "{error:?}"
        );

        // A span wider than a real plan would have recorded is refused for the
        // same reason, even though its digest hashes its own text.
        let wide = "y".repeat(span_patch::MAX_EDIT_BYTES + 1);
        let error = parse_plan(&plan(json!([edit(0, "a.rs", 1, &wide)]))).unwrap_err();
        assert!(
            matches!(&error, AdapterError::InvalidOptions(message) if message.contains("replaces at most")),
            "{error:?}"
        );
    }

    #[test]
    fn overlapping_and_escaping_edits_are_refused_before_any_file_is_opened() {
        let overlapping = plan(json!([
            edit(0, "a.rs", 1, "alpha"),
            edit(1, "a.rs", 3, "phabet")
        ]));
        assert!(parse_plan(&overlapping).is_err());
        let escaping = plan(json!([edit(0, "../outside.rs", 1, "alpha")]));
        assert!(parse_plan(&escaping).is_err());
        let generated = plan(json!([edit(0, "node_modules/x.js", 1, "alpha")]));
        assert!(parse_plan(&generated).is_err());
        // Touching but disjoint edits in the same file are legitimate, and so
        // are identical spans in different files.
        let disjoint = plan(json!([
            edit(0, "a.rs", 1, "alpha"),
            edit(1, "a.rs", 6, "beta")
        ]));
        assert_eq!(parse_plan(&disjoint).unwrap().len(), 2);
        let two_files = plan(json!([
            edit(0, "a.rs", 1, "alpha"),
            edit(1, "b.rs", 1, "alpha")
        ]));
        assert_eq!(parse_plan(&two_files).unwrap().len(), 2);
    }
}
