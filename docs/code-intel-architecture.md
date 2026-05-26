# Code Intel Architecture

This stack is the local stable-ops layer for repository understanding.

It is built around one rule: keep the entrypoint small, keep tool roles explicit, and keep failures honest.

## Layers

1. `invoke-code-intel.ps1`
   Thin operator entrypoint. Runs doctor first, then the pipeline. Supports one repo, a repo list, or all configured repos.

2. `check-code-intel-tools.ps1`
   Environment doctor. Verifies local tools, Understand Anything presence, repo path, and Sentrux scope state.

3. `run-code-intel.ps1`
   Main orchestrator. Produces artifacts, summary, report, and failure classification.

4. Tool adapters
   - `rg`: exact inventory
   - `repowise`: semantic index and optional docs
   - `Understand Anything`: graph artifact
   - `sentrux`: structure gate

5. Scoped helpers
   - `Invoke-ScopedRepowise.ps1`
   - `Run-ScopedRepowiseDocs.py`

6. Stable-ops helpers
   - `install-code-intel-pipeline.ps1`
   - `test-code-intel-provider.ps1`
   - `test-code-intel-pipeline.ps1`
   - `update-code-intel-index.ps1`

These exist because some repos are too dirty or too nested for raw `repowise init` at the root.

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
D:\projects\_tools\code-intel-pipeline\install-code-intel-pipeline.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo k-atana -CheckProvider
```

Install or repair a teammate machine:

```powershell
D:\projects\_tools\code-intel-pipeline\install-code-intel-pipeline.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo k-atana -CheckProvider -RepairSkillLinks -InstallMissing
```

`-InstallMissing` is explicit by design. The default installer is a doctor; the install mode attempts supported CLI installs and records every attempt in `installActions`.

`-RepairSkillLinks` installs the bundled `skill\` copy into the user profile when the shared `.agents` skill is absent, then links Codex and Claude to that shared copy.

Doctor and normal run:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo k-atana -Mode normal
```

Docs-enabled run:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo k-atana -Mode normal -RepowiseDocs
```

Batch run:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -All -Mode lite
```

Smoke test:

```powershell
D:\projects\_tools\code-intel-pipeline\test-code-intel-pipeline.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repo k-atana
```

Artifact index:

```powershell
D:\projects\_tools\code-intel-pipeline\update-code-intel-index.ps1
```

## Design Rule

Copy the operational shell, not the internal machinery.

That is the useful lesson from `gitnexus-stable-ops`.
