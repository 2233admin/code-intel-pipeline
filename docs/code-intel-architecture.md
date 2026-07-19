# Code Intel Architecture

This stack is the local stable-ops layer for repository understanding.

It is built around one rule: keep the entrypoint small, keep tool roles explicit, and keep failures honest.

## Layers

Artifact ownership and reader/writer boundaries are defined in `docs/artifact-data-contract.md`.

1. `orchestration/integrations.json` and `code-intel.exe orchestrate`
   Integration registry and fusion layer. New scanners, memory systems, graph providers, governance strategies, and compatibility shims must be registered here before they are wired into runner scripts.

   `orchestration/capability-contract.v1.json` defines the Capability Atom declaration/request/result, Snapshot Identity, Artifact Ref, Effect Boundary, Domain Verdict, Run Commit, Materialized View, cache-key, and transactional publication vocabulary. `orchestration/schemas/code-intel-capability-envelope.v1.schema.json` rejects malformed envelopes and impossible outcome combinations. Existing integrations migrate behind that contract one atom at a time; the registry remains the graph authority. Runtime effect enforcement is not yet implemented.

2. Rust targets
   - `crates/code-intel-cli`: compiled `code-intel` CLI for integration orchestration, artifact resume, classify, and artifact doctor contracts.
   - `crates/code-nexus-lite`: compiled `code-nexus-lite` iii worker for CodeNexus scan/lite/doctor behavior.

3. `invoke-code-intel.ps1`
   Thin operator entrypoint. Runs doctor first, then the pipeline. Supports one direct repo path, one configured repo alias, a repo list, or all configured repos.

4. `check-code-intel-tools.ps1`
   Environment doctor. Verifies local tools, Understand Anything presence, repo path, and Sentrux scope state.

5. `run-code-intel.ps1`
   Main orchestrator. Produces artifacts, summary, report, hospital diagnosis, and failure classification.

6. Tool adapters
   - `rg`: exact inventory
   - `repowise`: semantic index and optional docs
   - `Understand Anything`: graph artifact
   - `sentrux`: structure gate
   - `sentruxInsight`: parsed structural deltas and follow-up hints for agents

7. Scoped helpers
   - `Invoke-ScopedRepowise.ps1`
   - `Run-ScopedRepowiseDocs.py`
   - `Invoke-SentruxAgentTool.ps1`

8. Stable-ops helpers
   - `install-code-intel-pipeline.ps1`
   - `test-code-intel-provider.ps1`
   - `test-code-intel-pipeline.ps1`
   - `update-code-intel-index.ps1`
   - `tools/sentrux-shim`

9. On-demand evidence providers
   - `Invoke-EvidenceProvider.ps1`
   - `code-intel provider compete-adapt`
   - `code-intel provider react-doctor-adapt`

   Compete delegates web research to an external Agent and React Doctor runs a
   pinned local scanner. Both stop at the A04 advisory route and are absent
   from normal/full execution.

These exist because some repos are too dirty or too nested for raw `repowise init` at the root.
`Invoke-SentruxAgentTool.ps1` exists for a different reason: agents need a narrow JSON contract for structure governance, not raw terminal prose.
`tools/sentrux-shim` makes Sentrux Pro activation reproducible on new machines: the installer puts the shim in a PATH-prepended Code Intel bin directory, the shim auto-activates local open-source Pro features, and normal Sentrux commands still forward to the real binary when one exists. If the real binary is missing, `sentrux-lite-core.ps1` provides deterministic `scan`, `health`, `check`, `gate`, and `plugin list/validate` so a new machine still has a closed feedback loop.

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

The rule is simple: no new project-intelligence dependency goes straight into an agent-facing script. Register the integration first, declare its stage, capabilities, artifact contract, and adapter entrypoint, then wire the adapter into the runner. When a Rust target exists, the Rust crate is the real integration entrypoint and `.ps1` files are compatibility surfaces. `code-intel.exe orchestrate` validates the registry and can print the current plan for humans or agents.

