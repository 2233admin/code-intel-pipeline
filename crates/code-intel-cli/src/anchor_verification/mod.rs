//! Pre-publish anchor verification gate (issue #151): checks whether a
//! product's location claims -- a file path, a symbol at a claimed line --
//! still resolve against the repository at publish time, before those claims
//! ship. A location claim that no longer resolves is worse than no claim at
//! all: an agent that opens a dropped anchor's path finds nothing, or finds
//! the wrong thing, and either wastes a turn or draws a wrong conclusion from
//! it. This gate exists so that never happens silently.
//!
//! Three anchor kinds are checked, each cheaply and without ambiguity:
//!
//! - **file anchor**: does the claimed path exist in the repository at all.
//! - **line-range anchor**: does the claimed `startLine..endLine` fall inside
//!   the file's actual line count (checked as part of symbol re-resolution
//!   below, since every line-range claim this gate sees is a symbol's).
//! - **symbol anchor**: does the claimed symbol name still resolve at its
//!   claimed location, using the exact same per-language declaration
//!   heuristic that produced the claim
//!   ([`native_code_evidence::find_symbol_line`]).
//!
//! Symbol re-resolution is deliberately bounded to the one file the claim
//! names -- never a repository-wide search. If the name is not where it was
//! claimed and not anywhere else in that same file either, it is dropped, not
//! chased. "解不出就是解不出": failing to resolve in the claimed file is the
//! answer, not a cue to go search the rest of the repository for it.
//!
//! ## Three states, not two
//!
//! [`AnchorState`] mirrors the discipline [`crate::evidence_outcome`]
//! established for gate G1 (issue #141): the state that degrades a claim
//! carries the reason it degraded, and the state that drops it carries why,
//! so neither can be forged into looking like an unqualified pass by
//! flipping one field. `Approximate` cannot be constructed without the
//! corrected line it found; `Dropped` cannot be constructed without a
//! reason. [`AnchorState::from_json`] then closes the same gap G1 closed at
//! the deserialization boundary: each state's JSON object must carry
//! *exactly* its expected keys, so relabeling a real `Dropped` claim's
//! `state` to `"verified"` while leaving its `reason` behind is rejected by
//! the leftover key, not accepted because `"verified"` itself needs nothing
//! else.
//!
//! ## Where the counts land
//!
//! [`AnchorCounts`] is never allowed to be silently absent: the aggregate
//! `{verified, approximate, dropped}` is written into the new
//! `verification.anchors` artifact this module produces, *and* hoisted to
//! the top level of the `run execute` CLI summary
//! (`ExecutionResult::to_json`) so a caller sees `dropped` without opening
//! either the manifest or this artifact.
//!
//! ## Scope decision: a companion artifact, not an in-place rewrite
//!
//! The products this gate reads (`ranking.json`'s agent slice,
//! `code_evidence.symbols`, `surgery-plan.json`) are already-hashed,
//! already-committed-shape artifacts: their bytes are covered by
//! `run-manifest.json`'s per-node `sha256`, which `run-manifest-ref.json`
//! covers in turn, and their payload shapes are enforced by closed-key
//! (`exact`) validators in `artifact_ref.rs`. Rewriting a dropped anchor out
//! of `ranking.json` itself, in place, would mean recomputing that hash,
//! propagating it through both manifest layers, and widening every closed-key
//! validator that shape touches -- including the *nested* copy of
//! `surgery_plan` inside `hospital-report.json`, which carries its own
//! independent closed-key check. That is a materially larger and riskier
//! change than this gate needs to make its point.
//!
//! This gate instead publishes a new, never-before-hashed artifact
//! (`verification.anchors`, mirroring how `repository.iteration` and
//! `verification.session-evidence` are each folded into the terminal
//! manifest after the DAG completes -- see
//! `authoritative_run::completion::bind_anchor_verification`) that partitions
//! every anchor it checked into verified/approximate/dropped and excludes
//! the dropped ones from what it recommends trusting. The original products
//! are left byte-for-byte as the DAG produced them.
//!
//! Read literally, the issue's "dropped anchors are 剔除 (excluded)" could
//! mean excluded from `ranking.json`/`surgery-plan.json` themselves. This
//! gate satisfies it inside the new companion artifact instead -- a
//! deliberate scope call, not a silent gap; see the PR description for the
//! trade-off this reflects.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::capability_inventory::native_code_evidence;

#[cfg(test)]
mod tests;

