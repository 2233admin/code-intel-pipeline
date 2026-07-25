# E05 publication branch retirement

E05 owns only the two bounded legacy publication hunks in `run-code-intel.ps1`: staging-directory
allocation and final path rewrite/promotion/legacy completion-marker publication. Artifact generation
and E10 index traversal are excluded. The stable branch id is
`run-code-intel.publication.legacy-staging-marker`; no physical line number is authoritative.

The current `-DagCoordinate` facade returns after A09 and is not connected to A07. A07's internal
seven-phase fail-closed matrix and marker-last success are green, but they are not represented as a
completed facade failure matrix. Consequently the packet remains blocked and cannot authorize deletion.

```powershell
pwsh -NoProfile -File tools/compatibility/Test-PublicationRetirementBoundary.ps1
pwsh -NoProfile -File tools/compatibility/New-PublicationRetirementPacket.ps1 `
  -OutDir orchestration/retirements/e05-publication -EvaluatedAt <unix-seconds> `
  -CodeIntel work/e01-review-target/debug/code-intel.exe
pwsh -NoProfile -File tools/compatibility/Test-PublicationRetirementPacket.ps1 `
  -PacketRoot orchestration/retirements/e05-publication
```

Rollback is rehearsed only on an exclusive copy. Until A09→A07 routing, a real facade phase matrix,
the observation window, and independent approval are complete, `deletionExecuted=false` and
`retired=false`. `update-code-intel-index.ps1` is neither read as deletion input nor modified by E05.
