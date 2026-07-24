# Idea File
> Status: IMPLEMENTED_AND_LOCALLY_VERIFIED
> Created: 2026-07-15
> Source: `can1357/pon` selective internalization review

## Abstract
Add a monotonic floor over the Pipeline's existing parity fixtures. A floor records the
currently proven passing fixture set and minimum count; normal validation fails when any floor
case disappears or regresses, and floor updates require an explicit review reason.

## Core Insight
Golden equality proves one fixture still matches, but it does not state that the proven corpus as
a whole may only grow. A committed set-and-count floor turns current compatibility coverage into a
ratchet without changing the underlying parity oracle.

## Target Repo
- Path: `<repo>`
- Branch: current working tree
- Current state: existing parity fixtures and guarded golden updates; no corpus-level monotonic floor

## Success Criteria
- [x] Doctor passes.
- [x] Pipeline emits `summary.md`, `report.json`, and `understanding.md` for the source review.
- [x] Failure categories are explained if nonzero.
- [x] A human can identify the next action from the artifact.
- [x] Every committed floor case executes through the existing parity oracle.
- [x] Missing or failing floor cases reject the run and reject floor updates.
- [x] Floor updates require a non-empty review reason and cannot lower the passing set or count.
- [x] No new runtime dependency or external execution path is introduced.

## Constraints
- Do not add dependencies.
- Do not copy `pon` implementation code; the upstream repository has no declared license.
- Keep `scripts/tests/test-parity-baseline.ps1` as the behavioral oracle and avoid overlapping its current edits.
- Do not add a divergence waiver path until a real, independently reviewed exception class exists.
- Treat the source as a design reference; production authority remains local.

## Open Questions
1. Whether future non-parity suites should share the same floor schema after representative use.
2. Whether progress reporting belongs in CI output or a committed artifact after timing is measured.

## Implementation Notes
- Source review used revision `ab9067dbd2899c64c4d67a4bc27b8ad49472b126`.
- Add a separate floor file and checker so the existing parity test remains untouched.
- Validate the checker against the current five fixtures plus synthetic missing/lowered-floor cases.
