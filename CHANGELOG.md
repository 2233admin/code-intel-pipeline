# Changelog

All notable changes to **code-intel-pipeline** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] — 2026-07-02

The "understand any repo, cheaply" release. Docs generation now runs on any
LLM (MiniMax, local Ollama, custom OpenAI-compatible endpoints), the installed
toolchain self-heals, and the pipeline finishes with a three-stack workflow
recommendation telling you how to start working on the repo it just mapped.

### Added

- **Three-stack workflow recommender** — replaces the OpenSpec-only detector.
  Each pipeline run emits a `workflows` array in `report.json` (legacy
  `openSpec` block kept for compatibility) with layered, complementary
  verdicts: *matt-flow* (idea→ship: `/grill-with-docs`, `/to-prd`,
  `/to-issues`, `/triage`), *gstack* (delivery/quality: `/qa`,
  `/design-review`, `/ship`, `/canary`, `/review`), and *spec-driven*
  (picks OpenSpec OPSX for brownfield repos vs github/spec-kit for
  greenfield; detects `openspec/` / `.specify/` as already adopted).
- **Regression suite + fail-open lint** (`test-regression-fixes.ps1`) — 24
  cases locking down the fail-open/false-green fixes, plus an AST-based lint
  that flags `catch { return $true }` patterns across all `.ps1` files
  (`# lint-allow: fail-open` marker supported).
- **Self-healing repowise patch** — `install-code-intel-pipeline.ps1` now
  idempotently re-applies the ThinkingBlock fix to the installed repowise
  venv on every run (reasoning models behind Anthropic-compatible endpoints
  return thinking blocks first; upstream reads `content[0].text`). Survives
  `uv tool upgrade repowise`; documented in `overlays/repowise/README.md`.

### Changed

- **Docs LLM provider generalized: local models + custom APIs** — provider
  selected via `CODE_INTEL_PROVIDER` (default `anthropic`) with generic
  `CODE_INTEL_MODEL` / `CODE_INTEL_API_KEY` / `CODE_INTEL_BASE_URL`, reusing
  repowise's own provider registry. Keyless providers (ollama) work without
  credentials; `CODE_INTEL_ANTHROPIC_*` remains as backward-compatible
  fallback. Preflight covers anthropic / openai / ollama and runs on the
  repowise uv venv python (system-python dependency dropped).
- **Thin-forwarder install** — `Install-SentruxShim` generates forwarders
  into the user-local Code Intel bin directory instead of copying script bodies;
  repo edits take effect immediately via PATH, and a moved repo fails loudly.
- **Fail-closed hardening** — session_end no longer backfills baselines on
  zero parseable metrics; the surgery_plan→post_op guard evaluates real
  data; doctor survives malformed config JSON; overlay compare and global
  index refresh fail closed instead of open; baselines are backed up to
  `baseline.prev.json` before overwrite.
- **Detector accuracy** — code-size scan is now repo-root recursive (was a
  5-dir/7-extension whitelist that measured some repos as 1 file);
  repo age uses first-commit date (was last-commit, which judged every
  active old repo "greenfield"); multiple StrictMode crashes fixed.
- Local toolchain verified against **repowise 0.25** (upgraded from 0.21).

### Verified

- End-to-end on AIGX: 7/7 steps green, `workflows[3]` + legacy block emitted.
- Cold-start on an unfamiliar clone (fastapi/typer, 747 files): 15.7 s index,
  full understanding pack, and a sane three-stack verdict (108 contributors →
  PRD breakdown; deploy indicators → ship/canary; 2385-day brownfield →
  OpenSpec OPSX, score 65).
- Regression suite 24/24; provider preflight ok for MiniMax-M2.7, MiniMax-M3,
  and local Ollama; scoped docs generated 9-18 pages via MiniMax.

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

- `Invoke-SentruxAgentTool.ps1` — minor edits
- `templates/sentrux-rules.example.toml` — minor edits

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
