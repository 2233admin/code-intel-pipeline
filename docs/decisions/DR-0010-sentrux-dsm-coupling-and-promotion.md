# DR-0010 sentrux.dsm coupling gap: real engine fix, note honesty, and promotion

Status: active
Date: 2026-08-28

## Decision

Issue #376 (child of #373, builds on #148's E1 root-cause investigation) posed
three decisions. All three are resolved here, in the same PR:

**1. `dsm_edges`'s language/granularity gap is fixed as real engine work**, not
as a scope-narrowing of what "authoritative" means for `sentrux.dsm`:

- `module_name()` gets one more level of granularity for any
  `crates/<name>/...` or `packages/<name>/...` path rooted under
  `src`/`app`/`tests`: the module is that immediate subdirectory (or the
  file itself, when it sits directly under the root with no subdirectory) —
  mirroring the pre-existing `backend/<app|src|tests>/<next>` arm exactly.
  A crate/package file sitting outside a `src`/`app`/`tests` root (e.g.
  `crates/app/build.rs`) still falls back to the coarse `crates/<name>`
  bucket. This alone makes single-crate repositories (this one included)
  stop being structurally incapable of reporting their own coupling —
  `dsm_edges` excludes same-module edges by construction, and before this
  fix every file in this repo's one crate collapsed into one module.
- `resolve_module_token` (the `use <crate>::...` cross-crate resolver) is
  fixed to match a `use`/`import` token's dependency-crate name against the
  crate-name path *segment* first (`["crates"|"packages", name, ...]`),
  falling back to the pre-existing trailing-leaf match — needed because
  finer module buckets no longer necessarily have the crate name as their
  own trailing path segment.
- `dsm_edges`'s extension gate grows from `.py`/`.rs`/`.v` to also include
  `.ps1`/`.psm1`. A new `powershell_targets` resolver (plus
  `parse_ps1_path_expr`/`split_ps1_call_args`/`leading_ps1_expr`/
  `parse_ps1_string_literal`/`powershell_identifier` helpers) resolves the
  two dependency idioms actually used anywhere in this repository's own
  scripts: dot-sourcing (`. (Join-Path $PSScriptRoot "Foo.ps1")` /
  `. "Foo.ps1"`) and `Import-Module` of either a literal path or a variable
  assigned earlier in the same file from the same path-building forms
  (`$platformModule = Join-Path (Join-Path $PSScriptRoot "tools")
  "code-intel-platform.psm1"` then `Import-Module $platformModule`). This is
  a bounded heuristic text extractor over PowerShell 5.1's two-positional-
  argument `Join-Path` convention (the only shape observed in this repo's
  own scripts) — not a PowerShell parser. Bare module names, string
  concatenation, named `-Path`/`-ChildPath` parameters, and any other
  computed expression are left unresolved rather than guessed.

