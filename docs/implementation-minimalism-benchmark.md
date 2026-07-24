# Implementation Minimalism Benchmark

Implementation Minimalism Benchmark is Code Intel Pipeline's internal benchmark for implementation choices. It borrows the useful Ponytail principle: be lazy about the solution shape before coding, but never lazy about reading, evidence, safety, or verification.

It is a benchmark, not a runtime dependency. Do not install Ponytail to use Code Intel Pipeline. Do not wire Ponytail into the scanner, Agent Goal Intake, Harness Factory Reference, CI runtime, or artifact contract.

## Ladder

Before writing code, choose the first sufficient rung:

1. Do nothing: if the requested behavior already exists or the safest answer is documentation, configuration, or a runbook.
2. Reuse this repository: call an existing function, script, template, test helper, or documented workflow.
3. Use the standard library: prefer built-in language and shell capabilities over a new package.
4. Use platform native capability: prefer PowerShell, git, cargo, CI, OS, or host-agent features that are already required by this repo.
5. Use an already-installed dependency: reuse dependencies that are already part of the project contract.
6. Use a one-liner: when the behavior is genuinely a small expression or command and readability remains clear.
7. Write the smallest local implementation: add only the code needed for the verified behavior, with tests scaled to risk.

## Boundaries

Minimal implementation does not mean incomplete implementation. Do not remove or weaken:

- verification evidence, parser checks, tests, or CI gates
- error handling and clear failure categories
- security checks, secret handling, input validation, or supply-chain review
- accessibility requirements in user-facing surfaces
- data-loss prevention, idempotence, backups, and rollback paths
- artifact data contract fields or stable routing semantics
- documentation needed for handoff and future maintainers

If a lower rung would weaken one of these boundaries, move up the ladder to the smallest implementation that preserves it.

## Code Intel Application

This benchmark constrains implementation selection only. It does not produce artifact runs, change scanner output, replace Agent Goal Intake, replace Harness Factory Reference, or define the harness factory runtime.

Use it during Code Intel skill and pipeline changes to keep the repository thin: prefer deleting redundant logic, reusing existing tools, and keeping orchestration as shell over `rg`, Repowise, Understand Anything, Sentrux, and CodeNexus-lite.

Deferred cleanup gains are tracked in `docs/ponytail-gain-ledger.md`.
Measured impact is tracked in `docs/ponytail-impact-scoreboard.md`.

## Local Check

Run the skill benchmark gate after changing this document, `skills/code-intel-pipeline/SKILL.md`, or related ADRs:

```powershell
.\scripts/tests/test-skill-development-benchmark.ps1 -RepoPath .
```

The check verifies Code Intel internalized the implementation minimalism benchmark and its safety boundaries without making Ponytail a runtime dependency.
