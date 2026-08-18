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
//! Exit codes: 0 = clean, 1 = hits found, 74 = scan I/O failure.

use std::fs;
use std::path::Path;
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
            let name_start = i + 5;
            if name_start < bytes.len()
                && (bytes[name_start].is_ascii_alphabetic() || bytes[name_start] == b'_')
            {
                // Skip only a grammatically valid `$env:NAME` token.
                let mut j = name_start + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
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
            while j < bytes.len() {
                let c = bytes[j];
                if c == b'\\' {
                    if !last_segment.is_empty() {
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
            if last_segment == "code-intel-pipeline" {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// List git-tracked files matching the scan globs. Uses `git ls-files` with
/// the hardened wrapper so a scanned repository's `.git/config` cannot
/// inject hooks (same invariant as every other git call in this crate).
fn tracked_scan_files(repo: &Path) -> Result<Vec<String>, String> {
    let mut command = Command::new("git");
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("-C")
        .arg(repo)
        .arg("ls-files")
        .args(SCAN_GLOBS);
    let output = command
        .output()
        .map_err(|error| format!("run git ls-files for {}: {error}", repo.display()))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let status = output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "terminated".to_string());
        let suffix = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        return Err(format!(
            "git ls-files failed for {} (exit {status}){suffix}",
            repo.display()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .filter(|line| !line.is_empty())
        .collect())
}

/// Scan the repository rooted at `repo` (defaults to CWD when `None`).
pub(crate) fn scan(repo: &Path) -> Result<ScanResult, String> {
    let files = tracked_scan_files(repo)?;
    let mut hits = Vec::new();
    for file in &files {
        let path = repo.join(file);
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("read tracked file {}: {error}", path.display()))?;
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
    Ok(ScanResult {
        ok: hits.is_empty(),
        scanned_files: files.len(),
        hits,
    })
}

/// Run the scan and print the human/CI report; returns the process exit code
/// (0 clean, 1 hits). Mirrors the facade's console behavior.
pub(crate) fn run_and_report(repo: &Path) -> i32 {
    let result = match scan(repo) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("Hardcoded path scan: ERROR: {error}");
            return 74;
        }
    };
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
fn scan_json(result: &ScanResult) -> Value {
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
        None => match std::env::current_dir() {
            Ok(path) => path,
            Err(error) => return report_error(json, format!("resolve current directory: {error}")),
        },
    };
    if json {
        let result = match scan(&repo) {
            Ok(result) => result,
            Err(error) => return report_error(true, error),
        };
        println!("{}", scan_json(&result));
        return if result.ok { 0 } else { 1 };
    } else {
        return run_and_report(&repo);
    }
}

fn report_error(json_output: bool, error: String) -> i32 {
    if json_output {
        println!(
            "{}",
            json!({
                "ok": false,
                "error": {
                    "category": "local_tool_error",
                    "message": error,
                }
            })
        );
    } else {
        eprintln!("Hardcoded path scan: ERROR: {error}");
    }
    74
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
        assert!(!line_has_hit("dir: $env:_APPDATA\\temp"));
        assert!(line_has_hit("invalid: $env:1USERPROFILE"));
        assert!(line_has_hit("invalid: $env:-APPDATA"));
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
        assert!(line_has_hit("D:\\projects\\code-intel-pipeline"));
        assert!(line_has_hit("\"D:\\projects\\code-intel-pipeline\""));
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

    #[test]
    fn scan_fails_when_git_discovery_fails() {
        let repo = fixture_repo_path("not-a-repository");
        fs::create_dir_all(&repo).unwrap();

        let error = scan(&repo).unwrap_err();

        assert!(error.contains("git ls-files failed"), "{error}");
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn scan_fails_when_a_tracked_file_cannot_be_read() {
        let repo = initialized_fixture_repo("unreadable-tracked-file");
        let tracked = repo.join("tracked.md");
        fs::write(&tracked, "safe").unwrap();
        run_git(&repo, &["add", "tracked.md"]);
        fs::remove_file(&tracked).unwrap();

        let error = scan(&repo).unwrap_err();

        assert!(error.contains("read tracked file"), "{error}");
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn json_mode_returns_findings_exit_code() {
        let repo = initialized_fixture_repo("json-findings-exit");
        fs::write(repo.join("tracked.md"), "uses USERPROFILE directly").unwrap();
        run_git(&repo, &["add", "tracked.md"]);

        let exit = run_raw(&[repo.display().to_string(), "--json".to_string()]);

        assert_eq!(exit, 1);
        fs::remove_dir_all(repo).unwrap();
    }

    fn initialized_fixture_repo(name: &str) -> std::path::PathBuf {
        let repo = fixture_repo_path(name);
        fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "--quiet"]);
        repo
    }

    fn fixture_repo_path(name: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "code-intel-hardcoded-paths-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn run_git(repo: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?} failed with {status}");
    }
}
