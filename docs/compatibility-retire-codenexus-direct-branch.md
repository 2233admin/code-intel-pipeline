# E04 CodeNexus direct branch retirement

E04 is restricted to the one normal-run branch in `run-code-intel.ps1` that directly invokes
`Invoke-CodeNexusLite.ps1`. It does not authorize changes to the lite implementation, B04 provider
translation, B05 survival scanning, Hospital diagnosis, Sentrux, publication, or any other facade
branch.

The current call graph is not yet retired. The normal path still invokes the lite script directly.
Separate B04 and B05 facade entry points exist and the full, lite, and unavailable fixtures pass,
but those entry points have not replaced the live normal-path call. Therefore E04 records a real
blocked packet and leaves `run-code-intel.ps1` unchanged.

Generate and validate a fresh packet after building the Rust CLI:

```powershell
pwsh -NoProfile -File tools/compatibility/New-CodeNexusDirectRetirementPacket.ps1 `
  -OutDir orchestration/retirements/e04-codenexus-direct `
  -EvaluatedAt <unix-seconds>
pwsh -NoProfile -File tools/compatibility/Test-CodeNexusDirectRetirementPacket.ps1 `
  -PacketRoot orchestration/retirements/e04-codenexus-direct
```

The packet freezes three B04 modes through one port: full remains primary, lite remains an explicit
fallback/rollback implementation, and unavailable remains `provider_unavailable`. Unavailable then
selects B05, which may report basic repository inventory but must keep structural knowledge unknown.
Provider process and storage ownership remain outside the facade; mode-specific effects are recorded
instead of being flattened into a false parity claim.

The deletion diff is a one-file, one-hunk, replayable delete-only proposal. E01 validates it and then
rejects it because the real E00 decision is blocked. No route substitution or deletion is executed.
Until the normal facade uses B04/B05, the 30-day observation window completes, and independent E00
approval exists, `deletionExecuted=false` and `retired=false` are mandatory. The E00 projection in
`status.json` is mirrored by `PG-011` in the Gain Ledger; both remain blocked evidence of unfinished
retirement, not a completed gain.
