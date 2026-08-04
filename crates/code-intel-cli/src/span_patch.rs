//! The addresses-and-bytes substrate shared by the edit surface (#96, charter
//! gate G4 in #139).
//!
//! Three capabilities have to agree here byte-for-byte. `edit.ast-grep-plan`
//! *records* spans, `edit.ast-grep-apply` applies the spans it recorded, and
//! `edit.span-apply` applies spans an agent addressed by hand. If any two of
//! them disagreed by one byte about what `12:21-12:33` means — or about which
//! paths are editable at all — a plan could name a target the writer then
//! refuses, or, worse, one it accepts under looser rules. So the address
//! arithmetic, the path policy, and the write itself live in one module and
//! are *used* by the three adapters rather than restated in each.
//!
//! The address contract, stated once:
//!
//! - Lines are 1-based. Columns are 1-based **byte** offsets into that line's
//!   content, and the end column is exclusive — so a span can never swallow
//!   the terminator that ends its line.
//! - A file that ends with a terminator has exactly as many lines as it has
//!   terminators; the empty remainder after the final `\n` is not addressable.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

use super::AdapterError;

/// Refusal evidence quotes what was actually found. Bounded so a mis-addressed
/// span covering half a file cannot turn one refusal into a megabyte artifact.
pub(super) const MAX_EVIDENCE_BYTES: usize = 256;

/// The widest span a plan may replace, and the largest replacement it may
/// write.
///
/// `edit.ast-grep-plan` refuses to mark a wider match applicable, and
/// `edit.ast-grep-apply` enforces the same ceiling again on the plan it is
/// handed. Both, not one: a `--plan` is a JSON file on disk, so the apply
/// stage cannot assume the numbers in front of it came from a plan run that
/// had already checked them. Stating the constant once is what keeps the two
/// halves of that claim from drifting apart.
pub(super) const MAX_EDIT_BYTES: usize = 4096;

const GENERATED_COMPONENTS: [&str; 9] = [
    ".git",
    ".next",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "target",
    "vendor",
];

/// `startLine:startColumn-endLine:endColumn`, 1-based, end column exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SpanAddress {
    pub(super) start_line: usize,
    pub(super) start_column: usize,
    pub(super) end_line: usize,
    pub(super) end_column: usize,
}

impl SpanAddress {
    pub(super) fn start(&self) -> (usize, usize) {
        (self.start_line, self.start_column)
    }

    pub(super) fn end(&self) -> (usize, usize) {
        (self.end_line, self.end_column)
    }

    pub(super) fn label(&self) -> String {
        format!(
            "{}:{}-{}:{}",
            self.start_line, self.start_column, self.end_line, self.end_column
        )
    }

    pub(super) fn as_json(&self) -> Value {
        json!({
            "startLine": self.start_line,
            "startColumn": self.start_column,
            "endLine": self.end_line,
            "endColumn": self.end_column,
        })
    }
}

/// Byte range of every line's *content*, terminator excluded.
pub(super) struct LineIndex {
    bounds: Vec<(usize, usize)>,
}

impl LineIndex {
    pub(super) fn build(bytes: &[u8]) -> Self {
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
        Self { bounds }
    }

