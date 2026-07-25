# E09 direct doctor-wrapper retirement

E09 owns only the three direct production-doctor route segments in `invoke-code-intel.ps1`: the
`check-code-intel-tools.ps1` binding, the preflight invocation block, and its existence guard. It
does not own the retained fresh-machine bootstrap script, B10's Rust adapter, A09, publication,
indexing, Hospital, Native Code Evidence, or provider branches.

The branch is not retired. B10 already provides one envelope result for manifest drift and a
present-but-nonconforming provider, redacts secrets, and keeps readiness separate from conformance.
The public preflight still invokes the PowerShell bootstrap directly, however, and the retained
bootstrap is registered and explicitly observation-only but has no declared expiry. E09 must remain
`blocked`, with `deletionExecuted=false` and `retired=false`.

Generate and validate a fresh packet after the B07 toolchain digest is current:

```powershell
pwsh -NoProfile -File tools/compatibility/New-DoctorWrapperRetirementPacket.ps1 `
  -OutDir orchestration/retirements/e09-doctor-wrapper `
  -EvaluatedAt <unix-seconds>
pwsh -NoProfile -File tools/compatibility/Test-DoctorWrapperRetirementPacket.ps1 `
  -PacketRoot orchestration/retirements/e09-doctor-wrapper
```

The packet freezes the three exact B10 tests, static ownership, the retained bootstrap hash, a
three-hunk single-file deletion draft, and exact normalized rollback replay. E01 validates the
ticket/diff shape and rejects it only because E00 remains blocked. The draft never deletes or edits
`check-code-intel-tools.ps1` and is not deletion authority.

`PG-015` mirrors the blocked E00 Gain Ledger projection. A future change must first route public
preflight through B10 and give the retained bootstrap an owned expiry/removal criterion; only then
can observation-window and independent-approval evidence make deletion eligible.
