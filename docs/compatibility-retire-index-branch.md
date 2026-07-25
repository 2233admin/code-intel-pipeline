# E10 index branch retirement

E10 owns only the explicit legacy traversal below `function Read-JsonFile` in
`update-code-intel-index.ps1`. Its stable branch id is
`update-code-intel-index.legacy-compatibility-traversal`. A08 index semantics and E05 publication
ownership are excluded.

Normal public refresh already routes through A08 and emits `code-intel-artifact-index.v1`. Tests
prove rebuild/incremental byte parity and admit only valid A07 runs while diagnosing staging,
markerless, forged, and legacy trees. `-LegacyCompatibilityMode` remains publicly reachable, but its
array output is diagnostic compatibility data and cannot claim the authoritative A08 schema.

```powershell
pwsh -NoProfile -File tools/compatibility/Test-IndexRetirementBoundary.ps1
pwsh -NoProfile -File tools/compatibility/New-IndexRetirementPacket.ps1 `
  -OutDir orchestration/retirements/e10-index -EvaluatedAt <unix-seconds> `
  -CodeIntel work/e01-review-target/debug/code-intel.exe
pwsh -NoProfile -File tools/compatibility/Test-IndexRetirementPacket.ps1 `
  -PacketRoot orchestration/retirements/e10-index
```

E05 publication retirement remains blocked. Until that dependency, the observation window, usage
evidence, and independent approval pass, E00 remains blocked and E01 is only a draft boundary.
Therefore the legacy script remains present, `deletionExecuted=false`, and `retired=false`.
