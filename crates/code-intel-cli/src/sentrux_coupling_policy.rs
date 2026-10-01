//! Production-coupling policy and baseline compatibility for the native
//! Sentrux gate (#394).
//!
//! `[constraints] ignore_test_dependencies = true` in `.sentrux/rules.toml`
//! keeps the files `is_test_file` classifies as tests in the quality graph and
//! in every non-coupling metric, but takes them out of the production coupling
//! numerator and denominator. Every baseline records the policy it was
//! measured under. A baseline recorded under a different schema, engine
//! version, metric set or policy fails closed with the mismatching fields
//! named, so the gate never compares unlike numbers and never re-baselines on
//! its own.

use std::{fs, path::Path};

use serde_json::{json, Value};

pub(super) const COUPLING_POLICY_VERSION: &str = "production-coupling.v1";
pub(super) const TEST_PATH_CLASSIFIER_VERSION: &str = "repo-test-paths.v1";
pub(super) const QUALITY_GRAPH_SCOPE: &str = "all_included_files";
/// File name of the isolated baseline that legacy session gates forward to;
/// it lives under `.sentrux/cache/` and never replaces the canonical one.
pub(super) const SESSION_BASELINE_FILE: &str = "native-session-baseline.json";

/// Whether `.sentrux/rules.toml` sets `[constraints] ignore_test_dependencies`.
/// A missing rules file or key keeps every import in the coupling ratio.
pub(super) fn ignore_test_dependencies(repo: &Path) -> Result<bool, String> {
    let rules_path = repo.join(".sentrux").join("rules.toml");
    if !rules_path.is_file() {
        return Ok(false);
    }
    let rules = fs::read_to_string(&rules_path)
        .map_err(|error| format!("read {}: {error}", rules_path.display()))?;
    Ok(constraint_boolean_rule(&rules, "ignore_test_dependencies"))
}

fn constraint_boolean_rule(rules: &str, name: &str) -> bool {
    let mut in_constraints = false;
    for line in rules.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_constraints = trimmed == "[constraints]";
            continue;
        }
        if !in_constraints || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(name) {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return value.split('#').next().unwrap_or("").trim() == "true";
            }
        }
    }
    false
}

/// `repo-test-paths.v1`: a path segment named `test`, `tests` or `__tests__`,
/// or a file named `test`, `tests`, `test_*`, `*_test`, `*_tests`, `*.test.*`
/// or `*.spec.*`.
pub(super) fn is_test_file(relative: &str) -> bool {
    let lower = relative.to_ascii_lowercase();
    let leaf = lower.rsplit('/').next().unwrap_or(&lower);
    let stem = leaf.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(leaf);
    lower
        .split('/')
        .any(|part| matches!(part, "test" | "tests" | "__tests__"))
        || matches!(stem, "test" | "tests")
        || leaf.starts_with("test_")
        || stem.ends_with("_test")
        || stem.ends_with("_tests")
        || leaf.contains(".test.")
        || leaf.contains(".spec.")
}

/// The `couplingPolicy` block every baseline and metrics document records.
pub(super) fn policy_json(ignore_test_dependencies: bool) -> Value {
    json!({
        "version": COUPLING_POLICY_VERSION,
        "ignore_test_dependencies": ignore_test_dependencies,
        "test_path_classifier": TEST_PATH_CLASSIFIER_VERSION,
        "quality_graph_scope": QUALITY_GRAPH_SCOPE,
    })
}

/// Fields that stop `baseline` from being compared with metrics this engine
/// measured under `ignore_test_dependencies`, in report order. Empty means
/// the baseline is comparable.
pub(super) fn baseline_mismatch_fields(
    baseline: &Value,
    ignore_test_dependencies: bool,
) -> Vec<&'static str> {
    let policy = &baseline["couplingPolicy"];
    let policy_matches = policy["version"] == COUPLING_POLICY_VERSION
        && policy["test_path_classifier"] == TEST_PATH_CLASSIFIER_VERSION
        && policy["quality_graph_scope"] == QUALITY_GRAPH_SCOPE
        && policy["ignore_test_dependencies"].as_bool() == Some(ignore_test_dependencies);
    let mut fields = Vec::new();
    if baseline["schema"] != super::BASELINE_SCHEMA {
        fields.push("schema");
    }
    if baseline["engine"]["id"] != super::ENGINE_ID {
        fields.push("engine.id");
    }
    if baseline["engine"]["version"] != super::ENGINE_VERSION {
        fields.push("engine.version");
    }
    if baseline["metrics"].as_object().is_none() {
        fields.push("metrics");
    } else {
        fields.extend(
            super::GATED_METRIC_KEYS
                .iter()
                .filter(|key| baseline["metrics"][**key].as_f64().is_none())
                .copied(),
        );
    }
    if baseline["godFiles"].as_array().is_none() {
        fields.push("godFiles");
    }
    if !policy_matches {
        fields.push("couplingPolicy");
    }
    fields
}

/// The fail-closed diagnostic for a baseline `baseline_mismatch_fields`
/// rejected: what this engine expects, what the baseline holds, and the
/// command that re-baselines on purpose.
pub(super) fn mismatch_message(
    repo: &Path,
    baseline_path: &Path,
    baseline: &Value,
    fields: &[&str],
    ignore_test_dependencies: bool,
) -> String {
    format!(
        "baseline engine mismatch at {} (missing or invalid fields: {}): expected schema {}, engine {}@{}, coupling policy {} ignore_test_dependencies={}; observed schema {}, engine {}@{}, coupling policy {} ignore_test_dependencies={}; preserve this baseline and re-baseline with {}",
        baseline_path.display(),
        fields.join(", "),
        super::BASELINE_SCHEMA,
        super::ENGINE_ID,
        super::ENGINE_VERSION,
        COUPLING_POLICY_VERSION,
        ignore_test_dependencies,
        baseline["schema"].as_str().unwrap_or("unknown"),
        baseline["engine"]["id"].as_str().unwrap_or("unknown"),
        baseline["engine"]["version"].as_str().unwrap_or("unknown"),
        baseline["couplingPolicy"]["version"]
            .as_str()
            .unwrap_or("missing"),
        baseline["couplingPolicy"]["ignore_test_dependencies"]
            .as_bool()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "missing".into()),
        rebaseline_command(repo, baseline_path),
    )
}

/// The command that writes `baseline_path` on purpose: `session_save` for the
/// isolated session baseline, `save_baseline` for the canonical one.
pub(super) fn rebaseline_command(repo: &Path, baseline_path: &Path) -> String {
    let session =
        baseline_path.file_name().and_then(|name| name.to_str()) == Some(SESSION_BASELINE_FILE);
    let operation = if session {
        "session_save"
    } else {
        "save_baseline"
    };
    format!(
        "code-intel sentrux --operation {operation} --repo {}",
        repo.display()
    )
}
