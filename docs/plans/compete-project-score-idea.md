# Optional Compete project score
> Status: IMPLEMENTED
> Created: 2026-07-17
> Source: https://github.com/lbj96347/compete

## Abstract
Add a non-blocking adapter that prepares a competitive-intelligence task for an Orca-managed Agent and normalizes a completed `compete` dataset into a small Code Intel score artifact. Keep market scoring separate from structural quality and hospital discharge decisions.

## Core Insight
`compete` already owns the six-axis scoring algorithm in `build_report.py`; Code Intel only needs to orchestrate it and label the result advisory.

## Target Repo
- Path: `D:/projects/_tools/code-intel-pipeline`
- Branch: current working branch
- Current state: dirty; use additive files and a narrow registry edit only

## Success Criteria
- [x] Doctor passes.
- [x] Preparation emits a machine-readable request and Agent prompt without network access.
- [x] A completed `compete` dataset normalizes to an advisory overall score plus six axes.
- [x] The adapter is optional and does not change `hospital-report.json` or default pipeline execution.
- [x] A focused self-test passes.

## Constraints
- Do not vendor `compete` or copy its scoring formulas.
- Do not add dependencies.
- Do not invoke paid/network Agent work without an explicit adapter action.
- Keep generated artifacts out of source control.

## Open Questions
1. Promote this into the default pipeline only after repeated reports prove useful.

## Implementation Notes
- Register the adapter under an optional market-intelligence stage.
- Reuse `compete.build_report` functions to calculate view-model scores.
- Orca remains an execution coordinator; Claude/Codex plus web tools perform research.
