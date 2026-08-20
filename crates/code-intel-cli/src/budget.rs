//! `Budget` primitive type for bounding query dispatch costs.
//!
//! Two independent dimensions:
//! - **Wall-clock time** (seconds): overall execution deadline, per-node budgets
//! - **Input bytes** (per-node): evidence/artifact byte consumption limit
//!
//! Three-level precedence chain:
//! 1. Command-line flag (overrides all)
//! 2. Project configuration (overrides built-in)
//! 3. Built-in default (when both above are absent)
//!
//! Query methods allow dispatch stages to check feasibility (`estimate_ok`),
//! record actual consumption (`consume`), and query terminal state (`exceeded`).

use std::time::Duration;

/// Default wall-clock time budget (in seconds).
/// Set to 5 minutes — wide enough not to constrain routine
/// evidence queries under current project load, ensuring the budget
/// acts as a safety net rather than a throughput throttle.
const DEFAULT_WALL_CLOCK_SECONDS: u64 = 300;

/// Default per-node input bytes budget.
/// Set to 100 MB — wide enough to handle multi-file evidence artifacts
/// and large commit histories without triggering on current usage patterns.
const DEFAULT_BYTES_BUDGET: u64 = 100 * 1024 * 1024;

/// `Budget` tracks two independent cost dimensions: wall-clock time and
/// input bytes consumed. Either dimension may exceed its limit independently.
///
/// Consumed costs are cumulative; `consume` operations never decrement the
/// totals. Calling `consume` multiple times accumulates the sum.
#[derive(Clone, Debug)]
pub(crate) struct Budget {
    /// Total wall-clock time budget (in seconds).
    wall_clock_limit: u64,
    /// Wall-clock seconds consumed so far.
    wall_clock_consumed: u64,
    /// Per-node input bytes budget.
    bytes_limit: u64,
    /// Input bytes consumed so far.
    bytes_consumed: u64,
    /// When true, cost estimation is impossible and the dispatcher must
    /// use a separate branching path instead of the normal estimate-before-execute flow.
    pub estimable: bool,
}

impl Budget {
    /// Create a new budget from component limits.
    pub(crate) fn new(wall_clock_seconds: u64, bytes: u64, estimable: bool) -> Self {
        Self {
            wall_clock_limit: wall_clock_seconds,
            wall_clock_consumed: 0,
            bytes_limit: bytes,
            bytes_consumed: 0,
            estimable,
        }
    }

    /// Create a budget using built-in defaults.
    pub(crate) fn with_defaults() -> Self {
        Self {
            wall_clock_limit: DEFAULT_WALL_CLOCK_SECONDS,
            wall_clock_consumed: 0,
            bytes_limit: DEFAULT_BYTES_BUDGET,
            bytes_consumed: 0,
            estimable: true,
        }
    }

    /// Create a budget overriding wall-clock time limit.
    pub(crate) fn with_wall_clock(mut self, seconds: u64) -> Self {
        self.wall_clock_limit = seconds;
        self
    }

    /// Create a budget overriding bytes limit.
    pub(crate) fn with_bytes(mut self, bytes: u64) -> Self {
        self.bytes_limit = bytes;
        self
    }

    /// Create a budget marking costs as non-estimable.
    pub(crate) fn non_estimable(mut self) -> Self {
        self.estimable = false;
        self
    }

    /// Query whether an estimated cost fits within the remaining budget.
    ///
    /// Returns `true` if both time and bytes fit; `false` if either dimension
    /// is already exhausted or the estimate would push it over.
    ///
    /// If `estimable` is false, this method returns `false` unconditionally,
    /// signaling that the dispatcher must use the non-estimable branching path.
    /// This prevents silent mishandling of cases where cost cannot be predicted.
    pub(crate) fn estimate_ok(&self, wall_clock_secs: u64, bytes: u64) -> bool {
        if !self.estimable {
            return false;
        }
        let wall_ok = self.wall_clock_consumed + wall_clock_secs <= self.wall_clock_limit;
        let bytes_ok = self.bytes_consumed + bytes <= self.bytes_limit;
        wall_ok && bytes_ok
    }

