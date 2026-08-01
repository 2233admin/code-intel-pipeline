//! Combining the four per-signal subscores into a single 0-100 score
//! (`combine`), and deriving the percentile/level narration `execute` and
//! `render` need from a baseline sample.

use super::{
    BugMagnet, Churn, DiffShape, TestAsymmetry, WEIGHT_BUG_MAGNET, WEIGHT_CHURN, WEIGHT_DIFF_SHAPE,
    WEIGHT_TEST_ASYMMETRY,
};

pub(super) fn combine(
    diff: &DiffShape,
    asymmetry: &TestAsymmetry,
    bug_magnet: &BugMagnet,
    churn: &Churn,
) -> f64 {
    WEIGHT_DIFF_SHAPE * diff.subscore
        + WEIGHT_TEST_ASYMMETRY * asymmetry.subscore
        + WEIGHT_BUG_MAGNET * bug_magnet.subscore
        + WEIGHT_CHURN * churn.subscore
}

/// Percentage of the baseline sample scoring at or below the target: 100
/// means the target is at least as risky as every sampled commit, 0 means
/// every sampled commit scored higher. An empty sample (no history to
/// compare against, e.g. a brand-new repository) reports 0 rather than an
/// undefined value — a new gate should fail open when it has no evidence,
/// not fail closed.
pub(super) fn compute_percentile(target_score: f64, baseline: &[f64]) -> u32 {
    if baseline.is_empty() {
        return 0;
    }
    let at_or_below = baseline
        .iter()
        .filter(|&&score| score <= target_score)
        .count();
    ((at_or_below as f64 / baseline.len() as f64) * 100.0).round() as u32
}

pub(super) fn level_for_percentile(percentile: u32) -> &'static str {
    if percentile >= 90 {
        "high"
    } else if percentile >= 60 {
        "medium"
    } else {
        "low"
    }
}

pub(super) fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
