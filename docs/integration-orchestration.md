# Integration Orchestration

`code-intel-pipeline` is moving from a loose toolchain into a self-contained intelligence pipeline.

The integration orchestration layer is the boundary for that change:

```text
orchestration/integrations.json
crates/code-intel-cli
target/debug/code-intel.exe orchestrate
target/debug/code-intel.exe provider
target/debug/code-intel.exe route
```

## Greenfield Direction

`spec.greenfield` is an optional `behavior_specification` provider-contract integration. The pipeline-owned adapter writes `greenfield-manifest.json` and `greenfield-plan.md`, then points Greenfield at a dedicated workspace whose useful handoff lives under `greenfield-workspace/output/` and `greenfield-workspace/provenance/`.

Greenfield is currently a Claude Code plugin, so default Code Intel runs must not block on an interactive Claude session. Use `Invoke-GreenfieldSpecExtraction.ps1 -Analyze` only when the local machine has `claude` plus the Greenfield plugin installed and the operator intentionally wants the spec extraction pass.

## Rule

Normal code-intel operation must not depend on separately installed project-intelligence CLIs.

External projects and tools can still exist, but they must enter through this layer as one of:

- `internal-script`: owned by this repo.
- `internal-module`: implemented inside the pipeline.
- `internal-adapter`: a stable repo-owned adapter over a lower-level runtime.
- `internalizing-adapter`: an adapter that still calls outside code while it is being absorbed.
- `internal-first-adapter`: repo-owned implementation first, external binary only as an accelerator.
- `internal-rust-binary`: compiled Rust CLI owned by this repo.
- `internal-rust-worker`: compiled Rust worker owned by this repo.
- `rust-backed-adapter`: compatibility adapter whose real target is a Rust crate.
- `provider-contract`: a stable contract for providers that cannot be fully embedded yet.

Do not call a new scanner, memory system, graph generator, or governance tool directly from an agent-facing script. Register it first.

## Global Route Layer

Provider routing is separate from provider implementation.

Use `target\debug\code-intel.exe provider` as the source of truth for provider operations, then expose route views through `target\debug\code-intel.exe route`:

```powershell
.\target\debug\code-intel.exe provider --action Validate --json
.\target\debug\code-intel.exe provider --action List --json
.\target\debug\code-intel.exe provider --action Plan --provider understand --operation graph --repo <repo-path> --json
.\target\debug\code-intel.exe provider --action Plan --provider repowise --operation index --repo <repo-path> --json
.\target\debug\code-intel.exe route --action Validate --json
.\target\debug\code-intel.exe route --action List --json
```

Repowise and Understand-compatible graph operations share the `code-intel-provider-api.v1` schema: provider, operation, protocol, route, command template, artifact contract, requirement, status, source spec, and notes. Repowise routes live under `/api/providers/repowise/*`. Understand-compatible graph routes live under `/api/providers/understand/*`. Compatibility aliases such as `/scan`, `/lite`, `/doctor`, and `/understand` are allowed only as fallback surfaces. Future route-needing projects must add provider operations to this global provider layer first, then wire their implementation behind the route.

## Add A New Integration

1. Add an entry to `orchestration/integrations.json`.
2. Assign it to an existing stage, or add a new stage with an explicit `order`.
3. Declare `kind`, `required`, `entrypoint`, `capabilities`, `commands`, and `artifactContract`.
4. Put all CLI or provider quirks behind one adapter entrypoint.
5. Run:

```powershell
cargo build -p code-intel
.\target\debug\code-intel.exe orchestrate --action Validate
.\target\debug\code-intel.exe provider --action Validate
.\target\debug\code-intel.exe route --action Validate
.\target\debug\code-intel.exe orchestrate --action Plan --json
.\scripts/tests/test-integration-orchestration.ps1
```

6. Only then wire the adapter into the Rust runtime or a compatibility script such as `run-code-intel.ps1`, `check-code-intel-tools.ps1`, or `install-code-intel-pipeline.ps1`.

## Current Stages

| Stage | Meaning |
|---|---|
| `preflight` | Repo resolution and runtime/integration contract checks. |
| `rust_runtime` | Compiled Rust command and worker targets used by the pipeline. |
| `inventory` | Exact file and text surface. |
| `semantic_memory` | Long-term project memory, index, status, and docs. |
| `architecture_graph` | Graph snapshot and freshness. |
| `structure_governance` | Structural scan, rules, gate, DSM, evolution, and what-if signals. |
| `localization` | Hotspot and reference localization. |
| `diagnosis` | Hospital report, protocol, and surgery plan. |
| `artifact_index` | Durable index of runs for future sessions. |

## Repowise Direction

`memory.repowise` is the first internalizing adapter.

The near-term goal is to hide all Repowise CLI and Python package details behind the pipeline-owned adapter. The long-term goal is for normal indexing/status/docs behavior to be owned by this repo, with upstream Repowise code treated as implementation material rather than an agent-facing dependency.

Provider quota can disable model-backed docs. It must not disable index/status.

## Rust Runtime Direction

`runtime.code-intel` and `runtime.code-nexus-lite` are real execution targets.

`code-intel.exe orchestrate` is the preferred registry validator and plan reader. PowerShell scripts are allowed as stable Windows compatibility entrypoints, but they should not be treated as the source of truth when an equivalent Rust target owns the contract. The orchestration manifest must show the Rust crate first and any `.ps1` wrapper as compatibility.

## Sentrux Direction

`structure.sentrux` is internal-first.

The repo-owned lite core is the normal-operation fallback. A real `sentrux.exe` can accelerate or enrich output, but agent workflows should depend on `Invoke-SentruxAgentTool.ps1` and the artifact contract, not on a global Sentrux install.

Rust is the replacement target. The planned stable surface is `target\debug\code-intel.exe sentrux <scan|health|session_start|session_end|check_rules|test_gaps|what_if> <repo-path>`. Until that command is complete, the PS1 file is compatibility glue and known debt.

## Graph Direction

`graph.code-intel-understand` is the internal provider contract.

It emits `.understand-anything/knowledge-graph.json` through `target\debug\code-intel.exe graph --repo <repo-path> --language zh --write --json`. External `/understand` is now `graph.understand-external`, a compatibility fallback for richer Claude-side passes or internal provider failure. Future graph providers should preserve the same `architecture_graph` stage and artifact contract.

## Runner Direction

`run-code-intel.ps1` is also compatibility glue. The replacement target is `target\debug\code-intel.exe run --repo <repo-path> --mode <mode>`, with report writing, hospital diagnosis, graph generation, Repowise, and Sentrux orchestration moved behind Rust modules registered in this manifest.
