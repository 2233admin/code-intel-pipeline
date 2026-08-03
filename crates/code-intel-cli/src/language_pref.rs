//! Documentation language preference (issue #155).
//!
//! Decides which language *human-readable* views render in -- `hospital.md`,
//! `surgery-plan.md`, `summary.md`, `understanding.md`, Repowise docs, and
//! command-line summaries. First match wins:
//!
//! ```text
//! --language <code>  >  project config  >  user config  >  system locale  >  en
//! ```
//!
//! This module never touches machine-readable output. Every `schema`,
//! `type`, field name, and enum/status value stays English regardless of the
//! resolved language (issue #101: all artifacts are machine-first; the human
//! view is a derived, language-tagged projection of the same data). Nothing
//! here adds a language beyond the `zh`/`en` the pipeline already handles.
//!
//! Every tier is a plain, fallible read that falls through silently to the
//! next one on any absence or error -- there is no failure mode a caller
//! must handle, and nothing here ever prompts or blocks. The interactive
//! prompt (TTY-gated, install time only) lives in
//! `legacy/install-code-intel-pipeline.ps1`; this module only resolves and
//! persists, it never asks.

use std::path::{Path, PathBuf};
use std::{env, fs, io};

use serde_json::Value;

/// Which tier of the precedence chain produced the resolved language.
/// Exercised by this module's own precedence tests; not surfaced through the
/// CLI today (there is no `language show`), so `#[allow(dead_code)]` keeps a
/// plain non-test build quiet about the field only tests read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Source {
    Flag,
    Project,
    User,
    Locale,
    Default,
}

#[allow(dead_code)]
pub(crate) struct Resolution {
    pub(crate) language: String,
    pub(crate) source: Source,
}

/// Resolve the effective language for `repo` (or, with no repo, whatever a
/// caller like `route plan` can still see: user config, then locale, then
/// `en`). `explicit` is whatever `--language <code>` the caller was given,
/// if any -- it always wins and is never validated against a known-language
/// set, matching the flag's existing, already-unvalidated behavior.
pub(crate) fn resolve(explicit: Option<&str>, repo: Option<&Path>) -> Resolution {
    resolve_from(
        explicit,
        repo,
        &user_config_path(),
        system_locale_language().as_deref(),
    )
}

