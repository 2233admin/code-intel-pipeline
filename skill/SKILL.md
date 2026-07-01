---
name: code-intel-pipeline
description: Use when managing local repository understanding with rg, repowise, Understand Anything, and sentrux. Triggers for code indexing, architecture graph refreshes, structural gates, local repo intelligence, CodeNexus-style pipeline work, or when checking whether this machine has the required tools.
---

# Code Intel Pipeline

Use the local pipeline instead of inventing another code-indexing stack.

## Non-Skippable Contract

Do not jump between tools. A normal code-intel session must run the full chain in this order:

1. Resolve the real repo path.
2. Run the doctor with JSON and require Repowise.
3. Run the stable wrapper in `normal` mode.
4. Read `summary.md`.
5. Read `hospital.md`.
6. Read `understanding.md`.
7. Only then run focused Sentrux, Repowise docs, artifact JSON reads, or follow-up repair commands.

Repowise is a hard dependency for this skill. It can index/status without a model provider, so provider quota only disables docs generation, not the Repowise step. If `repowise` is missing, stop and repair or report the missing dependency before claiming the pipeline has run. Do not treat Repowise as optional unless the user explicitly requests a narrow exact-search-only emergency run and names `-SkipRepowise`.

New project-intelligence tools, memory systems, graph providers, or governance methods must enter through the integration orchestration layer first. Do not wire a new external project directly into runner scripts or agent instructions.

Canonical files:

- Pipeline: `C:\c\Users\Administrator\projects\code-intel-pipeline\run-code-intel.ps1`
- Integration orchestrator: `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe orchestrate`
- Integration orchestrator compatibility script: `C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-CodeIntelOrchestrator.ps1`
- Integration registry: `C:\c\Users\Administrator\projects\code-intel-pipeline\orchestration\integrations.json`
- Rust CLI: `C:\c\Users\Administrator\projects\code-intel-pipeline\crates\code-intel-cli`
- Rust worker: `C:\c\Users\Administrator\projects\code-intel-pipeline\crates\code-nexus-lite`
- Project discovery: `C:\c\Users\Administrator\projects\code-intel-pipeline\Find-CodeIntelProjects.ps1`
- Doctor: `C:\c\Users\Administrator\projects\code-intel-pipeline\check-code-intel-tools.ps1`
- Sentrux Agent tools: `C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1`
- Config: `C:\c\Users\Administrator\projects\code-intel-pipeline\pipeline.config.json`
- Artifacts: `%LOCALAPPDATA%\code-intel\artifacts\<repo>\<timestamp>\` by default, or `CODE_INTEL_ARTIFACT_ROOT` when set.
- Artifact data contract: `C:\c\Users\Administrator\projects\code-intel-pipeline\docs\artifact-data-contract.md`
- Agent goal intake: `C:\c\Users\Administrator\projects\code-intel-pipeline\docs\agent-goal-intake.md`
- Harness factory reference: `C:\c\Users\Administrator\projects\code-intel-pipeline\docs\harness-factory-reference.md`
- Skill development benchmark: `C:\c\Users\Administrator\projects\code-intel-pipeline\docs\skill-development-benchmark.md`
- Implementation minimalism benchmark: `C:\c\Users\Administrator\projects\code-intel-pipeline\docs\implementation-minimalism-benchmark.md`
- Ponytail impact scoreboard: `C:\c\Users\Administrator\projects\code-intel-pipeline\docs\ponytail-impact-scoreboard.md`
- Project management support: `C:\c\Users\Administrator\projects\code-intel-pipeline\docs\project-management-support.md`
- Issue tracker config: `C:\c\Users\Administrator\projects\code-intel-pipeline\docs\agents\issue-tracker.md`
- Triage label config: `C:\c\Users\Administrator\projects\code-intel-pipeline\docs\agents\triage-labels.md`
- Domain docs config: `C:\c\Users\Administrator\projects\code-intel-pipeline\docs\agents\domain.md`
- Templates: `C:\c\Users\Administrator\projects\code-intel-pipeline\templates\`

## Required First Step

On a new machine or teammate session, run the installer first:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\install-code-intel-pipeline.ps1 -RepoPath <repo-path>
```

