# Skill Development Benchmark

Skill Development Benchmark is the quality bar for Code Intel skills.

`yao-meta-skill` is a strong external reference for this layer because it treats skills as reusable engineered assets instead of prompt snippets. It emphasizes semantic contracts, trigger evaluation, output evidence, portability, release gates, failure libraries, and ongoing SkillOps.

## What To Borrow

- lean `SKILL.md` entrypoint with longer references kept outside the hot path
- explicit trigger surface and near-neighbor exclusions
- neutral skill metadata that can compile or adapt to multiple agent hosts
- eval fixtures for trigger behavior, output quality, and regressions
- failure library for known anti-patterns
- review reports that separate current evidence from aspirational claims
- release gates for package verification, install simulation, and claim guard
- adoption drift or feedback loop for deciding the next skill patch

## Boundary

Do not make `yao-meta-skill` a runtime dependency of Code Intel Pipeline.

This benchmark guides how we develop `skill/SKILL.md` and future Code Intel skills. It does not produce artifact runs, does not change scanner output, and does not replace Agent Goal Intake or Harness Factory Reference.

## Code Intel Application

When Code Intel skills become team-facing assets, require:

- clear trigger descriptions and non-trigger examples
- small main skill file plus linked references
- local validation commands
- failure cases for route confusion and unsafe scope expansion
- documentation for artifact data contract usage
- evidence that the skill works across Codex and other target hosts when claimed

Use this benchmark before promoting a local skill into a shared or published package.

## Local Check

Run the lightweight benchmark contract check before changing or publishing `skill/SKILL.md`:

```powershell
.\test-skill-development-benchmark.ps1 -RepoPath .
```

This check does not run `yao-meta-skill`. It verifies that Code Intel has internalized the benchmark boundaries and required quality surfaces.