/// The precedence chain over injected values, with no I/O of its own beyond
/// the two config-file reads. Split out from `resolve` so tests can exercise
/// every tier deterministically: a real environment's `LANG`/`PSUICulture`
/// and real `%LOCALAPPDATA%`/`HOME` are neither mutated (parallel `cargo
/// test` runs share one process's environment table) nor read (a test must
/// never depend on, or leak into, the machine's actual user config).
fn resolve_from(
    explicit: Option<&str>,
    repo: Option<&Path>,
    user_config: &Path,
    locale: Option<&str>,
) -> Resolution {
    if let Some(value) = non_empty(explicit) {
        return Resolution {
            language: value.to_string(),
            source: Source::Flag,
        };
    }
    if let Some(repo) = repo {
        if let Some(value) = read_language(&project_config_path(repo)) {
            return Resolution {
                language: value,
                source: Source::Project,
            };
        }
    }
    if let Some(value) = read_language(user_config) {
        return Resolution {
            language: value,
            source: Source::User,
        };
    }
    if let Some(value) = non_empty(locale) {
        return Resolution {
            language: value.to_string(),
            source: Source::Locale,
        };
    }
    Resolution {
        language: "en".to_string(),
        source: Source::Default,
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Project-level config: `<repo>/.code-intel/config.json`, e.g.
/// `{"language": "zh"}`.
///
/// A repo-root dot-directory of its own -- structurally parallel to
/// `.sentrux/` (sentrux's quality-gate config) and `.understand-anything/`
/// (the graph provider's artifact directory), each owned by the one
/// subsystem it belongs to. Language preference is a pipeline-wide setting,
/// not a sentrux one, so it does not live inside `.sentrux/`; `.code-intel/`
/// names it after the pipeline itself the same way the other two directories
/// are named after their owning subsystem. JSON (not TOML) because the
/// crate's only existing config parser (`serde_json`) already reads
/// `pipeline.config.json` this way, and `.sentrux/rules.toml` is not
/// actually parsed as TOML here -- `sentrux_gate::evaluate_rules` hand-reads
/// its lines as text -- so adopting TOML for a new file would mean writing a
/// second, real parser for a format this crate does not otherwise depend on.
pub(crate) fn project_config_path(repo: &Path) -> PathBuf {
    repo.join(".code-intel").join("config.json")
}

/// User-level config: `<platform data root>/config.json`, alongside the
/// per-user data root `doctor_bootstrap::data_root` already derives (which
/// mirrors `Get-CodeIntelDataRoot` from `code-intel-platform.psm1`: `
/// CODE_INTEL_DATA_ROOT`, else `%LOCALAPPDATA%\code-intel` on Windows,
/// `~/Library/Application Support/code-intel` on macOS, or
/// `$XDG_DATA_HOME/code-intel`/`~/.local/share/code-intel` on Linux).
/// Reusing it keeps one Rust-side implementation of that switch rather than
/// a second copy that could drift from the bootstrap probe's.
///
/// Rejected: a POSIX-only `~/.config/code-intel/` file alongside the
/// installer's `env.sh`. That convention has no Windows story (Windows
/// persists installer state as real user environment variables, not a
/// file), so it would need a bespoke Windows case invented from scratch --
/// exactly the second implementation `data_root` already exists to avoid.
pub(crate) fn user_config_path() -> PathBuf {
    let platform = resolve_platform_name();
    let home = crate::doctor_bootstrap::home_directory();
    crate::doctor_bootstrap::data_root(&platform, &home).join("config.json")
}

fn resolve_platform_name() -> String {
    crate::doctor_bootstrap::resolve_platform("auto").unwrap_or_else(|_| {
        // `resolve_platform("auto")` only fails on a target Rust itself does
        // not build for; every platform this binary can run on resolves.
        // POSIX is the closer fallback shape (env-var-driven data root)
        // if that ever somehow changes.
        "linux".to_string()
    })
}

fn read_language(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("language")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// `$PSUICulture` (Windows) / `LANG`, `LC_ALL` (POSIX) only -- issue #155
/// deliberately stops here rather than reading other files or history to
/// guess. `PSUICulture` is a PowerShell automatic *variable*, not an OS
/// environment variable, so `env::var("PSUICulture")` is almost always
/// absent even on a Windows box whose live PowerShell session reports one;
/// that is the intended "signal unavailable" outcome falling through to
/// `en`, not a bug to route around with a WinAPI call or a shell-out to
/// `powershell.exe` -- the issue asks for a deterministic signal or `en`,
/// not best-effort culture detection.
fn system_locale_language() -> Option<String> {
    let raw = if cfg!(windows) {
        env::var("PSUICulture").ok()
    } else {
        env::var("LANG").ok().or_else(|| env::var("LC_ALL").ok())
    }?;
    normalize_locale(&raw)
}

/// Maps a locale string (`zh-CN`, `zh_CN.UTF-8`, `en-US`, `C`, `fr-FR`, ...)
/// onto one of the two languages this pipeline actually supports. Scope
/// (issue #155): no new languages, so anything not recognizably `zh`
/// normalizes to `en` rather than being passed through verbatim or rejected.
fn normalize_locale(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.to_ascii_lowercase().starts_with("zh") {
        Some("zh".to_string())
    } else {
        Some("en".to_string())
    }
}

/// Persists `language` into the project config, merging onto whatever
/// object is already at `<repo>/.code-intel/config.json` (or starting a
/// fresh one) so unrelated keys a future setting adds to the same file
/// survive a `language set`. Used by the `language set` command, which is
/// the one deliberate, explicit action that changes the persisted default;
/// an ordinary `--language <code>` override on a single command must not
/// silently become the project's new default.
pub(crate) fn write_project_config(repo: &Path, language: &str) -> io::Result<PathBuf> {
    let path = project_config_path(repo);
    write_merged(&path, language)?;
    Ok(path)
}

fn write_merged(path: &Path, language: &str) -> io::Result<()> {
    let mut document = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    document["language"] = Value::String(language.to_string());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "code-intel-language-pref-test-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_config(path: &Path, language: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!(r#"{{"language": "{language}"}}"#)).unwrap();
    }

    /// A user config path that resolves to nothing, for tiers that must
    /// prove they work even when the tier below them is entirely absent.
    fn absent_user_config(root: &Path) -> PathBuf {
        root.join("absent-user-config.json")
    }

    #[test]
    fn explicit_flag_wins_over_every_configured_tier() {
        let root = unique_temp_dir("flag-wins");
        write_config(&project_config_path(&root), "en");
        let user_config = root.join("user").join("config.json");
        write_config(&user_config, "en");

        let resolved = resolve_from(Some("zh"), Some(&root), &user_config, Some("en"));

        assert_eq!(resolved.language, "zh");
        assert_eq!(resolved.source, Source::Flag);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn project_config_wins_when_no_explicit_flag() {
        let root = unique_temp_dir("project-wins");
        write_config(&project_config_path(&root), "zh");
        let user_config = root.join("user").join("config.json");
        write_config(&user_config, "en");

        let resolved = resolve_from(None, Some(&root), &user_config, Some("en"));

        assert_eq!(resolved.language, "zh");
        assert_eq!(resolved.source, Source::Project);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn user_config_wins_when_no_flag_and_no_project_config() {
        let root = unique_temp_dir("user-wins");
        // No project config written at all: repo exists, but
        // `.code-intel/config.json` is absent.
        let user_config = root.join("user").join("config.json");
        write_config(&user_config, "en");

        let resolved = resolve_from(None, Some(&root), &user_config, Some("zh"));

        assert_eq!(resolved.language, "en");
        assert_eq!(resolved.source, Source::User);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn user_config_is_still_consulted_with_no_repo_at_all() {
        let root = unique_temp_dir("user-wins-no-repo");
        let user_config = root.join("user").join("config.json");
        write_config(&user_config, "zh");

        let resolved = resolve_from(None, None, &user_config, Some("en"));

        assert_eq!(resolved.language, "zh");
        assert_eq!(resolved.source, Source::User);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn system_locale_wins_when_nothing_configured() {
        let root = unique_temp_dir("locale-wins");
        let user_config = absent_user_config(&root);

        let resolved = resolve_from(None, Some(&root), &user_config, Some("zh"));

        assert_eq!(resolved.language, "zh");
        assert_eq!(resolved.source, Source::Locale);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn falls_back_to_en_when_no_tier_resolves() {
        let root = unique_temp_dir("default-wins");
        let user_config = absent_user_config(&root);

        let resolved = resolve_from(None, Some(&root), &user_config, None);

        assert_eq!(resolved.language, "en");
        assert_eq!(resolved.source, Source::Default);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn an_explicit_flag_is_trimmed_but_never_validated_against_known_languages() {
        // Matches the flag's existing, already-unvalidated behavior: this
        // module only adds persistence and defaulting, not new validation.
        let resolved = resolve_from(Some("  fr  "), None, Path::new("does-not-exist"), None);
        assert_eq!(resolved.language, "fr");
        assert_eq!(resolved.source, Source::Flag);
    }

    #[test]
    fn blank_explicit_flag_falls_through_instead_of_winning() {
        let root = unique_temp_dir("blank-flag-falls-through");
        write_config(&project_config_path(&root), "zh");

        let resolved = resolve_from(Some("   "), Some(&root), &absent_user_config(&root), None);

        assert_eq!(resolved.language, "zh");
        assert_eq!(resolved.source, Source::Project);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn locale_strings_normalize_to_the_two_supported_languages() {
        assert_eq!(normalize_locale("zh-CN").as_deref(), Some("zh"));
        assert_eq!(normalize_locale("zh_CN.UTF-8").as_deref(), Some("zh"));
        assert_eq!(normalize_locale("ZH-Hans").as_deref(), Some("zh"));
        assert_eq!(normalize_locale("en-US").as_deref(), Some("en"));
        assert_eq!(normalize_locale("fr-FR").as_deref(), Some("en"));
        assert_eq!(normalize_locale("C").as_deref(), Some("en"));
        assert_eq!(normalize_locale(""), None);
        assert_eq!(normalize_locale("   "), None);
    }

    #[test]
    fn write_project_config_round_trips_through_read_language() {
        let root = unique_temp_dir("write-round-trip");

        let path = write_project_config(&root, "zh").unwrap();

        assert_eq!(path, project_config_path(&root));
        assert_eq!(read_language(&path).as_deref(), Some("zh"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn write_project_config_merges_instead_of_clobbering_other_keys() {
        let root = unique_temp_dir("write-merge");
        let path = project_config_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"otherSetting": "keep-me"}"#).unwrap();

        write_project_config(&root, "en").unwrap();

        let bytes = fs::read(&path).unwrap();
        let document: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["language"], "en");
        assert_eq!(document["otherSetting"], "keep-me");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn write_project_config_recovers_from_a_corrupt_existing_file() {
        let root = unique_temp_dir("write-corrupt-recovery");
        let path = project_config_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not valid json").unwrap();

        write_project_config(&root, "zh").unwrap();

        assert_eq!(read_language(&path).as_deref(), Some("zh"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn an_absent_config_file_is_neither_data_nor_an_error() {
        let root = unique_temp_dir("absent-config");
        assert_eq!(read_language(&project_config_path(&root)), None);
        fs::remove_dir_all(root).ok();
    }
}
