# Changelog

All notable changes to **code-intel-pipeline** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.0-beta.2] — 2026-07-30

### Changed — action required

- **Every PowerShell entry point moved under `archive/`.** Installation still
  runs through PowerShell — the compiled `code-intel` CLI cannot install the
  binary that provides it — so the documented command moves with it:

  ```powershell
  pwsh ./archive/install-code-intel-pipeline.ps1 -RepoPath <repo> -InstallMissing
  ```

  The same shift applies to `code-intel.ps1`, `check-code-intel-tools.ps1`,
  `run-code-intel.ps1`, the contract test suites under
  `archive/scripts/tests/`, and every `$env:CODE_INTEL_HOME/*.ps1` invocation
  the bundled skill documents. Release archives carry the same layout, so a
  packaged `code-intel.ps1` now lives at `archive/code-intel.ps1`.

  Nothing was deleted: this is a relocation, not the entry-point retirement
  tracked in the PS1-exit campaign. The 132 relocated files keep their
  relative structure, so script-to-script references are unchanged.

- The skill bootstrap installs both layouts. `find_payload_root` probes
  `archive/` first and falls back to the payload root, so releases published
  before this one stay installable.

### Fixed

- Path resolution in the relocated scripts, wherever a single root variable
  had been carrying two meanings — "where the PowerShell lives" and "where the
  repository is". Those diverged under `archive/`, and the split had to be made
  explicit in the doctor (its graph-provider probe and `CODE_INTEL_HOME`
  default), the installer (binary install, integrations manifest, vlang
  overlay, bundled skill, reported root), the beta packager and its verifier,
  the orchestrator's manifest validation, the compatibility retirement
  packets, and the archived contract suites.
- `cargo test --release` no longer fails on the inventory fault-injection
  contract test. The `CODE_INTEL_TEST_RG_EXTRA_PATH` hook it drives is
  deliberately `#[cfg(debug_assertions)]`, so the shipped binary carries no
  inventory fault injection; the test is now gated on the same cfg. CI only ran
  the debug profile, so the failure had never surfaced there.
- Removed a stale duplicate of the `resume` contract left in `main.rs` when that
  logic moved to `artifacts.rs`: the `ResumeSummary` struct, four JSON helpers,
  `next_read`, and a verbatim copy of two contract tests that
  `artifacts_tests.rs` already owns. `cmd_resume` has delegated to
  `artifacts::resume` throughout, so nothing was serving the dead copy.
- Dropped three unused imports and moved `MAX_JSON_DEPTH` to the only test that
  uses it, instead of re-exporting it crate-wide.
- `skill:codex` and `skill:claude` verify whose skill occupies the path instead
  of only that some `SKILL.md` exists there. Agent hosts share those directories
  with other skill managers, so an unrelated manager's junction served a stale
  skill while the installer reported `OK skill:claude` on every run. A drifted
  path is now reported, and `-RepairSkillLinks` moves the previous occupant
  aside — unlinking a reparse point, renaming a real directory — rather than
  deleting it.
- Installing the bundled skill no longer copies `__pycache__` / `*.pyc` into an
  agent host's skill directory. One stray `bootstrap.cpython-313.pyc` left by a
  local `bootstrap.py` run made the byte-parity check report `skill:source`
  outdated permanently.
- The `repowise-thinking-patch` overlay distinguishes "obsolete" from "broken".
  Upstream repowise 0.32.0 walks `response.content` and skips non-text blocks
  itself, so a healthy machine reported `install_failed` on every install run.
  The installer now reports `not_needed` for the upstream-fixed shape and keeps
  `install_failed` for a genuinely unrecognised layout.

## [0.7.0-beta.1] — 2026-07-28

This release moves Code Intel into the write path and makes the official
release installable on Windows, macOS, and Linux. It also closes the security,
audit, and release-gate findings discovered by the repository's own audit and
adversarial verification passes.

### Added

- Write-time workflow support: the Skill now triggers for implementation,
  refactoring, and fixes; `change impact --staleness advisory` provides
  explicitly non-gating guidance while a working tree is changing.
- Deterministic, non-mutating `edit.ast-grep-plan` previews, backed by an
  internalized and CI-pinned ast-grep toolchain.
- Official Windows, macOS, and Linux ZIP assets with matching SHA-256
  sidecars, manifests, provenance attestations, and platform-aware Skill
  bootstrap installation.
