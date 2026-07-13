# ADR 0009: Adopt an Atomic Capability Execution Model

- Status: accepted
- Date: 2026-07-13

## Context

The trust-boundary work made Hospital and scoped Repowise fail closed, but real project runs show a second-order problem: the pipeline can collect valid signals while losing identity, causality, or phase boundaries during orchestration.

Examples include stale graph files treated as current anatomy, Sentrux rule failures normalized only partially, enrichment selecting a surgery target while authoritative diagnosis is untrusted, and Repowise combining successful indexing with optional global hook installation in one process outcome.

The current system is conceptually layered but operationally monolithic. `run-code-intel.ps1` and `Invoke-SentruxAgentTool.ps1` still combine collection, normalization, policy, rendering, and publication. A full rewrite would mix language migration with boundary repair and would be difficult to verify.

## Decision

Adopt the machine contract in `orchestration/capability-contract.v1.json` and the vocabulary in `docs/atomic-development-model.md`.

Every **Capability Atom** moving across the orchestration boundary will converge on:

- one versioned request envelope;
- one versioned result envelope;
- Snapshot Identity bound to source inputs;
- Artifact Refs with payload schema, SHA-256 content identity, and consumed Snapshot Identity;
- an Effect Boundary with determinism, pre-execution allowed effects, and post-execution observed effects;
- Domain Verdict separated from process status;
- deterministic cache-key inputs;
- staging plus a final Run Commit marker;
- rebuildable Materialized Views.

Migration uses a strangler pattern. Existing scripts remain compatibility adapters until each atom has contract tests and parity evidence. Language migration is a separate decision.

## Immediate Consequence

ADR 0009 and A01 establish vocabulary, a machine-validatable JSON Schema, legal outcome combinations, and CI drift guards. They do not change current runtime execution, artifact publication, caching, portability, or side-effect enforcement.

## Target Consequences After Subsequent Atoms

- Individual capabilities will be rerunnable, cacheable, resumable, and independently testable.
- Cross-device artifacts will be verifiable by content and snapshot identity instead of absolute path.
- Capability declarations and request allowlists will let the orchestrator reject undeclared network, local-write, or repository-mutation effects.
- A tool will be able to complete successfully with a domain `fail` verdict without being mislabeled as a runtime crash.
- Transactional publication and the index reader guard will prevent incomplete runs from entering the artifact index.
- Hermetic execution, content-addressed artifacts, incremental caching, and portable transport remain separate atomic tickets.

## Rejected Alternatives

- **Big-bang Rust rewrite**: rejected because it couples language migration to behavioral decomposition.
- **Add a workflow engine**: rejected because the current graph is small and can be expressed in the existing registry.
- **Make every function a process**: rejected because large analysis payloads belong in artifacts, not pipes.
- **Use timestamps as run identity**: retained only as a human navigation view; content and snapshot digests become authority.
