//! Absolute-path resolution for external tools (`rg`, `git`) launched
//! against a repository under analysis.
//!
//! Every caller here sets the child process's working directory to the
//! target/scanned repository (`.current_dir(target)` / `.current_dir(repo)`).
//! Handing `Command::new` a bare program name (`"rg"`, `"git"`) leaves
//! resolution of that name to the platform's own search rules, which on
//! Windows are documented to be able to consult the current directory
//! before `PATH`. A scanned repository that plants an `rg.exe` or `git.exe`
//! at its own root could then run in place of the real tool. `Command::new`
//! never performs any such search once it is handed a path instead of a
//! bare name — found or not — so every call site here resolves first and
//! only ever launches by absolute path.
//!
//! This mirrors the candidate list `doctor_adapter::tool_available` already
//! used to check tool presence; `locate` below is now the shared
//! implementation, so the two stay in step instead of drifting apart as
//! separate copies.

use std::env;
use std::path::{Path, PathBuf};

/// Candidate file names to test for `name`, in search order.
fn candidates(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
            name.to_string(),
        ]
    } else {
        vec![name.to_string()]
    }
}

/// Search an optional prefix directory, then every directory on `PATH`, for
/// the first candidate that exists as a file. Never consults the current
/// directory: only `prefix` and `PATH` are searched, and relative `PATH`
/// entries are skipped outright by `find_first` below.
pub(crate) fn locate(name: &str, prefix: Option<&Path>) -> Option<PathBuf> {
    let names = candidates(name);
    if let Some(prefix) = prefix {
        if let Some(found) = names
            .iter()
            .map(|candidate| prefix.join(candidate))
            .find(|full| full.is_file())
        {
            return Some(found);
        }
    }
    let path = env::var_os("PATH")?;
    find_first(&names, env::split_paths(&path))
}

/// The directory scan behind `locate`'s `PATH` search. A relative `PATH`
/// entry (`.`, an empty segment) is never searched: `is_file` would probe it
/// against this process's working directory, but the returned relative path
/// would later execute against the child's — the scanned repository this
/// module exists to keep out of tool resolution — so only absolute
/// directories are consulted and only absolute paths are ever returned.
fn find_first(names: &[String], dirs: impl Iterator<Item = PathBuf>) -> Option<PathBuf> {
    dirs.filter(|dir| dir.is_absolute()).find_map(|dir| {
        names
            .iter()
            .map(|candidate| dir.join(candidate))
            .find(|full| full.is_file())
    })
}

/// Resolve `name` (`"rg"` or `"git"`) to an absolute path before any
/// `Command::new` call site builds a command that will run against a
/// scanned repository. Checks `PATH` (plus the directory this executable
/// lives in, as a secondary, non-repository location a bundled tool might
/// sit in) and returns the match. When nothing matches either, still
/// returns an absolute path — anchored at the executable's directory, or
/// the system temp directory if even that cannot be determined — so the
/// eventual `Command::new` fails closed with a plain "not found" the same
/// way a missing bare name would have, rather than ever handing
/// `Command::new` a bare name that the OS could search for.
pub(crate) fn resolve(name: &str) -> PathBuf {
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    if let Some(found) = locate(name, exe_dir.as_deref()) {
        return found;
    }
    let anchor = exe_dir.unwrap_or_else(env::temp_dir);
    anchor.join(candidates(name).remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn path_search_skips_relative_entries() {
        let names = candidates("__code_intel_relative_probe__");
        // One planted directory per included copy of this module: the file
        // is compiled under several parents and their tests run in parallel
        // inside the same process.
        let unique = module_path!().replace("::", "-");
        let relative = Path::new("../../target").join(format!("tool-path-{unique}"));
        let absolute = env::current_dir().expect("cwd").join(&relative);
        fs::create_dir_all(&absolute).expect("create probe dir");
        for name in &names {
            fs::write(absolute.join(name), b"").expect("plant probe tool");
        }
        // The planted candidate is visible through the relative entry from
        // this process's working directory...
        assert!(relative.join(&names[0]).is_file());
        // ...yet the search must skip the relative entry outright: it would
        // exec against the child's working directory, not this one.
        assert_eq!(
            find_first(&names, std::iter::once(relative.clone())),
            None
        );
        // The same directory offered as an absolute entry is searched.
        assert!(find_first(&names, std::iter::once(absolute.clone()))
            .is_some_and(|found| found.is_absolute()));
        fs::remove_dir_all(&absolute).ok();
    }

    #[test]
    fn locate_never_returns_a_relative_path() {
        if let Some(found) = locate("git", None) {
            assert!(found.is_absolute(), "{found:?} is not absolute");
        }
    }

    #[test]
    fn resolve_always_returns_an_absolute_path() {
        assert!(resolve("git").is_absolute());
        assert!(resolve("rg").is_absolute());
        assert!(resolve("__code_intel_tool_that_does_not_exist__").is_absolute());
    }

    #[test]
    fn locate_does_not_find_a_made_up_tool_name() {
        assert_eq!(
            locate("__code_intel_tool_that_does_not_exist__", None),
            None
        );
    }
}
