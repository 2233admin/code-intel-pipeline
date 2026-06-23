# Ponytail Impact Scoreboard

This scoreboard records measured implementation-minimalism impact. It exists to prevent claims like "less code", "lower cost", or "faster" without before/after evidence.

Rule: measured impact only. If a value was not measured, record `not_measured`. Do not infer savings from a deletion plan, a debt list, or a passing quality gate.

## Current Score

| Metric | Before | After | Delta | Status | Evidence |
| --- | --- | --- | --- | --- | --- |
| code_removed_lines | not_measured | not_measured | not_measured | not_measured | No harvest pass has been applied yet. |
| files_removed | not_measured | not_measured | not_measured | not_measured | No deletion has been committed. |
| dependencies_removed | not_measured | not_measured | not_measured | not_measured | No dependency removal has been applied. |
| commands_removed | not_measured | not_measured | not_measured | not_measured | No command surface has been removed. |
| benchmark_before_seconds | not_measured | not_measured | not_measured | not_measured | No before/after timing benchmark has been run. |
| benchmark_after_seconds | not_measured | not_measured | not_measured | not_measured | No before/after timing benchmark has been run. |
| cost_before | not_measured | not_measured | not_measured | not_measured | No token, API, or tool-cost baseline has been captured. |
| cost_after | not_measured | not_measured | not_measured | not_measured | No token, API, or tool-cost baseline has been captured. |
| quality_gate_status | passed | passed | unchanged | measured | `test-skill-development-benchmark.ps1 -RepoPath .` passed after adding the benchmark docs. |

## Scoreboard Contract

- `code_removed_lines`: count only committed or staged line removals from an actual harvest pass.
- `files_removed`: count files deleted from the repository, not files proposed for deletion.
- `dependencies_removed`: count dependency declarations removed from package manifests or lockfiles.
- `commands_removed`: count public command entrypoints removed or collapsed.
- `benchmark_before_seconds` and `benchmark_after_seconds`: use the same command, mode, repo, and flags.
- `cost_before` and `cost_after`: include source of measurement, such as token usage, provider invoice, or CI minutes.
- `quality_gate_status`: record the exact gate command and result.

## Next Measurement

The first real impact measurement should come from one `harvest-next` item in `docs/ponytail-gain-ledger.md`. Record the baseline immediately before the deletion and the result immediately after verification.
