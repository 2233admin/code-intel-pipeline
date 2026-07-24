# Repository Survival Scan

`repository.survival-scan` is the B05 minimum-survival boundary for a run where the B04 CodeNexus adapter has returned admitted `provider_unavailable` evidence.

It re-verifies the A02 repository snapshot and A01 rg inventory Artifact Refs through A03, then replays the embedded B04 evidence request through A04. The result contains only repository identity, revision/dirty state, basic file-count and extension inventory, and provider diagnosis. Each promoted basic fact identifies the digest of its source artifact.

The result always declares `completeness = reduced` and `structuralVerdict = unknown`. It does not infer dependency relationships, change propagation, hotspots, execution risk, or a current architecture view. Those claims require a separately admitted provider result and are outside this fallback.

Production CLI:

```text
code-intel repository survival-scan --request <request.json|-> --artifact-root <artifact-directory>
```

The PowerShell facade exposes the same route through `-SurvivalScanRequest` and `-SurvivalScanArtifactRoot`. The existing `inventory.rg` adapter remains the rollback-compatible inventory producer; B05 does not add a parser or provider database.
