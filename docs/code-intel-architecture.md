# Code Intel Architecture

This stack is the local stable-ops layer for repository understanding.

It is built around one rule: keep the entrypoint small, keep tool roles explicit, and keep failures honest.

## Layers

Artifact ownership and reader/writer boundaries are defined in `docs/artifact-data-contract.md`.

1. Compiled `code-intel`
   Primary Operator Entry and execution kernel. `code-intel .` runs the default
   `normal` profile; `--mode lite|full` selects an explicit alternative.

2. `orchestration/integrations.json` and `code-intel orchestrate`
   Integration registry and fusion layer. New scanners, memory systems, graph providers, governance strategies, and compatibility shims must be registered here before they are wired into runner scripts.

   `orchestration/capability-contract.v1.json` defines the Capability Atom declaration/request/result, Snapshot Identity, Artifact Ref, Effect Boundary, Domain Verdict, Run Commit, Materialized View, cache-key, and transactional publication vocabulary. `orchestration/schemas/code-intel-capability-envelope.v1.schema.json` rejects malformed envelopes and impossible outcome combinations. Existing integrations migrate behind that contract one atom at a time; the registry remains the graph authority. Runtime effect enforcement is not yet implemented.

3. Rust targets
   - `crates/code-intel-cli`: compiled `code-intel` CLI for integration orchestration, artifact resume, classify, and artifact doctor contracts.
   - `crates/code-nexus-lite`: incubated source, not a Cargo workspace member and not shipped as a beta binary. The supported beta surface is the optional CodeNexus compatibility adapter and artifact contract.

4. Environment doctor and installer
   - `code-intel doctor` / `code-intel doctor bootstrap`: verify local tools, Understand Anything presence, repo path, and Sentrux scope state.
   - `code-intel setup`: finishes a Skill install from an extracted release (binary, PATH, Skill links, doctor).

5. Tool adapters
   - `rg`: exact inventory
   - `repowise`: optional semantic index and docs; included in the default plan but non-blocking
   - `Understand Anything`: optional graph artifact
   - `sentrux`: structure gate
   - `sentruxInsight`: parsed structural deltas and follow-up hints for agents

6. Scoped helpers
   - `Run-ScopedRepowiseDocs.py`
   - `code-intel sentrux`: narrow structure-governance commands (`scan`, `health`, `check`, `gate`, `gate_save`, `check_rules`, `dsm`, `hotspots`)

These exist because some repos are too dirty or too nested for raw `repowise init` at the root.
The `code-intel sentrux` subcommands exist for a different reason: agents need a narrow contract for structure governance, not raw terminal prose.

## Why The Wrapper Exists

Raw tool invocations are fine for one person and one afternoon.

Team usage is different. A stable wrapper is needed for:

- preflight checks
- consistent artifacts
- one command path
- known repo aliases
- scoped repowise behavior
- explicit failure categories

Without that wrapper, the same tool failure gets interpreted three different ways by three different people. That is how teams end up debugging weather.

## Why The Integration Layer Exists

As the pipeline absorbs Repowise, Sentrux, graph providers, and future project-intelligence tools, the codebase needs one place to decide how those pieces attach.

That place is `orchestration/integrations.json`.

The rule is simple: no new project-intelligence dependency goes straight into an agent-facing script. Register the integration first, declare its stage, capabilities, artifact contract, and adapter entrypoint, then wire the adapter into the Rust runner. When a Rust target exists, the Rust crate is the integration entrypoint (PowerShell was removed, DR-0018). `code-intel orchestrate` validates the registry and can print the current plan for humans or agents.

Detailed extension rules are in `docs/integration-orchestration.md`.

## Failure Model

The pipeline classifies failures into six buckets:

- `provider_quota`
- `provider_unavailable`
- `config_error`
- `local_tool_error`
- `graph_missing`
- `sentrux_fail`

This classification is written into:

- `report.json`
- `summary.md`

`report.json` also includes `sentruxInsight`, a deterministic bridge between Sentrux output and agent action. It records parsed quality, coupling, cycle, and god-file deltas, scan scale, next actions, and CodeNexus hints so an operator can move from "score changed" to "inspect this dependency flow" without rereading raw gate text.

`codenexus-context.json` is the portable, optional CodeNexus compatibility layer. It selects files from Sentrux hotspots and DSM risk, then attaches recent git commits and reference hits. A full CodeNexus backend can replace this layer later; the contract is already artifact-first. Failure of this optional layer does not invalidate the beta core.