pub(crate) const ANCHOR_VERIFICATION_SCHEMA: &str = "code-intel-anchor-verification.v1";
pub(crate) const ANCHOR_VERIFICATION_TYPE: &str = "verification.anchors";

/// The verification state of one anchor. `Approximate` cannot be built
/// without the corrected line it found; `Dropped` cannot be built without a
/// reason -- there is no constructor for either that omits its evidence, the
/// same discipline `crate::evidence_outcome::EvidenceOutcome` uses for gate
/// G1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnchorState {
    /// The claim still resolves exactly where it said it would.
    Verified,
    /// The claim no longer resolves at its claimed location, but the same
    /// name was found elsewhere in the same file (never outside it) --
    /// `resolved_line` is where it actually is now.
    Approximate { resolved_line: usize },
    /// The claim does not resolve anywhere in the claimed file (or the file
    /// itself is gone). `reason` says which.
    Dropped { reason: String },
}

impl AnchorState {
    pub(crate) fn to_json(&self) -> Value {
        match self {
            AnchorState::Verified => json!({ "state": "verified" }),
            AnchorState::Approximate { resolved_line } => json!({
                "state": "approximate",
                "resolvedLine": resolved_line,
            }),
            AnchorState::Dropped { reason } => json!({
                "state": "dropped",
                "reason": reason,
            }),
        }
    }

    /// The inverse of [`Self::to_json`], and the anti-forgery enforcement
    /// point: a `"verified"` object carrying a leftover `reason` or
    /// `resolvedLine` (the shape a `Dropped`/`Approximate` value relabeled by
    /// touching only `state` would have) is rejected here, not silently
    /// accepted because the keys `"verified"` itself needs are present.
    #[cfg(test)]
    pub(crate) fn from_json(value: &Value) -> Result<Self, String> {
        match value.get("state").and_then(Value::as_str) {
            Some("verified") => {
                exact(value, &["state"], "a \"verified\" anchor state")?;
                Ok(AnchorState::Verified)
            }
            Some("approximate") => {
                exact(
                    value,
                    &["state", "resolvedLine"],
                    "an \"approximate\" anchor state",
                )?;
                let resolved_line = value
                    .get("resolvedLine")
                    .and_then(Value::as_u64)
                    .ok_or("\"approximate\" requires a numeric \"resolvedLine\"")?;
                Ok(AnchorState::Approximate {
                    resolved_line: resolved_line as usize,
                })
            }
            Some("dropped") => {
                exact(value, &["state", "reason"], "a \"dropped\" anchor state")?;
                let reason = value
                    .get("reason")
                    .and_then(Value::as_str)
                    .ok_or("\"dropped\" requires a \"reason\"")?;
                Ok(AnchorState::Dropped {
                    reason: reason.to_string(),
                })
            }
            other => Err(format!("unknown anchor state: {other:?}")),
        }
    }
}

/// Rejects any object whose key set is not *exactly* `fields`. Local to this
/// module rather than imported, matching the rest of this crate's per-module
/// `exact`/`exact_object`/`exact_keys` convention (e.g. `run_commit::exact`)
/// instead of a shared helper.
#[cfg(test)]
fn exact(value: &Value, fields: &[&str], label: &str) -> Result<(), String> {
    let actual = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let expected = fields
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label} fields are invalid"))
    }
}

/// A file-existence anchor: the whole claim is the path, so the only
/// possible states are `Verified` (the path exists) and `Dropped` (it does
/// not) -- there is no meaningful "approximate" for pure existence.
struct FileAnchorResult {
    path: String,
    state: AnchorState,
}

impl FileAnchorResult {
    fn to_json(&self) -> Value {
        let mut value = self.state.to_json();
        value
            .as_object_mut()
            .expect("AnchorState::to_json() is an object")
            .insert("path".to_string(), json!(self.path));
        value
    }
}

/// A symbol-at-a-line anchor: carries the claimed location alongside
/// whichever of the three states re-resolution found.
struct SymbolAnchorResult {
    file: String,
    name: String,
    claimed_line: usize,
    state: AnchorState,
}

impl SymbolAnchorResult {
    fn to_json(&self) -> Value {
        let mut value = self.state.to_json();
        let object = value
            .as_object_mut()
            .expect("AnchorState::to_json() is an object");
        object.insert("file".to_string(), json!(self.file));
        object.insert("name".to_string(), json!(self.name));
        object.insert("claimedLine".to_string(), json!(self.claimed_line));
        value
    }
}

