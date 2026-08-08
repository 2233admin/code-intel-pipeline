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
/// `None` means the check could not run (`rg` missing, spawn failure) --
/// callers must treat that as "cannot evaluate", not "everything ignored":
/// `repository_ignored` never excludes anything when this returns `None`.
pub(crate) fn governed_visible_files(target: &Path) -> Option<BTreeSet<String>> {
    let output = Command::new(tool_path::resolve("rg"))
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
        .ok()?;
    if !output.status.success() && output.status.code() != Some(1) {
        return None;
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
    Some(visible)
}
