# Repowise provider adapter

`provider.repowise-adapt` translates Repowise-native outcomes into the provider-neutral A04
Evidence Provider Port. It does not change Repowise internals and does not treat provider health
as evidence.

The adapter maintains three independent channels:

- `health` reports CLI/provider readiness only and always declares `evidence: false`.
- `index` declares index completeness, freshness, local index effects, and an A04 request when an
  index observation exists.
- `docs` separately declares docs completeness, freshness, network/model/filesystem effects, and
  an A04 request when docs evidence exists.

Provider quota maps only the docs observation to partial `provider_unavailable`; it does not erase
or downgrade a current index observation. Missing CLI is a local-tool health diagnosis and emits
no fabricated evidence. Stale observations are translated faithfully and rejected by A04 under
the caller's freshness policy. Successful but incomplete docs remain partial/domain-unknown.

Every translated result starts with `factPromotion.eligible=false`. Consumers must pass each
generated request through `code-intel evidence validate`; only the admitted Observed Evidence may
continue toward A05. The adapter never emits Engineering Facts.

The production route performs both operations as one fail-closed boundary:

```text
code-intel provider repowise-adapt --request <native.json|-> --artifact-root <artifact-directory> --evaluated-at <unix-seconds> --max-age-seconds <seconds>
```

Its `code-intel-repowise-route-result.v1` output keeps the translation and an A04 result for every
emitted evidence channel. Exit `0` means every emitted observation was admitted (including an
`unknown` domain verdict for partial docs); exit `65` means the native contract or at least one A04
check was rejected. Missing CLI emits no fabricated evidence. Native diagnostics are never copied.
The `run-code-intel.ps1 -RepowiseAdapterRequest ...` facade selects this same Rust route.

`Invoke-RepowiseProviderProbe.ps1` is the production health probe. The historical
`scripts/tests/test-code-intel-provider.ps1` name is now only a test wrapper over that production seam.
`run-code-intel.ps1` uses the production probe and continues index-only execution when optional
docs health fails. Existing direct Repowise CLI/index commands remain compatibility and rollback
surfaces; they are optional diagnostics/rollback only, and their raw output has no evidence or fact authority.