- A PowerShell-versus-Rust parity observation harness and a fast structural
  cycle regression test for the self-dogfood release gate.

### Changed

- macOS/Linux installs now persist `PATH` and `CODE_INTEL_HOME` through a
  generated POSIX environment file, install the integrations manifest beside
  the binary, and report platform-correct doctor guidance.
- Release publishing is split from unprivileged platform build jobs. Only the
  final publisher receives release and attestation permissions, and it
  validates the complete nine-asset inventory before publishing.
- Audit rendering, CI, release self-scan, and packaged-payload validation now
  share the same fail-closed audit validation path.

### Fixed

- Repository snapshots and inventory exclude nested linked worktree markers
  instead of treating them as ordinary files.
- `run commit` preserves the caller's manifest bytes and digest when artifact
  references do not change.
- A 29-defect hardening pass repaired Git/path handling, CJK path support,
  capability and routing dead paths, baseline metric validation, index
  replacement safety, CI gate coverage, and documentation drift.
- Tool discovery now resolves executables absolutely, internalization evidence
  binds the relevant digests, and the release/self-scan snapshot identity
  checks remain enforced.

### Security

- Audit department prompts define an explicit untrusted-content boundary;
  model-process output is bounded; adversarial audit fixtures cover fabricated
  evidence, reversed ranges, and false coverage claims.
- GitHub Actions expressions are removed from executable script bodies,
  actions and package inputs are pinned, Cargo release commands use
  `--locked`, and release assets can no longer be silently clobbered.

## [0.6.0] — 2026-07-26

This release adds the **audit layer**: audit dimensions run as hospital
departments over the modality evidence the pipeline already gathers, and emit
findings under one fail-closed contract. Three departments ship enabled
(`security`, `ai-safety`, `supply-chain`), reports render as Markdown or as a
self-contained HTML document, and an audit can be scoped to a git diff for
pull-request review.

The layer was pointed at this repository before shipping. It found the
unpinned toolchain and the mutable CI action tags fixed below; the git config
hardening came out of the same pass. Methodology adapted from
[Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain)
(MIT).

### Fixed

- Git no longer runs inside a scanned repository with that repository's own
  program-executing config keys live. `core.fsmonitor` names a program Git
  executes on ordinary read commands like `git status`, so a repository that
  arrived with its `.git` intact (an archive, a backup, a copied directory)
  got command execution before any gate looked at it. New
  `crates/code-intel-cli/src/hardened_git.rs` pins `core.fsmonitor`,
  `core.hooksPath`, `core.sshCommand`, `diff.external`, and `core.pager` empty
  via `-c` and sets `GIT_CONFIG_NOSYSTEM`; all five production call sites and
  both `archive/run-code-intel.ps1` invocations route through it. The pipeline already
  stripped `RIPGREP_CONFIG_PATH` at every ripgrep call site — this applies the
  same standard to the tool that can start a process.
- Audit evidence is now grounded in the tree it cites.
  `AuditReport::validate()` is filesystem-free, so a `confirmed` finding's
  evidence `path` was only checked for being non-empty — a department is an
  agent, so a fabricated or drifted citation validated green. New
  `validate_evidence_grounding(repo_root)`, run by `audit --operation validate
  --repo <root>`, resolves every `file` evidence entry under the repository,
  requires it to exist, and requires any line range to be ordered and within
  that file.
- The department registry no longer accepts path strings that escape the
  repository. It is read from `--repo` — a scanned repository's own file — but
  its rubric and prompt paths were joined onto the repo root and only
  existence-checked, so an absolute path replaced the base and `..` escaped
  it, letting a target repo satisfy the kernel's fail-closed existence
  invariant with host files and redirect a department's `prompt` (the
  instruction source an audit agent reads). Both path classes now parse under
  the portable repo-relative contract `artifact_ref.rs` already enforces.
- `archive/Invoke-WorkflowRecommendation.ps1 -Json` pins stdout to UTF-8 on Windows.
  pwsh encodes redirected stdout with the system codepage, so on a zh-CN host
  the proposal arrived as GBK bytes and the capability adapter's parse failed
  with `invalid unicode code point`; CI runners are UTF-8, so the suite only
  failed on real zh hosts.

### Changed