`hospital-report.json` is the diagnosis layer over those artifacts. It does not replace the tools; it organizes their output into modalities and protocols:

- `xray`: rg file inventory and repo surface.
- `anatomy`: Understand Anything graph freshness.
- `ct`: Sentrux DSM, hotspots, and file/function detail.
- `mri`: CodeNexus impact localization.
- `pet`: execution-risk proxy from evolution, what-if, and test gaps.
- `chart`: Repowise long-term semantic memory.
- `governance`: Sentrux rules, check, and gate.

The human version is `hospital.md`. The machine version exposes `triage.disposition`, `triage.primary_diagnosis`, `triage.overall_score`, `triage.next_protocol`, `triage.discharge_criteria`, `state_machine.current_state`, `state_machine.transitions`, `report_quality.dimensions`, and `treatment.plan`. This is the product boundary for "code hospital" behavior: the pipeline decides whether the operator should admit the project, triage, diagnose, govern, plan surgery, run post-op verification, or treat it as discharge-ready.

`surgery-plan.json` and `surgery-plan.md` are emitted beside the hospital report. They translate the first failing what-if scenario, Sentrux hotspot, and CodeNexus context into a surgical target, operating plan, and verification checklist.

For live Agent sessions, `code-intel sentrux` exposes `scan`, `health`, `check`, `gate`, `gate_save`, `check_rules`, `dsm`, and `hotspots`. `gate_save` saves the chosen scope baseline; `gate` compares the current structure against that baseline and fails on regression. Root paths are valid inputs: the command automatically excludes dependency, build-output, cache, and bundled static-asset code from governed source metrics, while reporting those exclusions under `scope.excluded_by_reason`. `dsm` is the visualization handoff and carries 9 color modes: `Size`, `Coupling`, `TestGap`, `Age`, `Churn`, `Risk`, `Git`, `ExecDepth`, and `BlastRadius`. It also carries file detail data for side panels, including per-function LOC, complexity, parameter count, async/public flags, and source line ranges.

The goal is not philosophical purity. The goal is that an operator can tell in ten seconds whether they should:

- wait for provider quota reset
- fix a local installation
- refresh the Understand graph
- repair an architecture regression

## Repo Strategy

For clean repos:

- direct `repowise init/update`

For noisy repos such as `k-atana`:

- sparse shadow worktree
- scoped file sync
- scoped docs generation

This keeps nested external repos from poisoning indexing and keeps current working-tree changes visible inside the indexed scope.

## Standard Commands

The default `normal` route is one manifest-bound Rust production spine:

```text
A02 snapshot
  -> doctor / inventory.rg / graph provider / Sentrux provider
  -> native code evidence / Hospital diagnosis
  -> A03 verified Artifact Refs
  -> A09 terminal manifest
  -> A07 atomic commit
  -> A08 completed-only index
  -> bounded query / impact / freshness reads
```

`domain_failed` and `domain_unknown` runs may be committed with verified diagnostic evidence for
forensics, but they never enter the authoritative index.

Install check:

```bash
code-intel doctor bootstrap --repo-path <repo-path>
```

Integration registry:

```bash
code-intel orchestrate --action Validate --json
code-intel orchestrate --action Plan --repo <repo-path> --mode normal --json
```

Install or repair a teammate machine: run the Skill bootstrap (`python scripts/bootstrap.py --repo-path <repo-path>` from `skills/code-intel-pipeline/`), or run `bin/code-intel setup --repo-path <repo-path>` from an extracted release. `setup` reports missing tools with an install command and never installs system tools itself; it installs the bundled `skills/code-intel-pipeline/` package into `~/.agents/skills` and links Codex and Claude to that shared copy.

Normal run:

```bash
code-intel <repo-path>
```

Explicit profiles:

```bash
code-intel <repo-path> --mode lite
code-intel <repo-path> --mode full
```

Artifact index:

```bash
code-intel artifact index --artifact-root <artifact-root>
```

## Design Rule

Copy the operational shell, not the internal machinery.

That is the useful lesson from `gitnexus-stable-ops`.

The updated version of this rule is: register integrations first, then adapt or internalize them behind the orchestration layer.

Atomicity means identifiable inputs, verifiable outputs, declared effects, and safe publication. It does not mean one process per function. See `docs/atomic-development-model.md` and ADR 0009.
