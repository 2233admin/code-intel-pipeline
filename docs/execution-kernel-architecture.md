# Execution Kernel Architecture

## Goal

Make one deep Rust module authoritative for a Code Intel run. The typed CLI adapter compiles
intent into one immutable policy; the kernel owns its application, DAG execution, outcome
classification, and atomic publication. PowerShell remains a compatibility adapter and batch
selector.

## External interface

```rust
execute(RunRequest) -> Result<ExecutionResult, RunError>
```

`RunRequest` contains repository identity, staging and authority destinations, one compiled
policy, optional admitted session evidence, and concurrency. It does not expose DAG nodes,
provider commands, or capability implementation details. `ExecutionResult` contains the typed
`RunOutcome`, terminal manifest, and typed publication record.

`ExecutionPolicy` is resolved once from the selected profile and compatibility overrides. It is
the only runtime source for:

- working-tree behavior and scope;
- provider requirements;
- capability effects;
- tool-path overrides.

`RunOutcome` is a typed value with the existing serialized outcomes:

- `completed` -> exit 0;
- `domain_failed` -> exit 10;
- `domain_unknown` -> exit 20;
- `process_failed` or `incomplete` -> exit 70.

The existing `code-intel-run-manifest.v1` schema and Artifact Ref contracts remain unchanged.

## Profiles

- `default`: internal graph evidence is required; external enrichments such as Sentrux acceleration
  are optional.
- `strict`: enabled provider evidence is required.
- `offline`: provider nodes are not admitted to the DAG; local snapshot, inventory, native-code,
  and optional session evidence remain available.

Unavailable optional providers degrade to `not_applicable`; contract, integrity, internal, and
I/O failures remain terminal. Strict and offline boundaries cannot be weakened or re-enabled by
legacy doctor overrides.

## Internal seams

- `execution_kernel.rs` owns the typed authoritative execution and publication boundary.
- `dag_run.rs` retains CLI parsing plus the non-authoritative `dag-coordinate` compatibility
  primitive.
- The coordinator remains an internal scheduling seam below the kernel.
- Capability adapters remain behind the capability envelope seam.
- Filesystem publication uses the existing atomic run-commit implementation.
- Provider processes remain adapters; provider command details never enter the DAG contract.

## Migration

1. Replace stringly run outcomes with a typed enum that owns exit semantics.
2. Replace duplicated CLI/executor policy fields with one immutable `ExecutionPolicy`.
3. Add a high-level Rust authoritative-run route that executes and publishes in one call.
4. Move the stable PowerShell wrapper to the high-level route.
5. Apply optional/offline provider behavior without changing artifact authority.
6. Retire compatibility routes only after their observation window and rollback gates pass.

## Regression contract

- Existing `dag-coordinate` output and exit codes remain compatible.
- Failed runs remain committed for audit and never replace the latest completed authority.
- The completion marker is still published last.
- Contract, integrity, and I/O failures are never downgraded as optional-provider absence.
- Tests cross the execution interface and assert observable manifests, publication, and exits.
