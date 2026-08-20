//! #301's threshold judgement and report assembly: computes the candidate's
//! improvement over baseline, whether it clears the PR-worthy threshold, and
//! (when it doesn't) why the search stopped where it did — the two different
//! "didn't clear the bar" stories the grilling session called out:
//! `steps_exhausted` (weco searched everything it was given and this just
//! isn't a promising target) vs `budget_exhausted` (the wall-clock backstop
//! cut the search short; more budget might still find something).

use serde_json::{json, Value};

pub(crate) const DEFAULT_MIN_IMPROVEMENT_PERCENT: f64 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Goal {
    Maximize,
    Minimize,
}

impl Goal {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "maximize" => Ok(Self::Maximize),
            "minimize" => Ok(Self::Minimize),
            other => Err(format!(
                "--goal must be 'maximize' or 'minimize', got {other:?}"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoppedBy {
    /// weco ran every step it was allocated; that step budget simply never
    /// produced a candidate clearing the threshold.
    StepsExhausted,
    /// The wall-clock backstop fired before the allocated steps completed —
    /// more budget might still find a better candidate.
    BudgetExhausted,
}

impl StoppedBy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::StepsExhausted => "steps_exhausted",
            Self::BudgetExhausted => "budget_exhausted",
        }
    }
}

/// `(improvement_percent, met_threshold)`. `improvement_percent` is signed
/// and always expressed as "how much better the candidate is than baseline
/// for this goal" — positive is good regardless of maximize/minimize.
pub(crate) fn improvement(baseline: f64, candidate: f64, goal: Goal) -> f64 {
    if baseline == 0.0 {
        return if candidate == baseline {
            0.0
        } else {
            f64::INFINITY.copysign(signed_delta(baseline, candidate, goal))
        };
    }
    let delta = signed_delta(baseline, candidate, goal);
    (delta / baseline.abs()) * 100.0
}

fn signed_delta(baseline: f64, candidate: f64, goal: Goal) -> f64 {
    match goal {
        Goal::Maximize => candidate - baseline,
        Goal::Minimize => baseline - candidate,
    }
}

pub(crate) fn meets_threshold(improvement_percent: f64, min_improvement_percent: f64) -> bool {
    improvement_percent >= min_improvement_percent
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_report(
    target: &str,
    metric: &str,
    goal: Goal,
    baseline: f64,
    best_candidate: Option<f64>,
    best_candidate_diff: Option<&str>,
    min_improvement_percent: f64,
    steps_run: u64,
    steps_planned: u64,
    stopped_by: Option<StoppedBy>,
) -> Value {
    let goal_str = match goal {
        Goal::Maximize => "maximize",
        Goal::Minimize => "minimize",
    };
    match best_candidate {
        None => json!({
            "schema": "code-intel-perf-optimize-run.v1",
            "target": target,
            "metric": metric,
            "goal": goal_str,
            "baseline": baseline,
            "bestCandidate": Value::Null,
            "bestCandidateDiff": Value::Null,
            "improvementPercent": Value::Null,
            "metThreshold": false,
            "minImprovementPercent": min_improvement_percent,
            "stepsRun": steps_run,
            "stepsPlanned": steps_planned,
            "stoppedBy": stopped_by.map(StoppedBy::as_str),
        }),
        Some(best) => {
            let improvement_percent = improvement(baseline, best, goal);
            let met_threshold = meets_threshold(improvement_percent, min_improvement_percent);
            // A threshold-clearing candidate is worth shipping regardless of
            // why the search stopped, so `stoppedBy` only carries meaning
            // (and is only reported) for the "didn't clear the bar" case.
            let stopped_by_field = if met_threshold {
                None
            } else {
                stopped_by.map(StoppedBy::as_str)
            };
            json!({
                "schema": "code-intel-perf-optimize-run.v1",
                "target": target,
                "metric": metric,
                "goal": goal_str,
                "baseline": baseline,
                "bestCandidate": best,
                "bestCandidateDiff": best_candidate_diff,
                "improvementPercent": improvement_percent,
                "metThreshold": met_threshold,
                "minImprovementPercent": min_improvement_percent,
                "stepsRun": steps_run,
                "stepsPlanned": steps_planned,
                "stoppedBy": stopped_by_field,
            })
        }
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
