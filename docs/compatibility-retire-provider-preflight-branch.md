# E03 provider-preflight branch retirement

E03 is restricted to the historical direct production call from `run-code-intel.ps1` to
`test-code-intel-provider.ps1`. The current production route calls
`Invoke-RepowiseProviderProbe.ps1`; the test-named script is a test-only compatibility wrapper.
`install-code-intel-pipeline.ps1 -CheckProvider` is a separate installer diagnostic and is not
authorized by this single-branch retirement.

Run the static boundary check with:

```powershell
pwsh -NoProfile -File tools/compatibility/Test-ProviderPreflightRetirementBoundary.ps1
```

Generate and validate the independent historical-base packet with an exclusive output directory:

```powershell
pwsh -NoProfile -File tools/compatibility/New-ProviderPreflightRetirementPacket.ps1 `
  -OutDir orchestration/retirements/e03-provider-preflight `
  -EvaluatedAt <unix-seconds> `
  -CodeIntel work/e01-review-target/debug/code-intel.exe
pwsh -NoProfile -File tools/compatibility/Test-ProviderPreflightRetirementPacket.ps1 `
  -PacketRoot orchestration/retirements/e03-provider-preflight
```

The rollback command accepts only an explicit target or an exclusive rehearsal root. Packet
generation uses the rehearsal form and verifies the live facade digest is unchanged:

```powershell
pwsh -NoProfile -File tools/compatibility/Restore-ProviderPreflightLegacyBranch.ps1 `
  -RehearsalRoot work/e03-provider-preflight-rollback-<unix-seconds>
```

The B01/A04 proving set is `scripts/tests/test-repowise-adapter-contract.ps1`, the quota/index-only case in
`repowise_route`, and the quota plus index-only cases in `repowise_adapter`. These prove that docs
quota does not erase a current index and that index-only evidence still passes A04.

The historical direct call is discoverable from Git for rollback provenance, but it is not live in
the current facade. Therefore E03 must not delete or reroute the current production probe. A final
retirement packet remains blocked until a historical-base replayable deletion proof, completed
observation window, and independent repository-governed E00 approval exist. Until then
`deletionExecuted=false` and `retired=false` are mandatory.
