# Schema lifecycle

`orchestration/schema-lifecycle.v1.json` is the executable catalog for contracts that define the
model-independent pipeline boundary. It deliberately covers the core artifact chain and query ports,
not every historical or optional capability in the repository.

The rules are small and strict:

- compatible changes to an active v1 contract are additive only;
- breaking changes publish a new versioned schema instead of mutating the old contract;
- retirement needs an evidence-backed compatibility window;
- each core contract must name its implementation surface and at least one regression test;
- CI verifies every schema is parseable, versioned, uniquely identified, and every registry schema
  reference resolves.

The catalog is not a second runtime registry. Runtime authority remains in the producer, Artifact Ref
validator, A07 commit verifier, and A08 index admission path. The catalog makes those bindings
auditable and prevents a model or adapter from silently inventing a new core contract.
