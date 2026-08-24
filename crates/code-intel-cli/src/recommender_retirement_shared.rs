//! Shared constants and primitives for the E02 recommender retirement
//! packet trio (`recommender_retirement_packet`, `recommender_retirement_diff`,
//! `recommender_retirement_restore`). See `recommender_retirement_packet`'s
//! module doc for the overall port's scope and non-goals.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::capability::sha256_hex;

pub(crate) const RETIREMENT_ID: &str = "retire-recommender-branch";
pub(crate) const BRANCH_ID: &str = "run-code-intel.workflow-recommender.inline";
pub(crate) const LEGACY_CAPABILITY: &str = "facade.workflow-recommender.inline";
pub(crate) const REPLACEMENT_ID: &str = "advisory.workflow-recommend";
pub(crate) const DEFAULT_SOURCE_REVISION: &str = "e6e73e4f720ab2ae2bca531a07ed638f55fecd1d";

pub(crate) const LEGACY_FUNCTIONS_START: &str =
    "# ============ 三栈工作流推荐器 (Workflow Stack Recommender) ============";
pub(crate) const LEGACY_INVOCATION_START: &str =
    "# Three-stack workflow recommender (matt-flow / gstack / spec-driven).";
pub(crate) const CURRENT_FUNCTIONS_START: &str =
    "# Workflow recommendations are owned by the standalone advisory atom in OpenSpec-Detector.ps1.";
pub(crate) const CURRENT_INVOCATION_START: &str =
    "# Historical options now map to the standalone advisory atom: Skip disables it and";
pub(crate) const FUNCTIONS_END: &str = "\nfunction Get-JsonProperty";
pub(crate) const INVOCATION_END: &str = "\nif (-not $toolState.rg)";

pub(crate) fn frozen_set() -> Vec<String> {
    vec![
        "run-code-intel.ps1".into(),
        "OpenSpec-Detector.ps1".into(),
        "Invoke-WorkflowRecommendation.ps1".into(),
        "manifest-projection:orchestration/integrations.json#advisory.workflow-recommend".into(),
    ]
}

pub(crate) fn expected_blockers() -> Vec<&'static str> {
    vec![
        "dependency_approval_set_mismatch",
        "unproven_compatibility_window",
        "unproven_dependency_approval",
        "unproven_independent_approval",
        "unproven_usage_observation",
    ]
}

/// Byte range `[start, end)` of the bounded block beginning at the first
/// occurrence of `start_marker` and ending immediately before the next
/// occurrence of `end_marker` (which is not itself part of the block) --
/// the same shape as the PowerShell originals' `(?s)START.*?(?=END)`.
pub(crate) fn find_bounded_block(
    haystack: &str,
    start_marker: &str,
    end_marker: &str,
) -> Result<(usize, usize), String> {
    let start = haystack
        .find(start_marker)
        .ok_or_else(|| format!("bounded deletion marker is absent: {start_marker}"))?;
    let end_offset = haystack[start..].find(end_marker).ok_or_else(|| {
        format!("bounded deletion marker end is absent: {end_marker} (after {start_marker})")
    })?;
    Ok((start, start + end_offset))
}

pub(crate) fn write_json_file(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

pub(crate) fn artifact_ref(
    out_dir: &Path,
    artifact_schema: &str,
    kind: &str,
    relative_path: &str,
    snapshot_identity: &str,
) -> Result<Value, String> {
    let path = out_dir.join(relative_path);
    let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(json!({
        "schema": "code-intel-artifact-ref.v1",
        "artifactSchema": artifact_schema,
        "type": kind,
        "path": relative_path.replace('\\', "/"),
        "sha256": sha256_hex(&bytes),
        "consumedSnapshotIdentity": snapshot_identity,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_block_excludes_the_end_marker_line() {
        let text = "before\nSTART middle text\nmore\nEND_LINE\nafter";
        let (s, e) = find_bounded_block(text, "START", "\nEND_LINE").unwrap();
        assert_eq!(&text[s..e], "START middle text\nmore");
    }

    #[test]
    fn bounded_block_reports_a_missing_start_marker() {
        assert!(find_bounded_block("no markers here", "START", "\nEND").is_err());
    }

    #[test]
    fn bounded_block_reports_a_missing_end_marker() {
        assert!(find_bounded_block("STARTonly, no end", "START", "\nEND").is_err());
    }
}
