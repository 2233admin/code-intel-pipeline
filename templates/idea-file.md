# Idea File
> Status: DRAFT
> Created: YYYY-MM-DD
> Source: local code-intel-pipeline

## Abstract
Describe what should be understood, changed, or validated in 2-3 sentences.

## Core Insight
State the one non-obvious observation that makes this worth doing.

## Target Repo
- Path:
- Branch:
- Current state:

## Success Criteria
- [ ] Doctor passes.
- [ ] Pipeline emits `summary.md`, `report.json`, and `understanding.md`.
- [ ] Failure categories are explained if nonzero.
- [ ] A human can identify the next action from the artifact.

## Constraints
- Do not add dependencies without explicit approval.
- Do not rewrite unrelated code.
- Keep generated artifacts out of source control unless intentionally promoted.

## Open Questions
1.
2.

## Implementation Notes
- Start with `install-code-intel-pipeline.ps1 -RepoPath <repo>`.
- Run `invoke-code-intel.ps1 -RepoPath <repo> -Mode lite` before normal mode.
- Read `understanding.md` before making project changes.
