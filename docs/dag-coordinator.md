# Deterministic DAG Coordinator v1

`run.dag-coordinate` is the A09 in-process coordinator. It owns graph validation, dependency
resolution, bounded ready-node scheduling, dependency outcome propagation, resumable state, and
the terminal run manifest. It does not implement capability atoms, interpret provider payloads,
select tools, write staged artifacts, publish a run, or update the index. Those remain A01/A04,
A06, A07, and A08 boundaries.

The checked-in contracts are:

- `orchestration/schemas/code-intel-run-dag.v1.schema.json`
- `orchestration/schemas/code-intel-run-state.v1.schema.json`
- `orchestration/schemas/code-intel-run-manifest.v1.schema.json`

## Explicit execution port

Each node declares only an id, registered capability id, and immutable request identity. The DAG
has explicit edges and a caller-selected concurrency bound. Provider names, commands, executables,
tool paths, and effect authority are deliberately absent. A caller wires existing A01 capability
execution through `NodeExecutor`; the coordinator never discovers or invokes a provider or tool.

The A03 adapter converts only already-verified artifacts into `VerifiedArtifactRef` tokens. There
is no field-based production constructor for these tokens. Ready
nodes receive tokens emitted by successful direct dependencies in sorted dependency order. Raw
paths or unverified Artifact Ref JSON are not scheduler inputs.

## Determinism and recovery

Validation rejects empty/invalid graphs, duplicate node ids, duplicate edges, unknown endpoints,
cycles, and concurrency outside `1..=256`. Nodes and edges are canonicalized before producing the
fixed-length `dag-v1:<sha256>` identity, so declaration order cannot change replay identity and
larger DAGs do not inflate every manifest, checkpoint, query response, and index entry.

`next_batch` returns lexically sorted ready nodes up to the remaining concurrency budget and marks
them running. `record` accepts a result only for a running node. A checkpoint persists terminal
states; running states become pending so interruption replay cannot pretend an unfinished attempt
completed. Resume rejects a different DAG identity, unknown nodes, wrong-snapshot artifacts, and a
history that completes a node before its dependencies.

`run_to_completion` executes one bounded ready batch concurrently, records results in deterministic
node order, and continues unrelated branches. Terminal node states distinguish `completed`,
`domain_failed`, `domain_unknown`, `process_failed`, and `dependency_blocked`. Domain failure and
domain unknown may retain A03-verified diagnostic artifacts in the manifest; those artifacts explain
the failed run but do not make it authoritative. Any non-completed terminal node makes the run
non-completed, and only descendants are blocked while unrelated ready nodes continue.

The production facade is `code-intel run dag-coordinate --repo <repo> --out <staging>`. Its default
normal graph computes the A02 snapshot and executes `repo.snapshot`, `doctor`, `inventory.rg`,
`evidence.native-code`, `provider.graph-adapt`, `provider.sentrux-adapt`, and
`diagnosis.hospital` through A01 request/result envelopes. The snapshot feeds doctor, inventory,
graph, and Sentrux only after A03 verification; verified inventory feeds Native Code Evidence, and
verified graph/Sentrux evidence feeds Hospital. Doctor domain failure remains fail-closed but does
not stop the unrelated evidence branch. Request/result envelopes and diagnostic failure artifacts
remain in staging for audit and recovery; every manifest artifact is A03-verified and snapshot-bound.

Tests and controlled conformance runs may add `--doctor-tool-path-prefix <directory>` to probe a
fully explicit tool environment. A09 records that directory in the doctor request, and B10 applies
it only to the bootstrap probe process `PATH`; it does not inject an observation, bypass diagnosis,
or reinterpret exit 10 as success. Normal production runs omit the option and probe the real host
environment.

The terminal manifest is the A07 handoff printed to stdout and persisted as
`run-manifest.json`; its bound Artifact Ref is persisted as `run-manifest-ref.json`. The
capability audit files remain in their readable A09 layout. A09 does not commit them, create the
A07 completion marker, publish a current-run pointer, or update the A08 index.