/// The aggregate the "dropped > 0 must never be silent" requirement is
/// checked against. Exhaustive match in [`Self::record`] over
/// [`AnchorState`]'s three variants means adding a fourth state without
/// updating this counter fails to compile, rather than silently
/// under-counting it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AnchorCounts {
    pub(crate) verified: u64,
    pub(crate) approximate: u64,
    pub(crate) dropped: u64,
}

impl AnchorCounts {
    fn record(&mut self, state: &AnchorState) {
        match state {
            AnchorState::Verified => self.verified += 1,
            AnchorState::Approximate { .. } => self.approximate += 1,
            AnchorState::Dropped { .. } => self.dropped += 1,
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "verified": self.verified,
            "approximate": self.approximate,
            "dropped": self.dropped,
        })
    }
}

/// Checks whether `relative_path` exists in the repository rooted at
/// `repo_root`. The only two reachable states are `Verified` and `Dropped`:
/// a bare path claim has nothing to degrade to.
pub(crate) fn verify_file_anchor(repo_root: &Path, relative_path: &str) -> AnchorState {
    match native_code_evidence::safe_join(repo_root, relative_path) {
        Ok(full) if full.is_file() => AnchorState::Verified,
        Ok(_) => AnchorState::Dropped {
            reason: format!("file not found in repository: {relative_path}"),
        },
        Err(_) => AnchorState::Dropped {
            reason: format!("anchor path is not a valid repository-relative path: {relative_path}"),
        },
    }
}

/// Re-resolves `name` at `claimed_line` (1-based) inside `relative_path`,
/// reading the file itself (used by tests and as the single-anchor entry
/// point; the bulk gate path in [`symbol_anchors`] reads each file once and
/// shares it across all of that file's symbols instead of calling this once
/// per symbol).
pub(crate) fn verify_symbol_anchor(
    repo_root: &Path,
    relative_path: &str,
    name: &str,
    claimed_line: usize,
) -> AnchorState {
    match read_repo_file_utf8(repo_root, relative_path) {
        Some(content) => {
            let language = native_code_evidence::language(relative_path);
            let lines = native_code_evidence::lines(&content);
            resolve_symbol_state(language, &lines, name, claimed_line, relative_path)
        }
        None => AnchorState::Dropped {
            reason: format!("file not found in repository: {relative_path}"),
        },
    }
}

fn resolve_symbol_state(
    language: &str,
    lines: &[&str],
    name: &str,
    claimed_line: usize,
    relative_path: &str,
) -> AnchorState {
    match native_code_evidence::find_symbol_line(language, lines, name, claimed_line) {
        Some(line) if line == claimed_line => AnchorState::Verified,
        Some(resolved_line) => AnchorState::Approximate { resolved_line },
        None => AnchorState::Dropped {
            reason: format!("symbol \"{name}\" no longer resolves in {relative_path}"),
        },
    }
}

fn read_repo_file_utf8(repo_root: &Path, relative_path: &str) -> Option<String> {
    let full = native_code_evidence::safe_join(repo_root, relative_path).ok()?;
    if !full.is_file() {
        return None;
    }
    String::from_utf8(fs::read(&full).ok()?).ok()
}

/// File anchors from `code_evidence.agent_slice` (`ranking.json`): every
/// ranked file's `path`.
fn ranking_anchors(
    bytes: &[u8],
    repo_root: &Path,
    counts: &mut AnchorCounts,
) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("parse agent-code-slice-ranking.v1 artifact: {error}"))?;
    let files = value["files"]
        .as_array()
        .ok_or("agent-code-slice-ranking.v1 artifact missing \"files\" array")?;
    Ok(files
        .iter()
        .map(|file| {
            let path = file["path"].as_str().unwrap_or("").to_string();
            let state = verify_file_anchor(repo_root, &path);
            counts.record(&state);
            FileAnchorResult { path, state }.to_json()
        })
        .collect())
}