This was the correct branch of decision 1 (real engine work, not
"rescope to inter-crate-only and rewrite the note to match a smaller
promise") because the structural bug was fixable with bounded, well-scoped
changes — module bucketing and one more import resolver, both following
patterns this file already used elsewhere — not a rewrite of `dsm`'s whole
analysis model.

**2. The engine's self-description is corrected, not left aspirational.**
`sentrux_analysis.rs::analyze`'s `"note"` field previously read *"Lightweight
DSM with 9 color modes. Git-derived modes depend on local git history; use
Sentrux/CodeNexus for authoritative graph detail."* The "use Sentrux/CodeNexus
for authoritative graph detail" clause is not true in production: an external
Sentrux tool's `toolPathPrefix` is only ever supplied by tests/the V-lang
overlay (`sentrux_gate.rs:6-7`), and #337's CodeNexus-lite port is
unconfirmed to even produce a module dependency graph (per #376's own
non-goals, this PR does not assume otherwise). Advertising a fallback that is
never actually reachable from a production run is exactly the kind of
misleading self-description this repo's own `unresolved_imports` precedent
(DR-0009) treats as something to fix, not carry forward. The note is
rewritten to describe what the engine actually does now: heuristic
text-based import detection across Python, Rust, V, and PowerShell, honest
about not resolving dynamic imports/aliases/re-exports, with a pointer to
this record. This PR does **not** commit to wiring an external Sentrux tool
or the CodeNexus port — that remains an open, unassumed option per #376's own
non-goals; only the misleading language is corrected.

(The legacy PS1 orchestrator's own `dsm` implementation,
`legacy/Invoke-SentruxAgentTool.ps1:2392`, carries the identical old note
text in its own separate code path. It is intentionally left untouched here:
this repo's rules forbid adding new PowerShell or new product behavior to
`.ps1` files, and rewording a string literal inside one is still an edit to
a `.ps1` file. That legacy engine's self-description is a pre-existing,
separate artifact outside this issue's scope.)

**3. `sentrux.dsm`'s `currentState` is promoted from `automatic_degraded` to
`authoritative_automatic`.** Unlike `sentrux.evolution`/`sentrux.what_if`
(DR-0008/#374), `sentrux.dsm` was never routed through a "lite" fallback —
`uses_lite_fallback` (`sentrux_capability_artifacts.rs:587-591`) never listed
`"dsm"`, and its route is `RouteKind::Command` calling
`sentrux_analysis::analyze` directly, the real engine, end to end, both
before and after this fix. Its `automatic_degraded` state traced entirely to
the structural edges-empty bug (#148 E1) fixed above, not to a
wiring-to-fallback problem. `capability_audit`
(`sentrux_capabilities.rs::capability_audit`) only ever trusts the
hand-declared `currentState` string — it does not execute `dsm` or inspect
its output (same mechanism DR-0009 already traced for `scan`/`rescan`) — so
promotion here, like there, is an engineering judgment call about whether the
underlying engine now genuinely justifies the label, not a mechanically
derived one.

## Why

`dsm_edges`'s own construction (`target != from`, excluding same-module
edges) turns "coarse module bucketing" into "structurally zero coupling
signal," not just "less precise" signal — for any repository whose source
lives in one crate (this one) or is written substantially in a previously
wholly-unparsed language (this repo's PowerShell layer). That is a
correctness bug in the engine's own terms (#148 E1's root cause, "verified
still true today" per #376's issue text), not a cosmetic gap, and
`decisionConsumers` for `sentrux.dsm` already include `change_impact` and
`test_selection` — consumers that plausibly want real coupling/blast-radius
signal, per #373's own text. Fixing the structural bug directly, rather than
redefining "authoritative" down to "inter-crate/inter-package only" (the
issue's other offered option), keeps the capability's promise intact instead
of shrinking it to match a bug.

For the promotion call: the operative precedent this codebase has already
established (DR-0009, `scan`/`rescan`) is "automatic route + real artifact +
wired decision consumer + no fabricated field" — not "zero heuristics
anywhere in the engine" (which no capability in this matrix could ever
satisfy; `check`/`gate`, already `authoritative_automatic`, carry their own
hardcoded diagnostic-text approximations per DR-0009). By that same bar,
`sentrux.dsm` now qualifies: nothing in its output is fabricated, the route
was already the real engine, and the one thing that made it categorically
non-functional for a large class of repositories (this one included) is
fixed. The remaining imprecision — a heuristic text parser that cannot
resolve dynamic imports, aliases, or re-exports — is the same character of
approximation this repo already accepts elsewhere in `dsm_edges`'s
`.py`/`.rs`/`.v` resolvers (which predate this fix and were never grounds by
themselves for `automatic_degraded`); it is now honestly disclosed in the
rewritten `"note"` field rather than silently assumed away.

`sentrux.evolution` is untouched by this record — DR-0008 already gave it
its own reasoning and it stays `automatic_degraded`, tracked by its own
follow-up issue.

## #148 disposition

Per #376's decision 3 (and #376's own recommendation), #148's only remaining
open item was E1 (this record's decision 1 fixes it in full; C2 was already
fixed by #152, closed). The PR that lands this record links `Fixes #148` so
it closes automatically alongside `Closes #376` — #148 is kept as the
historical dogfood-audit record its findings originated from, per #376's own
non-goal not to re-open C2.

## Enforcement

- `sentrux_analysis.rs::tests::dsm_single_crate_intra_crate_coupling_is_no_longer_structurally_empty`
  pins the finer module-bucketing fix for single-crate repos (module
  granularity + non-empty edges + correct `inbound_edges`/`outbound_edges`).
- `sentrux_analysis.rs::tests::dsm_dependency_graph_accumulates_imports_and_resolves_workspace_crates`
  pins the same fix for the pre-existing multi-crate cross-module case
  (`resolve_module_token`'s crate-segment-first fix).
- `sentrux_analysis.rs::tests::dsm_powershell_dot_source_and_import_module_edges_are_now_modeled`
  pins PowerShell dot-source and `Import-Module`-via-variable edge
  resolution end to end.
- `sentrux_capabilities.rs::tests::dsm_is_declared_authoritative_automatic`
  pins the promotion the same way DR-0009's
  `scan_and_rescan_are_declared_authoritative_automatic` pins `scan`/
  `rescan`'s: `capability_audit` never re-derives `currentState` from actual
  output, so nothing else in this codebase protects the promotion decision
  from a silent revert — a future edit that flips it back should update or
  supersede this record rather than just the assertion.
