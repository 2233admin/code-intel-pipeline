# Changelog

All notable changes to **code-intel-pipeline** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] — 2026-07-23

### Added

- One manifest-bound normal-run spine for snapshot, doctor, inventory, native code evidence,
  architecture graph, real Sentrux `gate`/`check` observations, Hospital diagnosis, atomic
  publication, committed-only indexing, evidence query, freshness, and conservative change impact.
- Optional, research-stage Mindwalk trace normalization for privacy-reduced session review; it is
  advisory-only and absent from default scans.
- Representative benchmark gates for deterministic replay, artifact size, unresolved coverage, and
  unsupported-file coverage.

- One-command automatic draft-PR orchestration from exact proposal through structured user decision,
  C07 record/replay, and the existing fail-closed executor.
- Zero-effect proactive `/investigate` suggestions for actionable Pipeline failures,
  plus a branch-local user decision request before any automatic draft-PR path.
- Public-beta package verification, including clean extracted-ZIP smoke coverage
  and release checksums.
- Runtime/CI and file-boundary evidence providers, transactional artifact
  contracts, model request synthesis, executable handles, and compatibility
  retirement approval evidence.

### Changed

- Non-completed runs are retained as audit diagnostics and can never replace the latest completed
  authority. Domain-failed nodes retain their verified evidence without becoming authoritative.
- The native seven-language adapter is explicitly graded `candidate + structural`; semantic,
  behavioral, and production claims remain unsupported.
- Project license metadata, README, and root license text are now consistently MIT.

- Repowise semantic memory remains in the default orchestration plan but is now
  explicitly optional and non-blocking for the beta core.
- CodeNexus context remains an optional compatibility adapter; generated
  `work/` paths are excluded from repository evidence.
- Sentrux debt normalization treats an improving quality signal as
  informational while structural metric increases remain blocking.
- The stable wrapper resolves the packaged `bin/code-intel.exe` before any
  development-tree or Cargo fallback.

### Security

- Production model delegation uses synthesized requests and validated
  executable handles; legacy raw CLI execution is rejected by default.

### Known limits

- The public beta package is Windows-only.
- The incubated `crates/code-nexus-lite` Rust worker is not a shipped workspace
  binary; CodeNexus indexing is not a beta-core dependency.

## [0.3.0-beta.1] — 2026-07-16

Pre-release for the Rust-first Code Intel control plane. This build is intended
for integration testing before the `0.3.0` stable release.

### Added

- **Rust Sentrux DSM analysis kernel** — repository inventory, dependency
  structure, complexity, health, rules, gaps, and evolution analysis now run in
  the Rust CLI, with the PowerShell path retained as a compatibility fallback.
- **Atomic capability contract v1** — defines the execution envelope used to
  coordinate capability ownership, effects, dependencies, and artifacts.
- **Trust-boundary hardening** — Hospital and scoped Repowise paths fail closed
  at repository and artifact boundaries.

### Changed

- Rust DSM executable discovery is cross-platform and accepts an explicit
  `CODE_INTEL_RUST_CLI` override.
- File inventory is self-contained when Git metadata is absent, and symlinked
  directories are not followed during recursive traversal.
- Concurrent DSM integration fixtures are isolated to prevent cross-test
  interference on Windows.

### Verified

- Rust unit and integration suites pass locally.
- Windows package/build and Windows, Ubuntu, and macOS smoke jobs pass in CI.
- Rust and PowerShell DSM providers produce matching core repository and module
  metrics on the release candidate repository.

### Beta limitations

- GitHub Release packaging currently publishes a Windows ZIP only.
- The PowerShell DSM provider remains the automatic fallback when the Rust CLI
  cannot be located or executed.
- Complexity scoring is intentionally heuristic and may count keywords inside
  trailing comments; naive comment stripping was rejected because it corrupts
  strings and URLs.

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
