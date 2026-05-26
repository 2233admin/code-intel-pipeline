# Code Intel Pipeline

Local orchestration for the current code-understanding stack:

- `rg`: precise text and file inventory.
- `repowise`: long-lived semantic index, wiki, and multi-repo workspace memory.
- `Understand Anything`: Claude skill that produces `.understand-anything/knowledge-graph.json`.
- `sentrux`: architectural rules and structural regression gate.

This is a thin pipeline. It does not vendor or wrap the tools internally yet. The goal is to prove the workflow before merging capabilities into a future CodeNexus-style tool.

## Run

Install or verify this machine first:

```powershell
D:\projects\_tools\code-intel-pipeline\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo -CheckProvider
```

On a teammate machine, add `-InstallMissing` to install missing command-line tools when a supported installer is available:

```powershell
D:\projects\_tools\code-intel-pipeline\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo -CheckProvider -RepairSkillLinks -InstallMissing
```

The installer checks `rg`, `git`, `python`, `repowise`, `sentrux`, the shared Codex/Claude skill links, Understand Anything, config, repo doctor, and optional provider access. It never writes API keys. JSON output includes `installActions` so another agent can tell whether each tool was already present, installed, needs a new shell, or failed.

Always start with the doctor:

```powershell
D:\projects\_tools\code-intel-pipeline\check-code-intel-tools.ps1 -RepoPath C:\path\to\repo
```

```powershell
D:\projects\_tools\code-intel-pipeline\run-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal
```

With aliases and defaults:

```powershell
D:\projects\_tools\code-intel-pipeline\run-code-intel.ps1 -Repo my-repo -Mode normal
```

Team default:

```powershell
D:\projects\_tools\code-intel-pipeline\check-code-intel-tools.ps1 -RepoPath C:\path\to\repo
D:\projects\_tools\code-intel-pipeline\run-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal
```

Stable wrapper:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal
```

Batch wrappers:

```powershell
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -Repos k-atana,glyph-arts -Mode normal
D:\projects\_tools\code-intel-pipeline\invoke-code-intel.ps1 -Config D:\projects\_tools\code-intel-pipeline\pipeline.config.json -All -Mode lite
```

For a first baseline:

```powershell
D:\projects\_tools\code-intel-pipeline\run-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal -SaveSentruxBaseline
```

For a full architecture refresh, run the Claude skill first:

```text
/understand C:\path\to\repo --language zh --full
```

Then run the pipeline again.

MiniMax Anthropic-compatible provider config is expected at user level:

```text
ANTHROPIC_BASE_URL=https://api.minimaxi.com/anthropic
REPOWISE_PROVIDER=anthropic
ANTHROPIC_API_KEY=<secret>
ANTHROPIC_AUTH_TOKEN=<secret>
```

Do not write secrets into repo files. Repo-local config may store provider/model names only.

## Agent Skill

The shared skill lives at:

```text
C:\Users\Administrator\.agents\skills\code-intel-pipeline
```

The distributable copy is included in this repo at:

```text
skill\SKILL.md
skill\agents\openai.yaml
```

Claude and Codex both point at that same directory:

```text
C:\Users\Administrator\.claude\skills\code-intel-pipeline
C:\Users\Administrator\.codex\skills\code-intel-pipeline
```

The OpenAI/Codex metadata is in `agents/openai.yaml` and is validated with the `skill-creator` quick validator.

## Modes

- `lite`: inventory and status checks only.
- `normal`: update or initialize `repowise`, check Understand graph, run Sentrux gate.
- `full`: same as normal, but the emitted Understand command includes `--full`.

Add `-RepowiseDocs` to make the pipeline run scoped wiki generation instead of index-only repowise refresh for repos that define scoped repowise settings.

When `-RepowiseDocs` is set, the pipeline runs a provider preflight first. If quota or rate limits are hit, docs generation is disabled for that run and the summary records the provider failure category.

When `repowiseScopePaths` or `repowiseRootFiles` is configured for a repo, the pipeline uses a sparse git worktree under `%LOCALAPPDATA%\code-intel\repowise\<repo>` by default and runs `repowise` there instead of at the noisy repo root.

For `k-atana`, that shadow worktree is the supported team path. It indexes current `backend` working-tree contents plus selected root metadata files without crawling nested external repos under `tools/`.

Example:

```powershell
D:\projects\_tools\code-intel-pipeline\run-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal -RepowiseDocs
```

Smoke test:

```powershell
D:\projects\_tools\code-intel-pipeline\test-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo
```

Provider preflight:

```powershell
D:\projects\_tools\code-intel-pipeline\test-code-intel-provider.ps1 -Json
```

Install check:

```powershell
D:\projects\_tools\code-intel-pipeline\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo
```

Use `-RepairSkillLinks` when the shared `code-intel-pipeline` skill should be installed or repaired for Codex and Claude. If `.agents\skills\code-intel-pipeline` is missing, the installer seeds it from this repo's bundled `skill\` directory first, then creates the `.codex` and `.claude` junctions. Use `-InstallMissing` only on machines where you want the script to install missing CLI tools. It uses `winget` for Git/Python/ripgrep, `pip` for repowise, and `cargo install sentrux --locked` only if Cargo is present. Unsupported installs are reported as manual fixes instead of hidden magic. The installer never writes API keys.

Artifact index:

```powershell
D:\projects\_tools\code-intel-pipeline\update-code-intel-index.ps1
```

The index is written to:

```text
%LOCALAPPDATA%\code-intel\artifacts\index.md
```

Set `CODE_INTEL_ARTIFACT_ROOT` or pass `-ArtifactRoot` when the team wants artifacts on a shared project disk.

## Rules

- Keep generated tool state local at first.
- Commit only intentional governance files such as `.sentrux/rules.toml`.
- Do not merge tool internals into the future unified tool until this pipeline has survived real repo work.
- Treat `Understand Anything` as the architecture snapshot, `repowise` as memory, and `sentrux` as the gate.
- Use `sentruxPath` for legacy-heavy repos where vendored or research-copy code would make a full-repo gate noisy.
- For `k-atana`, the pipeline now uses scoped `repowise`: sparse worktree plus live file sync for `backend` and a few root metadata files. That avoids nested tool repos while preserving current working-tree changes in the indexed scope.
- MiniMax-backed `repowise` index-only mode is stable.
- Scoped wiki generation now runs through `Run-ScopedRepowiseDocs.py` with a small default budget (`coverage_pct=0.02`) so the team can smoke-test docs without asking the provider to eat the whole repo in one shot.
- If the provider is rate-limited, the helper leaves `docs_enabled=false` plus `docs_skip_reason=no pages generated; likely provider quota or rate limit`. That means the local toolchain is fine and the blocker is upstream quota, not local indexing.

## Future Merge Shape

When the pipeline is stable, merge by interface, not by copying whole projects:

- `SearchProvider`: backed by `repowise`.
- `ArchitectureGraphProvider`: backed by `Understand Anything` graph artifacts.
- `StructureGateProvider`: backed by `sentrux`.
- `ExactSearchProvider`: backed by `rg`.

That keeps the unified tool small instead of becoming a museum of other people's abstractions.

## Docs

Architecture notes live here:

- `D:\projects\_tools\code-intel-pipeline\docs\code-intel-architecture.md`
