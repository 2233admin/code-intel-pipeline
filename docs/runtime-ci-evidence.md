# Runtime/CI Evidence

Runtime/CI evidence closes the gap between static engineering proxies and observations produced by tests, builds, and a running system. The first provider-neutral boundary reads one explicitly named local JSON artifact. It does not call a CI provider, authenticate to a service, or mutate an external run.

## Trust boundary

The ingest request pins four things: repository snapshot identity, artifact-relative path, artifact SHA-256, and a freshness policy. The source observation identifies the provider run and source revision separately from the collector provenance. Unknown fields, path traversal, digest mismatch, malformed signals, and positive claims marked `observed: false` are rejected.

Snapshot mismatch and stale evidence produce a rejected summary with `health: unknown`. A missing artifact also produces an explicit unknown summary. Missing is not green.

## Health semantics

- `green` requires `completeness: complete`, a current matching snapshot, observed passed tests, an observed passed build, and observed healthy runtime checks.
- `red` requires at least one observed test/build failure or a degraded/failed runtime signal.
- `unknown` covers missing, partial, stale, snapshot-mismatched, cancelled without a trustworthy success conclusion, and unobserved domains.

The stable `facts` array contains only conservative deterministic claims. Hospital/PET can consume the summary without learning provider-specific JSON and without converting absence into success.

`run-code-intel.ps1 -RuntimeCiEvidenceRequest <request.json> -RuntimeCiEvidenceArtifactRoot <root>` invokes the registered provider during a normal run. Hospital/PET cites the emitted `runtime-ci-summary.json`, reports health/freshness/completeness, and uses Sentrux evolution/what-if only as the fallback when no runtime/CI request was supplied.

## Deliberate limits

This adapter does not infer a result from log text, discover artifacts, fetch live CI state, or claim that a provider login proves a run. Live connectors can later export the same closed observation, but they remain outside this local read-only ingestion boundary.