- Pinned the Rust toolchain in `rust-toolchain.toml` (1.95.0, `rustfmt`,
  minimal profile). CI and release jobs now install from that file instead of
  running `rustup default stable`, and the release manifest records the
  resolved `rustc --version`. Released binaries were previously built by
  whatever `stable` happened to be current, so the shipped `.sha256` proved
  transport integrity but nobody could rebuild a tag's bytes to check them.

### Added

- Audit report HTML rendering and incremental (diff-scoped) audits.
  `code-intel audit --operation render --report <path> --format html` prints
  one self-contained HTML document — inline styles only, no external CSS/JS/
  fonts/images or other network reference, generated directly from the
  parsed, validated `AuditReport` model so there is no placeholder-
  substitution failure mode for a separate linter to catch; `--format
  markdown` stays the default and existing invocations are unchanged.
  Separately, the report contract gains an optional top-level `scope` block
  (`{"kind": "full" | "diff", "since", "files"}`); a new fail-closed rule in
  `validate()` requires a `"diff"` scope's `since`/`files` to be present and
  bounds every finding's file evidence to a path in `scope.files` (normalising
  `\`/`/` before comparing) — a finding outside the declared diff is a
  contract violation. `code-intel audit --operation scope --repo <root>
  --since <git-ref>` computes that file set (`git diff --name-only
  <since>...HEAD`, filtered to files still on disk) and prints a
  ready-to-embed scope block. See `docs/audit-report.md`.
- Audit departments `ai-safety` (T3) and `supply-chain` (T4): prompts under
  `orchestration/audit/prompts/`, adapted from
  [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain)
  (MIT) and rewritten to run on pipeline modality evidence. `ai-safety` gates
  on an AI/LLM surface being present and reports `not_assessed` with evidence
  when there is none; `supply-chain` parses manifests, lockfiles, and CI
  workflow permissions as structured facts before judging. Both are flipped to
  `enabled: true` in `orchestration/audit/departments.v1.json`; the kernel and
  the `code-intel audit` CLI needed no change.

- Audit kernel (T1): a shared contract for audit departments that run as
  hospital departments over existing modality evidence. Adds the
  `code-intel-audit-report.v1` schema, `orchestration/audit/rubrics/`
  (severity, confidence, evidence, coverage, scoring — adapted from
  [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain),
  MIT), and `crates/code-intel-cli/src/audit_report.rs` with a fail-closed
  `validate()` and a `departments.v1.json` registry loader. `validate()` is
  registry-authoritative: a report's `departments` must exactly match the
  registered department ids, and each department run's `status` must agree
  with the registry's `enabled` flag. `hospital-report.json`
  gains an optional `audit` summary block and `hospital.md` gains an optional
  `## Audit` section; both are additive and render nothing when no audit ran.
  See `docs/audit-report.md`.
- Security audit department (T2): `orchestration/audit/prompts/security.md`
  (adapted from
  [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain),
  MIT) and `orchestration/audit/departments.v1.json`'s `security` entry flips
  to `enabled: true`; `ai-safety` and `supply-chain` landed after it as T3/T4
  (see the entry above). New
  `code-intel audit --operation validate|render` CLI parses a report,
  validates it against the department registry (`--operation validate
  --repo <root> --report <path>`), or prints its `## Audit` markdown section
  (`--operation render --report <path>`, no registry needed). The audit
  report is now a first-class artifact
  (`code-intel-audit-report.v1` / `diagnosis.audit`) in `artifact_ref.rs`.

## [0.5.1-beta.1] — 2026-07-25

### Added

- A deterministic paired tool-effectiveness scorer for frozen `C0`, `C1`, and
  `Cfull` Agent runs, with externally attested outcomes, exact experiment
  profile matching, and atomic benchmark reports.
- ADRs that freeze canonical EntityRef identity and the first real-world
  dogfood evaluation baseline before expanding provider-specific ablations.
- Built-in `sentrux-native` structural gate engine (`code-intel sentrux --operation
  scan|health|check|gate|save_baseline`): deterministic metrics, structured
  violations with target files, and real resolved-import cycle detection for
  Rust modules (sentrux-lite hardcoded `cycle_count = 0`). The `evidence.sentrux`
  DAG node now runs this engine in-process by default; an external Sentrux is
  used only when `options.toolPathPrefix` is given.
