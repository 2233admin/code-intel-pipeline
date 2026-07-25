# E02 recommender branch retirement

E02 is limited to the duplicated inline workflow recommender in `run-code-intel.ps1` and its one
legacy invocation path. The replacement is `advisory.workflow-recommend`; provider preflight,
publication, CodeNexus, and every other facade branch are outside this ticket.

Generate a fresh, snapshot-bound packet only after building the Rust CLI:

```powershell
pwsh -NoProfile -File tools/compatibility/New-RecommenderRetirementPacket.ps1 `
  -OutDir orchestration/retirements/e02-recommender `
  -EvaluatedAt <unix-seconds>
```

The generator runs the recommender golden/contract/effect/authority checks, executes rollback on an
exclusive temporary copy, creates E00 evidence and an E01 draft ticket, and then runs E00. The E00
approval subject binds `run-code-intel.ps1::run-code-intel.workflow-recommender.inline` and the
single affected file. The deletion evidence is a two-hunk `replayable-delete-only-v1` patch with
base/result blob hashes and no added lines; its prose summary is non-authoritative. The generator
passes that patch through E01 and requires E01 to reject specifically because E00 is still blocked.
It never deletes code. A blocked E00 decision is a hard stop: `status.json` must keep
`deletionExecuted` and `retired` false.

The current packet is intentionally blocked because no 30-day production usage/compatibility
observation, D02 clean-machine repetition, or independent repository-governed approval exists.
Passing local parity tests or reducing line count cannot substitute for those gates.

Rollback rehearsal uses:

```powershell
pwsh -NoProfile -File tools/compatibility/Restore-RecommenderLegacyBranch.ps1 `
  -RehearsalRoot work/e02-recommender-rollback
```

The script extracts only the historical inline recommender and invocation from the requested Git
revision and applies them to a copy. Applying it to a real checkout requires the explicit
`-TargetPath` parameter and remains an independently authorized bounded-window action.