    /// Record actual cost consumption.
    ///
    /// Adds to the cumulative totals. Calling this multiple times sums the costs.
    /// Does not check whether the totals exceed limits; use `exceeded()` to query.
    pub(crate) fn consume(&mut self, wall_clock_secs: u64, bytes: u64) {
        self.wall_clock_consumed += wall_clock_secs;
        self.bytes_consumed += bytes;
    }

    /// Query whether either budget dimension has been exceeded.
    ///
    /// Returns `true` if wall-clock time OR input bytes have been exceeded.
    /// The caller can use this to distinguish "completed normally" from
    /// "hit budget limit and stopped".
    pub(crate) fn exceeded(&self) -> bool {
        let wall_exceeded = self.wall_clock_consumed > self.wall_clock_limit;
        let bytes_exceeded = self.bytes_consumed > self.bytes_limit;
        wall_exceeded || bytes_exceeded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_default_values() {
        let budget = Budget::with_defaults();
        assert_eq!(budget.wall_clock_limit, DEFAULT_WALL_CLOCK_SECONDS);
        assert_eq!(budget.bytes_limit, DEFAULT_BYTES_BUDGET);
        assert!(budget.estimable);
        assert_eq!(budget.wall_clock_consumed, 0);
        assert_eq!(budget.bytes_consumed, 0);
    }

    #[test]
    fn budget_custom_constructor() {
        let budget = Budget::new(60, 50_000_000, true);
        assert_eq!(budget.wall_clock_limit, 60);
        assert_eq!(budget.bytes_limit, 50_000_000);
        assert!(budget.estimable);
    }

    #[test]
    fn estimate_ok_empty_budget() {
        let budget = Budget::with_defaults();
        assert!(budget.estimate_ok(100, 1_000_000));
    }

    #[test]
    fn estimate_ok_both_dimensions_pass() {
        let budget = Budget::new(100, 1_000_000, true);
        assert!(budget.estimate_ok(50, 500_000));
    }

    #[test]
    fn estimate_ok_wall_clock_at_limit() {
        let budget = Budget::new(100, 1_000_000, true);
        // Exactly at limit should pass
        assert!(budget.estimate_ok(100, 500_000));
    }

    #[test]
    fn estimate_ok_wall_clock_exceeds_limit() {
        let budget = Budget::new(100, 1_000_000, true);
        // Over limit by 1 second
        assert!(!budget.estimate_ok(101, 500_000));
    }

    #[test]
    fn estimate_ok_bytes_at_limit() {
        let budget = Budget::new(100, 1_000_000, true);
        // Exactly at limit should pass
        assert!(budget.estimate_ok(50, 1_000_000));
    }

    #[test]
    fn estimate_ok_bytes_exceeds_limit() {
        let budget = Budget::new(100, 1_000_000, true);
        // Over limit by 1 byte
        assert!(!budget.estimate_ok(50, 1_000_001));
    }

    #[test]
    fn estimate_ok_after_consume() {
        let mut budget = Budget::new(100, 1_000_000, true);
        // Consume 50 seconds and 500k bytes
        budget.consume(50, 500_000);
        // Check remaining capacity: 50 seconds and 500k bytes
        assert!(budget.estimate_ok(50, 500_000));
        // Over either dimension should fail
        assert!(!budget.estimate_ok(51, 500_000));
        assert!(!budget.estimate_ok(50, 500_001));
    }

    #[test]
    fn estimate_ok_non_estimable_always_false() {
        let budget = Budget::new(1000, 10_000_000, false).non_estimable();
        // Even with huge available budget, should return false
        assert!(!budget.estimate_ok(1, 1));
        assert!(!budget.estimate_ok(100, 1_000_000));
    }

    #[test]
    fn consume_accumulates() {
        let mut budget = Budget::new(100, 1_000_000, true);
        budget.consume(10, 100_000);
        assert_eq!(budget.wall_clock_consumed, 10);
        assert_eq!(budget.bytes_consumed, 100_000);

        budget.consume(20, 200_000);
        assert_eq!(budget.wall_clock_consumed, 30);
        assert_eq!(budget.bytes_consumed, 300_000);
    }

    #[test]
    fn consume_does_not_clamp_to_limit() {
        let mut budget = Budget::new(100, 1_000_000, true);
        // Consume more than the limit
        budget.consume(150, 2_000_000);
        assert_eq!(budget.wall_clock_consumed, 150);
        assert_eq!(budget.bytes_consumed, 2_000_000);
    }

    #[test]
    fn exceeded_false_when_within_limits() {
        let mut budget = Budget::new(100, 1_000_000, true);
        budget.consume(50, 500_000);
        assert!(!budget.exceeded());
    }

    #[test]
    fn exceeded_false_at_exact_limits() {
        let mut budget = Budget::new(100, 1_000_000, true);
        budget.consume(100, 1_000_000);
        assert!(!budget.exceeded());
    }

    #[test]
    fn exceeded_true_wall_clock_by_1_second() {
        let mut budget = Budget::new(100, 1_000_000, true);
        budget.consume(101, 500_000);
        assert!(budget.exceeded());
    }

    #[test]
    fn exceeded_true_bytes_by_1_byte() {
        let mut budget = Budget::new(100, 1_000_000, true);
        budget.consume(50, 1_000_001);
        assert!(budget.exceeded());
    }

    #[test]
    fn exceeded_true_both_dimensions() {
        let mut budget = Budget::new(100, 1_000_000, true);
        budget.consume(150, 2_000_000);
        assert!(budget.exceeded());
    }

    #[test]
    fn wall_clock_override() {
        let budget = Budget::with_defaults().with_wall_clock(60);
        assert_eq!(budget.wall_clock_limit, 60);
        // bytes should still be default
        assert_eq!(budget.bytes_limit, DEFAULT_BYTES_BUDGET);
    }

    #[test]
    fn bytes_override() {
        let budget = Budget::with_defaults().with_bytes(50_000_000);
        assert_eq!(budget.bytes_limit, 50_000_000);
        // wall_clock should still be default
        assert_eq!(budget.wall_clock_limit, DEFAULT_WALL_CLOCK_SECONDS);
    }

    #[test]
    fn both_overrides() {
        let budget = Budget::with_defaults()
            .with_wall_clock(120)
            .with_bytes(200_000_000);
        assert_eq!(budget.wall_clock_limit, 120);
        assert_eq!(budget.bytes_limit, 200_000_000);
    }

    #[test]
    fn non_estimable_flag() {
        let budget = Budget::with_defaults().non_estimable();
        assert!(!budget.estimable);
    }

    #[test]
    fn precedence_chain_simulation() {
        // Built-in default
        let default_budget = Budget::with_defaults();
        assert_eq!(default_budget.wall_clock_limit, DEFAULT_WALL_CLOCK_SECONDS);

        // Project config override
        let with_config = Budget::with_defaults().with_wall_clock(180);
        assert_eq!(with_config.wall_clock_limit, 180);

        // CLI flag override (simulated by using custom new())
        let with_cli = Budget::new(90, DEFAULT_BYTES_BUDGET, true);
        assert_eq!(with_cli.wall_clock_limit, 90);

        // CLI should win over config
        assert!(with_cli.wall_clock_limit < with_config.wall_clock_limit);
    }

    #[test]
    fn independent_dimensions() {
        let mut budget = Budget::new(100, 1_000_000, true);
        // Exhaust only the wall-clock dimension
        budget.consume(150, 100_000);
        assert!(budget.exceeded()); // exceeded wall-clock

        // bytes should still be well within limit
        let bytes_estimate = budget.estimate_ok(0, 500_000);
        assert!(!bytes_estimate); // estimate_ok fails because wall-clock already exceeded
    }

    #[test]
    fn bytes_exceed_independently() {
        let mut budget = Budget::new(100, 1_000_000, true);
        // Exhaust only the bytes dimension
        budget.consume(10, 1_500_000);
        assert!(budget.exceeded()); // exceeded bytes

        // wall-clock should still be well within limit
        let wall_estimate = budget.estimate_ok(50, 0);
        assert!(!wall_estimate); // estimate_ok fails because bytes already exceeded
    }

    #[test]
    fn clone_independence() {
        let mut budget1 = Budget::with_defaults();
        budget1.consume(50, 500_000);

        let mut budget2 = budget1.clone();
        budget2.consume(30, 300_000);

        // budget1 should not be affected by budget2's consume
        assert_eq!(budget1.wall_clock_consumed, 50);
        assert_eq!(budget1.bytes_consumed, 500_000);

        assert_eq!(budget2.wall_clock_consumed, 80);
        assert_eq!(budget2.bytes_consumed, 800_000);
    }
}
