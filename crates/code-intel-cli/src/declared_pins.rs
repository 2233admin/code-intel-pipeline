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
//!
//! The same anchoring covers one more declaration shape: the toolchain digests
//! in `orchestration/integrations.json`, where `toolchainDigestEvidence.inputs[i]`
//! names a file and `capabilityDeclaration.implementation.toolchainDigests[i]`
//! is its digest, paired by array index (issue #129: after one `--write` that
//! pin holds an uncommitted digest, matches no HEAD digest, and the generic scan
//! reported `clean` on the next edit). An integration whose two arrays do not
//! pair up is `Unpaired`: none of its pins are rewritten. Bare 64-hex tokens
//! with no declared path next to them are still anchored to HEAD only, by the
//! generic scan in `repin`.

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
    /// An integration's `toolchainDigestEvidence.inputs` and
    /// `toolchainDigests` cannot be paired by index. The finding's `path`
    /// carries the integration `id`. Never rewritten.
    Unpaired,
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

/// Whether `path` is one of `excludes` or lies under one (prefix match on
/// whole path components).
pub(crate) fn is_excluded(path: &str, excludes: &[String]) -> bool {
    excludes.iter().any(|prefix| {
        let prefix = prefix.trim_end_matches('/');
        path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Fixed location of the integration registry, the one record whose pins are
/// declared by array index instead of by an adjacent key pair.
pub(crate) const REGISTRY: &str = "orchestration/integrations.json";

/// Collect the index-paired toolchain pins of every integration in `registry`.
/// Returns the ids of integrations whose arrays do not pair up; those add no
/// pins, because guessing an index would rewrite the wrong digest.
fn collect_registry(registry: &Value, out: &mut Vec<DeclaredPin>) -> Vec<String> {
    let mut unpaired = Vec::new();
    let integrations = registry["integrations"].as_array().into_iter().flatten();
    for integration in integrations {
        let Some(evidence) = integration.get("toolchainDigestEvidence") else {
            continue;
        };
        if is_unpaired(integration) {
            unpaired.push(integration["id"].as_str().unwrap_or_default().to_string());
            continue;
        }
        let inputs = evidence["inputs"].as_array().into_iter().flatten();
        let digests = integration["capabilityDeclaration"]["implementation"]["toolchainDigests"]
            .as_array()
            .into_iter()
            .flatten();
        for (input, digest) in inputs.zip(digests) {
            if let (Some(path), Some(sha)) = (input.as_str(), digest.as_str()) {
                if is_sha256(sha) {
                    out.push(DeclaredPin {
                        record: REGISTRY.to_string(),
                        path: path.to_string(),
                        declared: sha.to_string(),
                    });
                }
            }
        }
    }
    unpaired
}

/// An integration declares toolchain evidence its arrays cannot pair up.
fn is_unpaired(integration: &Value) -> bool {
    let Some(evidence) = integration.get("toolchainDigestEvidence") else {
        return false;
    };
    let inputs = evidence["inputs"].as_array();
    let digests =
        integration["capabilityDeclaration"]["implementation"]["toolchainDigests"].as_array();
    !matches!((inputs, digests), (Some(i), Some(d)) if i.len() == d.len())
}

/// Byte ranges of the unpaired integration objects in the registry text.
/// Walks the text (string-aware) instead of re-serializing, so the rewrite
/// keeps every other byte as it was.
fn unpaired_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = text.as_bytes();
    let Some(key) = text.find("\"integrations\"") else {
        return Vec::new();
    };
    let Some(open) = text[key..].find('[').map(|offset| key + offset) else {
        return Vec::new();
    };
    let (mut spans, mut depth, mut start) = (Vec::new(), 0usize, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for (index, &byte) in bytes.iter().enumerate().skip(open + 1) {
        if in_string {
            match (escaped, byte) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                if depth == 0 && byte == b'{' {
                    start = index;
                }
                depth += 1;
            }
            b'}' | b']' if depth == 0 => break,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 && byte == b'}' {
                    let range = start..index + 1;
                    let parsed = serde_json::from_str::<Value>(&text[range.clone()]);
                    if parsed.is_ok_and(|value| is_unpaired(&value)) {
                        spans.push(range);
                    }
                }
            }
            _ => {}
        }
    }
    spans
}

/// `text.replace(old, new)` skipping occurrences that start inside `protected`.
fn replace_outside(
    text: &str,
    old: &str,
    new: &str,
    protected: &[std::ops::Range<usize>],
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (index, _) in text.match_indices(old) {
        if protected.iter().any(|span| span.contains(&index)) {
            continue;
        }
        out.push_str(&text[last..index]);
        out.push_str(new);
        last = index + old.len();
    }
    out.push_str(&text[last..]);
    out
}

/// Everything `discover` found: pins, and ids of integrations that could not
/// be paired.
pub(crate) struct Discovered {
    pub(crate) pins: Vec<DeclaredPin>,
    pub(crate) unpaired: Vec<String>,
}

/// Every declared pin under every kind, sorted by record then path.
pub(crate) fn discover(repo: &Path, excludes: &[String]) -> Result<Discovered, String> {
    let mut pins = Vec::new();
    let mut unpaired = Vec::new();
    let registry = repo.join(REGISTRY);
    if registry.is_file() && !is_excluded(REGISTRY, excludes) {
        let text = fs::read_to_string(&registry)
            .map_err(|err| format!("read {}: {err}", registry.display()))?;
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|err| format!("parse {}: {err}", registry.display()))?;
        unpaired = collect_registry(&parsed, &mut pins);
    }
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
            if is_excluded(&relative, excludes) {
                continue;
            }
            let text = fs::read_to_string(&record)
                .map_err(|err| format!("read {}: {err}", record.display()))?;
            let parsed: Value = serde_json::from_str(&text)
                .map_err(|err| format!("parse {}: {err}", record.display()))?;
            collect(&parsed, &relative, kind.keys, &mut pins);
        }
    }
    pins.sort_by(|a, b| (&a.record, &a.path).cmp(&(&b.record, &b.path)));
    pins.dedup();
    Ok(Discovered { pins, unpaired })
}