Detailed extension rules are in `docs/integration-orchestration.md`.

## Failure Model

The pipeline classifies failures into four buckets:

- `provider_quota`
- `local_tool_error`
- `graph_missing`
- `sentrux_fail`

This classification is written into:

- `report.json`
- `summary.md`

`report.json` also includes `sentruxInsight`, a deterministic bridge between Sentrux output and agent action. It records parsed quality, coupling, cycle, and god-file deltas, scan scale, next actions, and CodeNexus hints so an operator can move from "score changed" to "inspect this dependency flow" without rereading raw gate text.

`codenexus-context.json` is the portable CodeNexus-lite layer. It selects files from Sentrux hotspots and DSM risk, then attaches recent git commits and reference hits. A full CodeNexus backend can replace this layer later; the contract is already artifact-first.

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

For live Agent sessions, `Invoke-SentruxAgentTool.ps1` exposes `scan`, `health`, `session_start`, `session_end`, `rescan`, `check_rules`, `evolution`, `dsm`, `test_gaps`, and `what_if`. `session_start` saves the chosen scope baseline; `session_end` compares the current structure against that baseline and returns `pass`, `signal_before`, `signal_after`, and a short summary. Root paths are valid inputs: the wrapper automatically excludes dependency, build-output, cache, and bundled static-asset code from governed source metrics, while reporting those exclusions under `scope.excluded_by_reason`. `dsm` is the visualization handoff and carries 9 color modes: `Size`, `Coupling`, `TestGap`, `Age`, `Churn`, `Risk`, `Git`, `ExecDepth`, and `BlastRadius`. It also carries file detail data for side panels, including per-function LOC, complexity, parameter count, async/public flags, and source line ranges. `evolution` adds trend, hotspots, coupling, and bus-factor details. `what_if` simulates stricter gates before the team encodes them as rules.

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

Install check:

```powershell
& "$env:CODE_INTEL_HOME/install-code-intel-pipeline.ps1" -RepoPath <repo-path> -CheckProvider
```

Integration registry:

```powershell
.\target\debug\code-intel.exe orchestrate --action Validate --json
.\target\debug\code-intel.exe orchestrate --action Plan --repo <repo-path> --mode normal --json
```

Install or repair a teammate machine:

```powershell
& "$env:CODE_INTEL_HOME/install-code-intel-pipeline.ps1" -RepoPath <repo-path> -CheckProvider -RepairSkillLinks -InstallMissing
```

`-InstallMissing` is explicit by design. The default installer is a doctor; the install mode attempts supported CLI installs and records every attempt in `installActions`.

`-RepairSkillLinks` installs the bundled `skill/` copy into the user profile when the shared `.agents` skill is absent, then links Codex and Claude to that shared copy.

Doctor and normal run:

```powershell
& "$env:CODE_INTEL_HOME/invoke-code-intel.ps1" -RepoPath <repo-path> -Mode normal
```

Docs-enabled run:

```powershell
& "$env:CODE_INTEL_HOME/invoke-code-intel.ps1" -RepoPath <repo-path> -Mode normal -RepowiseDocs
```

Batch run:

```powershell
& "$env:CODE_INTEL_HOME/invoke-code-intel.ps1" -Config "$env:CODE_INTEL_HOME/pipeline.config.json" -All -Mode lite
```

Smoke test:

```powershell
& "$env:CODE_INTEL_HOME/test-code-intel-pipeline.ps1" -RepoPath <repo-path>
```

Artifact index:

```powershell
& "$env:CODE_INTEL_HOME/update-code-intel-index.ps1"
```

## Design Rule

Copy the operational shell, not the internal machinery.

That is the useful lesson from `gitnexus-stable-ops`.

The updated version of this rule is: register integrations first, then adapt or internalize them behind the orchestration layer.

Atomicity means identifiable inputs, verifiable outputs, declared effects, and safe publication. It does not mean one process per function. See `docs/atomic-development-model.md` and ADR 0009.
