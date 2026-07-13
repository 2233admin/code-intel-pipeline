# Atomic Development Model

Code Intel Pipeline adopts a Linux-style atomic development model: a capability is not atomic because its source file is small, but because its inputs are identifiable, its output is verifiable, its effects are declared, and its publication is safe.

The canonical machine contract is `orchestration/capability-contract.v1.json`.

## Current Maturity

The pipeline already provides useful repository inventory, Code Evidence, structural checks, scoped Repowise egress controls, fail-closed Hospital routing, and cross-platform installation tests. It is useful today as a repository examination instrument and conservative gate.

It is not yet an autonomous repair authority:

- graph presence is not consistently bound to the current Git snapshot;
- Sentrux normalization does not yet cover every authoritative rule kind;
- a partial diagnosis can still compete with enrichment when selecting a surgery target;
- normal runs may invoke tools with undeclared local or global side effects;
- timestamped, machine-local artifact paths are not portable identity;
- there is no proven edit episode from `session_start` through regression rejection to post-op discharge.

These are contract and composition problems, not a reason for a big-bang rewrite.

## Seven Owned Concepts

1. **Capability Atom** — one responsibility with one request and one result contract.
2. **Snapshot Identity** — repository identity, HEAD, working-tree policy, scope, and input digest.
3. **Artifact Ref** — `{schema, artifactSchema, type, path, sha256, consumedSnapshotIdentity}`; `schema` versions the reference envelope, `artifactSchema` identifies payload validation, content digest is identity, and path is location.
4. **Effect Boundary** — determinism is declared separately; `allowedEffects` is fixed before execution and `observedEffects` is audited afterward. Permission effects are `repo_read`, `local_write`, `network`, or `repo_mutation`.
5. **Domain Verdict** — `pass`, `fail`, `unknown`, or `not_applicable`, separate from process execution status.
6. **Run Commit** — validate in staging, promote atomically, then write `run-complete.json` last.
7. **Materialized View** — Markdown and indexes are rebuildable views over machine JSON, never fact authority.

## Capability Graph

```text
repo.snapshot
├── inventory.rg
├── memory.repowise
├── graph.understand
└── structure.sentrux.collect
      └── structure.sentrux.normalize
             └── localization.codenexus

inventory + memory + graph + structure + localization
                         │
                         ▼
                 diagnosis.hospital
                         │
                 ┌───────┴────────┐
                 ▼                ▼
              view.render      run.publish
                                   │
                              artifact.index
```

The target graph allows collection atoms to run concurrently. Diagnosis is deterministic from artifact references but its adapter may still declare `local_write` when it persists output. Rendering cannot change a verdict. Indexing will only see committed runs after the corresponding atoms land.

## Process Contract

The intended stable entrypoint is:

```text
code-intel capability exec <capability-id> --request - --out <staging-dir>
```

- stdin carries one request envelope or `--request` names a file;
- stdout carries exactly one result envelope;
- stderr carries human diagnostics;
- large evidence crosses boundaries by Artifact Ref;
- exit code distinguishes a domain failure from invalid input, unavailable dependency, internal error, or I/O error.

Existing PowerShell, Python, and Rust implementations remain valid adapters while they converge on this contract.

The v1 JSON Schema now validates declaration, request, result, and Artifact Ref envelopes, including declaration dependency ids and legal `status × verdict × exitCode` combinations. The contract also defines cross-envelope coherence for identity, implementation, determinism, snapshot, and effect allowlists. This is a control-plane contract only: current runtime adapters do not yet emit these envelopes or enforce those invariants.

## Target Cache And Reproducibility

The future cache key will be a SHA-256 over capability id, contract and implementation versions, Snapshot Identity, canonical options, ordered input Artifact Ref digests, and toolchain digests. Timestamps, machine paths, and output directories are attempt metadata and will not enter deterministic identity.

Network-backed results will also bind provider, model, prompt/config digest, and network policy. They will be explicitly `external_nondeterministic` in provenance even when cached.

## Atomic Migration

1. Freeze current behavior with golden artifacts.
2. Land Capability Envelope v1 without moving implementations.
3. Add the atomic artifact writer and Snapshot Identity.
4. Extract `inventory.rg` and pure Hospital diagnosis first.
5. Wrap Repowise, Understand, and Sentrux behind effect-declaring adapters.
6. Add node-level cache, resume, and red-green invalidation.
7. Separate rendering, transactional publish, and indexing.
8. Keep `run-code-intel.ps1` as a compatibility façade until parity is proven.

## Non-Goals

- Do not turn every function into a process.
- Do not pass repository contents through stdout.
- Do not introduce a workflow engine, database, message queue, Nix, Bazel, or OPA runtime.
- Do not migrate all PowerShell to Rust before contracts and parity tests exist.
- Do not cache network output without provider and configuration provenance.

The project absorbs mechanisms from Unix pipes, reproducible builds, content-addressed storage, incremental query systems, SLSA provenance, and policy-as-code. It does not import those systems as mandatory runtime dependencies.
