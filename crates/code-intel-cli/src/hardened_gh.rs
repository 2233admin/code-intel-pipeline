//! `gh` (GitHub CLI) launched on the operator's behalf by `friction publish`
//! and `friction sync`, the first code paths in this crate that ever call
//! `gh`. Same resolution discipline as `hardened_git::command`: `gh` is
//! resolved to an absolute path (`tool_path::resolve`) rather than handed to
//! `Command::new` as a bare name, so a scanned repository cannot shadow it
//! with a planted executable earlier on a search path Windows would
//! otherwise consult.
//!
//! Callers must pass user-authored text (an issue title or body) through a
//! file (`--body-file`), never inline on argv: `gh`'s own argv becomes
//! visible to every other process on the machine for the life of the child,
//! and inline interpolation risks argument-injection if the text itself
//! starts with `-`.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "tool_path.rs"]
mod tool_path;

/// Resolve `gh` to an absolute path. See `tool_path::resolve` for why a bare
/// name is never handed to `Command::new`.
pub(crate) fn resolve() -> PathBuf {
    tool_path::resolve("gh")
}

/// A `gh` command rooted at `repo`, launched by absolute path.
pub(crate) fn command(repo: &Path) -> Command {
    let mut command = Command::new(resolve());
    command.current_dir(repo);
    command
}

/// Prefixes `gh`/GitHub issue tokens and PATs use, per
/// <https://github.blog/2021-04-05-behind-githubs-new-authentication-token-formats/>.
const TOKEN_PREFIXES: [&str; 5] = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"];

/// Scrub GitHub token-shaped substrings out of text before it is printed or
/// written to a friction entry: `gh` output (an error message, a `--json`
/// dump) can echo an ambient `GH_TOKEN`/`GITHUB_TOKEN` back, and a friction
/// entry is meant to be committed to the repository.
pub(crate) fn redact(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some(prefix) = TOKEN_PREFIXES
            .iter()
            .find(|prefix| text[index..].starts_with(**prefix))
        {
            let start = index;
            let mut end = index + prefix.len();
            while end < bytes.len() && is_token_byte(bytes[end]) {
                end += 1;
            }
            // A real token body runs well past the prefix; a bare prefix
            // followed by ordinary prose (e.g. "ghost_" is not "gho_st")
            // is left alone rather than partially swallowed.
            if end - start >= prefix.len() + 20 {
                out.push_str("[REDACTED_GITHUB_TOKEN]");
                index = end;
                continue;
            }
        }
        let char_len = text[index..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&text[index..index + char_len]);
        index += char_len;
    }
    out
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_always_returns_an_absolute_path() {
        assert!(resolve().is_absolute());
    }

    #[test]
    fn redacts_a_token_shaped_run_after_a_known_prefix() {
        let token = "ghp_".to_string() + &"a".repeat(36);
        let text = format!("gh: authenticated as x using token {token}\n");
        let redacted = redact(&text);
        assert!(!redacted.contains(&token));
        assert!(redacted.contains("[REDACTED_GITHUB_TOKEN]"));
    }

    #[test]
    fn leaves_ordinary_prose_starting_with_a_prefix_letters_alone() {
        let text = "ghost_writer wrote gho_ short and that's all";
        assert_eq!(redact(text), text);
    }

    #[test]
    fn leaves_text_with_no_token_unchanged() {
        let text = "issue #42 created: https://github.com/example/repo/issues/42";
        assert_eq!(redact(text), text);
    }
}