Use `-CheckProvider` to also ping the MiniMax Anthropic-compatible endpoint. Use `-RepairSkillLinks` when the shared skill should be installed or repaired for Codex and Claude. If the `.agents` copy is missing, the installer seeds it from the repo's bundled `skill\` directory first. Use `-InstallMissing` on teammate machines when missing CLI tools should be installed automatically where supported. Never ask the installer to write API keys; it only checks whether user-scoped env vars exist.

Team bootstrap:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\install-code-intel-pipeline.ps1 -RepoPath <repo-path> -CheckProvider -RepairSkillLinks -InstallMissing
```

For one-command teammate setup, use:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\bootstrap-new-machine.ps1 -RepoPath <repo-path>
```

The installer also installs the repo-owned Sentrux shim into `CODE_INTEL_BIN` or `%LOCALAPPDATA%\code-intel\bin`, prepends that directory to the user PATH, and verifies `sentrux pro status`. The shim auto-activates local open-source Pro features, forwards all non-`pro` commands to the real `sentrux.exe` when present, and falls back to `sentrux-lite-core.ps1` for portable `scan`, `health`, `check`, and `gate`.

The installer applies the repo-owned Sentrux V overlay by default because the upstream Windows `vlang` standard plugin package is incomplete in Sentrux 0.5.7. The overlay lives under `overlays\sentrux\vlang`, installs with `Install-SentruxVlangOverlay.ps1`, and should make `sentrux plugin list` show `vlang v0.2.0 [v]`. Use `Test-SentruxVlangOverlay.ps1` to prove the plugin builds a real V graph with 2 files, 1 import, and 1 call. Use `-SkipSentruxVlangOverlay` only when explicitly testing upstream plugin behavior.

For machine-readable bootstrap status, add `-Json` and read `installActions` first. Valid statuses are `already_present`, `not_requested`, `installed`, `installed_restart_required`, and `install_failed`.

Use `-AuditInstallPlan` before `-InstallMissing` when reviewing a new teammate machine. Read `installPlan` in JSON output for installer source, command, purpose, and supply-chain risk notes.

Always run the doctor before using the pipeline:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\check-code-intel-tools.ps1 -RepoPath <repo-path> -RequireRepowise -Json
```

Use JSON by default so another agent can read exact missing tools, strict flags, repo state, and Sentrux status. If the doctor fails because `repowise` is missing, do not run the wrapper yet.

Find candidate repositories before choosing a `RepoPath`:

```powershell
D:\projects\_tools\code-intel-pipeline\Find-CodeIntelProjects.ps1 -Root D:\projects -Json
D:\projects\_tools\code-intel-pipeline\Find-CodeIntelProjects.ps1 -Root D:\projects -WizTreeExe WizTree64.exe -Json
D:\projects\_tools\code-intel-pipeline\Find-CodeIntelProjects.ps1 -WizTreeCsv C:\tmp\wiztree.csv -Json
```

WizTree CLI/CSV is optional acceleration for project discovery only. It is not a scanner dependency.

Preferred stable wrapper:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\invoke-code-intel.ps1 -RepoPath <repo-path> -Mode normal
```

Batch wrappers:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\invoke-code-intel.ps1 -Config C:\c\Users\Administrator\projects\code-intel-pipeline\pipeline.config.json -Repos k-atana,glyph-arts -Mode normal
C:\c\Users\Administrator\projects\code-intel-pipeline\invoke-code-intel.ps1 -Config C:\c\Users\Administrator\projects\code-intel-pipeline\pipeline.config.json -All -Mode lite
```

Use `-Repo <alias>` only when `pipeline.config.json` already defines that alias. Prefer `-RepoPath <repo-path>` for teammate machines because their project disks may not be `D:`.

Stop and report clearly if any required tool is missing:

- `rg`
- `git`
- `python`
- `repowise`
- `sentrux`
- internal Rust graph provider: `target\debug\code-intel.exe graph`

## Workflow

1. Run the doctor first and require Repowise:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\check-code-intel-tools.ps1 -RepoPath <repo-path> -RequireRepowise -Json
```

