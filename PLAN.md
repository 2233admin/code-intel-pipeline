# Plan: Sentrux Failure Normalization Layer
_Locked via grill - by Codex + user_

## Goal
Make Sentrux check/gate failures in Code Intel Pipeline true, stable, and explainable for agents. This slice does not try to make `sentrux gate` pass. It stops diagnosis drift: if `sentrux check` reports `run-code-intel.ps1:Get-CodeEvidenceSymbols (cc=311)`, downstream artifacts must not silently choose a different primary target such as `Invoke-SentruxAgentTool.ps1:Get-ModuleBucket (cc=86)`.

## Approach
1. Add a minimal Sentrux failure parser/normalizer in `run-code-intel.ps1` that consumes existing `sentrux check` and `sentrux gate` step output.
2. Emit `sentrux-failures.json` with schema `code-intel-sentrux-failures.v1`.
3. Publish the artifact through normal discovery surfaces: `report.json`, `summary.md`, `understanding.md`, and `docs/artifact-data-contract.md`.
4. Treat `sentrux check` stdout as authoritative for `max_cc` rule failure existence and values when it names the offender. If stdout only reports an aggregate max-cc failure, emit `target.status = "unresolved"` instead of inventing a symbol from enrichment artifacts.
5. Treat `sentrux gate` stdout as authoritative for regression failures such as `Complex functions increased: 7 -> 11`. Model these as aggregate gate records unless a separate authoritative source maps them to concrete symbols.
6. Treat `sentrux-file-details.json` and `sentrux-hotspots.json` as enrichment-only sources. They may add candidate context but must not override check/gate primary failures.
7. If enrichment sources disagree with authoritative check/gate output, emit a `metric_conflict` with record ids, metric name, values, source names, provenance, raw source pointers, bounded stdout excerpts, parse timestamp, and resolution.
8. Update exact consumers that currently drift or publish mixed-source conclusions: `New-SentruxInsight`, `New-HospitalEvidenceBlock`, `New-CodeIntelHospitalReport`, `New-CodeIntelSurgeryPlan`, report assembly, summary markdown generation, understanding markdown generation, and GitHub research inputs.
9. GitHub research may consume normalized failures as context. If it also keeps raw failed-step text, that raw text must be labeled non-authoritative and must not select primary targets.
10. Add table-driven parser tests for named offender, aggregate-only max-cc, gate-only regression, baseline missing/manual-required gate, no-rules, skipped/not-run Sentrux, malformed stdout/parser-failure, and multi-offender stdout.
11. Add one integration assertion that no consumer chooses a hotspot primary target when normalized failure target is unresolved or when the authoritative check target conflicts with hotspot enrichment.
12. Record Ponytail impact only after tests prove normalized primary target and visible conflict behavior.

## Status Model
Artifact-level `status` values:
- `ok`: Sentrux ran and no check/gate failure was normalized.
- `failed`: at least one check/gate failure was normalized.
- `partial`: some failure information was parsed, but at least one source could not be fully parsed.
- `unparsed`: Sentrux output existed but no known parser matched it.
- `manual_required`: the pipeline required a manual Sentrux/baseline action.
- `skipped`: Sentrux was intentionally skipped.
- `not_run`: no Sentrux step was present.

Record-level `target.status` values:
- `resolved`: authoritative source named a concrete file/symbol target.
- `unresolved`: authoritative source reported a failure but did not name a concrete target.
- `aggregate`: failure is intentionally aggregate, such as gate regression counts.
- `not_applicable`: record type does not have a target.

`New-SentruxInsight` may keep legacy metric summaries for display, but it must not use enrichment summaries to drive primary target, next action, surgery plan, or admission reason when `sentrux-failures.json` has a check/gate authoritative record.

