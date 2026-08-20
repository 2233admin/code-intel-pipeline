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

/// Same as `run_to_completion`, plus an explicit, configurable
/// `OversizePolicy` (issue #307's `--oversize-threshold`). `run execute`
/// uses this instead of `run_to_completion` so the CLI can override the
/// default 80% threshold; direct callers that don't need that (existing
/// tests) keep using `run_to_completion` unchanged.
pub(crate) fn run_to_completion_with_oversize_threshold<E: NodeExecutor>(
    coordinator: Coordinator,
    executor: &E,
    budget: Budget,
    oversize_policy: OversizePolicy,
) -> Result<BudgetedRun, CoordinatorError> {
    run_to_completion_with_estimator_and_oversize_policy(
        coordinator,
        executor,
        budget,
        estimate_dispatch,
        oversize_policy,
    )
}

/// Per-node input-size admission policy (issue #307). When a dispatch's
/// estimated input bytes reach `threshold_percent` of the configured
/// `Budget::bytes_limit`, the dispatch is refused pre-dispatch with a
/// `NodeState::SkippedOversize` classification. Distinct from
/// `not_dispatched` (the budget was already exhausted by sibling
/// dispatches) and the future `timeout` classification (#306).
///
/// The threshold is intentionally a percentage (1..=100) rather than a
/// raw byte count so operators can express it relative to the byte
/// limit. Default 80% matches the alibaba/open-code-review threshold
/// referenced by issue #307.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OversizePolicy {
    threshold_percent: u8,
}

impl OversizePolicy {
    /// Build a policy with a given threshold percentage. 0 and 100
    /// are rejected because they silently degenerate the gate (0
    /// "nothing is oversize", 100 "everything is oversize"); both
    /// are operator mistakes worth failing fast on.
    pub(crate) fn new(threshold_percent: u8) -> Result<Self, CoordinatorError> {
        if (1..=100).contains(&threshold_percent) {
            Ok(Self { threshold_percent })
        } else {
            Err(CoordinatorError::new(
                crate::dag_coordinator::CoordinatorErrorKind::InvalidSpec,
                format!(
                    "oversize threshold must be between 1 and 100 (inclusive), got {}",
                    threshold_percent
                ),
            ))
        }
    }

    pub(crate) fn threshold_percent(&self) -> u8 {
        self.threshold_percent
    }

    /// Returns `true` when `estimated_bytes` meets or exceeds the
    /// threshold applied to `byte_limit`. Multiplies in u128 to
    /// avoid `u64` overflow at large thresholds near the limit.
    fn exceeds(&self, estimated_bytes: u64, byte_limit: u64) -> bool {
        // A zero byte limit is not "everything is oversize"; it's
        // "the budget was never configured", which the dispatch loop
        // reports as `not_dispatched` instead. Leave `exceeds` false
        // so the dispatch flows into the regular `estimate_ok` path.
        if byte_limit == 0 {
            return false;
        }
        (estimated_bytes as u128) * 100 >= (self.threshold_percent as u128) * (byte_limit as u128)
    }
}

impl Default for OversizePolicy {
    fn default() -> Self {
        Self {
            threshold_percent: 80,
        }
    }
}

/// Same semantics as `run_to_completion_with_estimator`, plus the
/// oversize-policy pre-dispatch refusal: a dispatch whose estimated
/// bytes meet the policy is marked `SkippedOversize` on the
/// coordinator and never reaches the executor. Per-dispatch and
/// non-stopping — independent siblings continue normally, the run
/// outcome is not rewritten, and the budget is not consumed.
pub(crate) fn run_to_completion_with_oversize_policy<E, F>(
    coordinator: Coordinator,
    executor: &E,
    budget: Budget,
    estimate: F,
    oversize_policy: OversizePolicy,
) -> Result<BudgetedRun, CoordinatorError>
where
    E: NodeExecutor,
    F: Fn(&Dispatch) -> DispatchCost,
{
    run_to_completion_with_estimator_and_oversize_policy(
        coordinator,
        executor,
        budget,
        estimate,
        oversize_policy,
    )
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
    run_to_completion_with_estimator_and_oversize_policy(
        coordinator,
        executor,
        budget,
        estimate,
        OversizePolicy::default(),
    )
}

