---
name: code-intel-pipeline
description: Install, validate, and run Code Intel Pipeline for local repository understanding, architecture analysis, structural regression gates, code indexing, hotspot diagnosis, and artifact-based handoff. Use when Codex needs to bootstrap Code Intel from a GitHub Release, check its dependencies, analyze a repository with rg/repowise/Understand/Sentrux providers, inspect pipeline health, or interpret Code Intel reports. Also use before implementing, refactoring, or fixing code in an analyzed repository, when a mid-edit question is what breaks if these files change or which tests should run, and when planning mechanical structural rewrites with a preview-only edit plan.
---

# Code Intel Pipeline

Use the released pipeline and its artifact contracts. Do not reconstruct its scanners inside the
skill.

## Resolve the installation

1. Check whether `code-intel --help` succeeds.
2. Reuse that installation when it is valid. Use `legacy/code-intel.ps1` only to repair a missing or
   invalid installation.
3. Bootstrap only when the user requested installation or the task explicitly requires the
   missing pipeline.
4. From this skill directory, inspect the latest published stable release plan:

```powershell
python scripts/bootstrap.py --repo-path "<repo-path>" --dry-run --json
```

5. Review the reported release tag, version source, asset URL, SHA-256 digest, destination, and
   target repository.
6. Install the verified stable release:

```powershell
python scripts/bootstrap.py --repo-path "<repo-path>" --json
```

The default follows GitHub's latest published stable release. Add `--version <tag>` only for a
requested version. Add `--channel prerelease` only when the user explicitly requests the latest
published prerelease. Add `--install-missing` only when the user authorizes installing third-party
dependencies. Never put provider keys in commands, repository files, artifacts, or Skill
resources.

The bootstrap script supports the currently published Windows release package. Stop with the
reported platform error on unsupported systems instead of substituting an unverified source
archive.

## Run the pipeline

Run the compiled Primary Operator Entry:

```powershell
code-intel "<repo-path>"
```

Use `--mode lite` for local-only core evidence. Use `--mode full` only when every optional provider
must be present. Do not call the legacy PowerShell pipeline.

The committed run directory is content-addressed: `run-complete.json` plus `objects/sha256/<hash>`
blobs. Report file names never exist there — `summary.md`, `understanding.md`, and `report.json`
are produced only by the legacy runner, and hospital output lives under artifact `type` identities
read through `artifact query --artifact-root <root> --repo <name>` (the run prints its publication
path as `<artifact-root>/<repo-name>/<run-id>`).

Read a published run in this order:

1. The command-line summary (add `--json` for the machine form): overall outcome, publication
   path, and the first failing node with its diagnostic.
2. `artifact query --type code_evidence.agent_slice` for ranked file and symbol navigation.
3. `artifact query --type diagnosis.hospital-view` for the governance read, or
   `--type diagnosis.hospital` when machine-readable detail matters.
4. `artifact query --type diagnosis.surgery-plan-view` when the hospital report selects
   `surgery_plan`.

The `diagnosis.*` artifacts exist only in `--mode` normal or full; `--mode lite` publishes core
evidence only (`code_evidence.*`, `inventory.files`, `repository.snapshot`, `doctor.observation`)
and no hospital report.

Report the publication path, outcome, first failing category, supporting evidence, and next
action. Do not describe a partial or domain-failed run as clean.

## Apply provider boundaries

Treat `rg`, Git, the native code evidence provider, and admitted Sentrux command evidence as exact
or governed evidence according to the report. Treat Repowise, external Understand graphs, and
other optional enrichments as unavailable or skipped when their provider is absent; do not turn an
optional provider outage into a core scanner failure.

Use these failure categories exactly:

- `provider_quota`
- `provider_unavailable`
- `config_error`
- `local_tool_error`
- `graph_missing`
- `sentrux_fail`

## Apply fallbacks

If a provider is unavailable, continue with admitted local evidence when the requested mode permits
it and label the missing evidence. If the stable release lacks a requested capability, report the
version boundary; do not install a prerelease or source checkout implicitly.

## Guard structural changes

Use the Sentrux session wrapper for an Agent coding session:

```powershell
& "$env:CODE_INTEL_HOME/legacy/Invoke-SentruxAgentTool.ps1" session_start "<scope-path>"
& "$env:CODE_INTEL_HOME/legacy/Invoke-SentruxAgentTool.ps1" session_end "<scope-path>"
```

Keep `.sentrux/rules.toml` separate from `.sentrux/baseline.json`. Rules define architecture
boundaries; baselines detect change. Never save a new baseline to hide a regression.

## While writing code

Run this loop whenever implementing, refactoring, or fixing code in an analyzed repository:

1. Start the regression baseline before the first edit:

```powershell
& "$env:CODE_INTEL_HOME/legacy/Invoke-SentruxAgentTool.ps1" session_start "<scope-path>"
```

2. Query blast radius and test candidates mid-edit:

```powershell
code-intel change impact --artifact-root <root> --repo <name> --repo-path <checkout> --changed <relative-path> --staleness advisory
```

