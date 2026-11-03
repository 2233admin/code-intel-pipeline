//! Which changed Rust source files touched an inline `#[cfg(test)] mod`
//! block. The path rules in `signals::is_test_file` cannot see a test
//! module that lives in the same file as the code it tests, so a change
//! that only adds a test next to its fix scored as "source changed, no
//! tests touched" (issue #504).
//!
//! The git half runs one `git diff -U0` and one `git cat-file --batch` per
//! scored item; everything else here is pure. Any failure degrades to "not
//! touched", the same direction `walk_log` degrades in.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

mod mask;

use mask::mask_non_code;

use super::signals::{is_source_file, is_test_file};
use super::{git, FileDiff, InlineTests};

/// Line span `(start, count)` of one side of a `-U0` hunk.
type Span = (usize, usize);
/// `(old side, new side)` of one `-U0` hunk.
type Hunk = (Span, Span);
/// Inclusive 1-based line range of one inline test module.
type LineRange = (usize, usize);

/// The paths among `files` whose diff over `range` hits an inline test
/// module in their `tip` version.
pub(crate) fn touched_inline_test_files(
    repo: &Path,
    range: &str,
    tip: &str,
    files: &[FileDiff],
) -> InlineTests {
    let candidates: BTreeSet<&str> = files
        .iter()
        .map(|(_, _, path)| path.as_str())
        .filter(|path| is_inline_candidate(path))
        .collect();
    if candidates.is_empty() {
        return InlineTests::new();
    }
    let Some(diff) = git::unified_zero_diff(repo, range) else {
        return InlineTests::new();
    };
    let (mut hunks, deleted) = parse_diff(&diff);
    hunks.retain(|path, _| candidates.contains(path.as_str()));
    let paths: Vec<&str> = hunks.keys().map(String::as_str).collect();
    let new_ranges = ranges_by_path(git::read_blobs_at(repo, tip, &paths));
    // Removed lines (a pure deletion has no new-side lines at all) are
    // matched against the module spans of the old side instead.
    let deleting: Vec<&str> = hunks
        .iter()
        .filter(|(_, list)| list.iter().any(|((_, count), _)| *count > 0))
        .map(|(path, _)| path.as_str())
        .collect();
    let old_ranges = match git::range_base(repo, range) {
        Some(base) if !deleting.is_empty() => {
            ranges_by_path(git::read_blobs_at(repo, &base, &deleting))
        }
        _ => BTreeMap::new(),
    };
    let none = Vec::new();
    hunks
        .iter()
        .filter(|(path, list)| {
            // Only a path the diff header marks `+++ /dev/null` may lack a
            // tip version; any other unreadable tip stays untouched.
            if !deleted.contains(path.as_str()) && !new_ranges.contains_key(path.as_str()) {
                return false;
            }
            hunks_hit(
                new_ranges.get(path.as_str()).unwrap_or(&none),
                old_ranges.get(path.as_str()).unwrap_or(&none),
                list,
            )
        })
        .map(|(path, _)| path.clone())
        .collect()
}

fn ranges_by_path(blobs: Vec<(String, Vec<u8>)>) -> BTreeMap<String, Vec<LineRange>> {
    blobs
        .into_iter()
        .filter_map(|(path, bytes)| {
            let text = std::str::from_utf8(&bytes).ok()?;
            Some((path, inline_test_ranges(text)))
        })
        .collect()
}

/// The inline-test question only applies to non-test Rust source files.
fn is_inline_candidate(path: &str) -> bool {
    path.ends_with(".rs") && is_source_file(path) && !is_test_file(path)
}

pub(super) fn touches_inline_tests(path: &str, inline: &InlineTests) -> bool {
    inline.contains(path) && is_inline_candidate(path)
}

/// Hunks per path from `git diff -U0 --no-prefix` output.
#[cfg(test)]
pub(super) fn parse_hunks(diff: &str) -> BTreeMap<String, Vec<Hunk>> {
    parse_diff(diff).0
}

/// Hunks per path, plus the paths whose header says `+++ /dev/null`.
fn parse_diff(diff: &str) -> (BTreeMap<String, Vec<Hunk>>, BTreeSet<String>) {
    let mut deleted = BTreeSet::new();
    let mut hunks: BTreeMap<String, Vec<Hunk>> = BTreeMap::new();
    let mut in_header = false;
    let mut current: Option<String> = None;
    let mut old_path: Option<String> = None;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            in_header = true;
            current = None;
            old_path = None;
        } else if in_header {
            if let Some(path) = line.strip_prefix("--- ") {
                old_path = Some(path.trim_end_matches('\t').to_string());
            } else if let Some(path) = line.strip_prefix("+++ ") {
                // A deleted file keys its hunks by the old path.
                current = if path == "/dev/null" {
                    let old = old_path.take().filter(|old| old != "/dev/null");
                    deleted.extend(old.clone());
                    old
                } else {
                    Some(path.trim_end_matches('\t').to_string())
                };
            } else if line.starts_with("@@ ") {
                in_header = false;
            }
        }
        // Not `else if`: the first `@@` line is also a hunk header.
        if !in_header {
            if let (Some(path), Some(hunk)) = (&current, parse_hunk_header(line)) {
                hunks.entry(path.clone()).or_default().push(hunk);
            }
        }
    }
    (hunks, deleted)
}

