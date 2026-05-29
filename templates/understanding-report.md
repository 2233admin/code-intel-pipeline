# Understanding Report

## Key Assumptions
- The local toolchain is the source of operational truth for this run.
- `rg` gives exact inventory, `repowise` gives semantic memory, Understand Anything gives graph context, and Sentrux gives structural regression signals.

## Verified
- Doctor status:
- Pipeline status:
- Artifact path:

## Unverified Or Inferred
- Understand graph freshness:
- Provider quota state:
- Sentrux baseline intent:
- Sentrux structural deltas:
- CodeNexus follow-up hints:

## Failure Categories
- provider_quota:
- local_tool_error:
- graph_missing:
- sentrux_fail:

## Human Inspection Required
- Read `summary.md`.
- If `graph_missing > 0`, run the emitted `/understand` command in Claude.
- If `sentrux_fail > 0`, inspect the listed complexity or regression output before saving a new baseline.
- If `provider_quota > 0`, retry after quota resets; do not debug local tools first.

## Next Action
State the single next action a teammate should take.