## Minimal Artifact Shape
```json
{
  "schema": "code-intel-sentrux-failures.v1",
  "status": "failed",
  "generatedAt": "2026-07-01T00:00:00.0000000+08:00",
  "primary": {
    "id": "check:max_cc:run-code-intel.ps1:Get-CodeEvidenceSymbols",
    "kind": "max_cc",
    "source": "sentrux check",
    "source_step": "sentrux check",
    "provenance": "stdout",
    "raw_output_path": "report.json#/steps/sentrux check/output",
    "stdout_excerpt": "run-code-intel.ps1:Get-CodeEvidenceSymbols (cc=311)",
    "parsed_at": "2026-07-01T00:00:00.0000000+08:00",
    "metric": "cyclomatic_complexity",
    "value": 311,
    "threshold": 70,
    "target": {
      "status": "resolved",
      "file": "run-code-intel.ps1",
      "symbol": "Get-CodeEvidenceSymbols"
    }
  },
  "gate": {
    "id": "gate:complex_functions_increased",
    "kind": "complex_functions_increased",
    "source": "sentrux gate",
    "source_step": "sentrux gate",
    "provenance": "stdout",
    "raw_output_path": "report.json#/steps/sentrux gate/output",
    "stdout_excerpt": "Complex functions increased: 7 -> 11",
    "parsed_at": "2026-07-01T00:00:00.0000000+08:00",
    "before": 7,
    "after": 11,
    "target": {
      "status": "aggregate"
    }
  },
  "conflicts": [
    {
      "kind": "metric_conflict",
      "authoritative_record_id": "check:max_cc:run-code-intel.ps1:Get-CodeEvidenceSymbols",
      "conflicting_record_id": "hotspots:max_cc:Invoke-SentruxAgentTool.ps1:Get-ModuleBucket",
      "metric": "cyclomatic_complexity",
      "authoritative_value": 311,
      "conflicting_value": 86,
      "authoritative_source": "sentrux check",
      "conflicting_source": "sentrux-hotspots",
      "raw_output_path": "sentrux-hotspots.json#/functions/0",
      "stdout_excerpt": "Get-ModuleBucket Invoke-SentruxAgentTool.ps1 (cc=86)",
      "parsed_at": "2026-07-01T00:00:00.0000000+08:00",
      "resolution": "primary target uses authoritative check/gate output"
    }
  ],
  "parser": {
    "status": "ok"
  }
}
```

This JSON sample must be validated with `ConvertFrom-Json` before implementation starts.

## Key Decisions & Tradeoffs
- Authoritative source precedence is fixed: `sentrux check` for check failures, `sentrux gate` for gate regressions, enrichment artifacts only for context.
- `sentrux-failures.json` is a formal artifact contract. It remains separate from `report.json` because hospital, surgery-plan, summary, and verifier lanes all need to reuse the same records. `report.json` publishes the path and compact summary.
- This slice deliberately does not refactor `Get-CodeEvidenceSymbols`, even though it is the reported complex function. That prevents mixing diagnosis normalization with complexity remediation.
- This slice does not change Sentrux thresholds, baseline, exclusions, or scan scope.
- Ponytail is used as an agent development guardrail, not as a runtime pipeline dependency.

## Risks / Open Questions
- Sentrux stdout format may vary across versions. Parser tests must cover named-offender and aggregate-only formats, and parser failures must produce `partial` or `unparsed` status with source pointers.
- Existing hospital/surgery-plan code has duplicated fallback logic. The implementation should redirect target selection to normalized failures while preserving enrichment display.
- The gate may still fail after this slice. That is acceptable if the failure is consistently represented and traceable.

## Out of Scope
- Passing `sentrux check` or `sentrux gate` by reducing complexity.
- Refactoring `Get-CodeEvidenceSymbols`.
- Changing `.sentrux/rules.toml`, `.sentrux/baseline.json`, or exclusion rules.
- Adding CCC, AST dump, semantic indexing, embedding, daemon, or new dependencies.
- Installing or calling Ponytail/ponly as a runtime tool.

## Ponytail Guardrails
- Prefer a small diagnostic contract over broad implementation changes.
- Reuse existing step outputs and artifacts; do not add a new scan step if existing stdout/artifacts are sufficient.
- Keep the diff budget limited to failure normalization, artifact wiring, tests, and documentation/ledger updates.
- Record measurable development impact in Ponytail ledger/scoreboard after validation.