    /// Address → byte range. Errors name the actual shape of the file, because
    /// "line 900 of a 4-line file" is the diagnostic that tells a caller its
    /// coordinates rotted rather than its digest.
    pub(super) fn resolve(&self, span: &SpanAddress) -> Result<(usize, usize), String> {
        let offset = |line: usize, column: usize| -> Result<usize, String> {
            let (start, end) = *self.bounds.get(line - 1).ok_or_else(|| {
                format!(
                    "line {line} is beyond the file, which has {} line(s)",
                    self.bounds.len()
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

    /// The byte range of the whole line (or lines) an address sits on,
    /// terminators excluded.
    ///
    /// A span digest proves *these bytes*, not *this meaning*: renaming
    /// `commit` to `commit_now` by hand leaves the six bytes at
    /// `271:5-271:11` reading `commit`, so a plan built before that rename
    /// would still apply — and produce `commit_change_now`. A plan derived
    /// from a parse of the line has no business applying to a line that has
    /// since changed, so the plan stage records this digest too and the apply
    /// stage refuses on it.
    pub(super) fn line_span(&self, span: &SpanAddress) -> Result<(usize, usize), String> {
        let first = self.bounds.get(span.start_line - 1);
        let last = self.bounds.get(span.end_line - 1);
        match (first, last) {
            (Some((start, _)), Some((_, end))) => Ok((*start, *end)),
            _ => Err(format!(
                "line {} is beyond the file, which has {} line(s)",
                span.end_line,
                self.bounds.len()
            )),
        }
    }

    /// Byte range → address. The plan stage takes byte offsets from ast-grep
    /// and must hand the apply stage the same coordinate system a hand-written
    /// `--span` uses; going through this function is what makes that true by
    /// construction instead of by convention.
    pub(super) fn address(&self, start: usize, end: usize) -> Result<SpanAddress, String> {
        let point = |offset: usize| -> Result<(usize, usize), String> {
            let line = self
                .bounds
                .partition_point(|(line_start, _)| *line_start <= offset);
            let (line_start, line_end) = *self
                .bounds
                .get(line.wrapping_sub(1))
                .ok_or_else(|| format!("byte offset {offset} is before the first line"))?;
            if offset > line_end {
                return Err(format!(
                    "byte offset {offset} falls inside the terminator of line {line}"
                ));
            }
            Ok((line, offset - line_start + 1))
        };
        if start >= end {
            return Err(format!("byte range {start}..{end} is empty or inverted"));
        }
        let (start_line, start_column) = point(start)?;
        let (end_line, end_column) = point(end)?;
        Ok(SpanAddress {
            start_line,
            start_column,
            end_line,
            end_column,
        })
    }
}

/// Substitute back-to-front so every replacement keeps the offsets it was
/// resolved at against the pre-edit bytes. `edits` must be sorted ascending
/// and disjoint; both are checked by the callers that build it.
pub(super) fn rewrite(before: &[u8], edits: &[(usize, usize, &str)]) -> Vec<u8> {
    let mut after = before.to_vec();
    for (start, end, replacement) in edits.iter().rev() {
        after.splice(*start..*end, replacement.bytes());
    }
    after
}

/// Resolve one edit target and refuse every path shape that could escape the
/// repository or the requested snapshot scope. Symlinks are refused outright:
/// writing through one is a write to wherever it points, which no scope check
/// upstream of the rename can constrain.
pub(super) fn resolve_target(
    repo: &Path,
    relative: &str,
    snapshot: &Value,
) -> Result<PathBuf, AdapterError> {
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

/// Shared by the plan stage and both apply stages rather than restated in
/// each: they must agree byte-for-byte on which paths are editable.
pub(super) fn normalize_relative(
    path: &str,
    reject_generated: bool,
) -> Result<String, AdapterError> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Err(AdapterError::InvalidOptions(format!(
            "edit path must be repository-relative: {}",
            path.display()
        )));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    AdapterError::InvalidOptions("edit path must be UTF-8".into())
                })?;
                if reject_generated
                    && GENERATED_COMPONENTS
                        .iter()
                        .any(|generated| value.eq_ignore_ascii_case(generated))
                {
                    return Err(AdapterError::InvalidOptions(format!(
                        "edit path targets excluded generated content: {}",
                        path.display()
                    )));
                }
                parts.push(value);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AdapterError::InvalidOptions(format!(
                    "edit path escapes repository: {}",
                    path.display()
                )))
            }
        }
    }
    Ok(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}

pub(super) fn within_scope(path: &str, scope: &str) -> bool {
    scope == "." || path == scope || path.starts_with(&format!("{scope}/"))
}

/// New content written to a sibling temporary, not yet visible at the target.
///
/// Splitting the write from the rename is what lets a multi-file apply be a
/// unit: every file is staged first, and only once all of them are on disk
/// does anything become visible. A staging failure therefore leaves the
/// repository exactly as it was.
pub(super) struct StagedWrite {
    target: PathBuf,
    temporary: PathBuf,
}

impl StagedWrite {
    pub(super) fn stage(target: &Path, bytes: &[u8]) -> Result<Self, String> {
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
        Ok(Self {
            target: target.to_path_buf(),
            temporary,
        })
    }

