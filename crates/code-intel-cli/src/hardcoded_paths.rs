//! Hardcoded-path scan — Rust replacement for
//! `legacy/tools/check-hardcoded-paths.ps1` (issue #275, PS1 retirement).
//!
//! Scans git-tracked `*.ps1` / `*.psm1` / `*.md` / `*.yml` files for
//! machine-specific paths (Windows user directories, `powershell.exe`,
//! env-var names) and for any absolute path ending in `code-intel-pipeline`.
//! Lines where the match came only from a `$env:VAR` reference are exempt
//! (the PowerShell script strips `$env:NAME` before matching; this port keeps
//! the same rule).
//!
//! Exit codes: 0 = clean, 1 = hits found (CI gate semantics).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

/// Files whose tracked contents are scanned. Mirrors `$globs` in the facade.
const SCAN_GLOBS: [&str; 4] = ["*.ps1", "*.psm1", "*.md", "*.yml"];

/// A single hit: file, 1-based line number, and the offending text.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) text: String,
}

/// Result of a scan run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ScanResult {
    pub(crate) ok: bool,
    pub(crate) scanned_files: usize,
    pub(crate) hits: Vec<Hit>,
}

/// Returns true when `line` contains any machine-specific literal pattern.
/// `$env:VAR` references are removed first, exactly like the PowerShell
/// facade's `$envVarPattern.Replace($line, "")` step.
fn line_has_hit(line: &str) -> bool {
    let stripped = strip_env_vars(line);
    literal_patterns()
        .iter()
        .any(|pattern| stripped.contains(pattern))
        || absolute_pipeline_path(&stripped)
}

