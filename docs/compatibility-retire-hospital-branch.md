# E08 Hospital branch retirement

E08 owns only the embedded Hospital diagnosis/rendering function block and its invocation block in
`run-code-intel.ps1`. Its stable branch id is
`run-code-intel.hospital.embedded-diagnosis-render`; physical line numbers are not authoritative.
B09 semantics, publication, index, and doctor ownership are excluded.

The B09 `diagnosis.hospital` capability is registered and its Rust contract tests prove fail-closed
precedence, rebuildable Markdown views, A09-seeded A01 execution, and stable machine parity against
the legacy facade on the same untrusted authoritative fixture. The normal facade still executes the
embedded PowerShell authority, however, so the replacement atom is not production-routed.

```powershell
pwsh -NoProfile -File tools/compatibility/Test-HospitalRetirementBoundary.ps1
pwsh -NoProfile -File tools/compatibility/New-HospitalRetirementPacket.ps1 `
  -OutDir orchestration/retirements/e08-hospital -EvaluatedAt <unix-seconds> `
  -CodeIntel work/e01-review-target/debug/code-intel.exe
pwsh -NoProfile -File tools/compatibility/Test-HospitalRetirementPacket.ps1 `
  -PacketRoot orchestration/retirements/e08-hospital
```

Rollback is rehearsed only on an exclusive copy. Until the normal facade routes through B09, the
30-day usage window completes, and independent approval exists, E00 remains blocked and E01 remains
a draft validation boundary. Therefore `deletionExecuted=false` and `retired=false`.
