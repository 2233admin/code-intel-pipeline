use std::time::Instant;

use serde_json::{json, Value};

use crate::budget::Budget;
use crate::dag_coordinator::{
    Coordinator, CoordinatorError, Dispatch, NodeExecutor, NodeState, RunManifest, RunOutcome,
};

pub(crate) const BUDGET_STOPPED_EXIT_CODE: i32 = 76;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DispatchCost {
    wall_clock_secs: u64,
    bytes: u64,
}

impl DispatchCost {
    pub(crate) fn new(wall_clock_secs: u64, bytes: u64) -> Self {
        Self {
            wall_clock_secs,
            bytes,
        }
    }
}

#[derive(Debug)]
pub(crate) struct BudgetedRun {
    pub(crate) manifest: RunManifest,
    pub(crate) manifest_json: Value,
}

pub(crate) fn run_to_completion<E: NodeExecutor>(
    coordinator: Coordinator,
    executor: &E,
    budget: Budget,
) -> Result<BudgetedRun, CoordinatorError> {
    run_to_completion_with_estimator(coordinator, executor, budget, estimate_dispatch)
}

pub(crate) fn run_to_completion_with_estimator<E, F>(
    mut coordinator: Coordinator,
    executor: &E,
    mut budget: Budget,
    estimate: F,
) -> Result<BudgetedRun, CoordinatorError>
where
    E: NodeExecutor,
    F: Fn(&Dispatch) -> DispatchCost,
{
    let mut stopped_at = None;
    while !coordinator.is_terminal() {
        let batch = coordinator.next_batch()?;
        if batch.is_empty() {
            return Err(CoordinatorError::new(
                crate::dag_coordinator::CoordinatorErrorKind::InvalidTransition,
                "DAG has unfinished nodes but no schedulable work",
            ));
        }
        let mut projected = budget.clone();
        let mut accepted = Vec::new();
        let mut remaining = batch.into_iter();
        while let Some(dispatch) = remaining.next() {
            let cost = estimate(&dispatch);
            let node_id = dispatch.node_id.clone();
            if !projected.estimate_ok(cost.wall_clock_secs, cost.bytes) {
                let reason = format!(
                    "estimated dispatch cost exceeds remaining budget: wallClockSeconds={}, bytes={}",
                    cost.wall_clock_secs, cost.bytes
                );
                coordinator.record_not_dispatched(&node_id, &reason)?;
                for pending in remaining {
                    coordinator.record_not_dispatched(&pending.node_id, &reason)?;
                }
                coordinator.mark_pending_not_dispatched(reason);
                stopped_at = Some(node_id);
                break;
            }
            projected.consume(cost.wall_clock_secs, cost.bytes);
            accepted.push((dispatch, cost));
        }
        if stopped_at.is_some() {
            break;
        }
        let mut results = std::thread::scope(|scope| {
            accepted
                .into_iter()
                .map(|(dispatch, cost)| {
                    let id = dispatch.node_id.clone();
                    scope.spawn(move || {
                        let started = Instant::now();
                        let outcome = executor.execute(dispatch);
                        (id, cost, outcome, started.elapsed().as_secs().max(1))
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| {
                    handle.join().map_err(|_| {
                        CoordinatorError::new(
                            crate::dag_coordinator::CoordinatorErrorKind::InvalidTransition,
                            "node executor panicked",
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })?;
        results.sort_by(|left, right| left.0.cmp(&right.0));
        for (id, cost, outcome, elapsed) in results {
            coordinator.record(&id, outcome)?;
            budget.consume(elapsed, cost.bytes);
        }
    }
    let mut manifest = coordinator.manifest();
    if stopped_at.is_some() {
        let executed = manifest.nodes.values().any(|state| {
            matches!(
                state,
                NodeState::Succeeded { .. }
                    | NodeState::DomainFailed { .. }
                    | NodeState::ProcessFailed { .. }
            )
        });
        manifest.outcome = if executed {
            RunOutcome::BudgetStopped
        } else {
            RunOutcome::Failed
        };
    }

    let (wall_limit, byte_limit) = budget.limits();
    let (wall_consumed, bytes_consumed) = budget.consumed();
    let budget_json = json!({
        "limits": {
            "wallClockSeconds": wall_limit,
            "bytes": byte_limit,
        },
        "consumed": {
            "wallClockSeconds": wall_consumed,
            "bytes": bytes_consumed,
        },
        "exceeded": stopped_at.is_some() || budget.exceeded(),
        "stoppedAt": stopped_at,
    });
    let mut manifest_json = manifest.to_json();
    manifest_json["budget"] = budget_json;
    Ok(BudgetedRun {
        manifest,
        manifest_json,
    })
}

fn estimate_dispatch(dispatch: &Dispatch) -> DispatchCost {
    let bytes = serde_json::to_vec(&json!({
        "node": dispatch.node_id,
        "capability": dispatch.capability,
        "requestIdentity": dispatch.request_identity,
        "inputs": dispatch.inputs.iter().map(|input| input.to_json()).collect::<Vec<_>>(),
    }))
    .expect("dispatch cost serializes")
    .len() as u64;
    DispatchCost::new(1, bytes.max(1))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use crate::budget::Budget;
    use crate::dag_coordinator::{
        Coordinator, CoordinatorErrorKind, DagSpec, Dispatch, DomainVerdict, NodeExecutor,
        NodeOutcome, NodeSpec, RunOutcome,
    };

    use super::{run_to_completion_with_estimator, DispatchCost, BUDGET_STOPPED_EXIT_CODE};

    struct PassExecutor;

    impl NodeExecutor for PassExecutor {
        fn execute(&self, _dispatch: Dispatch) -> NodeOutcome {
            NodeOutcome::success(DomainVerdict::Pass, Vec::new())
        }
    }

    fn coordinator() -> Coordinator {
        Coordinator::new(DagSpec::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
            vec![
                NodeSpec::new("a", "fixture.a", "request:a"),
                NodeSpec::new("b", "fixture.b", "request:b"),
            ],
            Vec::new(),
        ))
        .unwrap()
    }
    struct ParallelProbe {
        active: AtomicUsize,
        peak: AtomicUsize,
    }

    impl NodeExecutor for ParallelProbe {
        fn execute(&self, _dispatch: Dispatch) -> NodeOutcome {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(20));
            self.active.fetch_sub(1, Ordering::SeqCst);
            NodeOutcome::success(DomainVerdict::Pass, Vec::new())
        }
    }

    struct PanicProbe;

    impl NodeExecutor for PanicProbe {
        fn execute(&self, _dispatch: Dispatch) -> NodeOutcome {
            panic!("fixture executor panic")
        }
    }

    #[test]
    fn budget_dispatch_preserves_parallel_batches_and_maps_executor_panics() {
        let parallel_coordinator = Coordinator::new(DagSpec::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            2,
            vec![
                NodeSpec::new("a", "fixture.a", "request:a"),
                NodeSpec::new("b", "fixture.b", "request:b"),
                NodeSpec::new("c", "fixture.c", "request:c"),
            ],
            Vec::new(),
        ))
        .unwrap();
        let probe = ParallelProbe {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        };
        run_to_completion_with_estimator(
            parallel_coordinator,
            &probe,
            Budget::new(10, 100, true),
            |_| DispatchCost::new(1, 1),
        )
        .unwrap();
        assert_eq!(probe.peak.load(Ordering::SeqCst), 2);

        let error = run_to_completion_with_estimator(
            coordinator(),
            &PanicProbe,
            Budget::new(10, 100, true),
            |_| DispatchCost::new(1, 1),
        )
        .unwrap_err();
        assert_eq!(error.kind(), CoordinatorErrorKind::InvalidTransition);
    }

    #[test]
    fn budget_dispatch_covers_completed_stopped_and_first_node_failed() {
        assert_eq!(BUDGET_STOPPED_EXIT_CODE, 76);
        let completed = run_to_completion_with_estimator(
            coordinator(),
            &PassExecutor,
            Budget::new(10, 100, true),
            |_| DispatchCost::new(1, 1),
        )
        .unwrap();
        assert_eq!(completed.manifest.outcome, RunOutcome::Completed);

        let stopped = run_to_completion_with_estimator(
            coordinator(),
            &PassExecutor,
            Budget::new(1, 100, true),
            |_| DispatchCost::new(1, 1),
        )
        .unwrap();
        assert_eq!(stopped.manifest.outcome, RunOutcome::BudgetStopped);
        assert_eq!(
            stopped.manifest_json["nodes"]["b"]["status"],
            "not_dispatched"
        );
        assert_eq!(stopped.manifest_json["budget"]["exceeded"], true);

        let failed = run_to_completion_with_estimator(
            coordinator(),
            &PassExecutor,
            Budget::new(0, 100, true),
            |_| DispatchCost::new(1, 1),
        )
        .unwrap();
        assert_eq!(failed.manifest.outcome, RunOutcome::Failed);
    }
}
