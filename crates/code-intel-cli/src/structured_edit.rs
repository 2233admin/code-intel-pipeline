use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path};
use std::process::Command;

use serde_json::{json, Value};

use super::{
    publish_named, snapshot_adapter_error, tool_path, AdapterArtifact, AdapterError, AdapterOutput,
};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;
use crate::snapshot;

const MAX_PATTERN_BYTES: usize = 16 * 1024;
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PATHS: usize = 64;
const GENERATED_COMPONENTS: [&str; 9] = [
    ".git",
    ".next",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "target",
    "vendor",
];

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "edit.ast-grep-plan does not accept input artifacts".into(),
        ));
    }
    let options = request["options"]
        .as_object()
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options.keys().any(|key| {
        !matches!(
            key.as_str(),
            "repoPath" | "language" | "pattern" | "rewrite" | "paths"
        )
    }) {
        return Err(AdapterError::InvalidOptions(
            "edit.ast-grep-plan accepts only repoPath/language/pattern/rewrite/paths".into(),
        ));
    }
    let repo = required_string(options.get("repoPath"), "options.repoPath")?;
    let repo = Path::new(repo);
    if !repo.is_dir() {
        return Err(AdapterError::InvalidOptions(format!(
            "repoPath is not a directory: {}",
            repo.display()
        )));
    }
    let language = required_string(options.get("language"), "options.language")?;
    if language.len() > 64 {
        return Err(AdapterError::InvalidOptions(
            "options.language exceeds 64 bytes".into(),
        ));
    }
    let pattern = bounded_string(options.get("pattern"), "options.pattern")?;
    let rewrite = options
        .get("rewrite")
        .map(|value| bounded_string(Some(value), "options.rewrite"))
        .transpose()?;
    let paths = requested_paths(options.get("paths"))?;
    let canonical_repo = fs::canonicalize(repo)
        .map_err(|error| AdapterError::Io(format!("resolve repoPath: {error}")))?;
    let snapshot_scopes = request["snapshot"]["scope"]
        .as_array()
        .expect("validated snapshot scope")
        .iter()
        .map(|value| {
            normalize_relative(
                value.as_str().expect("validated snapshot scope item"),
                false,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let paths = paths
        .into_iter()
        .map(|path| validate_path(repo, &canonical_repo, &snapshot_scopes, path))
        .collect::<Result<Vec<_>, _>>()?;

    let lease =
        snapshot::begin_consumption(repo, &request["snapshot"]).map_err(snapshot_adapter_error)?;
    let version = ast_grep_version()?;
    let mut command = ast_grep_command();
    command
        .args(["run", "--pattern"])
        .arg(pattern)
        .args(["--lang", language]);
    if let Some(rewrite) = rewrite {
        command.args(["--rewrite", rewrite]);
    }
    command.args(["--json=compact", "--threads", "0", "--"]);
    command.args(&paths).current_dir(repo);
    let command_line = format!("{command:?}");
    let output = command
        .output()
        .map_err(|error| AdapterError::Unavailable(format!("start ast-grep: {error}")))?;
    if output.stdout.len() > MAX_OUTPUT_BYTES {
        return Err(AdapterError::Contract(format!(
            "ast-grep output exceeds {MAX_OUTPUT_BYTES} bytes; narrow options.paths or pattern"
        )));
    }
    let mut matches: Vec<Value> = match serde_json::from_slice(&output.stdout) {
        Ok(matches) => matches,
        Err(error) if !output.status.success() => {
            return Err(AdapterError::Internal(format!(
                "ast-grep failed: command={command_line}; stderr={}; invalid JSON={error}",
                bounded_diagnostic(&output.stderr)
            )))
        }
        Err(error) => {
            return Err(AdapterError::Internal(format!(
                "ast-grep emitted invalid JSON: {error}"
            )))
        }
    };
    if !ast_grep_status_is_acceptable(
        output.status.success(),
        output.status.code(),
        matches.is_empty(),
    ) {
        return Err(AdapterError::Internal(format!(
            "ast-grep failed: command={command_line}; stderr={}",
            bounded_diagnostic(&output.stderr)
        )));
    }
    let mut files = BTreeSet::new();
    for item in &mut matches {
        let file = item
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| AdapterError::Internal("ast-grep match has no file path".into()))?;
        let file = normalize_match_file(repo, &canonical_repo, file)?;
        item["file"] = Value::String(file.clone());
        files.insert(file);
    }
    lease.verify_after(repo).map_err(snapshot_adapter_error)?;

    let artifact = json!({
        "schema": "code-intel-structured-edit-plan.v1",
        "capability": "edit.ast-grep-plan",
        "snapshotIdentity": request["snapshot"]["identity"],
        "tool": {
            "name": "ast-grep",
            "version": version,
            "threads": "auto"
        },
        "query": {
            "language": language,
            "pattern": pattern,
            "rewrite": rewrite,
            "paths": paths
        },
        "summary": {
            "matches": matches.len(),
            "files": files.len(),
            "hasRewrite": rewrite.is_some()
        },
        "matches": matches,
        "authority": {
            "mode": "preview_only",
            "repositoryMutation": false
        }
    });
    let bytes = serde_json::to_vec(&artifact)
        .map_err(|error| AdapterError::Internal(format!("serialize edit plan: {error}")))?;
    publish_named(out, "structured-edit-plan.json", &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-structured-edit-plan.v1".into(),
            artifact_type: "edit.structured-plan".into(),
            relative_path: "structured-edit-plan.json".into(),
            bytes,
        }],
        observed_effects: vec![
            "repo_read".into(),
            "local_write".into(),
            "process_spawn".into(),
        ],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn required_string<'a>(value: Option<&'a Value>, name: &str) -> Result<&'a str, AdapterError> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AdapterError::InvalidOptions(format!("{name} must be non-empty")))
}

