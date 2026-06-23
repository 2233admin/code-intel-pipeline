# Agent Goal Intake

Agent Goal Intake is the layer before Code Intel Pipeline. It turns vague work into a task contract before the scanner produces repository evidence.

Use this layer for complex, long-running, or high-risk work, especially quantitative-system development where data lineage, target scope, credentials, and verification evidence matter.

## Supported Concepts

`qiaomu-goal-meta-skill` is a good fit for goal authoring. It turns a vague request into a paste-ready `/goal` with outcome, verification, constraints, boundaries, iteration policy, completion evidence, and pause conditions.

`awesome-agent-loops` is a useful loop-pattern catalog. It helps choose between:

- `/goal`: keep working until a verifiable condition is true.
- `/loop`: re-run a watch or maintenance prompt on an interval.
- `/schedule`: run a recurring cloud routine.

## Boundary

Do not wire these concepts into the scanner runtime.

The scanner remains responsible for fresh repository evidence and artifact runs. Goal and loop intake are upstream task-contract tools. They can reference artifact evidence, but they do not write `report.json`, `hospital-report.json`, or scanner-owned files.

## Recommended Flow

1. For vague work, write an Agent Goal Intake contract first.
2. Run Code Intel Pipeline on the target repository or target scope.
3. Read `summary.md`, `understanding.md`, and `hospital.md`.
4. Convert the hospital report's `next_protocol` into the right loop shape:
   - `diagnose`: use a short `/goal` to refresh missing evidence or understand the blocker.
   - `github_solution_research`: use a bounded `/goal` for upstream evidence gathering.
   - `surgery_plan`: use a focused `/goal` for one bounded repair.
   - `post_op`: use `/loop` only when watching repeated checks or CI status.
5. Stop when artifact evidence proves the condition, or pause if the contract requires human authority.

## Quant System Defaults

For quantitative-system work, the intake contract should name:

- target repository and target scope
- data source boundaries
- whether production credentials or live trading accounts are forbidden
- backtest or paper-trading evidence required
- exact checks, reports, or artifact files that prove completion
- lineage from goal to artifact run to final recommendation

If any task requires production capital, account credentials, irreversible migrations, paid data access, or financial advice, pause before execution.
