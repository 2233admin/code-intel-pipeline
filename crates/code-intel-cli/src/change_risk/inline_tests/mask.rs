//! Blanking of comments and string/char literals so brace counting only
//! sees code.

use super::{find, is_ident};

/// Copy of `source` with comments, string literals (raw included) and char
/// literals blanked to spaces, keeping byte offsets and newlines so braces
/// can be counted and line numbers read off. A `'` that starts a lifetime
/// is left alone.
pub(super) fn mask_non_code(source: &[u8]) -> Vec<u8> {
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
