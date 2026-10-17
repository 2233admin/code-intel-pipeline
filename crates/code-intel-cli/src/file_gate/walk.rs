//! Filesystem traversal and the external `rg`/`git` integration that backs
//! the `repository_ignored` gate. No gate logic lives here -- this module
//! only answers "what files exist" and "what does version control say about
//! them", both consumed by `mod.rs::evaluate`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

#[path = "../hardened_git.rs"]
mod hardened_git;
#[path = "../tool_path.rs"]
mod tool_path;

/// Every non-`.git`, non-symlink path under `repo`, relative and
/// forward-slash separated. `.git` is the one boundary this walker does not
/// even enumerate: it is version-control plumbing, not repository content,
/// and neither `sentrux scan` nor `sentrux dsm` ever treated it as a
/// candidate.
pub(crate) fn walk_candidates(
    root: &Path,
    directory: &Path,
    out: &mut Vec<String>,
) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // The directory vanished between enumeration and now (for example
            // a parallel test or tool created and removed a temp directory
            // under `target/` while this walk was in flight). Treat it as
            // empty rather than failing the whole scan: concurrent cleanup is
            // not a repository integrity problem. Other IO errors (permission,
            // hardware) still propagate.
            return Ok(());
        }
        Err(error) => return Err(format!("read {}: {error}", directory.display())),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("read {}: {error}", directory.display())),
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("inspect {}: {error}", path.display())),
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            walk_candidates(root, &path, out)?;
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("relativize {}: {error}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        out.push(relative);
    }
    Ok(())
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim()
        .trim_start_matches("./")
        .to_string()
}

/// `rg --files` (hidden files included, this repository's own `.gitignore`
/// honoured, parent/global/`.git/info/exclude` ignore sources disabled)
/// unioned with `git ls-files`, so a file is "governed visible" if either
/// ripgrep would list it or git already tracks it. `--no-require-git` keeps
/// the result identical whether or not `target` sits inside a git work tree
/// ("make DSM ignore handling deterministic", e68392d);
/// `RIPGREP_CONFIG_PATH` is stripped so a config file inside the scanned
/// tree cannot change what this check sees. Both `rg` and `git` are
/// launched by absolute path (`tool_path::resolve`, `hardened_git::command`)
/// rather than bare name, and `git`'s program-executing config keys are
/// disarmed -- see those modules for why.
///
/// An `Err` means the check could not run (`rg` missing, spawn failure,
/// abnormal exit). It is fatal for the whole gate (#410): silently skipping
/// `repository_ignored` would reclassify every ignored file as
/// `default_include` while the report still verified as success.
pub(crate) fn governed_visible_files(target: &Path) -> Result<BTreeSet<String>, String> {
    governed_visible_files_with(&tool_path::resolve("rg"), target)
}

fn governed_visible_files_with(rg: &Path, target: &Path) -> Result<BTreeSet<String>, String> {
    let output = Command::new(rg)
        .arg("--files")
        .args([
            "--hidden",
            "--no-require-git",
            "--no-ignore-parent",
            "--no-ignore-global",
            "--no-ignore-exclude",
        ])
        .env_remove("RIPGREP_CONFIG_PATH")
        .current_dir(target)
        .output()
        .map_err(|error| {
            format!(
                "repository_ignored gate cannot run: ripgrep (`rg`) failed to start at {}: {error}. \
                 Install ripgrep and ensure it is on PATH (`code-intel doctor` lists it as required)",
                rg.display()
            )
        })?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(format!(
            "repository_ignored gate cannot run: `rg --files` exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut visible = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(normalize_path)
        .collect::<BTreeSet<_>>();

    if let Ok(output) = hardened_git::command(target)
        .args(["ls-files", "-z"])
        .output()
    {
        if output.status.success() {
            for relative in String::from_utf8_lossy(&output.stdout).split('\0') {
                if !relative.is_empty() {
                    visible.insert(normalize_path(relative));
                }
            }
        }
    }
    Ok(visible)
}

#[cfg(test)]
mod governed_visible_tests {
    use super::*;

    #[test]
    fn missing_ripgrep_is_a_named_error_not_an_empty_exclusion() {
        let missing = std::env::temp_dir().join("__code_intel_rg_does_not_exist__");
        let error = governed_visible_files_with(&missing, &std::env::temp_dir())
            .expect_err("a missing rg must fail the gate, not silently disable repository_ignored");
        assert!(
            error.contains("repository_ignored gate cannot run"),
            "{error}"
        );
        assert!(error.contains("ripgrep"), "{error}");
    }
}