2. Run the stable wrapper:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\invoke-code-intel.ps1 -RepoPath <repo-path> -Mode normal
```

3. Use the raw pipeline only when a narrower mode or a special flag is needed:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\run-code-intel.ps1 -RepoPath <repo-path> -Mode normal
```

4. Add `-RepowiseDocs` when the user wants scoped repowise wiki generation instead of index-only refresh.

   The pipeline will run `test-code-intel-provider.ps1` first. If provider quota is unavailable, docs generation is disabled for that run and the failure category is recorded. Index-only Repowise must still run.

Use `-Mode lite` for a cheap status check. Use `-Mode full` when a fresh Understand-compatible graph is needed. Full mode must call the internal Rust graph provider before any external `/understand` fallback.

When a repo config defines `repowiseScopePaths` or `repowiseRootFiles`, the pipeline runs `repowise` inside a sparse shadow worktree under `%LOCALAPPDATA%\code-intel\repowise\<repo>` by default, or `CODE_INTEL_SHADOW_ROOT` when set. This is the default for noisy mono-repos with nested third-party repos.

Scoped Repowise has a bounded timeout. Use `-RepowiseTimeoutSeconds <seconds>` when a huge or dirty repo should fail fast. A Repowise timeout is a real Repowise failure unless the user explicitly authorized `-SkipRepowise`; do not silently downgrade it to optional semantic-memory skip.

5. If the report says the graph is missing, run the internal provider first:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe graph --repo <repo-path> --language zh --write --json
```

For a full graph rebuild:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe graph --repo <repo-path> --language zh --full --write --json
```

Then rerun the pipeline. Use `/understand <repo-path> --language zh` only as a compatibility fallback when the internal Rust graph provider fails or when the user explicitly asks for a richer external Understand Anything pass.

6. After each run, read `summary.md` first. Its `Sentrux Insight` section shows parsed quality/coupling/cycle/god-file deltas, scan scale, next actions, and CodeNexus follow-up hints. Open `report.json` when the summary shows failure, manual action, a category count above zero, or when the raw `sentruxInsight` object matters.

   If `summary.md` or `report.json` contains `node lint hygiene: manual_required`, treat it as a pre-push hygiene warning: root ESLint may scan generated/vendor static assets such as `apps/*/public/charting_library`, `apps/*/public/datafeeds`, or `packages/*/vendor`. Add explicit ESLint ignores or run root lint before pushing.

   The pipeline also writes `codenexus-context.json`. Read it when you need concrete hotspot files, recent commits, and reference hits instead of generic follow-up hints.

   `Invoke-SentruxAgentTool.ps1` supports both short names and MCP-style aliases for common reads: `scan`/`sentrux_scan`, `health`/`sentrux_health`, `dsm`/`sentrux_dsm`, `git_stats`/`sentrux_git_stats`, and `test_gaps`/`sentrux_test_gaps`.

7. Read `hospital.md` after `summary.md`, then read `understanding.md` before handing results to a teammate or another agent. `understanding.md` is the understanding-first layer: assumptions, verified facts, unverified areas, human inspection, and next action.

8. If the user asks whether the stack is healthy, answer from:
   - doctor result
   - summary counters
   - failure category counters
   - understanding report next action
   - repowise docs state
   - sentrux gate result
   - sentrux insight metric deltas and CodeNexus hints

## AI Invocation Map

Use these commands as the exposed callable surface. Prefer `-RepoPath <repo-path>` unless a configured alias is already known.