/// `@@ -a,b +c,d @@` -> `((a, b), (c, d))`; a missing count defaults to 1.
fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (old_side, rest) = rest.split_once(" +")?;
    Some((parse_span(old_side)?, parse_span(rest.split(' ').next()?)?))
}

fn parse_span(text: &str) -> Option<Span> {
    let (start, count) = text.split_once(',').unwrap_or((text, "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
}

/// A hunk hits a module when its added lines intersect `new_ranges` (new
/// side) or its removed lines intersect `old_ranges` (old side). A pure
/// deletion has no new-side lines, so only the old side can match it.
pub(super) fn hunks_hit(
    new_ranges: &[LineRange],
    old_ranges: &[LineRange],
    hunks: &[Hunk],
) -> bool {
    let overlaps = |(start, count): Span, ranges: &[LineRange]| {
        count > 0
            && ranges
                .iter()
                .any(|&(from, to)| start <= to && start + count - 1 >= from)
    };
    hunks
        .iter()
        .any(|&(old, new)| overlaps(new, new_ranges) || overlaps(old, old_ranges))
}

/// Line ranges of every `#[cfg(test)]`-gated `mod <name> { … }` block,
/// from the `mod` line to the line of its matching `}`.
pub(super) fn inline_test_ranges(source: &str) -> Vec<LineRange> {
    let code = mask_non_code(source.as_bytes());
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(found) = find(&code, b"#", from) {
        from = found + 1;
        let Some(end) = cfg_test_end(&code, found) else {
            continue;
        };
        from = end;
        if let Some((open, close)) = test_module_span(&code, end) {
            ranges.push((line_of(&code, open), line_of(&code, close)));
        }
    }
    ranges
}

/// Offset just past a `#[cfg(test)]` starting at `at`; ASCII whitespace is
/// allowed between its tokens, anything but a bare `test` predicate is not.
fn cfg_test_end(code: &[u8], mut at: usize) -> Option<usize> {
    for token in [&b"#"[..], b"[", b"cfg", b"(", b"test", b")", b"]"] {
        at = skip_whitespace(code, at);
        if !code[at..].starts_with(token) {
            return None;
        }
        at += token.len();
    }
    Some(at)
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

fn line_of(code: &[u8], offset: usize) -> usize {
    1 + code[..offset].iter().filter(|byte| **byte == b'\n').count()
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn skip_whitespace(code: &[u8], mut at: usize) -> usize {
    while code.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    at
}

/// After a `#[cfg(test)]` ending at `at`: skip further attributes, then
/// require `mod <name> {`. Returns the byte offsets of the `mod` keyword
/// and of the matching `}`.
fn test_module_span(code: &[u8], mut at: usize) -> Option<(usize, usize)> {
    loop {
        at = skip_whitespace(code, at);
        if code.get(at) != Some(&b'#') {
            break;
        }
        // `#` and `[` are separate tokens: whitespace may sit between them.
        let bracket = skip_whitespace(code, at + 1);
        if code.get(bracket) != Some(&b'[') {
            break;
        }
        at = skip_attribute(code, at)?;
    }
    at = skip_visibility(code, at);
    let keyword = at;
    if !code[at..].starts_with(b"mod") || code.get(at + 3).is_none_or(|byte| is_ident(*byte)) {
        return None;
    }
    at = skip_whitespace(code, at + 3);
    let name_start = at;
    while code.get(at).is_some_and(|byte| is_ident(*byte)) {
        at += 1;
    }
    at = skip_whitespace(code, at);
    if at == name_start || code.get(at) != Some(&b'{') {
        return None;
    }
    Some((keyword, matching_brace(code, at)?))
}

/// Skip an optional `pub` / `pub(crate)` / `pub(in path)` and the
/// whitespace after it; `at` unchanged when there is none.
fn skip_visibility(code: &[u8], at: usize) -> usize {
    if !code[at..].starts_with(b"pub") || code.get(at + 3).is_some_and(|byte| is_ident(*byte)) {
        return at;
    }
    let after = skip_whitespace(code, at + 3);
    if code.get(after) != Some(&b'(') {
        return after;
    }
    let mut depth = 0usize;
    for (offset, byte) in code[after..].iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return skip_whitespace(code, after + offset + 1);
                }
            }
            _ => {}
        }
    }
    at
}

/// Offset just past the `]` closing the attribute that starts at `at`.
fn skip_attribute(code: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in code[at..].iter().enumerate() {
        match byte {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at + offset + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn matching_brace(code: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in code[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}
