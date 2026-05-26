---
name: code-intel-pipeline
description: Use when managing local repository understanding with rg, repowise, Understand Anything, and sentrux. Triggers for code indexing, architecture graph refreshes, structural gates, local repo intelligence, CodeNexus-style pipeline work, or when checking whether this machine has the required tools.
---

# Code Intel Pipeline

Use the local pipeline instead of inventing another code-indexing stack.

Canonical files:

- Pipeline: `D:\projects\_tools\code-intel-pipeline\run-code-intel.ps1`
- Doctor: `D:\projects\_tools\code-intel-pipeline\check-code-intel-tools.ps1`
- Config: `D:\projects\_tools\code-intel-pipeline\pipeline.config.json`
- Artifacts: `D:\projects\_artifacts\code-intel\<repo>\<timestamp>\`

## Required First Step

On a new machine or teammate session, run the installer first:

```powershell
D:\projects\_tools\code-intel-pipeline\install-code-intel-pipeline.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo <repo-or-alias>
```

Use `-CheckProvider` to also ping the MiniMax Anthropic-compatible endpoint. Use `-RepairSkillLinks` only when the shared skill exists but Codex or Claude skill links are missing. Use `-InstallMissing` on teammate machines when missing CLI tools should be installed automatically where supported. Never ask the installer to write API keys; it only checks whether user-scoped env vars exist.

Team bootstrap:

```powershell
D:\projects\_tools\code-intel-pipeline\install-code-intel-pipeline.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo <repo-or-alias> -CheckProvider -RepairSkillLinks -InstallMissing
```

For machine-readable bootstrap status, add `-Json` and read `installActions` first. Valid statuses are `already_present`, `not_requested`, `installed`, `installed_restart_required`, and `install_failed`.

Always run the doctor before using the pipeline:

```powershell
D:\projects\_tools\code-intel-pipeline\check-code-intel-tools.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo <repo-or-alias>
```

Use `-Json` when another agent needs machine-readable output.

Preferred stable wrapper:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo <repo-or-alias> -Mode normal
```

Batch wrappers:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repos k-atana,glyph-arts -Mode normal
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -All -Mode lite
```

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
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo <repo-or-alias> -Mode normal
```

2. Use the raw pipeline only when a narrower mode or a special flag is needed:

```powershell
D:\projects\_tools\code-intel-pipeline\run-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo <repo-or-alias> -Mode normal
```

3. Add `-RepowiseDocs` when the user wants scoped repowise wiki generation instead of index-only refresh.

   The pipeline will run `test-code-intel-provider.ps1` first. If provider quota is unavailable, docs generation is disabled for that run and the failure category is recorded.

Use `-Mode lite` for a cheap status check. Use `-Mode full` when a fresh Understand Anything graph is needed.

When a repo config defines `repowiseScopePaths` or `repowiseRootFiles`, the pipeline runs `repowise` inside a sparse shadow worktree under `D:\projects\_cache\code-intel\repowise\<repo>`. This is the default for noisy mono-repos with nested third-party repos.

4. If the report says the Understand graph is missing, tell the user or Claude-side agent to run:

```text
/understand <repo-path> --language zh
```

For a full graph rebuild:

```text
/understand <repo-path> --language zh --full
```

Then rerun the pipeline.

5. After each run, read `summary.md` first. Open `report.json` only when the summary shows failure, manual action, or a category count above zero.

6. If the user asks whether the stack is healthy, answer from:
   - doctor result
   - summary counters
   - failure category counters
   - repowise docs state
   - sentrux gate result

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

## Sentrux Rules

Legacy-heavy repos may configure `sentruxPath` in `pipeline.config.json` so Sentrux gates only the core area, such as `backend`.

If baseline is missing, use one of:

```powershell
D:\projects\_tools\code-intel-pipeline\run-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo <repo> -Mode normal -SaveSentruxBaseline
```

or:

```powershell
D:\projects\_tools\code-intel-pipeline\run-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo <repo> -Mode normal -AutoSaveMissingSentruxBaseline
```

Do not save a new baseline to hide a real regression.

## Current Known Alias

`k-atana` maps to `D:\projects\_quant\k-atana`, with Sentrux gated on `backend`.

For `k-atana`, broad `repowise init` at the repo root is wrong. The repo contains many nested external repos under tool/research folders, and repowise workspace discovery treats them as 40 separate repos. Use the scoped pipeline path, which indexes `backend` plus selected root metadata files inside the shadow worktree.

Stable team command:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo k-atana -Mode normal
```

Docs-enabled variant:

```powershell
D:\projects\_tools\code-intel-pipeline\run-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo k-atana -Mode normal -RepowiseDocs
```

Smoke test:

```powershell
D:\projects\_tools\code-intel-pipeline\test-code-intel-pipeline.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo k-atana
```

Provider preflight:

```powershell
D:\projects\_tools\code-intel-pipeline\test-code-intel-provider.ps1 -Json
```

Install check:

```powershell
D:\projects\_tools\code-intel-pipeline\install-code-intel-pipeline.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo k-atana -CheckProvider
```

Artifact index refresh:

```powershell
D:\projects\_tools\code-intel-pipeline\update-code-intel-index.ps1
```

Scoped docs generation is available, but it is quota-sensitive and intentionally low-budget:

```powershell
D:\projects\_tools\code-intel-pipeline\Invoke-ScopedRepowise.ps1 -RepoPath D:\projects\_quant\k-atana -ScopePaths backend -RootFiles README.md,CLAUDE.md,pyproject.toml,requirements.txt,requirements-no-torch.txt,requirements-frozen.txt,.env.example,.gitignore -Docs
```

That path uses `Run-ScopedRepowiseDocs.py` with `coverage_pct=0.02`. If the provider is rate-limited, expect `docs_enabled=false` with a `docs_skip_reason` that points at provider quota rather than local tool failure.

## Output Handling

After each run, read `summary.md` first. Open `report.json` only when a step failed or details matter.

Check results in this order:

1. whether doctor passed
2. artifact path
3. summary counters
4. failure category counters
5. Understand graph state
6. repowise state
7. sentrux gate result
8. exact missing tools or failed checks

For machine checks, use:

```powershell
D:\projects\_tools\code-intel-pipeline\check-code-intel-tools.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo <repo-or-alias> -Json
```
