//! Assembling the agenda JSON and the `--format text` view derived from it.
//! Both are pure functions of already-computed data; `render_text` reads
//! only the `Value` the JSON output serializes, so the two cannot drift.

use std::path::Path;

use serde_json::{json, Value};

use crate::change_risk::Endpoint;

use super::cochange::Observed;
use super::{Unit, COCHANGE_WINDOW_DAYS};

pub(super) struct Report<'a> {
    pub(super) repo: &'a Path,
    pub(super) revspec: &'a str,
    pub(super) endpoint: &'a Endpoint,
    pub(super) anchor_unix: i64,
    pub(super) changed_files: usize,
    pub(super) units: &'a [Unit],
    pub(super) observed: &'a Observed,
    pub(super) min_cochange: usize,
    pub(super) warning: Option<&'a str>,
}

pub(super) fn build_report(report: Report<'_>) -> Value {
    let units: Vec<Value> = report
        .units
        .iter()
        .enumerate()
        .map(|(position, unit)| unit_value(position, unit))
        .collect();
    let mut result = json!({
        "schema": "code-intel-change-agenda.v1",
        // The resolved Git root the agenda was built against, so a saved
        // report never leaves the caller guessing which checkout produced
        // it (same contract as `change risk`, issue #114).
        "repo": report.repo.display().to_string(),
        "revspec": report.revspec,
        "range": report.endpoint.range,
        "tip": report.endpoint.tip,
        "anchorUnix": report.anchor_unix,
        "changedFiles": report.changed_files,
        "unitCount": units.len(),
        "units": units,
        "coChange": {
            "windowDays": COCHANGE_WINDOW_DAYS,
            "minCommits": report.min_cochange,
            "commitsWalked": report.observed.commits_walked,
            "pairsObserved": report.observed.pairs_observed,
            "edgesKept": report.observed.edges_kept,
            "wideCommitsSkipped": report.observed.wide_commits_skipped,
        },
        "enrichment": enrichment_value(),
        "limitations": [
            "Units are clustered from co-change history, not from imports or call graphs: two files that belong together but have never shipped together are separate units.",
            "A unit score ranks units against each other inside this change. It carries no percentile, because a percentile needs a per-unit historical baseline this command does not compute; use `change risk` for the whole change's percentile.",
            "Co-change evidence excludes the commits the scored range is built from, so a change is never evidence of its own coupling.",
        ],
    });
    if let Some(warning) = report.warning {
        result["warning"] = json!(warning);
    }
    result
}

/// What this command deliberately does not answer, and the command that
/// does. Reported as structured fields rather than prose so a consumer can
/// branch on `status` — and so "unavailable" can never be mistaken for
/// "none", which is the failure mode the pipeline's provider contracts
/// exist to prevent.
fn enrichment_value() -> Value {
    json!({
        "testSelection": {
            "status": "unavailable",
            "reason": "Test selection needs the reverse import graph from a committed authoritative run; this command is git-only by contract and will not guess at it.",
            "command": "code-intel change impact --artifact-root <root> --repo <name> --repo-path <checkout> --changed <path>...",
        },
        "structuralRules": {
            "status": "unavailable",
            "reason": "Monolith, layer-boundary, and cycle hits come from Sentrux evidence in a committed run, not from git history.",
            "command": "code-intel run execute --repo . --out <staging> --authority-root <root> --final-name <run-id>",
        },
    })
}

fn unit_value(position: usize, unit: &Unit) -> Value {
    let edges: Vec<Value> = unit
        .edges
        .iter()
        .map(|edge| {
            json!({
                "files": [edge.left, edge.right],
                "coCommits": edge.co_commits,
                "commits": edge.commits,
                "commitsTruncated": edge.commits_truncated,
            })
        })
        .collect();
    json!({
        // Rank-derived, so the worst unit is always unit-1. Stable for a
        // fixed revspec and repository state; not an identity across runs
        // of different ranges.
        "id": format!("unit-{}", position + 1),
        "score": unit.score.round() as i64,
        "files": unit.file_rows,
        "clusterReason": {
            "kind": if edges.is_empty() { "singleton" } else { "co-change" },
            "edges": edges,
        },
        "signals": unit.signals,
        "testSelection": {"status": "unavailable"},
    })
}

pub(super) fn render_text(value: &Value) -> String {
    let mut lines = Vec::new();
    lines.push(format!("repo: {}", value["repo"].as_str().unwrap_or("")));
    lines.push(format!(
        "revspec: {} (range {})",
        value["revspec"].as_str().unwrap_or(""),
        value["range"].as_str().unwrap_or("")
    ));
    if let Some(warning) = value.get("warning").and_then(Value::as_str) {
        lines.push(format!("warning: {warning}"));
    }
    let cochange = &value["coChange"];
    lines.push(format!(
        "changed files: {}  units: {}",
        value["changedFiles"], value["unitCount"]
    ));
    lines.push(format!(
        "co-change: window={}d minCommits={} commitsWalked={} edgesKept={} wideCommitsSkipped={}",
        cochange["windowDays"],
        cochange["minCommits"],
        cochange["commitsWalked"],
        cochange["edgesKept"],
        cochange["wideCommitsSkipped"]
    ));
    for unit in value["units"].as_array().cloned().unwrap_or_default() {
        let reason = &unit["clusterReason"];
        let edges = reason["edges"].as_array().cloned().unwrap_or_default();
        let joined = if edges.is_empty() {
            "singleton".to_string()
        } else {
            format!("co-change, {} edge(s)", edges.len())
        };
        lines.push(format!(
            "{} score {} — {} file(s), {joined}",
            unit["id"].as_str().unwrap_or(""),
            unit["score"],
            unit["files"].as_array().map_or(0, Vec::len)
        ));
        for file in unit["files"].as_array().cloned().unwrap_or_default() {
            let marker = if file["isTestFile"].as_bool().unwrap_or(false) {
                " [test]"
            } else {
                ""
            };
            lines.push(format!(
                "    {} +{}/-{}{marker} fix180d={} churn90d={}",
                file["path"].as_str().unwrap_or(""),
                file["insertions"],
                file["deletions"],
                file["bugFixCommits180d"],
                file["churnCommits90d"]
            ));
        }
        for edge in &edges {
            let files = edge["files"].as_array().cloned().unwrap_or_default();
            lines.push(format!(
                "    joined: {} + {} ({} co-commits: {})",
                files.first().and_then(Value::as_str).unwrap_or(""),
                files.get(1).and_then(Value::as_str).unwrap_or(""),
                edge["coCommits"],
                edge["commits"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|commit| commit.as_str().map(|hash| short_hash(hash).to_string()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    for field in ["testSelection", "structuralRules"] {
        let block = &value["enrichment"][field];
        lines.push(format!(
            "{field}: {} -> {}",
            block["status"].as_str().unwrap_or(""),
            block["command"].as_str().unwrap_or("")
        ));
    }
    lines.join("\n")
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
