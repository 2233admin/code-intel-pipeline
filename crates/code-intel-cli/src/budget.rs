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
    pub(crate) fn limits(&self) -> (u64, u64) {
        (self.wall_clock_limit, self.bytes_limit)
    }

    pub(crate) fn consumed(&self) -> (u64, u64) {
        (self.wall_clock_consumed, self.bytes_consumed)
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
