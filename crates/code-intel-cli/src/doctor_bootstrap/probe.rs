//! Tool presence and command-output probing for the doctor bootstrap.
//!
//! Every function here reports what it observed and never fails: the absence
//! of an optional overlay is the observation, not an error. Tool lookup goes
//! through the shared `tool_path` resolver so presence-checking here and
//! path-resolution at real launch sites cannot drift apart.

use std::env;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

use super::paths::display;

#[path = "../tool_path.rs"]
mod tool_path;

/// Substring `sentrux check --help` must print for the core overlay to count
/// as conforming. PowerShell's `-match` is case-insensitive, so this compares
/// case-insensitively too.
pub(super) const SENTRUX_CORE_MARKER: &str = "Enforce architectural rules";

pub(super) fn locate(name: &str, prefix: Option<&Path>) -> Option<std::path::PathBuf> {
    tool_path::locate(name, prefix)
}

pub(super) fn probe_tool(name: &str, required: bool, prefix: Option<&Path>) -> Value {
    let found = tool_path::locate(name, prefix);
    json!({
        "name": name,
        "required": required,
        "found": found.is_some(),
        "source": found.as_deref().map(display).unwrap_or_default()
    })
}

/// `python` falls back to `python3`, matching `Get-CodeIntelPythonCommand`.
/// The reported `name` stays `python` so the `missing` list wording does not
/// change with which interpreter happened to be installed.
pub(super) fn probe_python(prefix: Option<&Path>) -> Value {
    let found =
        tool_path::locate("python", prefix).or_else(|| tool_path::locate("python3", prefix));
    json!({
        "name": "python",
        "required": true,
        "found": found.is_some(),
        "source": found.as_deref().map(display).unwrap_or_default()
    })
}

/// Run `program args...` and decide `found` from exit status plus a predicate
/// over the merged stdout/stderr text. A program that cannot be located or
/// launched is a `found: false` observation, never an error.
pub(super) fn probe_command_output(
    name: &str,
    program: &str,
    args: &[&str],
    prefix: Option<&Path>,
    matches: impl Fn(&str) -> bool,
) -> Value {
    let Some(binary) = tool_path::locate(program, prefix) else {
        return json!({
            "name": name,
            "found": false,
            "output": format!("{program} was not found on PATH")
        });
    };
    let mut command = Command::new(&binary);
    command.args(args);
    if let Some(prefix) = prefix {
        if let Some(path) = prefixed_path(prefix) {
            command
                .env_remove("PATH")
                .env_remove("Path")
                .env("PATH", path);
        }
    }
    match command.output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            let text = text.trim().to_string();
            json!({
                "name": name,
                "found": output.status.success() && matches(&text),
                "output": text
            })
        }
        Err(error) => json!({"name": name, "found": false, "output": error.to_string()}),
    }
}

fn prefixed_path(prefix: &Path) -> Option<std::ffi::OsString> {
    let mut paths = vec![prefix.to_path_buf()];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    env::join_paths(paths).ok()
}

/// Whether Pro auto-activation is opted into, which decides whether a `free`
/// tier still counts as healthy.
pub(super) fn requires_pro_tier() -> bool {
    matches!(
        env::var("SENTRUX_AUTO_PRO").unwrap_or_default().as_str(),
        "1" | "true" | "True" | "TRUE"
    )
}

/// `Tier:\s+pro` when Pro auto-activation is opted into, `Tier:\s+(pro|free)`
/// otherwise. Hand-rolled because the crate carries no regex dependency, and
/// case-insensitive to match PowerShell `-match` semantics.
pub(super) fn matches_tier(text: &str, require_pro: bool) -> bool {
    let lower = text.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(index) = rest.find("tier:") {
        let after = &rest[index + "tier:".len()..];
        let trimmed = after.trim_start_matches([' ', '\t', '\r', '\n']);
        if trimmed.len() < after.len()
            && (trimmed.starts_with("pro") || (!require_pro && trimmed.starts_with("free")))
        {
            return true;
        }
        rest = after;
    }
    false
}

pub(super) fn contains_ignore_case(text: &str, needle: &str) -> bool {
    text.to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_pattern_accepts_free_only_without_the_pro_opt_in() {
        assert!(matches_tier("Sentrux\nTier:   free\n", false));
        assert!(matches_tier("Tier: pro", false));
        assert!(matches_tier("Tier: pro", true));
        assert!(!matches_tier("Tier:   free", true));
        // No whitespace after the colon is not a match, same as `\s+`.
        assert!(!matches_tier("Tier:free", false));
        assert!(!matches_tier("no tier line here", false));
    }

    #[test]
    fn core_marker_comparison_is_case_insensitive_like_powershell_match() {
        assert!(contains_ignore_case(
            "  ENFORCE ARCHITECTURAL RULES for a repo",
            SENTRUX_CORE_MARKER
        ));
        assert!(!contains_ignore_case(
            "some other help text",
            SENTRUX_CORE_MARKER
        ));
    }

    #[test]
    fn a_missing_tool_is_a_found_false_observation() {
        let probe = probe_tool("__code_intel_absent_tool__", true, None);
        assert_eq!(probe["found"], json!(false));
        assert_eq!(probe["required"], json!(true));
        assert_eq!(probe["source"], json!(""));
    }

    #[test]
    fn an_absent_program_reports_without_failing() {
        let probe = probe_command_output(
            "absent",
            "__code_intel_absent_tool__",
            &["--help"],
            None,
            |_| true,
        );
        assert_eq!(probe["found"], json!(false));
        assert!(probe["output"]
            .as_str()
            .unwrap()
            .contains("was not found on PATH"));
    }
}