| Intent | Command |
|---|---|
| Build Rust runtime | `cargo build --workspace` |
| Test Rust runtime | `cargo test --workspace` |
| Validate integration orchestration | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe orchestrate --action Validate --json` |
| Validate provider API | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe provider --action Validate --json` |
| List provider API operations | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe provider --action List --json` |
| Plan Repowise provider operation | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe provider --action Plan --provider repowise --operation index --repo <repo-path> --json` |
| Plan Understand provider operation | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe provider --action Plan --provider understand --operation graph --repo <repo-path> --json` |
| Validate global provider routes | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe route --action Validate --json` |
| List global provider routes | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe route --action List --json` |
| Plan Repowise route | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe route --action Plan --provider repowise --operation index --repo <repo-path> --json` |
| Plan Understand route | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe route --action Plan --provider understand --operation graph --repo <repo-path> --json` |
| Show integration plan | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe orchestrate --action Plan --repo <repo-path> --mode normal --json` |
| Build Understand-compatible graph | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe graph --repo <repo-path> --language zh --write --json` |
| Full graph rebuild | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe graph --repo <repo-path> --language zh --full --write --json` |
| Rust artifact doctor | `C:\c\Users\Administrator\projects\code-intel-pipeline\target\debug\code-intel.exe doctor --json` |
| Discover candidate repos | `C:\c\Users\Administrator\projects\code-intel-pipeline\Find-CodeIntelProjects.ps1 -Root <projects-root> -Json` |
| Bootstrap teammate machine | `C:\c\Users\Administrator\projects\code-intel-pipeline\install-code-intel-pipeline.ps1 -RepoPath <repo-path> -CheckProvider -RepairSkillLinks -InstallMissing -RequireRepowise -Json` |
| Audit install plan only | `C:\c\Users\Administrator\projects\code-intel-pipeline\install-code-intel-pipeline.ps1 -RepoPath <repo-path> -AuditInstallPlan -RequireRepowise -Json` |
| Required preflight | `C:\c\Users\Administrator\projects\code-intel-pipeline\check-code-intel-tools.ps1 -RepoPath <repo-path> -RequireRepowise -Json` |
| Normal full run | `C:\c\Users\Administrator\projects\code-intel-pipeline\invoke-code-intel.ps1 -RepoPath <repo-path> -Mode normal` |
| Cheap but still ordered run | `C:\c\Users\Administrator\projects\code-intel-pipeline\invoke-code-intel.ps1 -RepoPath <repo-path> -Mode lite` |
| Full graph-oriented run | `C:\c\Users\Administrator\projects\code-intel-pipeline\invoke-code-intel.ps1 -RepoPath <repo-path> -Mode full` |
| Repowise index only | `C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-ScopedRepowise.ps1 -RepoPath <repo-path> -ScopePaths <paths> -RootFiles <files>` |
| Repowise docs | `C:\c\Users\Administrator\projects\code-intel-pipeline\run-code-intel.ps1 -RepoPath <repo-path> -Mode normal -RepowiseDocs` |
| Provider preflight for docs | `C:\c\Users\Administrator\projects\code-intel-pipeline\test-code-intel-provider.ps1 -Json` |
| Sentrux coding session start | `C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1 session_start <repo-path>` |
| Sentrux coding session end | `C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1 session_end <repo-path>` |
| Sentrux focused scan | `C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1 scan <repo-path>` |
| Sentrux rules | `C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1 check_rules <repo-path>` |
| Sentrux DSM UI data | `C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1 dsm <repo-path>` |
| Sentrux test gaps | `C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1 test_gaps <repo-path>` |
| Sentrux what-if gates | `C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1 what_if <repo-path>` |
| Smoke test this stack | `C:\c\Users\Administrator\projects\code-intel-pipeline\test-code-intel-pipeline.ps1 -RepoPath <repo-path>` |
| Refresh artifact index | `C:\c\Users\Administrator\projects\code-intel-pipeline\update-code-intel-index.ps1` |

PS1 replacement direction is Rust-first: `run-code-intel.ps1` should collapse into `target\debug\code-intel.exe run --repo <repo-path> --mode normal`, and `Invoke-SentruxAgentTool.ps1` should collapse into `target\debug\code-intel.exe sentrux <scan|health|session_start|session_end|check_rules|test_gaps|what_if> <repo-path>`. Until those commands are fully implemented, the PS1 files are compatibility wrappers and known debt, not the source of truth.

Global provider routing is mandatory for route-needing integrations. `code-intel.exe provider` is the source of truth and emits the shared `code-intel-provider-api.v1` schema for Repowise and Understand-compatible graph operations. Repowise operations must route through `/api/providers/repowise/*` and `code-intel.exe provider --provider repowise`; Understand-compatible graph operations must route through `/api/providers/understand/*` and `code-intel.exe provider --provider understand`. Add future providers to the global provider layer before exposing new agent commands or HTTP paths.