fn bounded_string<'a>(value: Option<&'a Value>, name: &str) -> Result<&'a str, AdapterError> {
    let value = required_string(value, name)?;
    if value.len() > MAX_PATTERN_BYTES {
        Err(AdapterError::InvalidOptions(format!(
            "{name} exceeds {MAX_PATTERN_BYTES} bytes"
        )))
    } else {
        Ok(value)
    }
}

fn requested_paths(value: Option<&Value>) -> Result<Vec<&str>, AdapterError> {
    match value {
        None => Ok(vec!["."]),
        Some(value) => {
            let paths = value.as_array().ok_or_else(|| {
                AdapterError::InvalidOptions("options.paths must be an array".into())
            })?;
            if paths.is_empty() || paths.len() > MAX_PATHS {
                return Err(AdapterError::InvalidOptions(format!(
                    "options.paths must contain 1..={MAX_PATHS} entries"
                )));
            }
            paths
                .iter()
                .map(|value| required_string(Some(value), "options.paths[]"))
                .collect()
        }
    }
}

fn validate_path(
    repo: &Path,
    canonical_repo: &Path,
    snapshot_scopes: &[String],
    path: &str,
) -> Result<String, AdapterError> {
    let normalized = normalize_relative(path, true)?;
    if !snapshot_scopes
        .iter()
        .any(|scope| within_scope(&normalized, scope))
    {
        return Err(AdapterError::Contract(format!(
            "edit path is outside the requested snapshot scope: {path}"
        )));
    }
    let full = fs::canonicalize(repo.join(Path::new(&normalized))).map_err(|error| {
        AdapterError::InvalidOptions(format!("resolve edit path {path}: {error}"))
    })?;
    if !full.starts_with(canonical_repo) {
        return Err(AdapterError::Contract(format!(
            "edit path escapes repository: {path}"
        )));
    }
    Ok(normalized)
}

