//! Issue #307 tests for `OversizePolicy` and the `skipped_oversize`
//! classification. Sibling test module to `budget_dispatch.rs`: the
//! original three tests stay in that file (under the 800-line god-file
//! gate); the seven oversize tests live here.
//!
//! Each test follows the existing budget-dispatch pattern: build a tiny
//! DAG, supply a counting executor, drive `run_to_completion_with_oversize_policy`,
//! and assert both the manifest and the side-effect count.
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::budget::Budget;
    use crate::budget_dispatch::{
        run_to_completion_with_oversize_policy, DispatchCost, OversizePolicy,
    };
    use crate::dag_coordinator::{
        Coordinator, DagSpec, Dispatch, DomainVerdict, EdgeSpec, NodeExecutor, NodeOutcome,
        NodeSpec, RunOutcome,
    };

    /// Local pass-through executor. The one in `budget_dispatch::tests`
    /// is `pub(super)`-private to that module and not visible here.
    struct PassExecutor;

    impl NodeExecutor for PassExecutor {
        fn execute(&self, _dispatch: Dispatch) -> NodeOutcome {
            NodeOutcome::success(DomainVerdict::Pass, Vec::new())
        }
    }

    fn single_node_coordinator(id: &str, capability: &str) -> Coordinator {
        Coordinator::new(DagSpec::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
            vec![NodeSpec::new(id, capability, format!("request:{id}"))],
            Vec::new(),
        ))
        .unwrap()
    }

    /// Counting executor that records how many times the dispatcher
    /// actually invoked the executor. Lets the regression assertion be
    /// "executor was never called", independent of what the manifest
    /// happens to record.
    struct CountingExecutor {
        dispatched: AtomicUsize,
    }

    impl NodeExecutor for CountingExecutor {
        fn execute(&self, _dispatch: Dispatch) -> NodeOutcome {
            self.dispatched.fetch_add(1, Ordering::SeqCst);
            NodeOutcome::success(DomainVerdict::Pass, Vec::new())
        }
    }

    #[test]
    fn budget_dispatch_marks_oversize_input_and_skips_executor() {
        let counter = CountingExecutor {
            dispatched: AtomicUsize::new(0),
        };
        let budgeted = run_to_completion_with_oversize_policy(
            single_node_coordinator("a", "fixture.a"),
            &counter,
            Budget::new(60, 100, true),
            |_| DispatchCost::new(1, 90),
            OversizePolicy::new(80).expect("policy 80"),
        )
        .expect("oversize dispatch returns a manifest, not an error");
        assert_eq!(
            counter.dispatched.load(Ordering::SeqCst),
            0,
            "oversize node must not reach the executor"
        );
        let node = &budgeted.manifest_json["nodes"]["a"];
        assert_eq!(node["status"], "skipped_oversize");
        assert_eq!(node["actualBytes"], 90);
        assert_eq!(node["byteLimit"], 100);
    }

    #[test]
    fn budget_dispatch_oversize_manifest_exposes_actual_bytes_and_limit() {
        let counter = CountingExecutor {
            dispatched: AtomicUsize::new(0),
        };
        let budgeted = run_to_completion_with_oversize_policy(
            single_node_coordinator("a", "fixture.a"),
            &counter,
            Budget::new(60, 1_000_000, true),
            |_| DispatchCost::new(1, 950_000),
            OversizePolicy::new(80).expect("policy 80"),
        )
        .unwrap();
        assert_eq!(budgeted.manifest_json["nodes"]["a"]["actualBytes"], 950_000);
        assert_eq!(budgeted.manifest_json["nodes"]["a"]["byteLimit"], 1_000_000);
        let oversize = &budgeted.manifest_json["budget"]["oversize"];
        assert_eq!(oversize["thresholdPercent"], 80);
        assert_eq!(oversize["skippedNodes"][0]["nodeId"], "a");
        assert_eq!(oversize["skippedNodes"][0]["actualBytes"], 950_000);
        assert_eq!(oversize["skippedNodes"][0]["byteLimit"], 1_000_000);
    }

    #[test]
    fn budget_dispatch_oversize_threshold_is_configurable() {
        let counter = CountingExecutor {
            dispatched: AtomicUsize::new(0),
        };
        let strict = run_to_completion_with_oversize_policy(
            single_node_coordinator("a", "fixture.a"),
            &counter,
            Budget::new(60, 100, true),
            |_| DispatchCost::new(1, 60),
            OversizePolicy::new(50).expect("policy 50"),
        )
        .unwrap();
        assert_eq!(
            strict.manifest_json["nodes"]["a"]["status"],
            "skipped_oversize"
        );
        assert_eq!(counter.dispatched.load(Ordering::SeqCst), 0);

        counter.dispatched.store(0, Ordering::SeqCst);
        let loose = run_to_completion_with_oversize_policy(
            single_node_coordinator("a", "fixture.a"),
            &counter,
            Budget::new(60, 100, true),
            |_| DispatchCost::new(1, 60),
            OversizePolicy::new(70).expect("policy 70"),
        )
        .unwrap();
        assert_eq!(loose.manifest_json["nodes"]["a"]["status"], "succeeded");
        assert_eq!(counter.dispatched.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn budget_dispatch_rejects_eight_megabyte_input_against_ten_megabyte_limit() {
        let counter = CountingExecutor {
            dispatched: AtomicUsize::new(0),
        };
        let eight_mib: u64 = 8 * 1024 * 1024;
        let ten_mib: u64 = 10 * 1024 * 1024;
        let budgeted = run_to_completion_with_oversize_policy(
            single_node_coordinator("graph", "evidence.graph"),
            &counter,
            Budget::new(60, ten_mib, true),
            |_| DispatchCost::new(1, eight_mib),
            OversizePolicy::new(50).expect("policy 50"),
        )
        .expect("8MB dispatch returns a manifest, not a process failure");
        assert_eq!(
            counter.dispatched.load(Ordering::SeqCst),
            0,
            "8MB input must be refused before any dispatch"
        );
        let node = &budgeted.manifest_json["nodes"]["graph"];
        assert_eq!(node["status"], "skipped_oversize");
        assert_eq!(node["actualBytes"], eight_mib);
        assert_eq!(node["byteLimit"], ten_mib);
    }

    /// Acceptance criterion 5: `skipped_oversize`, `not_dispatched`, and
    /// `timeout` (the future #306 sibling classification) are three
    /// distinct manifest statuses. This test guards the disjointness
    /// of the two classifications that this PR owns.
    #[test]
    fn budget_dispatch_distinguishes_skipped_oversize_from_not_dispatched() {
        // The "exhausted" path: budget already too small for any
        // dispatch, so a 1-byte cost trips the budget.
        let exhausted = run_to_completion_with_oversize_policy(
            single_node_coordinator("a", "fixture.a"),
            &PassExecutor,
            Budget::new(1, 0, true),
            |_| DispatchCost::new(1, 1),
            OversizePolicy::new(80).expect("policy 80"),
        )
        .unwrap();
        assert_eq!(
            exhausted.manifest_json["nodes"]["a"]["status"],
            "not_dispatched"
        );
        assert_ne!(
            exhausted.manifest_json["nodes"]["a"]["status"],
            "skipped_oversize"
        );

        // The "oversize" path: budget has room (1 MB limit) but a 900 KB
        // estimate trips the 80% threshold.
        let counter = CountingExecutor {
            dispatched: AtomicUsize::new(0),
        };
        let oversize = run_to_completion_with_oversize_policy(
            single_node_coordinator("a", "fixture.a"),
            &counter,
            Budget::new(60, 1_000_000, true),
            |_| DispatchCost::new(1, 900_000),
            OversizePolicy::new(80).expect("policy 80"),
        )
        .unwrap();
        assert_eq!(
            oversize.manifest_json["nodes"]["a"]["status"],
            "skipped_oversize"
        );
        assert_ne!(
            oversize.manifest_json["nodes"]["a"]["status"],
            "not_dispatched"
        );
        // Per the per-dispatch refusal semantics, the run is NOT
        // rewritten to `BudgetStopped` and the executor is not invoked.
        assert_eq!(
            oversize.manifest.outcome,
            RunOutcome::Completed,
            "oversize is a per-dispatch refusal, not a run-level failure"
        );
        assert_eq!(counter.dispatched.load(Ordering::SeqCst), 0);
    }

    /// Oversize check must precede `estimate_ok` so any input above the
    /// 100% of the limit is classified `skipped_oversize` (the correct
    /// admission reason), not `not_dispatched` (which is reserved for
    /// the "budget already exhausted" case).
    #[test]
    fn budget_dispatch_oversize_check_runs_before_budget_exhaustion_check() {
        let counter = CountingExecutor {
            dispatched: AtomicUsize::new(0),
        };
        // 200-byte input against a 100-byte limit (200%): above both
        // the 80% oversize threshold and the 100% byte limit. The
        // oversize reason is the more informative one, so it wins.
        let budgeted = run_to_completion_with_oversize_policy(
            single_node_coordinator("a", "fixture.a"),
            &counter,
            Budget::new(60, 100, true),
            |_| DispatchCost::new(1, 200),
            OversizePolicy::new(80).expect("policy 80"),
        )
        .unwrap();
        assert_eq!(
            budgeted.manifest_json["nodes"]["a"]["status"],
            "skipped_oversize"
        );
        assert_eq!(budgeted.manifest_json["nodes"]["a"]["actualBytes"], 200);
        assert_eq!(counter.dispatched.load(Ordering::SeqCst), 0);
    }

    /// A node whose dependents depend on its output must see those
    /// dependents become `dependency_blocked`, because the input the
    /// skipped node would have produced is not coming.
    #[test]
    fn budget_dispatch_oversize_node_cascade_blocks_its_dependents() {
        let coordinator = Coordinator::new(DagSpec::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
            vec![
                NodeSpec::new("a", "fixture.a", "request:a"),
                NodeSpec::new("b", "fixture.b", "request:b"),
            ],
            vec![EdgeSpec::new("a", "b")],
        ))
        .unwrap();
        let counter = CountingExecutor {
            dispatched: AtomicUsize::new(0),
        };
        let budgeted = run_to_completion_with_oversize_policy(
            coordinator,
            &counter,
            Budget::new(60, 100, true),
            |_| DispatchCost::new(1, 90),
            OversizePolicy::new(80).expect("policy 80"),
        )
        .unwrap();
        assert_eq!(
            budgeted.manifest_json["nodes"]["a"]["status"],
            "skipped_oversize"
        );
        assert_eq!(
            budgeted.manifest_json["nodes"]["b"]["status"],
            "dependency_blocked"
        );
        assert_eq!(counter.dispatched.load(Ordering::SeqCst), 0);
    }

    /// Independent siblings in the same batch as the oversize node must
    /// continue: the oversize classification is per-dispatch, not a
    /// batch-wide stop.
    #[test]
    fn budget_dispatch_oversize_does_not_block_independent_siblings() {
        let counter = CountingExecutor {
            dispatched: AtomicUsize::new(0),
        };
        let coordinator = Coordinator::new(DagSpec::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            2,
            vec![
                NodeSpec::new("oversize", "fixture.big", "request:big"),
                NodeSpec::new("small", "fixture.small", "request:small"),
            ],
            Vec::new(),
        ))
        .unwrap();
        let budgeted = run_to_completion_with_oversize_policy(
            coordinator,
            &counter,
            Budget::new(60, 100, true),
            |dispatch: &Dispatch| {
                if dispatch.node_id == "oversize" {
                    DispatchCost::new(1, 95)
                } else {
                    DispatchCost::new(1, 10)
                }
            },
            OversizePolicy::new(80).expect("policy 80"),
        )
        .unwrap();
        assert_eq!(
            budgeted.manifest_json["nodes"]["oversize"]["status"],
            "skipped_oversize"
        );
        assert_eq!(
            budgeted.manifest_json["nodes"]["small"]["status"],
            "succeeded"
        );
        assert_eq!(counter.dispatched.load(Ordering::SeqCst), 1);
    }

    /// `OversizePolicy::new` rejects out-of-range threshold values
    /// (0 means "nothing is oversize", 100 means "everything is
    /// oversize"; both are operator mistakes worth failing fast on).
    #[test]
    fn budget_dispatch_oversize_policy_rejects_out_of_range_thresholds() {
        assert!(OversizePolicy::new(0).is_err());
        assert!(OversizePolicy::new(101).is_err());
        assert!(OversizePolicy::new(80).is_ok());
        assert_eq!(OversizePolicy::new(80).unwrap().threshold_percent(), 80);
        assert_eq!(OversizePolicy::default().threshold_percent(), 80);
    }
}
