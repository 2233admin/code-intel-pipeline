//! #301: converts #157's wall-clock budget into a static `--steps` cap for
//! `weco run`. weco's local CLI never exposes per-step cost ahead of time
//! (that's a cloud Dashboard feature), so this is a fixed-empirical-value
//! conversion, not a measured one — see the issue's grilling notes.

/// Assumed average wall-clock cost of one weco step, in seconds. Deliberately
/// conservative: overestimating cuts `--steps` short (the wall-clock backstop
/// still catches a run that overruns it), underestimating wastes the whole
/// budget on a run that could have taken more steps.
pub(crate) const DEFAULT_SECONDS_PER_STEP: u64 = 30;

/// `steps = wall_clock_budget_secs / seconds_per_step`, floored. A budget
/// too small to afford even one step returns 0 — the caller treats that as
/// "failed before the first step could run" (#157's own precedent: a budget
/// that can't afford the first unit of work is a failure, not a degraded
/// success), not a valid, if tiny, report.
pub(crate) fn steps_from_wall_clock_budget(
    wall_clock_budget_secs: u64,
    seconds_per_step: u64,
) -> u64 {
    if seconds_per_step == 0 {
        return 0;
    }
    wall_clock_budget_secs / seconds_per_step
}

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;
