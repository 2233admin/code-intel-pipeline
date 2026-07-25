# CodeNexus provider adapter

`provider.codenexus-adapt` is the B04 boundary between CodeNexus evidence producers and A04 evidence admissibility. Full CodeNexus and the local lite compatibility script both translate into `code-intel-codenexus-port.v1`. Their port fields are identical, while `providerId`, `implementationId`, activation, effects, source revision, and snapshot identity remain explicit.

The Pipeline owns only the adapter and port. CodeNexus owns its process, indexing, storage, retrieval, and impact semantics. The adapter consumes an Artifact Ref and treats `providerData` as opaque. It does not import CodeNexus libraries, open or share a CodeNexus database, reconstruct impact relationships, or copy provider ranking semantics into Pipeline code.

## Admission and use

The adapter initially emits `perceptionUsable: false` and `engineeringFacts: []`. A consumer must submit `evidence.request` to A04 and validate the admitted payload against the adapter identity before using the observation. Wrong-snapshot and stale observations fail closed. Partial observations remain domain `unknown`.

Provider absence is an observation, not an empty result. It is represented as `status: unavailable`, `completeness: partial`, `failure.kind: provider_unavailable`, and opaque `providerData: null`; A04 admits that diagnostic as domain `unknown` without producing Engineering Facts.

## Full/lite swap and rollback

Full CodeNexus is the `primary` provider. `Invoke-CodeNexusLite.ps1` is a compatibility implementation and may appear only as `explicit_fallback` or `legacy_rollback`. It uses the same port and A04 path, but its provenance and repository/Git/Sentrux effects remain distinct. The demoted Rust worker is not part of this adapter.

This design makes provider replacement an adapter-level change: no shared storage migration, Pipeline-side impact algorithm, or change to downstream admission semantics is required.