- Baseline v2 (`code-intel-sentrux-baseline.v2`) carries engine identity,
  version, source commit, and scope. The gate fail-closes on engine mismatch
  instead of comparing numbers produced by different engines and scales.
- Structural rules may carry `details.violations` (rule, message, targets)
  through the adapter/admission chain; the Hospital report and surgery plan now
  name the first failing rule, its target files, and the smallest rerun
  command, and plan a bounded surgery whenever an actionable target exists.
- Execution results expose a `failures` block that separates process failures
  from domain failures so a tooling error can no longer visually swallow an
  architecture-gate verdict.
- CI and the release workflow gain a release-blocking authoritative self-scan
  (`run execute` on the exact candidate tree, no skip flags), landed in the
  companion `ci(release)` workflow commit on this branch.
- Stable release zips are packaged from `git archive HEAD`, ship with a
  `.sha256` and a `release-manifest.json` (tag, commit, zip digest), and the
  packaged binary must pass its own `check` and `gate` against the packaged
  payload before publication (same companion workflow commit).

### Fixed

- Removed the `dag_run` <-> `execution_kernel` module cycle by moving the run
  CLI front-end to `run_cli.rs` and `RunError` to `run_error.rs`, and the
  previously undetected `artifact_ref` <-> `capability` cycle by extracting the
  shared content-contract primitives into `content_contract.rs`.
- Kernel runs no longer process-fail outside wrapper environments: the doctor
  adapter falls back to a native tool observation when the PowerShell bootstrap
  is unavailable or nonconforming, and the built-in gate engine satisfies the
  Sentrux requirement when no external Sentrux is installed (a present but
  nonconforming external overlay still fails conformance on purpose).
- Repository inventory no longer treats the `.git` pointer file of a linked
  git worktree as an unexplained extra path.

## [0.4.0] — 2026-07-24

### Added

- One authoritative typed execution kernel that owns DAG execution, structured outcomes,
  atomic publication, and the stable process exit contract.
- Default, strict, offline, and compatibility execution profiles compiled into one immutable
  policy, with optional provider unavailability represented explicitly.

### Changed

- The PowerShell entrypoint is now a thin adapter over `run execute` and validates the
  versioned execution-result schema before returning.
- The internal graph remains authoritative by default; Sentrux acceleration and external
  enrichment stay optional, while strict mode requires every enabled provider.

## [0.3.0] — 2026-07-24

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
- An installable Codex Skill whose stable bootstrap is pinned to the verified
  `v0.3.0` GitHub Release and validates the published SHA-256 digest before
  extraction.

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
- PowerShell contract tests now live under `scripts/tests/`; the seven public
  compatibility entry points remain at the repository root.

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
- **Self-healing repowise patch** — `archive/install-code-intel-pipeline.ps1` now
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

- **`crates/code-nexus-lite/`** — Rust + iii worker binary, wraps Repowise + Sentrux for Agent-friendly code-understanding context. 5.2 MB stripped + LTO. Cross-platform replacement for the Windows-only `archive/Invoke-CodeNexusLite.ps1`. Apache-2.0 license (matches iii SDK).
  - 3 iii functions: `codenexus::scan` / `codenexus::lite` / `codenexus::doctor`
  - 3 HTTP triggers: `POST /scan` / `POST /lite` / `POST /doctor`
  - Depends on `iii-sdk = "0.11"` (crates.io, Apache-2.0) + `repowise` 0.10.0 (Python) + `sqlite3` CLI
  - See `crates/code-nexus-lite/README.md` for the full design

- **`.github/workflows/skill-check.yml`** — PR-time quality gate. Runs a heuristic 8-dim darwin-skill scoring on every changed SKILL.md, validates YAML frontmatter, checks for broken internal links. Threshold 70/100 to pass. Triggers on PRs that touch `crates/code-nexus-lite/`, `.claude/skills/`, or `skills/`.

- **`.gitignore` updates** — Added `target/` (Rust build artifacts), IDE files (`.idea/`, `.vscode/`), OS files (`.DS_Store`, `Thumbs.db`), PowerShell artifacts (`*.ps1.xml`).

- **`crates/code-nexus-lite/.gitignore`** — Same as above, scoped to the sub-crate.

### Changed

- `archive/Invoke-SentruxAgentTool.ps1` — minor edits
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
