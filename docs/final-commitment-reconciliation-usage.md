# Final commitment reconciliation usage

The authoritative reconciliation is
`orchestration/evidence/final-commitment-reconciliation.json`. Its 69 records are derived from the
ticket headings in the frozen `docs/plans/adr-0010-execution-plan.md`; the JSON records claim state,
evidence commands, existing artifacts, independent verdict, and blockers. The Markdown table in
`docs/final-commitment-reconciliation.md` is only a human-readable projection.
Its structural contract is
`orchestration/schemas/code-intel-final-commitment-reconciliation.v1.schema.json`.

Validate both representations with:

```powershell
pwsh -NoProfile -File tools/Test-FinalCommitmentReconciliation.ps1
```

The validator fails on a changed ADR digest, any missing/duplicate/reordered ID, an unknown status,
a missing evidence artifact, an incoherent verdict/blocker combination, a retirement packet that
does not actually say `blocked`/`deletionExecuted=false`/`retired=false`, or projection drift.

Status rules are intentionally conservative:

- `implemented_verified` requires a concrete command, existing artifact, independent `verified`
  verdict, and no blocker.
- `implemented_pending_verification` and `implemented_blocked` are implementation states, not plan
  states. They require corresponding verification evidence and must never be used just because a
  plan, draft, or packet exists.
- `retirement_blocked` means the retirement evidence packet exists but deletion did not execute.
- `not_implemented` is used for in-progress work that has no verified implementation claim.

When a ticket changes, update the JSON first, run its evidence command, record only the verdict that
the evidence supports, update the Markdown projection in the same change, and rerun the validator.
Do not turn `planned`, `draft`, `packet exists`, or `in progress` into an implemented claim.
