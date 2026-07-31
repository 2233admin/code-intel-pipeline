//! Path and platform derivation for the doctor bootstrap probe.
//!
//! Mirrors the helpers the retired `legacy/tools/code-intel-platform.psm1`
//! exposed to `check-code-intel-tools.ps1`: platform resolution,
//! `Resolve-CodeIntelPath`, the home/data-root/bin triple, and the
//! platform-correct binary name.

use std::env;
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

/// `Get-CodeIntelPlatform`. `auto` resolves from the compile target.
pub(super) fn resolve_platform(requested: &str) -> Result<String, String> {
    match requested {
        "windows" | "macos" | "linux" => Ok(requested.to_string()),
        "auto" => {
            if cfg!(windows) {
                Ok("windows".into())
            } else if cfg!(target_os = "macos") {
                Ok("macos".into())
            } else if cfg!(target_os = "linux") {
                Ok("linux".into())
            } else {
                Err("Unsupported platform. Pass --platform windows|macos|linux.".into())
            }
        }
        other => Err(format!(
            "--platform must be auto|windows|macos|linux, got {other}"
        )),
    }
}

pub(super) fn binary_name(platform: &str) -> String {
    if platform == "windows" {
        "code-intel.exe".into()
    } else {
        "code-intel".into()
    }
}

/// The `paths` block of the observation: `Get-CodeIntelPaths`'s home,
/// dataRoot, bin and codeIntelHome, with the same env-var overrides.
pub(super) fn platform_paths(platform: &str, pipeline_root: &Path) -> Value {
    let home = home_directory();
    let data_root = data_root(platform, &home);
    let bin = match env::var("CODE_INTEL_BIN") {
        Ok(value) if !value.trim().is_empty() => resolve_code_intel_path(Path::new(&value)),
        _ => data_root.join("bin"),
    };
    let code_intel_home = match env::var("CODE_INTEL_HOME") {
        Ok(value) if !value.trim().is_empty() => resolve_code_intel_path(Path::new(&value)),
        _ => resolve_code_intel_path(pipeline_root),
    };
    json!({
        "home": display(&home),
        "dataRoot": display(&data_root),
        "bin": display(&bin),
        "codeIntelHome": display(&code_intel_home)
    })
}

/// `Get-CodeIntelDataRoot`: `CODE_INTEL_DATA_ROOT`, else the platform's
/// conventional per-user data location.
fn data_root(platform: &str, home: &Path) -> PathBuf {
    if let Ok(value) = env::var("CODE_INTEL_DATA_ROOT") {
        if !value.trim().is_empty() {
            return resolve_code_intel_path(Path::new(&value));
        }
    }
    match platform {
        "windows" => env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|base| !base.as_os_str().is_empty())
            .unwrap_or_else(|| home.join(".code-intel"))
            .join("code-intel"),
        "macos" => home
            .join("Library")
            .join("Application Support")
            .join("code-intel"),
        _ => env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|base| !base.as_os_str().is_empty())
            .unwrap_or_else(|| home.join(".local").join("share"))
            .join("code-intel"),
    }
}

pub(super) fn home_directory() -> PathBuf {
    let raw = if cfg!(windows) {
        env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"))
    } else {
        env::var_os("HOME")
    };
    raw.map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| resolve_code_intel_path(&path))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `Resolve-CodeIntelPath`: the on-disk absolute path when it exists, an
/// absolute lexically-normalized path when it does not. Windows verbatim
/// (`\\?\`) prefixes are stripped so the value stays comparable to the path
/// strings every other producer in this pipeline emits.
pub(super) fn resolve_code_intel_path(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(resolved) => strip_verbatim(&resolved),
        Err(_) => normalize(&absolute_from_cwd(path)),
    }
}

fn absolute_from_cwd(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Lexical `.`/`..` collapse, matching `[Path]::GetFullPath` for paths that do
/// not exist on disk (where `canonicalize` cannot help).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir) | Some(Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn strip_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_path_buf())
}

pub(super) fn trim_trailing_separator(value: &str) -> String {
    let trimmed = value.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        value.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_resolution_rejects_unknown_values() {
        assert_eq!(resolve_platform("linux").unwrap(), "linux");
        assert!(resolve_platform("auto").is_ok());
        assert!(resolve_platform("solaris").is_err());
    }

    #[test]
    fn binary_name_is_platform_correct() {
        assert_eq!(binary_name("windows"), "code-intel.exe");
        assert_eq!(binary_name("linux"), "code-intel");
        assert_eq!(binary_name("macos"), "code-intel");
    }

    #[test]
    fn path_normalization_collapses_dot_segments_for_absent_paths() {
        assert_eq!(
            normalize(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
    }

    #[test]
    fn trailing_separators_are_trimmed_for_config_path_comparison() {
        assert_eq!(trim_trailing_separator("C:/repo/"), "C:/repo");
        assert_eq!(trim_trailing_separator(r"C:\repo\"), r"C:\repo");
        // A bare separator must survive: trimming it to "" would make every
        // configured path compare equal to the filesystem root.
        assert_eq!(trim_trailing_separator("/"), "/");
    }

    #[test]
    fn resolved_paths_are_absolute_and_carry_no_verbatim_prefix() {
        let resolved = resolve_code_intel_path(Path::new("."));
        assert!(resolved.is_absolute());
        assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
    }
}
