//! Exhaustive Rust-native `sentrux-hotspots.json` producer.
//!
//! Issue #361: the only prior producer of this artifact was
//! `legacy/run-code-intel.ps1` (`Select-Object -First 30/50/20` on
//! files/functions/modules). That arbitrary top-N cut silently dropped any
//! file below the cutoff, so consumers joining per-session evidence against
//! it (`session_evidence.rs`'s `--hotspots`) got zero matches for sessions
//! that only touched lower-complexity files -- a real, reproducible data
//! loss with no error or degradation signal. No decision record in
//! `docs/decisions/` pins the top-N cut as intentional.
//!
//! This module reuses [`sentrux_analysis::analyze`]'s DSM output and
//! reprojects it into the same `{tool, path, generated_from, modules, files,
//! functions}` shape the legacy artifact used, but keeps every row: no
//! `Select-Object -First N` equivalent. Ordering (descending by risk/
//! complexity) is preserved for readability; coverage is exhaustive.

use serde_json::{json, Value};
use std::path::Path;

use crate::sentrux_analysis;

pub fn hotspots(target: &Path) -> Result<Value, String> {
    let dsm = sentrux_analysis::analyze(target)?;
    Ok(project(&dsm))
}

/// Reprojects an already-computed DSM `Value` (as returned by
/// [`sentrux_analysis::analyze`]) into the exhaustive hotspots shape.
/// Split out from [`hotspots`] so callers that already hold a DSM value
/// (tests, or a future in-process consumer) don't recompute it.
pub fn project(dsm: &Value) -> Value {
    let empty = Vec::new();
    let file_details = dsm["file_details"].as_array().unwrap_or(&empty);
    let modules = dsm["modules"].as_array().unwrap_or(&empty);

    let mut module_hotspots: Vec<Value> = modules.to_vec();
    module_hotspots.sort_by(|left, right| {
        risk_score(right)
            .partial_cmp(&risk_score(left))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let module_hotspots: Vec<Value> = module_hotspots
        .iter()
        .map(|module| {
            json!({
                "id": module["id"],
                "name": module["name"],
                "risk": module["metrics"]["risk"],
                "riskScore": module["colors"]["Risk"]["score"],
                "color": module["colors"]["Risk"]["color"],
                "files": module["files"],
                "blastRadius": module["metrics"]["blast_radius"],
                "gitFiles": module["metrics"]["git_files"],
            })
        })
        .collect();

    // `file_details` is already sorted descending by max_complexity by
    // `sentrux_analysis::analyze`; reproject every row (no truncation).
    let file_hotspots: Vec<Value> = file_details
        .iter()
        .map(|file| {
            json!({
                "id": file["id"],
                "path": file["path"],
                "sourceAnchor": file["source_anchor"],
                "functionCount": file["function_count"],
                "maxComplexity": file["max_complexity"],
                "avgComplexity": file["avg_complexity"],
                "loc": file["loc"],
                "git": file["git"],
            })
        })
        .collect();

    let mut function_hotspots: Vec<Value> = Vec::new();
    for file in file_details {
        let file_id = file["id"].clone();
        let file_path = file["path"].clone();
        let empty_fns = Vec::new();
        for function in file["functions"].as_array().unwrap_or(&empty_fns) {
            function_hotspots.push(json!({
                "id": function["id"],
                "fileId": file_id,
                "file": file_path,
                "name": function["name"],
                "sourceAnchor": function["source_anchor"],
                "startLine": function["start_line"],
                "endLine": function["end_line"],
                "complexity": function["complexity"],
                "loc": function["loc"],
                "params": function["params"],
                "async": function["async"],
                "public": function["public"],
            }));
        }
    }
    function_hotspots.sort_by(|left, right| {
        let left_complexity = left["complexity"].as_i64().unwrap_or(0);
        let right_complexity = right["complexity"].as_i64().unwrap_or(0);
        right_complexity
            .cmp(&left_complexity)
            .then_with(|| left["file"].as_str().cmp(&right["file"].as_str()))
    });

    json!({
        "tool": "hotspots",
        "path": dsm["path"],
        "generated_from": {
            "dsm": "code-intel sentrux dsm",
            "fileDetails": "code-intel sentrux dsm#/file_details",
        },
        "modules": module_hotspots,
        "files": file_hotspots,
        "functions": function_hotspots,
    })
}

fn risk_score(module: &Value) -> f64 {
    module["colors"]["Risk"]["score"].as_f64().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_detail(path: &str, max_complexity: i64) -> Value {
        json!({
            "id": format!("file-{path}"),
            "path": path,
            "max_complexity": max_complexity,
            "avg_complexity": max_complexity as f64 / 2.0,
            "function_count": 1,
            "loc": 10,
            "source_anchor": {"path": path, "line": 1, "label": format!("{path}:1")},
            "git": {"churn": 0, "dirty": false},
            "functions": [{
                "id": format!("fn-{path}"),
                "name": "f",
                "complexity": max_complexity,
                "start_line": 1,
                "end_line": 2,
                "loc": 1,
                "params": 0,
                "async": false,
                "public": true,
                "source_anchor": {"path": path, "line": 1, "label": format!("{path}:1")},
            }],
        })
    }

    fn dsm_with_files(count: usize) -> Value {
        let file_details: Vec<Value> = (0..count)
            .map(|index| file_detail(&format!("src/file_{index:03}.rs"), (count - index) as i64))
            .collect();
        json!({
            "tool": "dsm",
            "path": ".",
            "modules": [],
            "file_details": file_details,
        })
    }

    #[test]
    fn hotspots_projection_keeps_every_file_past_the_old_top_30_cut() {
        let dsm = dsm_with_files(45);
        let hotspots = project(&dsm);
        let files = hotspots["files"].as_array().unwrap();
        let functions = hotspots["functions"].as_array().unwrap();
        assert_eq!(
            files.len(),
            45,
            "exhaustive hotspots must not drop rows below any top-N cutoff"
        );
        assert_eq!(functions.len(), 45);
        assert_eq!(hotspots["tool"], "hotspots");
    }

    #[test]
    fn hotspots_projection_stays_sorted_descending_by_complexity() {
        let dsm = dsm_with_files(10);
        let hotspots = project(&dsm);
        let files = hotspots["files"].as_array().unwrap();
        let complexities: Vec<i64> = files
            .iter()
            .map(|file| file["maxComplexity"].as_i64().unwrap())
            .collect();
        let mut sorted = complexities.clone();
        sorted.sort_by(|left, right| right.cmp(left));
        assert_eq!(complexities, sorted);
    }

    #[test]
    fn hotspots_projection_produces_zero_rows_for_an_empty_dsm() {
        let dsm = dsm_with_files(0);
        let hotspots = project(&dsm);
        assert!(hotspots["files"].as_array().unwrap().is_empty());
        assert!(hotspots["functions"].as_array().unwrap().is_empty());
        assert!(hotspots["modules"].as_array().unwrap().is_empty());
    }

    #[test]
    fn legacy_facade_calls_rust_hotspots_and_has_no_truncating_fallback() {
        // Issue #361: the facade must call the exhaustive Rust producer and
        // must not carry a reintroduced Select-Object -First N fallback for
        // hotspots specifically (dsm/evolution/what_if keep their own
        // unrelated fallbacks; only hotspots' fallback was provably lossy).
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = std::fs::read_to_string(root.join("legacy/run-code-intel.ps1")).unwrap();
        assert!(
            source.contains("sentrux hotspots $sentruxTargetPath --json"),
            "facade must call the Rust sentrux hotspots producer"
        );
        assert!(
            source.contains("has no PowerShell fallback (issue #361)"),
            "facade must fail closed instead of falling back for hotspots"
        );
        assert!(
            !source.contains("hotspotsProvider = \"powershell_compatibility\""),
            "the lossy PowerShell hotspots fallback must not be reintroduced"
        );
    }
}
