---
name: code-intel-pipeline
description: Use when managing local repository understanding with rg, repowise, Understand Anything, and sentrux. Triggers for code indexing, architecture graph refreshes, structural gates, local repo intelligence, CodeNexus-style pipeline work, or when checking whether this machine has the required tools.
---

# Code Intel Pipeline

Use the local pipeline instead of inventing another code-indexing stack.

Canonical files:

- Pipeline: `$env:CODE_INTEL_HOME/run-code-intel.ps1`
- Doctor: `$env:CODE_INTEL_HOME/check-code-intel-tools.ps1`
- Sentrux Agent tools: `$env:CODE_INTEL_HOME/Invoke-SentruxAgentTool.ps1`
- Config: `$env:CODE_INTEL_HOME/pipeline.config.json`
- Artifacts: `CODE_INTEL_ARTIFACT_ROOT` when set, otherwise the platform code-intel data root under `artifacts/<repo>/<timestamp>/`.
- Templates: `$env:CODE_INTEL_HOME/templates/`

## Required First Step

On a new machine or teammate session, run the installer first:

```powershell
& "$env:CODE_INTEL_HOME/install-code-intel-pipeline.ps1" -RepoPath <repo-path>
```

Use `-CheckProvider` to also ping the MiniMax Anthropic-compatible endpoint. Use `-RepairSkillLinks` when the shared skill should be installed or repaired for Codex and Claude. If the `.agents` copy is missing, the installer seeds it from the repo's bundled `skill/` directory first. Use `-InstallMissing` on teammate machines when missing CLI tools should be installed automatically where supported. Never ask the installer to write API keys; it only checks whether user-scoped env vars exist.

Team bootstrap:

```powershell
& "$env:CODE_INTEL_HOME/install-code-intel-pipeline.ps1" -RepoPath <repo-path> -CheckProvider -RepairSkillLinks -InstallMissing
```

For one-command teammate setup, use:

```powershell
& "$env:CODE_INTEL_HOME/bootstrap-new-machine.ps1" -RepoPath <repo-path>
```

The installer also installs the repo-owned Sentrux shim into `CODE_INTEL_BIN` or the platform code-intel data root under `bin`, prepends that directory to PATH, and verifies `sentrux pro status`. The shim auto-activates local open-source Pro features, forwards all non-`pro` commands to the real `sentrux` when present, and falls back to `sentrux-lite-core.ps1` for portable `scan`, `health`, `check`, and `gate`.

The installer applies the repo-owned Sentrux V overlay by default because the upstream Windows `vlang` standard plugin package is incomplete in Sentrux 0.5.7. The overlay lives under `overlays/sentrux/vlang`, installs with `Install-SentruxVlangOverlay.ps1`, and should make `sentrux plugin list` show `vlang v0.2.0 [v]`. Use `Test-SentruxVlangOverlay.ps1` to prove the plugin builds a real V graph with 2 files, 1 import, and 1 call when this platform has a bundled grammar artifact. Use `-SkipSentruxVlangOverlay` only when explicitly testing upstream plugin behavior.

For machine-readable bootstrap status, add `-Json` and read `installActions` first. Valid statuses are `already_present`, `not_requested`, `installed`, `installed_restart_required`, and `install_failed`.

Use `-AuditInstallPlan` before `-InstallMissing` when reviewing a new teammate machine. Read `installPlan` in JSON output for installer source, command, purpose, and supply-chain risk notes.

Always run the doctor before using the pipeline:

```powershell
& "$env:CODE_INTEL_HOME/check-code-intel-tools.ps1" -RepoPath <repo-path>
```

Use `-Json` when another agent needs machine-readable output.

Preferred stable wrapper:

```powershell
& "$env:CODE_INTEL_HOME/invoke-code-intel.ps1" -RepoPath <repo-path> -Mode normal
```

Batch wrappers:

```powershell
& "$env:CODE_INTEL_HOME/invoke-code-intel.ps1" -Config "$env:CODE_INTEL_HOME/pipeline.config.json" -Repos k-atana,glyph-arts -Mode normal
& "$env:CODE_INTEL_HOME/invoke-code-intel.ps1" -Config "$env:CODE_INTEL_HOME/pipeline.config.json" -All -Mode lite
```

