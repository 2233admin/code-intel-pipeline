# Idea File
> Status: COMPLETE
> Created: 2026-07-15
> Source: local code-intel-pipeline

## Abstract
Provide one language-neutral acceptance entry point for agent, landing, and promotion decisions. Reuse `Test-CodeIntelProjectConformance.ps1` and its policy so each stage selects an existing project profile instead of copying suite logic.

## Core Insight
The three stages differ by required evidence strength, not by language: agent work needs explicit targeted checks plus the fast project profile, landing needs fast project conformance, and promotion needs full project conformance.

## Target Repo
- Path: `D:\projects\_tools\code-intel-pipeline`
- Branch: `codex/code-intel-atomic-model`
- Current state: dirty shared workspace with concurrent agent changes; this lane owns only new acceptance-layer files.

## Success Criteria
- [x] `Invoke-CodeIntelAcceptance.ps1 -Stage agent|land|promote` maps stages to targeted+fast, fast, and full respectively.
- [x] The entry point delegates project suites to `Test-CodeIntelProjectConformance.ps1` without duplicating suite implementations.
- [x] Machine output uses only `pass`, `fail`, `blocked`, or `skipped-with-reason` outcome statuses and fails closed.
- [x] Agent targeted checks are explicit argv arrays, repository-contained, and reject shell syntax or path escape.
- [x] Self-contained fixtures prove stage mapping, failure propagation, injection resistance, and repository paths containing spaces.
- [x] Doctor, lite baseline, targeted tests, real fast acceptance, and Sentrux session verification pass; final normal pipeline verification is delegated to the integrating Agent.

## Constraints
- Do not add dependencies.
- Do not modify Merge Queue files, `orchestration/integrations.json`, CI, or Rust hotspot code.
- Do not rewrite unrelated code or overwrite concurrent user/agent changes.
- Keep generated artifacts outside source control.
- Promotion remains an acceptance decision only; this entry point performs no push, merge, or publication.

## Open Questions
1. None blocking: targeted checks will be supplied as JSON argv arrays to preserve language neutrality and avoid shell evaluation.

## Implementation Notes
- Use an acceptance policy to declare stage/profile mapping and targeted-command safety limits.
- Invoke child processes with PowerShell call-operator argument arrays; never use `Invoke-Expression`, `cmd /c`, or `powershell -Command`.
- Start with `install-code-intel-pipeline.ps1 -RepoPath <repo>`, then doctor/lite/normal as required by the Code Intel skill.
