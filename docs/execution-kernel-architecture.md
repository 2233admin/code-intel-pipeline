# Execution Kernel Architecture

## Goal

Make one deep Rust module authoritative for a Code Intel run. The typed CLI adapter compiles
intent into one immutable policy; the authoritative-run controller owns the production request,
committed-run validation, and completed-only index publication. Its private kernel owns policy
application, DAG execution, outcome classification, and atomic run publication. PowerShell
remains a compatibility adapter and batch selector.

## External interface

```rust
authoritative_run::execute(RunRequest) -> Result<ProductionRunResult, RunError>
```

`RunRequest` contains repository identity, staging and authority destinations, one compiled
policy, optional admitted session evidence, and concurrency. It does not expose DAG nodes,
provider commands, or capability implementation details. `ProductionRunResult` contains the
typed `RunOutcome`, terminal manifest, and typed publication record. The private kernel request
and result are not available to CLI routes.

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

A run that completes but cannot be published is not an outcome — nothing was published. When
`--final-name` is already taken under the authority root, `run execute` exits `73`
(`EX_CANTCREAT`), names the run, the destination path, and the remedy on stderr, and prints the
completed manifest on stdout as `code-intel-execution-failure.v1` so ~90s of analysis is not
discarded silently. `73` is distinct from `65` (malformed input) precisely because the caller's
arguments were well-formed and the run itself succeeded.

The existing `code-intel-run-manifest.v1` schema and Artifact Ref envelope remain unchanged.
Immediately after DAG completion, the private controller adds exactly one
`repository.iteration` Artifact Ref. Its payload uses
`code-intel-repository-iteration-provenance.v1`, declares purpose
`repository_intelligence_iteration`, identifies a versioned producer, and binds the run identity,
snapshot identity, repository key, and publication name before marker-last publication.

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

- `authoritative_run.rs` is the sole production request/result facade and controller.
- `authoritative_run/completion.rs` privately owns provenance binding, committed-run validation,
  and completed-only index admission/publication.
- `authoritative_run/execution_kernel.rs` owns the private typed execution and atomic run
  publication boundary.
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
- A completed run is successful only when it is the completed-only index selection; an older
  publication name is rejected as non-authoritative.
- Index publication failure is reported after marker publication and remains repairable through
  the explicit administrative index command.
- The completion marker is still published last.
- Raw administrative `run commit`, decision runs, and snapshot-only manifests cannot synthesize
  repository-iteration provenance and therefore cannot enter the repository authority index.
- Index admission requires exactly one succeeded, verified provenance ref whose payload, ref,
  manifest, marker, repository directory, and publication directory agree on every binding.
- Contract, integrity, and I/O failures are never downgraded as optional-provider absence.
- Tests cross the execution interface and assert observable manifests, publication, and exits.

This is an integrity and provenance boundary, not a cryptographic signer or hostile-local-user
security boundary. A user who can rewrite an entire local publication coherently can manually
forge this JSON contract; signed attestations or an external trust root would be required to
exclude that residual threat.
