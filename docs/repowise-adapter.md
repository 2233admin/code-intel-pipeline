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
The `legacy/run-code-intel.ps1 -RepowiseAdapterRequest ...` facade selects this same Rust route.

`legacy/Invoke-RepowiseProviderProbe.ps1` is the production health probe. The historical
`legacy/scripts/tests/test-code-intel-provider.ps1` name is now only a test wrapper over that production seam.
`legacy/run-code-intel.ps1` uses the production probe and continues index-only execution when optional
docs health fails. Existing direct Repowise CLI/index commands remain compatibility and rollback
surfaces; they are optional diagnostics/rollback only, and their raw output has no evidence or fact authority.

## Installed-version decisions

```text
code-intel repowise-version --reported "repowise, version 0.55.0.post1" --minimum 0.55.0
```

This pure native command parses the complete named Repowise report and compares installed
versions using PEP 440 ordering (`pep440_rs`), not SemVer or a three-component prefix. Release
candidates and development releases precede their final release; post releases, local versions,
epochs, and equivalent release-component spellings retain their Python version semantics.
Unrelated warning numbers cannot supply a Repowise version.

Stdout is `code-intel-repowise-version.v1` JSON with `version`, `minimum`, `ordering`,
`meetsMinimum`, and `status`. Status is `parsed` without a floor, `accepted` or `below_minimum`
with a floor, or `unknown` for invalid, missing, or conflicting named reports. Unknown reports
leave `version`, `ordering`, and `meetsMinimum` null. Malformed floors and unknown or duplicate
options exit `64` with no stdout; decisions exit `0`. The command does not spawn processes,
install packages, access the network, or write artifacts.

The retained installer compatibility surface forwards complete Repowise version probes and
drift-repair completion decisions to this command. It selects the bundled `bin/code-intel`
owner before checkout build outputs or PATH, so first installation does not depend on a
previously installed CLI. A missing or incompatible owner fails clearly rather than reverting
to the old comparison. Existing pip acquisition actions and native doctor presence checks
remain unchanged. Installed versions at or above the floor are never downgraded.
