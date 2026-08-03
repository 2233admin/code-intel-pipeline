//! Assembling the JSON report (`build_report`) and the `--format text`
//! rendering derived from it (`render_text`). Both are pure functions of
//! already-scored data — nothing here touches `git` or recomputes a signal.

use std::path::Path;

use serde_json::{json, Value};

use super::scoring::{level_for_percentile, round2};
use super::{
    Scored, BUG_MAGNET_WINDOW_DAYS, CHURN_WINDOW_DAYS, WEIGHT_BUG_MAGNET, WEIGHT_CHURN,
    WEIGHT_DIFF_SHAPE, WEIGHT_TEST_ASYMMETRY,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn build_report(
    repo: &Path,
    revspec: &str,
    scored: Option<&Scored>,
    files_value: Vec<Value>,
    sample_requested: u32,
    sample_used: usize,
    percentile: u32,
    warning: Option<&str>,
) -> Value {
    let score = scored.map_or(0, |scored| scored.score.round() as i64);
    let level = level_for_percentile(percentile);
    // A warning report (no diff to score) has no `Scored` at all; a
    // zero-valued default stands in so the signal shape below is written
    // once and can never drift between the scored and unscored paths.
    let default_scored = Scored::default();
    let scored = scored.unwrap_or(&default_scored);
    let mut signals = signals_value(scored);
    signals["sampleRequested"] = json!(sample_requested);
    signals["sampleUsed"] = json!(sample_used);
    let mut result = json!({
        "schema": "code-intel-change-risk.v1",
        // Resolved Git root the score was computed against — always
        // present, whether it came from `--repo <path>` or (absent that
        // flag) the current directory, so a caller never has to guess which
        // repository a saved report was scored from (issue #114).
        "repo": repo.display().to_string(),
        "revspec": revspec,
        "score": score,
        "risk_percentile": percentile,
        "level": level,
        "signals": signals,
        "files": files_value,
    });
    if let Some(warning) = warning {
        result["warning"] = json!(warning);
    }
    result
}

/// The four signal breakdowns and their weights, without the percentile
/// sampling fields — those belong to a whole-change report, not to a score
/// as such. Shared with `change_risk::score_subset`, which reports the same
/// breakdown per review unit (issue #150); one writer keeps the two shapes
/// from drifting.
pub(super) fn signals_value(scored: &Scored) -> Value {
    json!({
        "diff": {
            "filesTouched": scored.diff.files_touched,
            "insertions": scored.diff.insertions,
            "deletions": scored.diff.deletions,
            "linesChanged": scored.diff.lines_changed,
            "maxFileShare": round2(scored.diff.max_file_share),
            "subscore": round2(scored.diff.subscore),
        },
        "testAsymmetry": {
            "sourceFilesChanged": scored.test_asymmetry.source_files_changed,
            "testFilesChanged": scored.test_asymmetry.test_files_changed,
            "asymmetric": scored.test_asymmetry.asymmetric,
            "subscore": round2(scored.test_asymmetry.subscore),
        },
        "bugMagnet": {
            "windowDays": BUG_MAGNET_WINDOW_DAYS,
            "totalFixCommits": scored.bug_magnet.total_fix_commits,
            "filesWithHistory": scored.bug_magnet.files_with_history,
            "subscore": round2(scored.bug_magnet.subscore),
        },
        "churn": {
            "windowDays": CHURN_WINDOW_DAYS,
            "totalCommits": scored.churn.total_commits,
            "subscore": round2(scored.churn.subscore),
        },
        "weights": {
            "diffShape": WEIGHT_DIFF_SHAPE,
            "testAsymmetry": WEIGHT_TEST_ASYMMETRY,
            "bugMagnet": WEIGHT_BUG_MAGNET,
            "churn": WEIGHT_CHURN,
        },
    })
}

/// Derived strictly from the same JSON `Value` the JSON output serializes —
/// never recomputed — so `--format text` cannot drift from `--format json`.
pub(super) fn render_text(value: &Value) -> String {
    let mut lines = Vec::new();
    lines.push(format!("repo: {}", value["repo"].as_str().unwrap_or("")));
    lines.push(format!(
        "revspec: {}",
        value["revspec"].as_str().unwrap_or("")
    ));
    if let Some(warning) = value.get("warning").and_then(Value::as_str) {
        lines.push(format!("warning: {warning}"));
    }
    lines.push(format!(
        "score: {} (percentile: {}, level: {})",
        value["score"],
        value["risk_percentile"],
        value["level"].as_str().unwrap_or("")
    ));
    let signals = &value["signals"];
    lines.push(format!(
        "diff: {} file(s), +{}/-{}, maxFileShare={}",
        signals["diff"]["filesTouched"],
        signals["diff"]["insertions"],
        signals["diff"]["deletions"],
        signals["diff"]["maxFileShare"]
    ));
    lines.push(format!(
        "testAsymmetry: source={} test={} asymmetric={}",
        signals["testAsymmetry"]["sourceFilesChanged"],
        signals["testAsymmetry"]["testFilesChanged"],
        signals["testAsymmetry"]["asymmetric"]
    ));
    lines.push(format!(
        "bugMagnet (window={}d): totalFixCommits={} filesWithHistory={}",
        signals["bugMagnet"]["windowDays"],
        signals["bugMagnet"]["totalFixCommits"],
        signals["bugMagnet"]["filesWithHistory"]
    ));
    lines.push(format!(
        "churn (window={}d): totalCommits={}",
        signals["churn"]["windowDays"], signals["churn"]["totalCommits"]
    ));
    lines.push(format!(
        "sample: used {} of {} requested",
        signals["sampleUsed"], signals["sampleRequested"]
    ));
    let files = value["files"].as_array().cloned().unwrap_or_default();
    lines.push(format!("files: {}", files.len()));
    for file in &files {
        let marker = if file["isTestFile"].as_bool().unwrap_or(false) {
            " [test]"
        } else {
            ""
        };
        lines.push(format!(
            "  {} +{}/-{}{marker}",
            file["path"].as_str().unwrap_or(""),
            file["insertions"],
            file["deletions"]
        ));
    }
    lines.join("\n")
}