/// Compare every declared pin against the file it names.
pub(crate) fn audit(repo: &Path, excludes: &[String]) -> Result<Vec<PinFinding>, String> {
    let Discovered { pins, unpaired } = discover(repo, excludes)?;

    // A digest stated for two different paths inside one record cannot be
    // rewritten textually without guessing which occurrence meant which file.
    let mut owners: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for pin in &pins {
        owners
            .entry((pin.record.clone(), pin.declared.clone()))
            .or_default()
            .push(pin.path.clone());
    }

    let mut findings: Vec<PinFinding> = unpaired
        .into_iter()
        .map(|id| PinFinding {
            pin: DeclaredPin {
                record: REGISTRY.to_string(),
                path: id,
                declared: String::new(),
            },
            state: PinState::Unpaired,
        })
        .collect();
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
            PinState::Unpaired => out.push_str(&format!(
                "repin: UNRESOLVED {} integration {} has toolchain inputs and digests that do not pair by index \u{2014} never rewritten\n",
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
/// Returns the records that changed. `Ambiguous`, `SourceMissing` and `Unpaired`
/// findings are never rewritten: both need a human to say what the record now means.
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
        // Spans the literal replacement must not enter: an unpaired
        // integration's digests are never rewritten, even when they spell the
        // same old digest as a paired one.
        let protected = if record == REGISTRY {
            unpaired_spans(&text)
        } else {
            Vec::new()
        };
        for (old, new) in replacements {
            text = replace_outside(&text, &old, &new, &protected);
        }
        fs::write(&path, text.as_bytes())
            .map_err(|err| format!("write {}: {err}", path.display()))?;
        written.push(record);
    }
    Ok(written)
}
