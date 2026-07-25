---
name: code-intel-pipeline
description: Install, validate, and run Code Intel Pipeline for local repository understanding, architecture analysis, structural regression gates, code indexing, hotspot diagnosis, and artifact-based handoff. Use when Codex needs to bootstrap Code Intel from a GitHub Release, check its dependencies, analyze a repository with rg/repowise/Understand/Sentrux providers, inspect pipeline health, or interpret Code Intel reports.
---

# Code Intel Pipeline

Use the released pipeline and its artifact contracts. Do not reconstruct its scanners inside the
skill.

## Resolve the installation

1. Check whether `CODE_INTEL_HOME` points to a directory containing
   `invoke-code-intel.ps1`.
2. Reuse that installation when it is valid.
3. Bootstrap only when the user requested installation or the task explicitly requires the
   missing pipeline.
4. From this skill directory, inspect the stable release plan:

```powershell
python scripts/bootstrap.py --repo-path "<repo-path>" --dry-run --json
```

5. Review the reported release tag, asset URL, SHA-256 digest, destination, and target repository.
6. Install the verified stable release:

```powershell
python scripts/bootstrap.py --repo-path "<repo-path>" --json
```

Add `--version <tag>` only for a requested version. Add `--channel prerelease` only when the user
explicitly requests a prerelease. Add `--install-missing` only when the user authorizes installing
third-party dependencies. Never put provider keys in commands, repository files, artifacts, or
Skill resources.

The bootstrap script supports the currently published Windows release package. Stop with the
reported platform error on unsupported systems instead of substituting an unverified source
archive.

## Run the pipeline

Run the doctor before analysis:

```powershell
& "$env:CODE_INTEL_HOME/check-code-intel-tools.ps1" -RepoPath "<repo-path>" -Json
```

Run the stable wrapper:

```powershell
& "$env:CODE_INTEL_HOME/invoke-code-intel.ps1" -RepoPath "<repo-path>" -Mode normal
```

Use `-Mode lite` for a cheap environment and inventory pass. Use `-Mode full` only when a fresh,
richer graph is required. Use the raw `run-code-intel.ps1` entry point only for a flag unavailable
on the stable wrapper.

Read generated artifacts in this order:

1. `summary.md`
2. `hospital.md`
3. `understanding.md`
4. `report.json` or `hospital-report.json` when a failure or machine-readable detail matters
5. `surgery-plan.md` when the hospital report selects `surgery_plan`

Report the artifact directory, outcome, first failing category, supporting evidence, and next
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
& "$env:CODE_INTEL_HOME/Invoke-SentruxAgentTool.ps1" session_start "<scope-path>"
& "$env:CODE_INTEL_HOME/Invoke-SentruxAgentTool.ps1" session_end "<scope-path>"
```

Keep `.sentrux/rules.toml` separate from `.sentrux/baseline.json`. Rules define architecture
boundaries; baselines detect change. Never save a new baseline to hide a regression.

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
