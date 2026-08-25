//! `scan.ast-grep-security`: run the bundled, originally-authored ast-grep
//! security rule library (`orchestration/ast-grep-rules/<language>.yaml`)
//! against a snapshot-bound path set and publish the findings.
//!
//! Deliberately separate from `edit.ast-grep-plan`: that capability runs a
//! single caller-supplied pattern and previews a rewrite; this one runs a
//! fixed, pipeline-owned rule set and never proposes a rewrite. Findings are
//! advisory-only — `authority.mode` and the absence of any gate reference to
//! this artifact schema are how that boundary is enforced, not prose. See
//! `orchestration/internalization/ast-grep-security-rules.json` for why the
//! rule content is reimplemented rather than vendored from any upstream
//! project.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::structured_edit::{
    ast_grep_command, ast_grep_status_is_acceptable, ast_grep_version, bounded_diagnostic,
    normalize_match_file, normalize_relative, requested_paths, required_string, validate_path,
};
use super::{publish_named, snapshot_adapter_error, AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;
use crate::snapshot;

const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Languages with a bundled, originally-authored security rule file. Keep in
/// sync with `orchestration/ast-grep-rules/`; `every_bundled_language_has_a_rule_file_on_disk`
/// fails if this list and the directory drift apart.
pub(super) const BUNDLED_LANGUAGES: [&str; 7] = [
    "csharp",
    "go",
    "java",
    "javascript",
    "python",
    "rust",
    "typescript",
];

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "scan.ast-grep-security does not accept input artifacts".into(),
        ));
    }
    let options = request["options"]
        .as_object()
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options
        .keys()
        .any(|key| !matches!(key.as_str(), "repoPath" | "language" | "paths"))
    {
        return Err(AdapterError::InvalidOptions(
            "scan.ast-grep-security accepts only repoPath/language/paths".into(),
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
    if !BUNDLED_LANGUAGES.contains(&language) {
        return Err(AdapterError::InvalidOptions(format!(
            "no bundled ast-grep security rules for language {language:?}; available: {}",
            BUNDLED_LANGUAGES.join(", ")
        )));
    }
    let rule_file = super::pipeline_root()
        .join("orchestration")
        .join("ast-grep-rules")
        .join(format!("{language}.yaml"));
    if !rule_file.is_file() {
        return Err(AdapterError::Unavailable(format!(
            "bundled rule file missing from this installation: {}",
            rule_file.display()
        )));
    }
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
    command.args(["scan", "--rule"]).arg(&rule_file).args([
        "--json=compact",
        "--include-metadata",
        "--threads",
        "0",
        "--",
    ]);
    command.args(&paths).current_dir(repo);
    let command_line = format!("{command:?}");
    let output = command
        .output()
        .map_err(|error| AdapterError::Unavailable(format!("start ast-grep: {error}")))?;
    if output.stdout.len() > MAX_OUTPUT_BYTES {
        return Err(AdapterError::Contract(format!(
            "ast-grep output exceeds {MAX_OUTPUT_BYTES} bytes; narrow options.paths"
        )));
    }
    let mut findings: Vec<Value> = match serde_json::from_slice(&output.stdout) {
        Ok(findings) => findings,
        Err(error) if !output.status.success() => {
            return Err(AdapterError::Internal(format!(
                "ast-grep scan failed: command={command_line}; stderr={}; invalid JSON={error}",
                bounded_diagnostic(&output.stderr)
            )))
        }
        Err(error) => {
            return Err(AdapterError::Internal(format!(
                "ast-grep scan emitted invalid JSON: {error}"
            )))
        }
    };
    if !ast_grep_status_is_acceptable(
        output.status.success(),
        output.status.code(),
        findings.is_empty(),
    ) {
        return Err(AdapterError::Internal(format!(
            "ast-grep scan failed: command={command_line}; stderr={}",
            bounded_diagnostic(&output.stderr)
        )));
    }
    let mut files = BTreeSet::new();
    let mut by_severity: BTreeMap<String, usize> = BTreeMap::new();
    for item in &mut findings {
        let file = item
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| AdapterError::Internal("ast-grep finding has no file path".into()))?;
        let file = normalize_match_file(repo, &canonical_repo, file)?;
        item["file"] = Value::String(file.clone());
        files.insert(file);
        let severity = item
            .get("severity")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        *by_severity.entry(severity).or_insert(0) += 1;
    }
    lease.verify_after(repo).map_err(snapshot_adapter_error)?;

    let artifact = json!({
        "schema": "code-intel-ast-grep-security-findings.v1",
        "capability": "scan.ast-grep-security",
        "snapshotIdentity": request["snapshot"]["identity"],
        "tool": {
            "name": "ast-grep",
            "version": version,
            "threads": "auto"
        },
        "query": {
            "language": language,
            "ruleFile": format!("orchestration/ast-grep-rules/{language}.yaml"),
            "paths": paths
        },
        "summary": {
            "findings": findings.len(),
            "files": files.len(),
            "bySeverity": by_severity
        },
        "findings": findings,
        "authority": {
            "mode": "advisory_only",
            "repositoryMutation": false,
            "gates": false
        }
    });
    let bytes = serde_json::to_vec(&artifact)
        .map_err(|error| AdapterError::Internal(format!("serialize security findings: {error}")))?;
    publish_named(out, "ast-grep-security-findings.json", &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-ast-grep-security-findings.v1".into(),
            artifact_type: "scan.security-findings".into(),
            relative_path: "ast-grep-security-findings.json".into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::sha256_hex;

    #[test]
    fn registry_digest_is_bound_to_this_adapter() {
        let registry: Value =
            serde_json::from_slice(include_bytes!("../../../orchestration/integrations.json"))
                .unwrap();
        let integration = registry["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "scan.ast-grep-security")
            .unwrap();
        assert_eq!(
            integration["capabilityDeclaration"]["implementation"]["toolchainDigests"][0],
            sha256_hex(include_bytes!("ast_grep_security_scan.rs"))
        );
    }

    #[test]
    fn every_bundled_language_has_a_rule_file_on_disk() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../orchestration/ast-grep-rules");
        for language in BUNDLED_LANGUAGES {
            let file = root.join(format!("{language}.yaml"));
            assert!(
                file.is_file(),
                "missing bundled rule file: {}",
                file.display()
            );
        }
    }
}
