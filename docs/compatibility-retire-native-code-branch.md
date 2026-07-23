# E07 embedded Native Code Evidence retirement

E07 is restricted to the embedded Native Code Evidence function family and its direct
`New-CodeEvidenceLayer` call in `run-code-intel.ps1`. It does not own the B08 algorithm, A09 DAG,
publication, committed-run indexing, Hospital diagnosis, provider adapters, or other facade branches.

The branch is not retired. Normal and full modes still reach the embedded call unless the separate
`-DagCoordinate` route is explicitly selected. B08 is declared and A09 can execute it after Snapshot
Identity and inventory, but that opt-in route has not replaced the public normal/full path. E07 must
therefore remain `blocked`, with `deletionExecuted=false` and `retired=false`.

Generate and validate a fresh packet only after the R06/B07 toolchain digest is current:

```powershell
pwsh -NoProfile -File tools/compatibility/New-NativeCodeRetirementPacket.ps1 `
  -OutDir orchestration/retirements/e07-native-code `
  -EvaluatedAt <unix-seconds>
pwsh -NoProfile -File tools/compatibility/Test-NativeCodeRetirementPacket.ps1 `
  -PacketRoot orchestration/retirements/e07-native-code
```

The packet freezes normalized legacy/B08 artifact parity, eight verified Artifact Refs, the exact
`repo_read` and `local_write` effects, explicit unknown relationship/call-graph precision for
unsupported languages, B07 registry reconciliation, and a two-segment single-branch rollback replay.
E01 validates the replayable deletion draft and rejects it while E00 remains blocked.

`PG-012` mirrors the E00 Gain Ledger projection. It is unfinished retirement evidence, not a
completed deletion or a claim that normal/full already execute through A09.
