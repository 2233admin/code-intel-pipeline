# DR-0011 Sentrux Quality Signal computation kernel: formula version, redundancy gap, baseline schema break

Status: active
Date: 2026-08-28

## Decision

Issue #385 (child of #285) replaces `sentrux_gate.rs`'s `10000 - penalty`
proxy score with an upstream-compatible Quality Signal: the geometric mean
of five normalized root causes (modularity, acyclicity, depth, equality,
redundancy), ported into a new self-contained `sentrux_quality_signal.rs`
kernel from the upstream docs (<https://sentrux.dev/docs/quality-signal/>)
and the pinned upstream source commit
`6f8ff3c14b0423e4b58f42d1813d4d5f7fdc1d11`
(`sentrux-core/src/metrics/root_causes.rs`). Four decisions this record
exists to make explicit, per #385's own instruction that any place this
engine's numbers diverge from the reference source or from a literal
five-factor computation must be versioned, not silently reproduced or
silently dropped:

**1. The pinned source's `max(0.01)` per-factor floor is followed, not the
docs page's bare formula.** The docs page states
`quality_signal = (a*b*c*d*e)^(1/5) * 10000`. The pinned source instead
computes `values.iter().map(|v| v.max(0.01)).product()` before taking the
fifth root — one factor collapsing to exactly `0.0` cannot zero out the
whole signal. `sentrux_quality_signal::normalize_and_aggregate` implements
the pinned-source behavior (the reference #385 names for computation), and
`FORMULA_VERSION` (`"sentrux-upstream@6f8ff3c14b0423e4b58f42d1813d4d5f7fdc1d11+max-floor-0.01"`)
records exactly which of the two documented behaviors this engine
implements, exposed in every `scan`/`health` payload.

**2. `equality` uses upstream's own documented LOC fallback, not a new
approximation.** Upstream's `equality` factor is Gini of per-function
cyclomatic complexity, and upstream's own `compute_complexity_gini`
"falls back to per-file line counts if no CC data available." This
repository's scanner has never tracked per-function complexity (only a
per-*file* `branch_density_per_fn` approximation — see that field's own
comment in `sentrux_gate.rs`'s `metrics_json`: "A true max needs
per-function body ranges, which no scanner in this repository has"), so
`equality` always takes upstream's documented fallback path. `root_causes.equality`
carries `"basis": "file_loc_fallback"` and `"completeness": "full"` — this
is not a degraded computation, it is upstream's own normal path for
engines without function-level data.

**3. `redundancy` implements the `duplicate` half only; `dead` is honestly
absent, not fabricated as `0`.** Upstream's redundancy raw value is
`(dead_count + duplicate_count) / total_functions`. This engine computes
`duplicate_count` from whole-file byte-for-byte duplication after
`strip_comments_and_strings` + whitespace normalization (files below
`MIN_DUPLICATE_CONTENT_LEN` = 40 normalized bytes are excluded, to stop
near-empty boilerplate files from forming false-positive duplicate groups).
`dead_count` (reachability-based dead-function detection) is **not
implemented**. This repository's own `AGENTS.md` already documents why a
naive text-heuristic dead-code signal would be actively misleading here:
`cargo check`'s ~100 dead-code warnings on a clean tree are a compiler
artifact of `#[path]` module re-inclusion (one source file compiled once
per including module, each instantiation warning about the slice it does
not use), not real removable debt — and a name-based "never called"
heuristic would misclassify every `pub` API function, trait impl, test
function, and framework entry point as dead, producing a number that
actively misleads rather than one that is merely incomplete. Shipping a
number likely to be wrong is worse than shipping an honestly-partial one.
`root_causes.redundancy` carries `"completeness": "partial"` and a `note`
field naming the gap explicitly, following the same "`null`/honest-status
over a fabricated number" pattern DR-0009 already established for
`unresolved_imports`.

**4. `BASELINE_SCHEMA` bumps v5 → v6 (`ENGINE_VERSION` 2.2.0 → 3.0.0), not
just an engine-version bump.** `quality_signal`'s *meaning* changed, not
just its computation: a v5 baseline's `quality_signal` number was never a
Quality Signal at all, so comparing it against this engine's new output
would produce a fabricated before/after delta (v3-class silent-regression
risk `sentrux_gate.rs`'s own top-of-file comment already names as "worse
than refusing"). The existing generic mismatch check
(`baseline["schema"] != BASELINE_SCHEMA`) already fails this closed with
`baseline_engine_mismatch` and a machine-readable reason — no new
enforcement mechanism was needed, only the version bump itself. This
repository's own `.sentrux/baseline.json` was re-saved against the new
engine as part of this change: not to suppress a regression (nothing in
the tree changed structurally), but the direct, intended consequence of a
schema bump this repository's own convention (`Re-baseline intentionally
with: code-intel sentrux --operation save_baseline`) already prescribes.

One implementation detail beyond upstream's own scope, needed because this
repository's file-dependency graphs are not guaranteed acyclic (upstream's
own depth spec assumes a DAG via "iterative longest-path DFS from entry
points"): `max_depth` is computed over the graph after collapsing every
detected cycle (reusing this file's existing Tarjan SCC output,
`strongly_connected_cycles`) to a single representative node, so depth
stays well-defined and terminates on real, possibly-cyclic repositories
instead of only on upstream's assumed acyclic input.

`module_of` (this engine's community assignment for Newman's Q) is a
directory-based heuristic, independent of `sentrux_analysis.rs`'s own
`module_name()` (DR-0010, a different capability) — not required to match
it. It does converge on the same base-case shape DR-0010 arrived at for
the same reason: a bare "first path segment" partition collapses every
file in a single-crate `crates/<name>/src/**` repository (this one
included) into one module, making Q trivially near-zero for the exact
repositories this signal most needs to say something useful about, so
`crates/<name>/src|app|tests/<file-or-subdir>` gets one extra level of
granularity.

## Why

Issue #385's own text requires this exact kind of explicit versioning: "文档公式与固定源码版本中各因子 `max(0.01)` 的差异必须显式版本化，不能静默改写" and "未测量项以 honest unknown/completeness 表达，不伪造 0". Absent this record, a future session reading `sentrux_quality_signal.rs` in isolation could "fix" the floor to match the docs page (silently changing every repository's score), "fix" redundancy by adding a naive dead-function heuristic (reproducing the exact misleading-signal failure mode `AGENTS.md` already warns about for this repository), or wonder why `equality`'s basis is LOC instead of CC without knowing that is upstream's own defined behavior, not a gap. Each of those would be a plausible, well-intentioned regression this record forecloses by naming the decision and its reasoning once, in the place `docs/decisions/README.md` says agents must check first.

## Enforcement

- `sentrux_quality_signal.rs`'s own `#[cfg(test)] mod tests` pins: one
  golden fixture per root cause where it is the sole bottleneck
  (`modularity_is_the_sole_bottleneck_when_it_is_the_only_weak_factor` and
  its four siblings), the deterministic bottleneck tie-break order
  (`bottleneck_tie_break_is_deterministic_priority_order`), the `max(0.01)`
  floor (`max_floor_prevents_one_zero_factor_from_zeroing_the_whole_signal`),
  zero/empty-input safety
  (`full_aggregate_never_produces_nan_or_inf_on_empty_input`,
  `zero_and_single_element_gini_is_zero_not_nan`,
  `zero_functions_redundancy_ratio_is_zero_not_nan`), the pinned-source
  fixture (`root_cause_scores_match_pinned_upstream_fixture`, same inputs
  as upstream's own `root_cause_scores_normalize` test), and the
  `crates/<name>/...` module granularity
  (`module_of_gives_crates_paths_one_more_level_of_granularity`).
- `sentrux_gate.rs::tests::scan_json_exposes_the_full_quality_signal_v1_contract`
  pins the full `scan`/`health` JSON contract, including
  `root_causes.redundancy.completeness == "partial"` and its `note` text —
  a future change that quietly flips this back to `"full"` without
  implementing real dead-function detection fails this test.
- `sentrux_gate.rs::tests::empty_repository_quality_signal_has_no_nan_and_reports_trivial_defaults`
  and `::quality_signal_bottleneck_names_the_true_lowest_root_cause_on_a_real_cycle`
  pin the end-to-end pipeline (file scan → resolved multi-language edges →
  SCC/depth/modularity → aggregate), not just the pure-math kernel.
- `sentrux_gate.rs::tests::gate_rejects_a_v5_schema_baseline_from_before_the_quality_signal_rewrite`
  pins that a v5-schema baseline fails closed under the v6 engine with a
  message naming the required schema (machine-readable rejection reason).
- `crates/code-intel-cli/tests/sentrux_gate_cli.rs::save_baseline_records_the_v6_god_file_identity_list`
  pins the same v6 schema through the shipped CLI surface, the mechanism
  the authoritative self-scan and CI actually invoke.