/// Core dispatch loop. `oversize_policy` is checked BEFORE
/// `Budget::estimate_ok` (issue #307): any input above the
/// threshold is classified `SkippedOversize` even if the budget
/// would have approved the dispatch. The two classifications
/// (`skipped_oversize` vs `not_dispatched`) are disjoint on the
/// bytes dimension: oversize wins whenever it applies.
pub(crate) fn run_to_completion_with_estimator_and_oversize_policy<E, F>(
    mut coordinator: Coordinator,
    executor: &E,
    mut budget: Budget,
    estimate: F,
    oversize_policy: OversizePolicy,
) -> Result<BudgetedRun, CoordinatorError>
where
    E: NodeExecutor,
    F: Fn(&Dispatch) -> DispatchCost,
{
    let mut stopped_at = None;
    while !coordinator.is_terminal() {
        let batch = coordinator.next_batch()?;
        // Issue #307: cascade-blocked dependents after an oversize refusal
        // leave the coordinator terminal but with no further schedulable
        // work. Recheck terminal before treating empty batch as a hard
        // error.
        if batch.is_empty() {
            if coordinator.is_terminal() {
                break;
            }
            return Err(CoordinatorError::new(
                crate::dag_coordinator::CoordinatorErrorKind::InvalidTransition,
                "DAG has unfinished nodes but no schedulable work",
            ));
        }
        let mut projected = budget.clone();
        let mut accepted = Vec::new();
        let mut remaining = batch.into_iter();
        // `Budget::limits()` returns (wall_clock_limit, bytes_limit).
        let (_, byte_limit) = budget.limits();
        while let Some(dispatch) = remaining.next() {
            let cost = estimate(&dispatch);
            let node_id = dispatch.node_id.clone();
            if oversize_policy.exceeds(cost.bytes, byte_limit) {
                let reason = format!(
                    "estimated dispatch input size {} bytes exceeds {}% of the {} byte budget limit (actual bytes; pre-dispatch refusal)",
                    cost.bytes,
                    oversize_policy.threshold_percent(),
                    byte_limit,
                );
                coordinator.record_skipped_oversize(&node_id, reason, cost.bytes, byte_limit)?;
                continue;
            }
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
    let oversize_skipped: Vec<(&str, u64, u64)> = manifest
        .nodes
        .iter()
        .filter_map(|(id, state)| match state {
            crate::dag_coordinator::NodeState::SkippedOversize {
                actual_bytes,
                byte_limit,
                ..
            } => Some((id.as_str(), *actual_bytes, *byte_limit)),
            _ => None,
        })
        .collect();
    let oversize_json = json!({
        "thresholdPercent": oversize_policy.threshold_percent(),
        "skippedNodes": oversize_skipped.iter().map(|(id, actual, limit)| json!({
            "nodeId": id,
            "actualBytes": actual,
            "byteLimit": limit,
        })).collect::<Vec<_>>(),
    });
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
        "oversize": oversize_json,
    });
    let mut manifest_json = manifest.to_json();
    manifest_json["budget"] = budget_json;
    Ok(BudgetedRun {
        manifest,
        manifest_json,
    })
}

fn estimate_dispatch(dispatch: &Dispatch) -> DispatchCost {
    // Issue #307: the previous estimator serialized the dispatch
    // envelope and used its JSON length as the byte estimate. That
    // missed the actual referenced file size — a 0-byte envelope
    // pointing at an 8 MB artifact looked free, then mid-run the
    // executor crashed. Now sum each referenced input's
    // `VerifiedArtifactRef::byte_size`, which A03 captured at
    // verification time so the estimator sees real bytes without
    // re-reading the files. Falls back to the dispatch JSON size
    // (clamped to 1) when the dispatch has no referenced inputs
    // — the conservative behavior the old estimator always used.
    let mut bytes: u64 = 0;
    for input in &dispatch.inputs {
        bytes += input.byte_size();
    }
    if bytes == 0 {
        bytes = serde_json::to_vec(&json!({
            "node": dispatch.node_id,
            "capability": dispatch.capability,
            "requestIdentity": dispatch.request_identity,
            "inputs": dispatch.inputs.iter().map(|input| input.to_json()).collect::<Vec<_>>(),
        }))
        .expect("dispatch cost serializes")
        .len() as u64;
    }
    DispatchCost::new(1, bytes.max(1))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use crate::budget::Budget;
    use crate::dag_coordinator::{
        Coordinator, CoordinatorErrorKind, DagSpec, Dispatch, DomainVerdict, EdgeSpec,
        NodeExecutor, NodeOutcome, NodeSpec, RunOutcome,
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