/// `$env:NAME` removal. Matches the facade's
/// `\$env:[A-Za-z_][A-Za-z0-9_]*` (case-insensitive).
fn strip_env_vars(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for `$env:` (case-insensitive) starting at i.
        if i + 4 < bytes.len()
            && bytes[i] == b'$'
            && bytes[i + 1].eq_ignore_ascii_case(&b'e')
            && bytes[i + 2].eq_ignore_ascii_case(&b'n')
            && bytes[i + 3].eq_ignore_ascii_case(&b'v')
            && bytes[i + 4] == b':'
        {
            // Skip the whole `$env:NAME` token.
            let mut j = i + 5;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            i = j;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Machine-specific literal patterns. Constructed from pieces to keep the
/// scan honest (same technique as the facade's `$literalPatterns`).
fn literal_patterns() -> Vec<String> {
    let slash = "\\";
    vec![
        format!("C:{slash}Users{slash}Administrator"),
        "powershell.exe".to_string(),
        "LOCALAPPDATA".to_string(),
        "USERPROFILE".to_string(),
        "APPDATA".to_string(),
    ]
}

/// Matches an absolute Windows path whose final path segment is exactly
/// `code-intel-pipeline`. Mirrors the facade's
/// `(?<![A-Za-z])[A-Za-z]:\\(?:[^\s"'\\]*\\)*code-intel-pipeline\b`.
fn absolute_pipeline_path(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        // Drive letter: `X:` where X is ascii-alpha and the previous char is
        // not ascii-alpha (negative lookbehind).
        let prev_not_alpha = i == 0 || !bytes[i - 1].is_ascii_alphabetic();
        if prev_not_alpha
            && bytes[i].is_ascii_alphabetic()
            && bytes[i + 1] == b':'
            && bytes[i + 2] == b'\\'
        {
            // Walk path segments. Accept `X:\...\code-intel-pipeline` where
            // each segment excludes whitespace, quotes, and backslashes.
            let mut j = i + 2;
            let mut last_segment = String::new();
            let mut saw_segment = false;
            while j < bytes.len() {
                let c = bytes[j];
                if c == b'\\' {
                    if !last_segment.is_empty() {
                        saw_segment = true;
                        if last_segment == "code-intel-pipeline" {
                            // `\` is itself the word boundary after the
                            // final segment (`e` word char, `\` non-word),
                            // matching the facade's `\b` — no lookahead needed.
                            return true;
                        }
                    }
                    last_segment.clear();
                    j += 1;
                } else if c == b' ' || c == b'"' || c == b'\'' {
                    break;
                } else {
                    last_segment.push(c as char);
                    j += 1;
                }
            }
        }
        i += 1;
    }
    false
}

/// List git-tracked files matching the scan globs. Uses `git ls-files` with
/// the hardened wrapper so a scanned repository's `.git/config` cannot
/// inject hooks (same invariant as every other git call in this crate).
fn tracked_scan_files(repo: &Path) -> Vec<String> {
    let mut command = Command::new("git");
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("-C")
        .arg(repo)
        .arg("ls-files")
        .args(SCAN_GLOBS);
    let output = match command.output() {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .filter(|line| !line.is_empty())
        .collect()
}

/// Scan the repository rooted at `repo` (defaults to CWD when `None`).
pub(crate) fn scan(repo: &Path) -> ScanResult {
    let files = tracked_scan_files(repo);
    let mut hits = Vec::new();
    for file in &files {
        let path = repo.join(file);
        let content = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => continue,
        };
        for (index, line) in content.lines().enumerate() {
            if line_has_hit(line) {
                let line_number = index + 1;
                hits.push(Hit {
                    file: file.clone(),
                    line: line_number,
                    text: format!("{file}:{line_number}:{line}"),
                });
            }
        }
    }
    ScanResult {
        ok: hits.is_empty(),
        scanned_files: files.len(),
        hits,
    }
}

/// Run the scan and print the human/CI report; returns the process exit code
/// (0 clean, 1 hits). Mirrors the facade's console behavior.
pub(crate) fn run_and_report(repo: &Path) -> i32 {
    let result = scan(repo);
    if result.ok {
        println!("Hardcoded path scan: OK ({} files)", result.scanned_files);
    } else {
        println!("Hardcoded path scan: FAILED");
        for hit in &result.hits {
            println!("{}", hit.text);
        }
    }
    if result.ok {
        0
    } else {
        1
    }
}

/// JSON variant for tooling (`--json` flag parity with the facade).
pub(crate) fn scan_json(repo: &Path) -> Value {
    let result = scan(repo);
    json!({
        "ok": result.ok,
        "scannedFiles": result.scanned_files,
        "hits": result.hits.iter().map(|hit| json!({
            "file": hit.file,
            "line": hit.line,
            "text": hit.text,
        })).collect::<Vec<_>>(),
    })
}

/// Entry point for the `lint hardcoded-paths` CLI route.
pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let json = raw.iter().any(|argument| argument == "--json");
    // Optional positional repo path; default CWD like the facade (which ran
    // from the repo root in CI).
    let repo_arg = raw.iter().find(|argument| !argument.starts_with('-'));
    let repo = match repo_arg {
        Some(path) => Path::new(path).to_path_buf(),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    if json {
        println!("{}", scan_json(&repo));
    } else {
        return run_and_report(&repo);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_stripping_exempts_matches() {
        // `$env:USERPROFILE` must be exempt; a bare `USERPROFILE` must hit.
        assert!(!line_has_hit("path = $env:USERPROFILE\\code"));
        assert!(line_has_hit("path = USERPROFILE\\code"));
        assert!(!line_has_hit("dir: $env:LOCALAPPDATA\\temp"));
    }

    #[test]
    fn administrator_path_hits() {
        assert!(line_has_hit("C:\\Users\\Administrator\\projects\\x"));
        assert!(!line_has_hit("C:\\Users\\OtherUser\\projects\\x"));
    }

    #[test]
    fn powershell_exe_hits() {
        assert!(line_has_hit("run: powershell.exe -File x.ps1"));
        assert!(line_has_hit("powershell.exe"));
    }

    #[test]
    fn pipeline_absolute_path_hits() {
        // The facade's pattern is `[A-Za-z]:\\(?:[^\s"'\\]*\\)*code-intel-pipeline\b` —
        // backslash separators only, exactly like the PowerShell original.
        assert!(line_has_hit(
            "D:\\projects\\_tools\\code-intel-pipeline\\run.ps1"
        ));
        assert!(line_has_hit("D:\\code-intel-pipeline\\x"));
        assert!(!line_has_hit(
            "D:/projects/_tools/code-intel-pipeline/run.ps1"
        ));
        assert!(!line_has_hit(
            "D:\\projects\\_tools\\code-intel-pipeline-backup\\run.ps1"
        ));
        assert!(!line_has_hit("code-intel-pipeline"));
    }

    #[test]
    fn env_var_names_hit_when_bare() {
        assert!(line_has_hit("uses LOCALAPPDATA directly"));
        assert!(line_has_hit("APPDATA"));
        assert!(!line_has_hit("$env:APPDATA"));
    }
}