Artifact reads after a run must start with `summary.md`, then `hospital.md`, then `understanding.md`. Use `report.json`, `hospital-report.json`, `codenexus-context.json`, `sentrux-dsm.json`, `sentrux-file-details.json`, `sentrux-hotspots.json`, `sentrux-evolution.json`, and `sentrux-what-if.json` only for exact fields, UI data, or failure details.

For an Agent coding session that needs Sentrux as a live guardrail, use the dedicated wrapper:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1 scan <scope-path>
C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1 session_start <scope-path>
C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-SentruxAgentTool.ps1 session_end <scope-path>
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
- `code-intel graph`: internal Understand-compatible architecture graph snapshot.
- `Understand Anything`: optional richer external graph pass and compatibility fallback.
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

- Idea file first for nontrivial pipeline changes: use `templates\idea-file.md`.
- Agentic loop: doctor -> lite -> normal -> read summary -> read understanding -> fix -> rerun -> commit.
- Minimalism: internalize project-intelligence dependencies through the orchestration layer and stable Rust commands; do not scatter direct calls to external tools across runner scripts or agent instructions.
- Implementation minimalism: before coding choose the first sufficient rung from `docs\implementation-minimalism-benchmark.md`: do nothing, reuse this repository, standard library, platform native capability, already-installed dependency, one-liner, then smallest local implementation.
- Lazy about solution, never lazy about reading/evidence/safety: implementation minimalism cannot remove verification, error handling, security, accessibility, data-loss prevention, or artifact contract guarantees.
- Project management intake: before turning findings into tracked work, read `docs\project-management-support.md` and `docs\agents\*.md`. Linear and Obsidian/LLM wiki support is optional intake/output, not scanner runtime or credential storage.
- Supply-chain hygiene: inspect `installPlan` before approving new install surfaces.
- Understanding-first: never report a run as handled without knowing the next action from `understanding.md`.

## Sentrux Rules

Legacy-heavy repos may configure `sentruxPath` in `pipeline.config.json` so Sentrux gates only the core area, such as `backend`.

Rules are separate from baselines. A baseline answers "did this session degrade structure"; `.sentrux/rules.toml` answers "did this session cross an architecture boundary." If `check_rules` reports `rules_missing`, copy `templates\sentrux-rules.example.toml` into the chosen scope and replace the sample layer/boundary names with real project boundaries.

If baseline is missing, use one of:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\run-code-intel.ps1 -RepoPath <repo-path> -Mode normal -SaveSentruxBaseline
```

or:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\run-code-intel.ps1 -RepoPath <repo-path> -Mode normal -AutoSaveMissingSentruxBaseline
```

Do not save a new baseline to hide a real regression.

## Current Known Alias

`k-atana` can be added as a local alias in `pipeline.config.json`, with Sentrux gated on `backend`.

For `k-atana`, broad `repowise init` at the repo root is wrong. The repo contains many nested external repos under tool/research folders, and repowise workspace discovery treats them as 40 separate repos. Use the scoped pipeline path, which indexes `backend` plus selected root metadata files inside the shadow worktree.

Stable team command:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\invoke-code-intel.ps1 -RepoPath <k-atana-path> -Mode normal
```

Docs-enabled variant:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\run-code-intel.ps1 -RepoPath <k-atana-path> -Mode normal -RepowiseDocs
```

Smoke test:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\test-code-intel-pipeline.ps1 -RepoPath <k-atana-path>
```

Provider preflight:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\test-code-intel-provider.ps1 -Json
```

Install check:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\install-code-intel-pipeline.ps1 -RepoPath <repo-path> -CheckProvider
```

Artifact index refresh:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\update-code-intel-index.ps1
```

Scoped docs generation is available, but it is quota-sensitive and intentionally low-budget:

```powershell
C:\c\Users\Administrator\projects\code-intel-pipeline\Invoke-ScopedRepowise.ps1 -RepoPath <k-atana-path> -ScopePaths backend -RootFiles README.md,CLAUDE.md,pyproject.toml,requirements.txt,requirements-no-torch.txt,requirements-frozen.txt,.env.example,.gitignore -Docs
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
C:\c\Users\Administrator\projects\code-intel-pipeline\check-code-intel-tools.ps1 -RepoPath <repo-path> -Json
```