/// Symbol anchors from `code_evidence.symbols`: every symbol's `name` at its
/// claimed `file`/`startLine`. Groups by file first so a file with many
/// symbols is read from disk once, not once per symbol.
fn symbol_anchors(
    bytes: &[u8],
    repo_root: &Path,
    counts: &mut AnchorCounts,
) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("parse code-evidence-symbols.v1 artifact: {error}"))?;
    let symbols = value["symbols"]
        .as_array()
        .ok_or("code-evidence-symbols.v1 artifact missing \"symbols\" array")?;
    let mut by_file: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for symbol in symbols {
        by_file
            .entry(symbol["file"].as_str().unwrap_or(""))
            .or_default()
            .push(symbol);
    }
    let mut results = Vec::with_capacity(symbols.len());
    for (file, file_symbols) in by_file {
        let content = read_repo_file_utf8(repo_root, file);
        let language = native_code_evidence::language(file);
        for symbol in file_symbols {
            let name = symbol["name"].as_str().unwrap_or("").to_string();
            let claimed_line = symbol["startLine"].as_u64().unwrap_or(0) as usize;
            let state = match &content {
                Some(text) => {
                    let lines = native_code_evidence::lines(text);
                    resolve_symbol_state(language, &lines, &name, claimed_line, file)
                }
                None => AnchorState::Dropped {
                    reason: format!("file not found in repository: {file}"),
                },
            };
            counts.record(&state);
            results.push(
                SymbolAnchorResult {
                    file: file.to_string(),
                    name,
                    claimed_line,
                    state,
                }
                .to_json(),
            );
        }
    }
    Ok(results)
}

/// The file anchor from `diagnosis.surgery-plan`: `primary_target.file`,
/// when the plan names one. Returns no anchors (not a "verified" claim about
/// nothing) when no target was named -- a plan with `file: null` makes no
/// location claim to check.
fn surgery_plan_anchors(
    bytes: &[u8],
    repo_root: &Path,
    counts: &mut AnchorCounts,
) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("parse code-intel-surgery-plan.v1 artifact: {error}"))?;
    match value["primary_target"]["file"].as_str() {
        Some(path) => {
            let path = path.to_string();
            let state = verify_file_anchor(repo_root, &path);
            counts.record(&state);
            Ok(vec![FileAnchorResult { path, state }.to_json()])
        }
        None => Ok(Vec::new()),
    }
}

/// Runs the anchor gate over every recognized product a just-completed DAG
/// run produced, re-verifying each anchor against `repo_root`. Scans
/// `succeeded` and `domain_failed` nodes (the same inclusion rule
/// `run_commit::manifest_artifact_refs` uses to decide what is real,
/// hashable output) for the three known `(artifactSchema, type)` pairs; any
/// other artifact is left alone, and an artifact whose node never ran simply
/// contributes no source entry.
///
/// Returns the `verification.anchors` artifact document and the aggregate
/// counts, for the caller ([`crate::authoritative_run::completion`]) to
/// write to disk, fold into the manifest, and re-persist.
pub(crate) fn verify_and_report(
    run_root: &Path,
    repo_root: &Path,
    manifest: &Value,
) -> Result<(Value, AnchorCounts), String> {
    let nodes = manifest["nodes"]
        .as_object()
        .ok_or("run manifest nodes must be an object")?;
    let mut sources = Vec::new();
    let mut counts = AnchorCounts::default();
    for node in nodes.values() {
        if !matches!(node["status"].as_str(), Some("succeeded" | "domain_failed")) {
            continue;
        }
        let Some(artifacts) = node["artifacts"].as_array() else {
            continue;
        };
        for artifact in artifacts {
            let schema = artifact["artifactSchema"].as_str().unwrap_or("");
            let kind = artifact["type"].as_str().unwrap_or("");
            let anchor_kind = match (schema, kind) {
                ("agent-code-slice-ranking.v1", "code_evidence.agent_slice") => "file",
                ("code-evidence-symbols.v1", "code_evidence.symbols") => "symbol",
                ("code-intel-surgery-plan.v1", "diagnosis.surgery-plan") => "file",
                _ => continue,
            };
            let path = artifact["path"]
                .as_str()
                .ok_or("artifact ref missing \"path\"")?;
            let bytes = fs::read(run_root.join(path))
                .map_err(|error| format!("read {path} for anchor verification: {error}"))?;
            let anchors = match kind {
                "code_evidence.agent_slice" => ranking_anchors(&bytes, repo_root, &mut counts)?,
                "code_evidence.symbols" => symbol_anchors(&bytes, repo_root, &mut counts)?,
                _ => surgery_plan_anchors(&bytes, repo_root, &mut counts)?,
            };
            if anchors.is_empty() {
                continue;
            }
            sources.push(json!({
                "artifactType": kind,
                "artifactPath": path,
                "anchorKind": anchor_kind,
                "anchors": anchors,
            }));
        }
    }
    let report = json!({
        "schema": ANCHOR_VERIFICATION_SCHEMA,
        "counts": counts.to_json(),
        "sources": sources,
    });
    Ok((report, counts))
}
