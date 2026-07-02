# Changelog

All notable changes to **code-intel-pipeline** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2] — 2026-06-10

First public release of code-intel-pipeline. Headline addition is
the Rust + iii worker binary `code-nexus-lite` that replaces the
Windows-only PowerShell surface with a cross-platform Agent-callable
HTTP API. Also adds a PR-time skill-check quality gate.

### Added

- **`crates/code-nexus-lite/`** — Rust + iii worker binary, wraps Repowise + Sentrux for Agent-friendly code-understanding context. 5.2 MB stripped + LTO. Cross-platform replacement for the Windows-only `Invoke-CodeNexusLite.ps1`. Apache-2.0 license (matches iii SDK).
  - 3 iii functions: `codenexus::scan` / `codenexus::lite` / `codenexus::doctor`
  - 3 HTTP triggers: `POST /scan` / `POST /lite` / `POST /doctor`
  - Depends on `iii-sdk = "0.11"` (crates.io, Apache-2.0) + `repowise` 0.10.0 (Python) + `sqlite3` CLI
  - See `crates/code-nexus-lite/README.md` for the full design

- **`.github/workflows/skill-check.yml`** — PR-time quality gate. Runs a heuristic 8-dim darwin-skill scoring on every changed SKILL.md, validates YAML frontmatter, checks for broken internal links. Threshold 70/100 to pass. Triggers on PRs that touch `crates/code-nexus-lite/`, `.claude/skills/`, or `skills/`.

- **`.gitignore` updates** — Added `target/` (Rust build artifacts), IDE files (`.idea/`, `.vscode/`), OS files (`.DS_Store`, `Thumbs.db`), PowerShell artifacts (`*.ps1.xml`).

- **`crates/code-nexus-lite/.gitignore`** — Same as above, scoped to the sub-crate.

### Changed

- **Docs LLM provider generalized: local models + custom APIs** — provider is
  now selected via `CODE_INTEL_PROVIDER` (default `anthropic`) with generic
  `CODE_INTEL_MODEL` / `CODE_INTEL_API_KEY` / `CODE_INTEL_BASE_URL`, reusing
  repowise's own provider registry (anthropic / openai-compatible / ollama /
  anything else it ships). Keyless providers (ollama) work without
  credentials. `test-code-intel-provider.ps1` preflights all three families
  through the repowise uv venv python (system-python dependency dropped) and
  keeps `-Provider`/`-Model` overrides. `CODE_INTEL_ANTHROPIC_*` remains as
  backward-compatible fallback for provider=anthropic. See README "Docs LLM
  provider 配置".
- **Provider credentials moved to dedicated `CODE_INTEL_ANTHROPIC_*` env vars** — `test-code-intel-provider.ps1` and `Invoke-ScopedRepowise.ps1` now prefer user/process-scoped `CODE_INTEL_ANTHROPIC_API_KEY` / `CODE_INTEL_ANTHROPIC_BASE_URL` and inject them into the child process's `ANTHROPIC_*`. Global `ANTHROPIC_*` is no longer required (or checked by the installer): on dev machines it belongs to the Claude Code proxy chain and must not be repointed at the docs provider. `CODE_INTEL_MODEL` overrides the docs model (default `MiniMax-M2.7`).
- `Invoke-ScopedRepowise.ps1` — `Run-ScopedRepowiseDocs.py` is now executed with the repowise uv-tool venv python (`%APPDATA%\uv\tools\repowise\Scripts\python.exe`) instead of system python, which lacks the `repowise` package and made every docs run fail with `ModuleNotFoundError`.
- **`overlays/repowise/README.md`** — documents the local patch to repowise's `anthropic.py` (join text blocks, skip `ThinkingBlock`) required for reasoning models behind Anthropic-compatible endpoints (MiniMax-M2.x). Patch lives in the installed venv and must be re-applied after `uv tool upgrade repowise`.

- `Invoke-SentruxAgentTool.ps1` — minor edits
- `templates/sentrux-rules.example.toml` — minor edits
- `install-code-intel-pipeline.ps1` — `Install-SentruxShim` no longer copies `sentrux-shim.ps1`/`sentrux-lite-core.ps1` bodies into `%LOCALAPPDATA%\code-intel\bin\`. It now generates thin forwarder scripts that hardcode the repo path and forward `$args`/exit code to the real files under `tools\sentrux-shim\` in the repo, plus a `repo.json` recording the resolved repo root. Editing the repo's shim scripts now takes effect immediately on the next PATH invocation — no reinstall needed. If the repo path is later moved or deleted, the forwarder fails loudly with `repo not found at <path>. Re-run install-code-intel-pipeline.ps1` instead of silently running stale code.

### Verified

- ✅ `cargo build --release` succeeds (52 s first build, ~5 s incremental)
- ✅ Smoke test: binary starts, registers 3 functions + 3 HTTP triggers, attempts engine connection (engine not running locally — expected)
- ✅ Doctor: `repowise --version` reports v0.10.0, all 4 required tools (rg / git / repowise / sentrux) found

## v0.1.1 - 2026-05-30

Release infrastructure patch.

- GitHub Actions now exports the installed Code Intel tool bin directory through `GITHUB_PATH`, so later CI steps can find the Sentrux shim.
- CI smoke tests can explicitly allow the expected `graph_missing` manual step while still failing on local tool errors and Sentrux regressions.
- GitHub-hosted smoke tests skip the historical Sentrux baseline gate when running on the lite fallback, because lite metrics are not compatible with a real-core baseline.
- Release workflow is idempotent: if a GitHub Release already exists for a tag, it uploads or replaces the zip asset instead of failing.
- Release package avoids bundling local `pipeline.config.json`; it ships `pipeline.config.example.json` instead.

## v0.1.0 - 2026-05-30

Code Intel Pipeline 的第一个公开版本。

这一版把本地代码理解工具链整理成一条可重复的流程：刚从 GitHub clone 下来的项目，先摊成地图，再交给 Agent 动手。

- 便携安装器、doctor、自检脚本和一条命令入口。
- 串起 `rg`、Repowise、Understand Anything、Sentrux、CodeNexus-lite。
- 大仓库支持 scoped Repowise，避免根目录里的外部轮子污染判断。
- Governance 状态机输出 `hospital-report.json` 和 `surgery-plan.md`。
- Sentrux Agent 工具：`scan`、`health`、`session_start`、`session_end`、`rescan`、`check_rules`、`evolution`、`dsm`、`git_stats`、`test_gaps`、`what_if`。
- Sentrux lite fallback 和开源部署下的本地 auto-Pro 激活。
- Windows 下的 Sentrux V language 插件覆盖包。
- GitHub Actions Windows smoke test。
- 中文 README、GPT娘横幅、实际部署说明。

已知边界：

- Understand Anything 图谱生成仍依赖宿主 Agent skill。如果缺 `.understand-anything/knowledge-graph.json`，先运行 `/understand <repo> --language zh`，再重跑 pipeline。
