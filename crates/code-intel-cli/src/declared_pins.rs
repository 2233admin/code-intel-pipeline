//! Digest pins that a record declares about itself.
//!
//! A record states which files it is evidence over, as a path sitting next to
//! its digest: `{ "path": ..., "sha256": ... }` in
//! `orchestration/internalization/`, `{ "conformancePath": ...,
//! "conformanceDigest": ... }` in `orchestration/acceptance/`. See `KINDS`.
//! The same digest is then repeated in prose-ish shapes elsewhere in the
//! record — inside the semicolon-joined `subject.source.revision` string, and
//! inside `evidenceIds` such as `local:i40:conformance-sha256:<digest>`.
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

/// Where declared pins live, and under which key names.
///
/// Started as one hardcoded directory and one hardcoded key pair, which is the
/// same mistake this module exists to fix: scoped to the records I happened to
/// be looking at rather than to the invariant. The acceptance reports declare
/// the identical thing under different names (`conformancePath` /
/// `conformanceDigest`) in a different directory, went stale on the same edit,
/// and no tool could resync them. Adding a kind is now a one-line change.
struct PinKind {
    /// Directory to scan, relative to the repository root.
    dir: &'static str,
    /// Object key holding the path, paired with the key holding its digest.
    keys: &'static [(&'static str, &'static str)],
}

const KINDS: &[PinKind] = &[
    PinKind {
        dir: "orchestration/internalization",
        keys: &[("path", "sha256")],
    },
    PinKind {
        dir: "orchestration/acceptance",
        keys: &[
            ("sourcePath", "sourceDigest"),
            ("conformancePath", "conformanceDigest"),
        ],
    },
];

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
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Collect every path/digest pair anywhere in `value`, under the given keys.
fn collect(value: &Value, record: &str, keys: &[(&str, &str)], out: &mut Vec<DeclaredPin>) {
    match value {
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect(item, record, keys, out)),
        Value::Object(map) => {
            for (path_key, digest_key) in keys {
                if let (Some(Value::String(path)), Some(Value::String(sha))) =
                    (map.get(*path_key), map.get(*digest_key))
                {
                    if is_sha256(sha) {
                        out.push(DeclaredPin {
                            record: record.to_string(),
                            path: path.clone(),
                            declared: sha.clone(),
                        });
                    }
                }
            }
            map.values()
                .for_each(|item| collect(item, record, keys, out));
        }
        _ => {}
    }
}

/// Every declared pin under every kind, sorted by record then path.
pub(crate) fn discover(repo: &Path) -> Result<Vec<DeclaredPin>, String> {
    let mut pins = Vec::new();
    for kind in KINDS {
        let dir = repo.join(kind.dir);
        // A repository without the directory — a fixture tree, or any repo that
        // is not this one — simply declares no pins of that kind. Only this
        // pipeline keeps these records, and `repin` runs against arbitrary repos.
        if !dir.is_dir() {
            continue;
        }
        let entries = fs::read_dir(&dir).map_err(|err| format!("read {}: {err}", dir.display()))?;

        let mut records: Vec<PathBuf> = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|err| format!("read dir entry: {err}"))?
                .path();
            if path.extension().is_some_and(|ext| ext == "json") {
                records.push(path);
            }
        }
        records.sort();

        for record in records {
            let name = record
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let relative = format!("{}/{name}", kind.dir);
            let text = fs::read_to_string(&record)
                .map_err(|err| format!("read {}: {err}", record.display()))?;
            let parsed: Value = serde_json::from_str(&text)
                .map_err(|err| format!("parse {}: {err}", record.display()))?;
            collect(&parsed, &relative, kind.keys, &mut pins);
        }
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
        let bytes = fs::read(&target).map_err(|err| format!("read {}: {err}", target.display()))?;
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

/// Report the findings a human needs to act on. Fresh pins say nothing.
///
/// `wrote` selects the tense: after `--write` these are things that were
/// fixed, before it they are things that need fixing.
pub(crate) fn render_findings(findings: &[PinFinding], wrote: bool) -> String {
    let mut out = String::new();
    for finding in findings.iter().filter(|finding| finding.needs_attention()) {
        let pin = &finding.pin;
        match &finding.state {
            PinState::Stale { actual } if wrote => out.push_str(&format!(
                "repin: {} declared {} -> {} ({})\n",
                pin.record,
                &pin.declared[..12],
                &actual[..12],
                pin.path
            )),
            PinState::Stale { actual } => out.push_str(&format!(
                "repin: STALE declared pin {} in {} (declares {}, file is {}) \u{2014} rerun with --write\n",
                pin.path,
                pin.record,
                &pin.declared[..12],
                &actual[..12]
            )),
            PinState::SourceMissing => out.push_str(&format!(
                "repin: UNRESOLVED {} pins {} which no longer exists \u{2014} decide what the record now claims\n",
                pin.record, pin.path
            )),
            PinState::Ambiguous => out.push_str(&format!(
                "repin: UNRESOLVED {} states one digest for several paths ({}) \u{2014} never rewritten\n",
                pin.record, pin.path
            )),
            PinState::Fresh => {}
        }
    }
    out
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
