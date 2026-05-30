# Code Intel Architecture

This stack is the local stable-ops layer for repository understanding.

It is built around one rule: keep the entrypoint small, keep tool roles explicit, and keep failures honest.

## Layers

1. `invoke-code-intel.ps1`
   Thin operator entrypoint. Runs doctor first, then the pipeline. Supports one direct repo path, one configured repo alias, a repo list, or all configured repos.

2. `check-code-intel-tools.ps1`
   Environment doctor. Verifies local tools, Understand Anything presence, repo path, and Sentrux scope state.

3. `run-code-intel.ps1`
   Main orchestrator. Produces artifacts, summary, report, hospital diagnosis, and failure classification.

4. Tool adapters
   - `rg`: exact inventory
   - `repowise`: semantic index and optional docs
   - `Understand Anything`: graph artifact
   - `sentrux`: structure gate
   - `sentruxInsight`: parsed structural deltas and follow-up hints for agents

5. Scoped helpers
   - `Invoke-ScopedRepowise.ps1`
   - `Run-ScopedRepowiseDocs.py`
   - `Invoke-SentruxAgentTool.ps1`

6. Stable-ops helpers
   - `install-code-intel-pipeline.ps1`
   - `test-code-intel-provider.ps1`
   - `test-code-intel-pipeline.ps1`
   - `update-code-intel-index.ps1`
   - `tools/sentrux-shim`

These exist because some repos are too dirty or too nested for raw `repowise init` at the root.
`Invoke-SentruxAgentTool.ps1` exists for a different reason: agents need a narrow JSON contract for structure governance, not raw terminal prose.
`tools/sentrux-shim` makes Sentrux Pro activation reproducible on new machines: the installer puts the shim in a PATH-prepended Code Intel bin directory, the shim auto-activates local open-source Pro features, and normal Sentrux commands still forward to the real binary when one exists. If the real binary is missing, `sentrux-lite-core.ps1` provides deterministic `scan`, `health`, `check`, and `gate` so a new machine still has a closed feedback loop.

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

The human version is `hospital.md`. The machine version exposes `triage.primary_diagnosis`, `triage.overall_score`, `triage.next_protocol`, `report_quality.dimensions`, and `treatment.plan`. This is the product boundary for "code hospital" behavior: the pipeline decides whether the operator should triage, diagnose, govern, plan surgery, or run post-op verification.

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
D:\projects\_tools\code-intel-pipeline\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo -CheckProvider
```

Install or repair a teammate machine:

```powershell
D:\projects\_tools\code-intel-pipeline\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo -CheckProvider -RepairSkillLinks -InstallMissing
```

`-InstallMissing` is explicit by design. The default installer is a doctor; the install mode attempts supported CLI installs and records every attempt in `installActions`.

`-RepairSkillLinks` installs the bundled `skill\` copy into the user profile when the shared `.agents` skill is absent, then links Codex and Claude to that shared copy.

Doctor and normal run:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal
```

Docs-enabled run:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal -RepowiseDocs
```

Batch run:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -All -Mode lite
```

Smoke test:

```powershell
D:\projects\_tools\code-intel-pipeline\test-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo
```

Artifact index:

```powershell
D:\projects\_tools\code-intel-pipeline\update-code-intel-index.ps1
```

## Design Rule

Copy the operational shell, not the internal machinery.

That is the useful lesson from `gitnexus-stable-ops`.
