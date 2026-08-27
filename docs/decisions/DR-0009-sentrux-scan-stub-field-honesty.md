# DR-0009 sentrux.scan stub-field honesty and promotion

Status: active
Date: 2026-08-27

## Decision

`sentrux.scan`/`sentrux.rescan`'s built-in engine (`sentrux_gate.rs::metrics_json`)
must never present an unimplemented metric as a computed one. Where a metric
cannot honestly be computed, the field says so (`null` + a sibling
`<field>_status: "not_implemented"`) instead of a fabricated number.
Concretely (issue #375):

- `unresolved_imports` was a hardcoded `0` — inherited unchanged from the
  sentrux-lite PowerShell shim it replaced
  (`legacy/tools/sentrux-shim/sentrux-lite-core.ps1:248`, also `0`) — with no
  import-resolution pass anywhere in this repository's history to back it.
  It now reports `"unresolved_imports": null` plus
  `"unresolved_imports_status": "not_implemented"`. Building a real
  import-resolution pass here would duplicate #297's separate effort (which
  explicitly excludes Sentrux capabilities from its own scope), so real
  implementation is deliberately deferred to a follow-up issue rather than
  attempted as a second resolver inside this one (see #375 for the
  follow-up-issue instruction; a tracking issue should be filed the next time
  #297's scope is revisited, since duplicating unresolved import-tracking
  logic now would create two divergent resolvers).
- The `"0 inherit edges"` diagnostic log line the issue also named
  (`sentrux_gate.rs` `resolve_header`) is **not actually part of
  `sentrux.scan`/`sentrux.rescan`'s output at all** — `resolve_header` is
  only called by `run_check` and `run_gate` (the `check`/`gate` capabilities,
  both already `authoritative_automatic` and explicitly out of scope for
  #375). `scan_json` never calls `resolve_header`; it only calls
  `measure_project` + `metrics_json`. This corrects a premise in #375/#373:
  the "two hardcoded fields in scan's own output" claim collapses to one
  (`unresolved_imports`) once traced against the actual call graph.
  `resolve_header` (and its own hardcoded `"0 unresolved"` / `"0 inherit
  edges"`) is untouched here, matching the "do not touch check/gate/health"
  non-goal.

`sentrux.scan` and `sentrux.rescan`'s `currentState` in
`orchestration/sentrux-capability-matrix.v1.json` are promoted from
`automatic_degraded` to `authoritative_automatic`.

## Why

`sentrux_capabilities.rs::capability_audit` never executes `scan`/`rescan`
or inspects their actual output — it only reads the hand-declared
`currentState` string from the matrix JSON (confirmed by code reading, not
inference: `capability_audit` touches `currentState`, `executionMode`,
`artifacts`, `decisionConsumers`, `requiredForRelease` only). There is no
code-level linkage between "the output is honest" and `currentState` at
all — #373's own investigation already said as much. Promotion is therefore
an engineering judgment call, not a mechanically-derived one, and it must be
made and justified explicitly rather than left silently unresolved (per
#375's own instruction not to leave this undecided without a stated reason).

The `unresolved_imports` field was the only fabricated value actually
reachable in `scan_json`'s output; every other field in `metrics_json`
traces to a real `ProjectMetrics` computation shared with `check`/`gate`
(the same `measure_project` pipeline the module doc already advertises), and
this file's own prose comments are otherwise diligent about calling out every
approximation honestly (e.g. `branch_density_per_fn`'s "same number, honest
name" comment, the `cycle_count` vs. sentrux-lite's hardcoded zero). No
second hidden stub was found. `check`/`gate` — already `authoritative_automatic`
— carry the *same* `"0 unresolved"` / `"0 inherit edges"` hardcoded pattern
in their own diagnostic text (`resolve_header`) and were promoted anyway;
that is the operative precedent in this codebase for what
`authoritative_automatic` has actually required in practice (automatic
route + real artifact + wired decision consumer, all already true for
`scan`/`rescan` before this fix), not "zero hardcoded literals anywhere in
the engine." `scan`/`rescan`'s structured JSON is now at least as honest as
`check`/`gate`'s diagnostic text, and strictly more honest than before this
fix.

`sentrux.dsm` and `sentrux.what_if` remain `automatic_degraded` — out of
scope for #375 (see #376, #374). This decision does not claim or imply
anything about their promotion.

## Enforcement

`sentrux_gate.rs::tests::scan_json_reports_unresolved_imports_as_unmeasured_not_zero`
pins the `null` + `not_implemented` shape. `sentrux_capabilities.rs::tests`
and the capability-artifact/dag integration tests exercise `capability_audit`
against the real matrix file, so a future edit that regresses `currentState`
without updating this record's reasoning will not fail a test by itself
(nothing enforces that the record and the matrix state stay explained
together) — convention only; a session that flips `currentState` again for
`sentrux.scan`/`sentrux.rescan` should update or supersede this DR rather
than silently re-deciding.