/// Shared with `edit.span-apply` rather than restated there: the plan stage
/// and the apply stage must agree byte-for-byte on which paths are editable,
/// or a plan could name a target the writer would then refuse (or worse,
/// accept under looser rules).
pub(super) fn normalize_relative(
    path: &str,
    reject_generated: bool,
) -> Result<String, AdapterError> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Err(AdapterError::InvalidOptions(format!(
            "edit path must be repository-relative: {}",
            path.display()
        )));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    AdapterError::InvalidOptions("edit path must be UTF-8".into())
                })?;
                if reject_generated
                    && GENERATED_COMPONENTS
                        .iter()
                        .any(|generated| value.eq_ignore_ascii_case(generated))
                {
                    return Err(AdapterError::InvalidOptions(format!(
                        "edit path targets excluded generated content: {}",
                        path.display()
                    )));
                }
                parts.push(value);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AdapterError::InvalidOptions(format!(
                    "edit path escapes repository: {}",
                    path.display()
                )))
            }
        }
    }
    Ok(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}

pub(super) fn within_scope(path: &str, scope: &str) -> bool {
    scope == "." || path == scope || path.starts_with(&format!("{scope}/"))
}

fn normalize_match_file(
    repo: &Path,
    canonical_repo: &Path,
    file: &str,
) -> Result<String, AdapterError> {
    let file = Path::new(file);
    let full = if file.is_absolute() {
        file.to_path_buf()
    } else {
        repo.join(file)
    };
    let full = fs::canonicalize(&full)
        .map_err(|error| AdapterError::Internal(format!("resolve ast-grep match: {error}")))?;
    let relative = full.strip_prefix(canonical_repo).map_err(|_| {
        AdapterError::Contract(format!(
            "ast-grep returned a match outside the repository: {}",
            full.display()
        ))
    })?;
    normalize_relative(&relative.to_string_lossy(), true)
}

/// A command that launches `ast-grep` by absolute path, resolved through
/// `tool_path` like every `rg`/`git` call site: `execute` runs the child in
/// the scanned repository, so a bare name must never reach `Command::new`.
/// npm-style installs ship a `.cmd` shim on Windows, which `CreateProcess`
/// cannot start directly, so those are wrapped in `cmd.exe` the same way
/// `builtin_provider_evidence::external_command` wraps them.
fn ast_grep_command() -> Command {
    let path = tool_path::resolve("ast-grep");
    #[cfg(windows)]
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat"))
    {
        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c"]).arg(path);
        return command;
    }
    Command::new(path)
}

fn ast_grep_version() -> Result<String, AdapterError> {
    let output = ast_grep_command()
        .arg("--version")
        .output()
        .map_err(|error| AdapterError::Unavailable(format!("start ast-grep: {error}")))?;
    if !output.status.success() {
        return Err(AdapterError::Unavailable(format!(
            "ast-grep --version failed: {}",
            bounded_diagnostic(&output.stderr)
        )));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| {
            AdapterError::Unavailable(format!("ast-grep version is not UTF-8: {error}"))
        })
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(4096)])
        .trim()
        .to_string()
}

fn ast_grep_status_is_acceptable(success: bool, code: Option<i32>, empty: bool) -> bool {
    success || (code == Some(1) && empty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::sha256_hex;

    #[test]
    fn registry_digest_and_path_guards_are_bound_to_this_adapter() {
        let registry: Value =
            serde_json::from_slice(include_bytes!("../../../orchestration/integrations.json"))
                .unwrap();
        let integration = registry["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "edit.ast-grep-plan")
            .unwrap();
        assert_eq!(
            integration["capabilityDeclaration"]["implementation"]["toolchainDigests"][0],
            sha256_hex(include_bytes!("structured_edit.rs"))
        );
        assert!(normalize_relative("src/../secret.py", true).is_err());
        assert!(normalize_relative("node_modules/pkg/index.js", true).is_err());
        assert!(within_scope("backend/api.py", "backend"));
        assert!(!within_scope("frontend/api.ts", "backend"));
    }

    #[test]
    fn ast_grep_empty_json_is_success_only_for_no_match_exit() {
        assert!(ast_grep_status_is_acceptable(false, Some(1), true));
        assert!(!ast_grep_status_is_acceptable(false, Some(1), false));
        assert!(!ast_grep_status_is_acceptable(false, Some(2), true));
    }
}