Use `-Repo <alias>` only when `pipeline.config.json` already defines that alias. Prefer `-RepoPath <repo-path>` for teammate machines because project disks and mount points differ across machines.

Stop and report clearly if any required tool is missing:

- `rg`
- `git`
- `python`
- `repowise`
- `sentrux`
- Understand Anything skill/plugin

## Workflow

1. Run the stable wrapper first:

```powershell
& "$env:CODE_INTEL_HOME/invoke-code-intel.ps1" -RepoPath <repo-path> -Mode normal
```

2. Use the raw pipeline only when a narrower mode or a special flag is needed:

```powershell
& "$env:CODE_INTEL_HOME/run-code-intel.ps1" -RepoPath <repo-path> -Mode normal
```

3. Add `-RepowiseDocs` when the user wants scoped repowise wiki generation instead of index-only refresh.

   The pipeline will run `test-code-intel-provider.ps1` first. If provider quota is unavailable, docs generation is disabled for that run and the failure category is recorded.

Use `-Mode lite` for a cheap status check. Use `-Mode full` when a fresh Understand Anything graph is needed.

When a repo config defines `repowiseScopePaths` or `repowiseRootFiles`, the pipeline runs `repowise` inside a sparse shadow worktree under `CODE_INTEL_SHADOW_ROOT` when set, otherwise under the platform code-intel data root. This is the default for noisy mono-repos with nested third-party repos.

Scoped Repowise has a bounded timeout. Use `-RepowiseTimeoutSeconds <seconds>` when a huge or dirty repo should fail fast. A Repowise timeout is treated as an optional semantic-memory skip; Understand, Sentrux, and CodeNexus should still complete.

4. If the report says the Understand graph is missing, tell the user or Claude-side agent to run:

```text
/understand <repo-path> --language zh
```

For a full graph rebuild:

```text
/understand <repo-path> --language zh --full
```

Then rerun the pipeline.

5. After each run, read `summary.md` first. Its `Sentrux Insight` section shows parsed quality/coupling/cycle/god-file deltas, scan scale, next actions, and CodeNexus follow-up hints. Open `report.json` when the summary shows failure, manual action, a category count above zero, or when the raw `sentruxInsight` object matters.

   The pipeline also writes `codenexus-context.json`. Read it when you need concrete hotspot files, recent commits, and reference hits instead of generic follow-up hints.

   `Invoke-SentruxAgentTool.ps1` supports both short names and MCP-style aliases for common reads: `scan`/`sentrux_scan`, `health`/`sentrux_health`, `dsm`/`sentrux_dsm`, `git_stats`/`sentrux_git_stats`, and `test_gaps`/`sentrux_test_gaps`.

6. Read `understanding.md` before handing results to a teammate or another agent. It is the understanding-first layer: assumptions, verified facts, unverified areas, human inspection, and next action.

7. If the user asks whether the stack is healthy, answer from:
   - doctor result
   - summary counters
   - failure category counters
   - understanding report next action
   - repowise docs state
   - sentrux gate result
   - sentrux insight metric deltas and CodeNexus hints

For an Agent coding session that needs Sentrux as a live guardrail, use the dedicated wrapper:

```powershell
& "$env:CODE_INTEL_HOME/Invoke-SentruxAgentTool.ps1" scan <scope-path>
& "$env:CODE_INTEL_HOME/Invoke-SentruxAgentTool.ps1" session_start <scope-path>
& "$env:CODE_INTEL_HOME/Invoke-SentruxAgentTool.ps1" session_end <scope-path>
```

It exposes exactly these tools: `scan`, `health`, `session_start`, `session_end`, `rescan`, `check_rules`, `evolution`, `dsm`, `test_gaps`, and `what_if`. Root paths are valid inputs: the wrapper automatically excludes dependency, build-output, cache, and bundled static-asset code from governed source metrics, and reports the filtered material under `scope.excluded_by_reason`. Use narrower scopes only when the team intentionally wants a separate baseline for a subsystem.