`--staleness advisory` answers from the last committed run and never gates; use it for impacted
files and test selection while the working tree is dirty.

3. Preview a structural edit plan before any mechanical rewrite:

```powershell
code-intel capability exec edit.ast-grep-plan --request <request.json> --out <staging-dir>
```

The plan is preview-only (`repositoryMutation=false`); it never rewrites files.

4. Edit, run the selected tests, then close the gate:

```powershell
& "$env:CODE_INTEL_HOME/legacy/Invoke-SentruxAgentTool.ps1" session_end "<scope-path>"
```

`session_end` fails on structural regression. Verify it passes before reporting the change
complete.

## Reach for full-chain commands, not just the local loop

`session_start` / `session_end` and `change impact` above answer from the last committed run and never
trigger a fresh authoritative scan. Four more commands are CI-grade — visible only via
`code-intel --help --all`, not the default `--help`:

- `artifact query --artifact-root <root> --repo <name> --type <artifact-type>`: read a committed run's
  evidence directly. Prefer this over rerunning the pipeline when the answer is already committed.
- `run execute --repo <repo-root> --out <staging-dir> --authority-root <publication-root> --final-name <name>
  --manifest orchestration/integrations.json`: the authoritative full scan; the same command
  `ci.yml` / `release.yml` self-scan steps run.
- `change risk <revspec> --format json`: git-only PR defect-risk score, no index / network / LLM;
  powers `pr-gate.yml`. Run it from inside the target repository — it takes no `--repo` flag.
- `repin --write --json`: resync stale sha256 pins repository-wide in one pass.

See README's "全链路命令" section for the access-tier mental model (直查 / 跑管线 / 门禁) and one verified
invocation each.

## Route to reviewed assistance when the pipeline has no answer

The pipeline answers what changed, what it touches, what is risky, and which tests to run. It does
not read a diff for defects, judge type shape, hunt swallowed errors, propose an implementation
shape, or scan for vulnerabilities. Those blanks are filled by external agent assets bound by
reference in `orchestration/agent-assistance-catalog.v1.json`; nothing is vendored into this
repository.

Ask for candidates instead of picking one from memory — the catalog carries a fit, license,
security, integration, and reversibility rating decided once and committed:

```powershell
code-intel capability exec assistance.discovery --request <request.json> --out <staging-dir>
```

`options` takes `gap` (a `code-intel-engineering-capability-gap.v1` object) and `candidateIds`.
The result is `assistance.discovery` — dossiers only, `proposalOnly=true`, zero effects. It never
installs, adopts, or executes anything; the operator decides. An id that is not in the catalog is
refused rather than rated on the spot.

Route directly when the signal is unambiguous:

| Pipeline signal | Reach for |
|---|---|
| `change risk` reports high/critical `review_priority` and no one has read the diff | `code-review` |
| `change impact` returns an empty or partial test selection | `pr-test-analyzer` |
| `diagnosis.hospital-view` names a changed file whose finding is about error handling | `silent-failure-hunter` |
| The diff adds a type crossing a module boundary in `code_evidence.agent_slice` | `type-design-analyzer` |
| A touched file breaks the monolith rule (over 800 lines, or over 25 functions in over 400) | `code-simplifier` |
| A feature must land in a module `get_risk` marks a hotspot, with no shape proposed yet | `code-architect` |
| `diagnosis.hospital-view` selects `surgery_plan` for a module | `modernize-transform` |
| A module must be rewritten and no test pins its current behaviour | `modernize-extract-rules` |
| A runtime or framework major version is behind the target, same stack | `modernize-uplift` |
| A release needs a vulnerability read no `diagnosis.*` artifact provides | `claude-security` |

Bracket anything that writes — `code-simplifier`, `claude-security` patches — with the Sentrux
session gate above, and confirm `session_end` passes before reporting the change complete.

`doctor bootstrap` reports each candidate under `checks.assistancePlugins`, and the doctor node
publishes it as an `assistance:<candidate-id>` provider row. A missing candidate is an observation,
never a bootstrap failure: install it with the entry's `install.command`.

`claude-security` is the one candidate whose license is not Apache-2.0. It is a proprietary
Anthropic grant limited to internal use with Claude Code that forbids redistribution — the reason
it is bound by reference. Its dossier reports `license: review_required`; treat that as a decision
the operator still owes, not a cleared check.

## Load detailed contracts only when needed

Read these installed references from `CODE_INTEL_HOME` only for the named task:

- Artifact fields: `docs/artifact-data-contract.md`
- Goal normalization: `docs/agent-goal-intake.md`
- Harness decisions: `docs/harness-factory-reference.md`
- Skill quality gates: `docs/skill-development-benchmark.md`
- Implementation minimalism: `docs/implementation-minimalism-benchmark.md`
- Measured impact: `docs/ponytail-impact-scoreboard.md`
- Issue and domain intake: `docs/project-management-support.md` and `docs/agents/*.md`

When modifying this Skill in its source repository, run
`python tests/test_skill_package.py -v` and the official `quick_validate.py` before publishing.
