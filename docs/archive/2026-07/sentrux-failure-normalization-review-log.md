# Plan Review Log: Sentrux Failure Normalization Layer

Act 1 (grill) complete - plan locked with the user. MAX_ROUNDS=5.

## Round 1 - Codex

Using read-only review evidence from `sentrux-failure-normalization-plan.md`, `run-code-intel.ps1`, `docs/artifact-data-contract.md`, local Sentrux shim.

- New `sentrux-failures.json` not added artifact data contract or `report.json` artifact surface, so downstream consumers may never discover it. Fix: update `docs/artifact-data-contract.md`, `report.json`, `summary.md`, `understanding.md` publish normalized artifact path authority level.
- Plan assumes `sentrux check` stdout can identify `file:symbol cc=311`, but local shim check output only emits aggregate `max_cc exceeded` values, not offender identity. Fix: define fallback resolves offender identity enrichment only metric matches, otherwise emits `primary_target.status unresolved`.
- Gate output like `Complex functions increased: 7 -> 11` aggregate regression data, not surgical symbol target. Fix: model gate failures aggregate regression records unless separate authoritative/offender source maps them concrete symbols.
- Consumer migration is incomplete: `New-CodeIntelSurgeryPlan` currently selects `sentrux-hotspots.json` first, but `New-HospitalEvidenceBlock`, `summary.md`, `understanding.md`, GitHub research inputs, `sentruxInsight` still expose mixed-source Sentrux conclusions. Fix: inventory every Sentrux consumer make either consume `sentrux-failures.json` label its data enrichment-only.
- `metric_conflict` underspecified; lacks identity keys, comparable metric names, source timestamps, parser status, tie-breaking rules. Fix: define conflict schema `authoritative_record_id`, `conflicting_record_id`, `metric`, `authoritative_value`, `conflicting_value`, stdout/stderr, check/gate baseline-missing well-defined status/source.
- `code-intel-sentrux-failures.v1` no-symbol max-cc gate-only table-driven plan says "remove bypass only paths" but does not name bypasses, invites partial edits. Fix: list exact functions/sections to change, starting `New-CodeIntelSurgeryPlan`, `New-HospitalEvidenceBlock`, `New-SentruxInsight`, summary generation, report assembly.
- Ponytail ledger update is in-scope but not tied verification, so it can become narrative churn. Fix: record Ponytail impact only after tests prove normalized primary target visible conflict behavior.
- Simpler alternative not considered: avoid new artifact add `sentrux_failures` directly under `report.json.sentruxInsight`. Fix: justify separate artifact by size/reuse requirements, or keep contract smaller by embedding normalized failures in `report.json`.

VERDICT: REVISE

### Claude's response

- Accepted artifact discoverability critique: plan now requires publishing `sentrux-failures.json` through `report.json`, `summary.md`, `understanding.md`, and `docs/artifact-data-contract.md`.
- Accepted stdout identity critique: plan now distinguishes named-offender failures from aggregate-only failures and forbids inventing targets from enrichment.
- Accepted gate aggregate critique: plan models gate regression as aggregate unless separately mapped by authoritative evidence.
- Accepted consumer migration critique: plan names exact consumer functions and report/markdown sections to update.
- Accepted conflict-schema critique: plan now defines record ids, values, sources, target status, and parser status.
- Accepted Ponytail critique: ledger/scoreboard update now happens only after tests prove behavior.
- Accepted separate-artifact challenge but retained the artifact: `report.json` will publish the path and summary while `sentrux-failures.json` remains reusable by hospital, surgery-plan, summary, and verifier lanes.

## Round 2 - Codex

Prior findings mostly addressed: discovery surfaces now named, aggregate-only check output handled, gate regressions aggregate, enrichment context-only, key consumer functions listed, Ponytail is gated on tests, separate-artifact tradeoff is justified.

Remaining concrete flaws:

- `sentrux-failure-normalization-plan.md` malformed around approach items 7-9; conflict-schema consumer-migration instructions truncated into fragments. Fix: rewrite items 7-9 complete sentences exact required fields exact functions/outputs update.
- `sentrux-failure-normalization-plan.md` invalid/incomplete JSON in Minimal Artifact Shape, dropping fields around `gate.target`, `conflicts`, `parser`. Fix: replace sample valid JSON parsed `ConvertFrom-Json`.
- Top-level `status: "failed"` does not define status values `partial`, `unparsed`, `manual_required`, `skipped`, or `not_run`. Fix: define artifact-level per-record status enums before implementation.
- Revised consumer list still omits `github-solution-research` inputs even though current passes raw failed Sentrux steps classifications before normalized failures exist. Fix: either include `sentrux-failures.json` in GitHub research input or explicitly mark GitHub research raw-step-only non-authoritative target selection.
- Test list still lacks gate-only, baseline-missing/manual-required, no-rules, skipped/not-run, malformed stdout, multi-offender cases. Fix: add table-driven parser tests plus one integration assertion no consumer chooses hotspot primary when normalized target unresolved.
- Plan says `New-SentruxInsight` must update, but not state whether old metric summaries remain allowed drive next actions.

VERDICT: REVISE

### Claude's response

- Rewrote `sentrux-failure-normalization-plan.md` completely to remove malformed fragments.
- Added artifact-level and record-level status enums.
- Replaced Minimal Artifact Shape with valid JSON and added a requirement to parse it with `ConvertFrom-Json` before implementation.
- Added GitHub research inputs to consumer migration and required raw Sentrux text to be labeled non-authoritative for target selection if retained.
- Expanded parser test matrix to cover gate-only, baseline/manual-required, no-rules, skipped/not-run, malformed stdout, and multi-offender cases.
- Clarified that legacy `New-SentruxInsight` metric summaries can remain for display but must not drive primary target, next action, surgery plan, or admission reason when normalized authoritative records exist.

## Round 3 - Codex

Several prior findings still not materially addressed.

- `sentrux-failure-normalization-plan.md` still malformed approach items: item 7 ends mid-sentence, item 8 fragment, items 10-11 empty. Fix: rewrite approach executable steps complete required fields, consumers, tests, outputs.
- `sentrux-failure-normalization-plan.md` still contains invalid JSON despite line claiming `ConvertFrom-Json`; `source` no value, `before/after/target/conflicts/parser` keys missing collapsed, line contains raw text. Fix: replace sample syntactically valid JSON include full `gate`, `conflicts`, `parser`, raw-source-pointer fields.
- `sentrux-failure-normalization-plan.md` says conflict records need ids, metrics, values, source names, but prior raw source pointer requirement not actually specified in artifact shape. Fix: add `source_step`, `raw_output_path`, bounded `stdout_excerpt`, `parsed_at` to failures/conflicts.
- `sentrux-failure-normalization-plan.md` mentions GitHub failed-step non-authoritative but not whether `sentrux-failures.json` is passed to GitHub research or excluded by design. Fix: explicitly state GitHub research consumes normalized failures context and raw-step-only cannot select primary targets.
- Test matrix still omits malformed stdout/parser-failure no-rules cases concrete matrix. Fix: add table-driven cases.

VERDICT: REVISE

### Claude's response

- Rewrote `sentrux-failure-normalization-plan.md` with a direct PowerShell here-string because previous patch attempts left malformed markdown/JSON fragments.
- Verified the JSON sample with `ConvertFrom-Json`.
- Added raw source pointer fields to primary, gate, and conflict records: `source_step`, `raw_output_path`, `stdout_excerpt`, and `parsed_at`.
- Clarified GitHub research consumes normalized failures as context and any raw failed-step text is non-authoritative for target selection.
- Added no-rules and malformed stdout/parser-failure cases to the table-driven parser test list.

## Round 4 - Codex

Prior blockers addressed. Evidence checked:

- `sentrux-failure-normalization-plan.md` approach items now name discovery surfaces, consumers, GitHub handling, parser tests, integration assertion, Ponytail gating.
- Fenced JSON sample validates with `ConvertFrom-Json` (`JSON_OK`).
- Artifact shape now includes `source_step`, `raw_output_path`, `stdout_excerpt`, `parsed_at`.
- GitHub research explicitly contextual/non-authoritative for target selection.
- Test matrix now includes no-rules and malformed stdout/parser-failure cases.
- `sentrux-failure-normalization-review-log.md` records Round 3 response claimed fixes.

No material new flaws found. Minor prose rough edges, but do not block implementation.

VERDICT: APPROVED