The `dsm` output is the visualization handoff. It includes 9 color modes: `Size`, `Coupling`, `TestGap`, `Age`, `Churn`, `Risk`, `Git`, `ExecDepth`, and `BlastRadius`. Every module includes raw `metrics` plus normalized heat `colors` with a `score` and hex `color`. It also includes `file_details` for a file detail panel: file stats plus per-function lines, LOC, complexity, parameter count, async, and public/exported flags.

The `evolution` output carries session trend plus hotspot, coupling, and bus-factor details. The `what_if` output simulates stricter governance gates for complexity, coupling, blast radius, tests, bus factor, and scope pollution before the team hardens `.sentrux/rules.toml`.

Pipeline runs persist map data as `sentrux-dsm.json`, panel data as `sentrux-file-details.json`, sorted sidebar data as `sentrux-hotspots.json`, evolution data as `sentrux-evolution.json`, simulated gate data as `sentrux-what-if.json`, CodeNexus context as `codenexus-context.json`, and the hospital layer as `hospital.md` plus `hospital-report.json` next to `summary.md`, `report.json`, and `understanding.md`.

The hospital layer is the default human/agent diagnosis surface. It groups checks into modalities: `xray` (rg inventory), `anatomy` (Understand graph), `ct` (Sentrux DSM/hotspots), `mri` (CodeNexus localization), `pet` (execution-risk proxy from evolution/what-if/test gaps), `chart` (Repowise memory), and `governance` (rules/gate). Read `hospital.md` after `summary.md` when deciding whether to triage, diagnose, govern, plan surgery, keep the project admitted, or run post-op verification. Machine readers should use `hospital-report.json` fields `triage.disposition`, `triage.primary_diagnosis`, `triage.overall_score`, `triage.next_protocol`, `triage.discharge_criteria`, `state_machine.current_state`, `state_machine.transitions`, `report_quality.dimensions`, and `treatment.plan`. When `next_protocol=surgery_plan`, read `surgery-plan.md` or `surgery-plan.json` for the first operation target and verification checklist.

## Provider Config

The local Anthropic-compatible provider is MiniMax:

- `ANTHROPIC_BASE_URL=https://api.minimaxi.com/anthropic`
- `REPOWISE_PROVIDER=anthropic`
- `ANTHROPIC_API_KEY` is for repowise and Anthropic SDK clients.
- `ANTHROPIC_AUTH_TOKEN` is for Claude-compatible clients.

Never write API keys into repo files or reports. Store only non-secret provider/model settings in repo-local config.

If a teammate already set these at user scope, the scoped repowise helper reads them into the current process automatically.

## Tool Roles

- `rg`: exact search and file inventory.
- `repowise`: long-term semantic/wiki memory.
- `Understand Anything`: architecture graph snapshot and visual understanding.
- `sentrux`: structural regression gate.

Do not add Archon, Sourcegraph, or another RAG tool unless it clearly replaces one of these roles.

## Failure Handling

The pipeline classifies failures into four buckets. Use these exact meanings:

- `provider_quota`: upstream model or token quota/rate limit. Do not describe this as a local script failure.
- `local_tool_error`: broken local command, bad script path, parse crash, CLI failure, or invalid local environment.
- `graph_missing`: `.understand-anything/knowledge-graph.json` missing or explicitly required but unavailable.
- `sentrux_fail`: structural regression, missing baseline requiring operator action, or sentrux gate failure.

When reporting results, say the category first and the step second.

Examples:

- `provider_quota: repowise scoped docs`
- `graph_missing: understand graph`
- `sentrux_fail: sentrux gate`

If all four category counters are zero, say the run is clean instead of summarizing every step again.

## Karpathy-Inspired Operating Rules

Use only these absorbed rules from the Karpathy skills repo:

- Idea file first for nontrivial pipeline changes: use `templates/idea-file.md`.
- Agentic loop: doctor -> lite -> normal -> read summary -> read understanding -> fix -> rerun -> commit.
- Minimalism: keep this as an orchestration shell over `rg`, `repowise`, Understand Anything, and `sentrux`; do not copy tool internals into this repo.
- Supply-chain hygiene: inspect `installPlan` before approving new install surfaces.
- Understanding-first: never report a run as handled without knowing the next action from `understanding.md`.

## Sentrux Rules

Legacy-heavy repos may configure `sentruxPath` in `pipeline.config.json` so Sentrux gates only the core area, such as `backend`.

Rules are separate from baselines. A baseline answers "did this session degrade structure"; `.sentrux/rules.toml` answers "did this session cross an architecture boundary." If `check_rules` reports `rules_missing`, copy `templates/sentrux-rules.example.toml` into the chosen scope and replace the sample layer/boundary names with real project boundaries.

If baseline is missing, use one of:

```powershell
& "$env:CODE_INTEL_HOME/run-code-intel.ps1" -RepoPath <repo-path> -Mode normal -SaveSentruxBaseline
```

or:

```powershell
& "$env:CODE_INTEL_HOME/run-code-intel.ps1" -RepoPath <repo-path> -Mode normal -AutoSaveMissingSentruxBaseline
```

Do not save a new baseline to hide a real regression.

## Current Known Alias

`k-atana` can be added as a local alias in `pipeline.config.json`, with Sentrux gated on `backend`.

For `k-atana`, broad `repowise init` at the repo root is wrong. The repo contains many nested external repos under tool/research folders, and repowise workspace discovery treats them as 40 separate repos. Use the scoped pipeline path, which indexes `backend` plus selected root metadata files inside the shadow worktree.

Stable team command:

```powershell
& "$env:CODE_INTEL_HOME/invoke-code-intel.ps1" -RepoPath <k-atana-path> -Mode normal
```

Docs-enabled variant:

```powershell
& "$env:CODE_INTEL_HOME/run-code-intel.ps1" -RepoPath <k-atana-path> -Mode normal -RepowiseDocs
```

Smoke test:

```powershell
& "$env:CODE_INTEL_HOME/test-code-intel-pipeline.ps1" -RepoPath <k-atana-path>
```

Provider preflight:

```powershell
& "$env:CODE_INTEL_HOME/test-code-intel-provider.ps1" -Json
```

Install check:

```powershell
& "$env:CODE_INTEL_HOME/install-code-intel-pipeline.ps1" -RepoPath <repo-path> -CheckProvider
```

Artifact index refresh:

```powershell
& "$env:CODE_INTEL_HOME/update-code-intel-index.ps1"
```

Scoped docs generation is available, but it is quota-sensitive and intentionally low-budget:

```powershell
& "$env:CODE_INTEL_HOME/Invoke-ScopedRepowise.ps1" -RepoPath <k-atana-path> -ScopePaths backend -RootFiles README.md,CLAUDE.md,pyproject.toml,requirements.txt,requirements-no-torch.txt,requirements-frozen.txt,.env.example,.gitignore -Docs
```

That path uses `Run-ScopedRepowiseDocs.py` with `coverage_pct=0.02`. If the provider is rate-limited, expect `docs_enabled=false` with a `docs_skip_reason` that points at provider quota rather than local tool failure.

## Output Handling

After each run, read `summary.md` first. Then read `hospital.md` for diagnosis, modality quality, treatment plan, and next protocol. Open `report.json` or `hospital-report.json` only when a step failed, details matter, or another agent needs machine-readable fields.
Read `understanding.md` before delegating follow-up work.

Check results in this order:

1. whether doctor passed
2. artifact path
3. summary counters
4. failure category counters
5. hospital disposition, triage status, overall score, and next protocol
6. state machine guards and modality quality gaps in `hospital-report.json`
7. Understand graph state
8. repowise state
9. sentrux gate/check result
10. sentrux insight deltas and CodeNexus hints
11. exact missing tools or failed checks
12. understanding report next action
13. surgery plan target when `next_protocol=surgery_plan`

For machine checks, use:

```powershell
& "$env:CODE_INTEL_HOME/check-code-intel-tools.ps1" -RepoPath <repo-path> -Json
```
