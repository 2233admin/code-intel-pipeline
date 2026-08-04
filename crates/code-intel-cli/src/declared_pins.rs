//! Digest pins that an internalization record declares about itself.
//!
//! A record in `orchestration/internalization/` states which files it is
//! evidence over, as `{ "path": "...", "sha256": "..." }` objects. The same
//! digest is then repeated in prose-ish shapes elsewhere in the record — inside
//! the semicolon-joined `subject.source.revision` string, and inside
//! `evidenceIds` such as `local:i40:conformance-sha256:<digest>`.
//!
//! `repin` cannot resync these. It defines a stale pin as a digest matching a
//! file's content at HEAD but not in the worktree, which only holds while the
//! edit is uncommitted. Commit the edit — the ordinary workflow — and the pin
//! matches neither HEAD nor worktree, so `repin` classifies it as nothing at
//! all and reports the tree clean while the record still points at the previous
//! revision. It is not a bug in the fixpoint scan; the file identity a resync
//! would need simply is not in `repin`'s model.
//!
//! It is in the record, though: the `path` sits right next to the `sha256`.
//! This module reads that declaration, so the writer that refreshes a pin and
//! the gate that fails on a stale one work from one list instead of two that
//! agree until they don't.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::capability::sha256_hex;

/// Records live here, relative to the repository root.
pub(crate) const RECORD_DIR: &str = "orchestration/internalization";

/// One `{ path, sha256 }` declaration inside one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredPin {
    /// Record file, relative to the repository root.
    pub(crate) record: String,
    /// Pinned file, relative to the repository root.
    pub(crate) path: String,
    /// Digest as the record currently states it.
    pub(crate) declared: String,
}

/// How a declared pin compares to the file it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PinState {
    /// Declaration matches the file on disk.
    Fresh,
    /// File exists and its digest has moved on.
    Stale { actual: String },
    /// The pinned path is gone; a human has to decide what the record now means.
    SourceMissing,
    /// The record states this digest for more than one distinct path, so a
    /// textual rewrite cannot tell the occurrences apart. Never rewritten.
    Ambiguous,
}

#[derive(Debug, Clone)]
pub(crate) struct PinFinding {
    pub(crate) pin: DeclaredPin,
    pub(crate) state: PinState,
}

impl PinFinding {
    pub(crate) fn needs_attention(&self) -> bool {
        !matches!(self.state, PinState::Fresh)
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Collect every `{ path, sha256 }` pair anywhere in `value`.
fn collect(value: &Value, record: &str, out: &mut Vec<DeclaredPin>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| collect(item, record, out)),
        Value::Object(map) => {
            if let (Some(Value::String(path)), Some(Value::String(sha))) =
                (map.get("path"), map.get("sha256"))
            {
                if is_sha256(sha) {
                    out.push(DeclaredPin {
                        record: record.to_string(),
                        path: path.clone(),
                        declared: sha.clone(),
                    });
                }
            }
            map.values().for_each(|item| collect(item, record, out));
        }
        _ => {}
    }
}

/// Every declared pin under `RECORD_DIR`, sorted by record then path.
pub(crate) fn discover(repo: &Path) -> Result<Vec<DeclaredPin>, String> {
    let dir = repo.join(RECORD_DIR);
    // A repository without the record directory — a fixture tree, or any repo
    // that is not this one — simply declares no pins. Only this pipeline keeps
    // internalization records, and `repin` runs against arbitrary repos.
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&dir).map_err(|err| format!("read {}: {err}", dir.display()))?;

    let mut records: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry.map_err(|err| format!("read dir entry: {err}"))?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            records.push(path);
        }
    }
    records.sort();

    let mut pins = Vec::new();
    for record in records {
        let name = record
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let relative = format!("{RECORD_DIR}/{name}");
        let text = fs::read_to_string(&record)
            .map_err(|err| format!("read {}: {err}", record.display()))?;
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|err| format!("parse {}: {err}", record.display()))?;
        collect(&parsed, &relative, &mut pins);
    }
    pins.sort_by(|a, b| (&a.record, &a.path).cmp(&(&b.record, &b.path)));
    pins.dedup();
    Ok(pins)
}

/// Compare every declared pin against the file it names.
pub(crate) fn audit(repo: &Path) -> Result<Vec<PinFinding>, String> {
    let pins = discover(repo)?;

    // A digest stated for two different paths inside one record cannot be
    // rewritten textually without guessing which occurrence meant which file.
    let mut owners: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for pin in &pins {
        owners
            .entry((pin.record.clone(), pin.declared.clone()))
            .or_default()
            .push(pin.path.clone());
    }

    let mut findings = Vec::new();
    for pin in pins {
        let paths = owners
            .get(&(pin.record.clone(), pin.declared.clone()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let distinct = paths
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if distinct > 1 {
            findings.push(PinFinding {
                pin,
                state: PinState::Ambiguous,
            });
            continue;
        }

        let target = repo.join(&pin.path);
        if !target.is_file() {
            findings.push(PinFinding {
                pin,
                state: PinState::SourceMissing,
            });
            continue;
        }
        let bytes =
            fs::read(&target).map_err(|err| format!("read {}: {err}", target.display()))?;
        let actual = sha256_hex(&bytes);
        let state = if actual == pin.declared {
            PinState::Fresh
        } else {
            PinState::Stale { actual }
        };
        findings.push(PinFinding { pin, state });
    }
    Ok(findings)
}

/// Rewrite every stale declared pin in place.
///
/// The digest is replaced textually across the whole record, which is what
/// carries the `revision` string and the `evidenceIds` along with the
/// structured field — the same reason `repin` scans for the raw token rather
/// than modeling each JSON shape.
///
/// Returns the records that changed. `Ambiguous` and `SourceMissing` findings
/// are never rewritten: both need a human to say what the record now means.
pub(crate) fn resync(repo: &Path, findings: &[PinFinding]) -> Result<Vec<String>, String> {
    let mut edits: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for finding in findings {
        if let PinState::Stale { actual } = &finding.state {
            edits
                .entry(finding.pin.record.clone())
                .or_default()
                .push((finding.pin.declared.clone(), actual.clone()));
        }
    }

    let mut written = Vec::new();
    for (record, replacements) in edits {
        let path = repo.join(&record);
        let mut text =
            fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        for (old, new) in replacements {
            text = text.replace(&old, &new);
        }
        fs::write(&path, text.as_bytes())
            .map_err(|err| format!("write {}: {err}", path.display()))?;
        written.push(record);
    }
    Ok(written)
}