    /// Rename over the target, so an interrupted write can never leave the
    /// file truncated: either the rename lands the whole new content or the
    /// original is untouched.
    pub(super) fn commit(self) -> Result<(), String> {
        match fs::rename(&self.temporary, &self.target) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&self.temporary);
                Err(format!("replace {}: {error}", self.target.display()))
            }
        }
    }

    pub(super) fn discard(self) {
        let _ = fs::remove_file(&self.temporary);
    }
}

pub(super) fn atomic_replace(target: &Path, bytes: &[u8]) -> Result<(), String> {
    StagedWrite::stage(target, bytes)?.commit()
}

pub(super) fn evidence_text(found: &[u8]) -> Value {
    if found.len() > MAX_EVIDENCE_BYTES {
        return Value::Null;
    }
    match std::str::from_utf8(found) {
        Ok(text) => json!(text),
        Err(_) => Value::Null,
    }
}

pub(super) fn quoted_evidence(found: &[u8]) -> String {
    match evidence_text(found) {
        Value::String(text) => format!(": {text:?}"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start_column: usize, end_column: usize) -> SpanAddress {
        SpanAddress {
            start_line: 1,
            start_column,
            end_line: 1,
            end_column,
        }
    }

    #[test]
    fn columns_address_line_content_and_never_the_terminator() {
        for text in ["ab\ncd\n", "ab\r\ncd\r\n", "ab\ncd"] {
            let index = LineIndex::build(text.as_bytes());
            // Column 3 is the exclusive end of a two-byte line; column 4 is
            // past it and must not reach into the terminator.
            assert!(index.resolve(&span(1, 3)).is_ok(), "{text:?}");
            assert!(index.resolve(&span(1, 4)).is_err(), "{text:?}");
            // A trailing terminator does not open a third line: whatever
            // follows the last `\n` is empty and not addressable.
            let error = index
                .resolve(&SpanAddress {
                    start_line: 3,
                    start_column: 1,
                    end_line: 3,
                    end_column: 2,
                })
                .unwrap_err();
            assert!(error.contains("has 2 line(s)"), "{text:?}: {error}");
        }
    }

    #[test]
    fn a_span_beyond_the_file_is_reported_with_the_actual_line_count() {
        let index = LineIndex::build(b"one\n");
        let error = index
            .resolve(&SpanAddress {
                start_line: 9,
                start_column: 1,
                end_line: 9,
                end_column: 2,
            })
            .unwrap_err();
        assert!(error.contains("has 1 line(s)"), "{error}");
    }

    /// The plan stage converts ast-grep byte offsets into addresses and the
    /// apply stage converts them back. A round trip that lost a byte would
    /// move every edit one column, so it is checked over multi-byte
    /// characters and CRLF, where a char-vs-byte confusion would show.
    #[test]
    fn byte_offsets_and_addresses_round_trip_through_multibyte_and_crlf_lines() {
        let source = "let 中文 = 1;\r\nlet retry_budget = 3;\n";
        let index = LineIndex::build(source.as_bytes());
        for needle in ["中文", "retry_budget", "1"] {
            let start = source.find(needle).expect("needle is present");
            let end = start + needle.len();
            let address = index.address(start, end).expect("addressable span");
            assert_eq!(
                index.resolve(&address).expect("resolves back"),
                (start, end),
                "{needle} at {}",
                address.label()
            );
        }
        // Byte 16 is the '\n' that ends line 1: a terminator is not an
        // addressable position, and saying so beats silently clamping.
        assert_eq!(source.as_bytes()[16], b'\n');
        assert!(index.address(16, 18).is_err());
    }

    #[test]
    fn back_to_front_rewriting_keeps_every_edit_at_its_resolved_offsets() {
        let before = b"aa bb cc".to_vec();
        assert_eq!(
            rewrite(&before, &[(0, 2, "AAAAA"), (6, 8, "C")]),
            b"AAAAA bb C"
        );
    }

    #[test]
    fn path_policy_refuses_traversal_and_generated_content() {
        assert!(normalize_relative("src/../secret.py", true).is_err());
        assert!(normalize_relative("node_modules/pkg/index.js", true).is_err());
        assert!(within_scope("backend/api.py", "backend"));
        assert!(!within_scope("frontend/api.ts", "backend"));
    }
}
