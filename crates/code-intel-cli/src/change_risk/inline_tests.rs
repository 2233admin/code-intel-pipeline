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

use super::signals::{is_source_file, is_test_file};
use super::{git, FileDiff, InlineTests};

/// New-side line range `(start, count)` of one `-U0` hunk.
type Hunk = (usize, usize);
/// Inclusive 1-based line range of one inline test module.
type LineRange = (usize, usize);

const CFG_TEST: &[u8] = b"#[cfg(test)]";

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
    let mut hunks = parse_hunks(&diff);
    hunks.retain(|path, _| candidates.contains(path.as_str()));
    let paths: Vec<&str> = hunks.keys().map(String::as_str).collect();
    git::read_blobs_at(repo, tip, &paths)
        .into_iter()
        .filter(|(path, bytes)| {
            std::str::from_utf8(bytes)
                .is_ok_and(|text| hunks_hit(&inline_test_ranges(text), &hunks[path]))
        })
        .map(|(path, _)| path)
        .collect()
}

/// The inline-test question only applies to non-test Rust source files.
fn is_inline_candidate(path: &str) -> bool {
    path.ends_with(".rs") && is_source_file(path) && !is_test_file(path)
}

pub(super) fn touches_inline_tests(path: &str, inline: &InlineTests) -> bool {
    inline.contains(path) && is_inline_candidate(path)
}

/// New-side hunks per path from `git diff -U0 --no-prefix` output.
pub(super) fn parse_hunks(diff: &str) -> BTreeMap<String, Vec<Hunk>> {
    let mut hunks: BTreeMap<String, Vec<Hunk>> = BTreeMap::new();
    let mut in_header = false;
    let mut current: Option<String> = None;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            in_header = true;
            current = None;
        } else if in_header {
            if let Some(path) = line.strip_prefix("+++ ") {
                current = (path != "/dev/null").then(|| path.trim_end_matches('\t').to_string());
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
    hunks
}

/// `@@ -a,b +c,d @@` -> `(c, d)`; `d` defaults to 1.
fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let new_side = rest.split(" +").nth(1)?.split(' ').next()?;
    let (start, count) = match new_side.split_once(',') {
        Some((start, count)) => (start, count),
        None => (new_side, "1"),
    };
    Some((start.parse().ok()?, count.parse().ok()?))
}

/// A hunk hits a module when its new-side lines `[c, c+d-1]` intersect the
/// module; a pure deletion (`d == 0`) counts when line `c` is inside it.
pub(super) fn hunks_hit(ranges: &[LineRange], hunks: &[Hunk]) -> bool {
    hunks.iter().any(|&(start, count)| {
        let last = if count == 0 { start } else { start + count - 1 };
        ranges.iter().any(|&(from, to)| start <= to && last >= from)
    })
}

/// Line ranges of every `#[cfg(test)]`-gated `mod <name> { … }` block,
/// from the `mod` line to the line of its matching `}`.
pub(super) fn inline_test_ranges(source: &str) -> Vec<LineRange> {
    let code = mask_non_code(source.as_bytes());
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(found) = find(&code, CFG_TEST, from) {
        from = found + CFG_TEST.len();
        if let Some((open, close)) = test_module_span(&code, from) {
            ranges.push((line_of(&code, open), line_of(&code, close)));
        }
    }
    ranges
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
        if !code[at..].starts_with(b"#[") {
            break;
        }
        at = skip_attribute(code, at)?;
    }
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

/// Copy of `source` with comments, string literals (raw included) and char
/// literals blanked to spaces, keeping byte offsets and newlines so braces
/// can be counted and line numbers read off. A `'` that starts a lifetime
/// is left alone.
fn mask_non_code(source: &[u8]) -> Vec<u8> {
    let mut out = source.to_vec();
    let mut at = 0;
    while at < source.len() {
        let Some(end) = literal_end(source, at) else {
            at += 1;
            continue;
        };
        for byte in &mut out[at..end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
        at = end;
    }
    out
}

/// Where the comment or literal starting at `at` ends, `None` if one does
/// not start there.
fn literal_end(source: &[u8], at: usize) -> Option<usize> {
    let rest = &source[at..];
    match rest[0] {
        b'/' if rest.get(1) == Some(&b'/') => Some(find(source, b"\n", at).unwrap_or(source.len())),
        b'/' if rest.get(1) == Some(&b'*') => Some(block_comment_end(source, at)),
        b'"' => Some(string_end(source, at + 1)),
        b'r' if starts_raw_string(source, at) => raw_string_end(source, at),
        b'\'' => char_literal_end(source, at),
        _ => None,
    }
}

/// Rust block comments nest.
fn block_comment_end(source: &[u8], at: usize) -> usize {
    let mut depth = 1;
    let mut cursor = at + 2;
    while cursor < source.len() && depth > 0 {
        if source[cursor..].starts_with(b"/*") {
            depth += 1;
            cursor += 2;
        } else if source[cursor..].starts_with(b"*/") {
            depth -= 1;
            cursor += 2;
        } else {
            cursor += 1;
        }
    }
    cursor.min(source.len())
}

/// `from` is just past the opening quote.
fn string_end(source: &[u8], mut from: usize) -> usize {
    while from < source.len() {
        match source[from] {
            b'\\' => from += 2,
            b'"' => return from + 1,
            _ => from += 1,
        }
    }
    source.len()
}

/// An `r` opens a raw string only when it is not the tail of an identifier
/// (`b` before it is the byte-string prefix).
fn starts_raw_string(source: &[u8], at: usize) -> bool {
    let before = |back: usize| at.checked_sub(back).map(|index| source[index]);
    let boundary = match before(1) {
        None => true,
        Some(b'b') => !before(2).is_some_and(is_ident),
        Some(byte) => !is_ident(byte),
    };
    boundary && {
        let hashes = source[at + 1..]
            .iter()
            .take_while(|byte| **byte == b'#')
            .count();
        source.get(at + 1 + hashes) == Some(&b'"')
    }
}

fn raw_string_end(source: &[u8], at: usize) -> Option<usize> {
    let hashes = source[at + 1..]
        .iter()
        .take_while(|byte| **byte == b'#')
        .count();
    let mut closing = vec![b'"'];
    closing.resize(1 + hashes, b'#');
    let body = at + 2 + hashes;
    Some(find(source, &closing, body).map_or(source.len(), |found| found + closing.len()))
}

/// `'x'` / `'\n'` / `'\u{..}'` end; `None` for a lifetime like `'a`.
fn char_literal_end(source: &[u8], at: usize) -> Option<usize> {
    let first = *source.get(at + 1)?;
    if first == b'\\' {
        // Skip the escaped byte itself: `'\''` has a quote right after `\`.
        let close = find(source, b"'", at + 3)?;
        return Some(close + 1);
    }
    let width = match first {
        0..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    };
    (source.get(at + 1 + width) == Some(&b'\'')).then_some(at + width + 2)
}
