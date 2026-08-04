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

/// Every test here asserts a *number*, not a shape.
///
/// The arithmetic above decides whether a pull request is blocked, and until
/// issue #179 it had no direct test at all. Everything reaching it came
/// through end-to-end paths that assert the report's *shape* -- field
/// present, schema valid, array non-empty -- so `cargo mutants --file
/// .../scoring.rs -- --bin code-intel` left 26 of its 31 injected defects
/// alive against the 544-test unit-test oracle, including `combine`
/// returning a constant `0.0`, `compute_percentile` returning a constant
/// `0`, and `level_for_percentile` returning `""`. Every one of those stayed
/// green.
///
/// So the subscores below are chosen deliberately: distinct, non-zero, never
/// `1.0`, and exactly representable in binary (halves, quarters, eighths).
/// Distinct and non-`1.0` so that swapping any `*` for `+` or `/`, or any
/// `+` for `-` or `*`, lands on a different total -- `w * 1.0` and `w / 1.0`
/// are the same number, which is exactly how an untested formula hides.
/// Exactly representable so the expected values can be compared with `==`
/// instead of an epsilon that would re-open the slack this file is closing.
#[cfg(test)]
mod tests {
    use super::*;

    /// The four subscores used across these tests: distinct, exact, and each
    /// one multiplied by its own weight gives a distinct product.
    /// 30*0.5=15, 25*0.25=6.25, 25*0.75=18.75, 20*0.125=2.5 -- total 42.5.
    const DIFF: f64 = 0.5;
    const ASYMMETRY: f64 = 0.25;
    const BUG_MAGNET: f64 = 0.75;
    const CHURN: f64 = 0.125;

    fn signals(
        diff: f64,
        asymmetry: f64,
        bug_magnet: f64,
        churn: f64,
    ) -> (DiffShape, TestAsymmetry, BugMagnet, Churn) {
        (
            DiffShape {
                subscore: diff,
                ..Default::default()
            },
            TestAsymmetry {
                subscore: asymmetry,
                ..Default::default()
            },
            BugMagnet {
                subscore: bug_magnet,
                ..Default::default()
            },
            Churn {
                subscore: churn,
                ..Default::default()
            },
        )
    }

    fn combined(diff: f64, asymmetry: f64, bug_magnet: f64, churn: f64) -> f64 {
        let (d, a, b, c) = signals(diff, asymmetry, bug_magnet, churn);
        combine(&d, &a, &b, &c)
    }

    /// Pins the whole formula to one number. Every arithmetic operator in
    /// `combine` -- the four `*` and the three `+` -- moves this total off
    /// 42.5 when substituted, so this single assertion is what stands
    /// between the weighting structure and a silent rewrite.
    #[test]
    fn combine_is_the_documented_weighted_sum_of_the_four_subscores() {
        assert_eq!(combined(DIFF, ASYMMETRY, BUG_MAGNET, CHURN), 42.5);
    }

    /// The 30/25/25/20 split is a documented product decision (see the
    /// `WEIGHT_*` doc comments), not an implementation detail: test
    /// asymmetry and bug magnet are deliberately equal, and diff shape
    /// deliberately outranks both. Isolating one signal at a time pins each
    /// weight to its own number so a reshuffle cannot pass by keeping the
    /// total constant.
    #[test]
    fn each_signal_contributes_exactly_its_documented_weight() {
        assert_eq!(combined(DIFF, 0.0, 0.0, 0.0), 15.0, "diff shape @ 30");
        assert_eq!(combined(0.0, ASYMMETRY, 0.0, 0.0), 6.25, "asymmetry @ 25");
        assert_eq!(
            combined(0.0, 0.0, BUG_MAGNET, 0.0),
            18.75,
            "bug magnet @ 25"
        );
        assert_eq!(combined(0.0, 0.0, 0.0, CHURN), 2.5, "churn @ 20");
    }

    /// The module doc calls this "a single 0-100 score". Nothing tested that
    /// claim: the weights summing to 100 is what makes it true, and a single
    /// weight edit would have broken it silently.
    #[test]
    fn saturated_signals_score_exactly_one_hundred_and_idle_signals_score_zero() {
        assert_eq!(combined(1.0, 1.0, 1.0, 1.0), 100.0);
        assert_eq!(combined(0.0, 0.0, 0.0, 0.0), 0.0);
    }

    /// Five samples, three at or below the target: 3/5 -> 60. The asymmetric
    /// split is deliberate -- with an even split, counting the samples
    /// *above* the target would return the same percentile and the
    /// comparison direction would go untested.
    #[test]
    fn compute_percentile_is_the_share_of_the_sample_at_or_below_the_target() {
        let baseline = [10.0, 20.0, 30.0, 40.0, 50.0];

        assert_eq!(compute_percentile(35.0, &baseline), 60);
    }

    /// `<=`, not `<`: a change that scores exactly as high as every sampled
    /// commit is at the 100th percentile, not the 0th.
    #[test]
    fn a_score_matching_the_sample_counts_as_at_or_below_it() {
        assert_eq!(compute_percentile(10.0, &[10.0, 10.0]), 100);
        assert_eq!(compute_percentile(9.0, &[10.0, 10.0]), 0);
    }

    /// Documented fail-open: a brand-new repository has no history to
    /// compare against, and a gate with no evidence must not manufacture a
    /// high percentile out of an empty sample.
    #[test]
    fn an_empty_baseline_reports_zero_rather_than_an_undefined_percentile() {
        assert_eq!(compute_percentile(99.0, &[]), 0);
    }

    /// Both thresholds pinned from both sides. These three strings are the
    /// words a reviewer actually reads, and 90/60 are the numbers the PR
    /// gate acts on -- issue #170 was a false "high" that no test objected
    /// to.
    #[test]
    fn level_thresholds_hold_at_their_documented_boundaries() {
        assert_eq!(level_for_percentile(0), "low");
        assert_eq!(level_for_percentile(59), "low");
        assert_eq!(level_for_percentile(60), "medium");
        assert_eq!(level_for_percentile(89), "medium");
        assert_eq!(level_for_percentile(90), "high");
        assert_eq!(level_for_percentile(100), "high");
    }

    /// Half-cent rounding is half-away-from-zero (Rust's `f64::round`), and
    /// an already-short value must not acquire trailing noise.
    #[test]
    fn round2_keeps_two_decimals() {
        assert_eq!(round2(42.4949), 42.49);
        assert_eq!(round2(42.495), 42.5);
        assert_eq!(round2(42.5), 42.5);
    }
}
