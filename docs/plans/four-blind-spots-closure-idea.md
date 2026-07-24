# Four Blind Spots Closure
> Status: APPROVED_FOR_IMPLEMENTATION
> Created: 2026-07-14
> Source: user-approved local code-intel-pipeline work

## Abstract
Close four verified architecture gaps without adding a new indexing stack: generated-workspace pollution in CodeNexus-lite, missing per-file boundary evidence, raw executable paths without snapshot-bound handles or safe request synthesis, and runtime/CI evidence represented only by static proxies.

## Core Insight
Each gap belongs behind an existing Pipeline-owned boundary. The fix is to add small deterministic observations and adapters, then admit them through existing snapshot/freshness/provenance rules instead of coupling providers or copying their internals.

## Target Repo
- Path: `<repo-root>`
- Branch: current working tree
- Current state: full Rust tests pass, but normal self-scan is red because Sentrux reports two blocking worsened debts; CodeNexus-lite incorrectly ranks a generated `work/` rollback file.

## Success Criteria
- [ ] CodeNexus-lite excludes generated `work/`, artifact, staging, dependency, and VCS paths before ranking; a regression fixture proves the generated file cannot become `topFile`.
- [ ] A closed, snapshot-bound per-file boundary observation can resolve local `.aigx/files.aigx`-style entries without requiring AIGX or changing source files.
- [ ] Model execution uses a short-lived, content-bound executable handle; request synthesis accepts only a verified inventory candidate plus explicit model and consent and cannot weaken egress/spend gates.
- [ ] Runtime/CI evidence is a provider-neutral, closed observation with provenance, freshness, completeness, and explicit unknown/missing states; Hospital/PET can cite it without turning absence into success.
- [ ] No new dependency is added and no real model, paid endpoint, CI system, or external service is invoked by tests.
- [ ] Targeted tests, registry audit, full Rust tests, PowerShell regression tests, and a fresh normal self-scan are reported honestly.

## Constraints
- Preserve the dirty working tree and unrelated user changes.
- Regression tests precede behavior changes for the CodeNexus bug.
- CC Switch remains presence-only; secrets and config values do not enter artifacts.
- AIGX remains optional input; the Pipeline owns the normalized boundary contract.
- Runtime/CI ingestion is file/request based and read-only; it does not mutate CI providers.
- Compatibility retirement authority is unchanged by this work.

## Open Questions Resolved For This Pass
1. A per-file boundary adapter supports the minimal local subset needed for `role`, `forbid`, `gotcha`, and `check`; unsupported AIGX constructs remain explicit unknown diagnostics.
2. Executable handles are process-independent signed-by-content envelopes using path, SHA-256, observed metadata, expiry, and adapter identity; execution re-verifies all fields.
3. Runtime/CI evidence is ingested from explicit local JSON artifacts first. Live connectors are future providers, not part of this change.

## Verification Order
1. Targeted negative and fixture tests.
2. Closed-schema validation and integration registry audit.
3. `cargo fmt --all -- --check`, `cargo check -p code-intel`, full `cargo test -q -p code-intel`.
4. `invoke-code-intel.ps1 -RepoPath <repo-root> -Mode normal`, followed by summary, hospital, and understanding review.
