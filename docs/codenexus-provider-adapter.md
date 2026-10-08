# CodeNexus provider adapter

`provider.codenexus-adapt` is the B04 boundary between CodeNexus evidence producers and A04 evidence admissibility. Full CodeNexus and the local lite compatibility script both translate into `code-intel-codenexus-port.v1`. Their port fields are identical, while `providerId`, `implementationId`, activation, effects, source revision, and snapshot identity remain explicit.

The Pipeline owns only the adapter and port. CodeNexus owns its process, indexing, storage, retrieval, and impact semantics. The adapter consumes an Artifact Ref and treats `providerData` as opaque. It does not import CodeNexus libraries, open or share a CodeNexus database, reconstruct impact relationships, or copy provider ranking semantics into Pipeline code.

## Admission and use

The adapter initially emits `perceptionUsable: false` and `engineeringFacts: []`. A consumer must submit `evidence.request` to A04 and validate the admitted payload against the adapter identity before using the observation. Wrong-snapshot and stale observations fail closed. Partial observations remain domain `unknown`.

Provider absence is an observation, not an empty result. It is represented as `status: unavailable`, `completeness: partial`, `failure.kind: provider_unavailable`, and opaque `providerData: null`; A04 admits that diagnostic as domain `unknown` without producing Engineering Facts.

## Full/lite swap and rollback

Full CodeNexus is the `primary` provider. `legacy/Invoke-CodeNexusLite.ps1` is a compatibility implementation and may appear only as `explicit_fallback` or `legacy_rollback`. It uses the same port and A04 path, but its provenance and repository/Git/Sentrux effects remain distinct. The demoted Rust worker is not part of this adapter.

This design makes provider replacement an adapter-level change: no shared storage migration, Pipeline-side impact algorithm, or change to downstream admission semantics is required.

## Runtime source disposition (#419)

The owner source is [2233admin/codenexus](https://github.com/2233admin/codenexus), inspected at revision `d15b289f38b4a016f3dfc14623302127b99232b8`. Its `core/Cargo.toml` declares package version `0.0.1` and Apache-2.0; the repository includes LICENSE/NOTICE and describes the product as alpha. No published release or tag was present at inspection. This corrects the earlier research assumption that no owner source could be located; package metadata is not a stable-release acquisition target or proof of Pipeline artifact-contract compatibility.

Do not promote the full provider, fabricate a stable version, or reinterpret frozen historical attestations from this discovery. Existing unavailable/partial observations and explicit source identity remain mandatory until the real full implementation proves the port and admission contracts.
