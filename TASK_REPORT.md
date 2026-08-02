# TASK_REPORT — issue #106: sentrux check/gate parity

## Divergence found

Two independent code paths existed for "Sentrux structural evidence":

- **Authoritative** (`code-intel run execute` → `evidence.sentrux` DAG node →
  `builtin_provider_evidence::sentrux_admission`, `crates/code-intel-cli/src/builtin_provider_evidence.rs:110-232`):
  runs **both** `sentrux_gate::run_check` (static `.sentrux/rules.toml`
  thresholds) **and** `sentrux_gate::run_gate(repo, false)` (baseline ratchet
  against `.sentrux/baseline.json`) as two independent authoritative rules
  (`"sentrux_check"` and `"sentrux_gate"`). Either one failing fails the node.

- **Standalone CLI** (`code-intel sentrux check <path>`,
  `crates/code-intel-cli/src/sentrux.rs:54` pre-fix): called **only**
  `sentrux_gate::run_check`. It never opened `.sentrux/baseline.json` and
  never ran the ratchet comparison at all.

`sentrux_gate::run_check` (`evaluate_rules`, `sentrux_gate.rs:481-528`)
evaluates absolute thresholds: `max_cc`, `no_god_files`, `max_cycles`,
`max_coupling`. `sentrux_gate::run_gate` (`sentrux_gate.rs:296-339`) evaluates
a completely different rule set: `quality_degraded`, `coupling_increased`,
`cycles_increased`, `god_files_increased` — regression against the saved
baseline. These are disjoint. This repository's own `.sentrux/rules.toml`
documents the resulting trap directly: `no_god_files = false` is set
deliberately, with the comment "god-file monotonicity is enforced as
monotonic non-increase ... by the no-degradation gate against
`.sentrux/baseline.json`" — a guarantee only `run_gate` was keeping. So a
tree that regressed god-file count or quality vs. baseline could report `All
rules passed` from `sentrux check` while `run execute` correctly failed with
`god_files_increased` / `quality_degraded` — a false green in exactly the
command the hospital/surgery-plan guidance text told agents to rerun as "the
smallest gate."

Note: at the time I verified this (2026-08-02, fresh worktree off
`origin/main`), the tree's actual numbers were `god_file_count 33→33`,
`quality_signal 3554→3640` (no active regression — see Parity proof below),
different from the `33→34` / `3554→3510` snapshot in the issue. The
*mechanism* bug (check never consults the baseline) is what's fixed here; it
is independent of which numbers happen to be in the tree at any given moment.

## Alignment chosen

`crates/code-intel-cli/src/sentrux_gate.rs`: added
`run_check_aligned(repo, ratchet: bool)`, composing `run_check` +
`run_gate(repo, false)`:
- `success = check.success && (gate.success || !gate.governed)` — an
  ungoverned gate (no baseline saved yet) does not fail the check, mirroring
  `command_rule`'s ungoverned-is-pass semantics in
  `builtin_provider_evidence.rs` so a never-baselined repo isn't punished.
- `ratchet=false` (CLI `--no-ratchet`) returns the plain static-only verdict
  with an explicit `"Ratchet comparison skipped (--no-ratchet)"` line in
  stdout — the pre-fix behavior stays reachable, but only on purpose and
  never silently.

`crates/code-intel-cli/src/sentrux.rs`: `"check"` now calls
`run_check_aligned(&repo, !options.no_ratchet)` (ratchet on by default).
`"check_rules"` still calls plain `run_check` unchanged — kept as the
static-only alias for parity with the legacy PS1 tool's own `check_rules`
operation (`docs/code-intel-architecture.md:117`), which is a distinct,
narrower, pre-existing concept from `check`.

`crates/code-intel-cli/src/main.rs`: added `--no-ratchet` switch (`Args`,
`set_switch_arg`, `cmd_sentrux`), plumbed through to `sentrux::Options`.
Updated `FULL_HELP_TEXT`.

**Deliberately NOT changed**: `builtin_provider_evidence::run_sentrux`'s
internal `"check" => sentrux_gate::run_check(repo)` call. That flow already
runs `run_gate` separately as its own `"sentrux_gate"` rule; composing the
ratchet into its `"check"` call too would just report the same violation
twice under two different rule kinds. Documented explicitly in
`run_check_aligned`'s doc comment so this doesn't read as a missed spot.

**Guidance text** (`hospital_diagnosis.rs:508`, "Rerun the smallest gate:
`code-intel sentrux --operation check --repo <repo-root>`.") — left
unchanged. It already names the command; the fix makes that command honest
by construction, so the text needed no edit. Verified via the existing test
`architecture_gate_failure_names_the_rule_targets_and_smallest_rerun_command`
(`tests/hospital_diagnosis.rs`), still green. Grepped the whole repo (not
just `*.rs`) for `"smallest gate"` and `"sentrux ... check"` — no other
active (non-archived) doc asserts the old static-only semantics;
`docs/code-intel-architecture.md:117` and `docs/integration-orchestration.md:109`
both describe the **legacy PS1 tool's** own `check_rules`/`session_start`
split, which this fix does not touch or contradict.

## Bonus (god_files_increased delta targets) — SKIPPED, not cheap

`.sentrux/baseline.json` stores only aggregate counts (`god_file_count: 33`),
no per-file list — confirmed by reading the file directly. Computing a real
"file(s) present now, absent at baseline" delta needs one of:
1. A baseline schema bump (v4→v5) to start recording the god-file path list.
   This file's own doc comment treats schema bumps as consequential ("Both
   moves make an older baseline describe a tree this engine cannot
   reproduce") — it invalidates every already-saved baseline repo-wide and
   forces a re-save. Too large for a bonus riding on the primary fix, in the
   repo's #1 bug-magnet file.
2. Reconstructing the baseline-time tree via `git show <sourceCommit>` /
   `git archive` and re-running `measure_project` against it at every gate
   invocation, to get an old file list to diff against. Meaningfully more
   code (git plumbing + temp-dir lifecycle) and roughly doubles the cost of
   every gate run; also fragile under shallow clones (CI checkouts).

Neither is cheap. Skipped per the task's own instruction to skip and say so.

## Regression test

`crates/code-intel-cli/src/sentrux_gate.rs`, test
`check_aligned_matches_gate_verdict_when_a_new_god_file_violates_the_ratchet`:
builds a fixture repo, saves a baseline, grows one file past the god-file
threshold (loc > 800) with `no_god_files = false` in rules.toml (mirroring
this repo's real config), then asserts:
- plain `run_check` still reports success (demonstrates the pre-fix false
  green directly, using the unchanged pre-fix function).
- `run_gate` fails with `god_files_increased`.
- `run_check_aligned(&root, true)` fails with the **same rule set** as
  `run_gate` (`aligned_rules == gate_rules`) — the parity property #106 asks
  for.
- `run_check_aligned(&root, false)` (`--no-ratchet`) passes and its stdout
  contains "skipped".

Also added `parse_args_reads_sentrux_no_ratchet_switch` (main.rs) for the CLI
flag plumbing, and asserted `!args.no_ratchet` in the existing
`parse_args_preserves_sentrux_positional_operation_and_repo`.

**Negative verification**: wrote the test calling `run_check_aligned` before
implementing the function (TDD). Compiling at that point failed cleanly with
exactly `error[E0425]: cannot find function 'run_check_aligned' in this
scope` at both call sites — no other errors — proving the test genuinely
depends on the fix. (Considered `git stash` per the task's suggested
mechanism, but the new test and new function live in the same file/diff, so
isolating one via stash means either losing the test too or hand-splitting
the patch; the TDD red-state above is the same proof without that fragility.
The parity proof below already independently exercises the real tree
post-fix, and comparison against pre-fix behavior is additionally embedded
*inside* the permanent test itself via the `static_only.success` assertion
against the unchanged `run_check`.)

## Parity proof (post-fix, real tree)

`code-intel sentrux check .`:
```
-- .sentrux/rules.toml --
[resolve] 1161 resolved, 0 unresolved
[build_graphs] 258 files | 1161 import, 53881 call, 0 inherit edges
[coupling_basis] 163 of 258 files in import-modelled languages
All rules passed - Quality: 3640
-- .sentrux/baseline.json ratchet --
[resolve] 1161 resolved, 0 unresolved
[build_graphs] 258 files | 1161 import, 53881 call, 0 inherit edges
[coupling_basis] 163 of 258 files in import-modelled languages
Quality: 3554 -> 3640
Coupling: 74.48 -> 71.23
Cycles: 0 -> 0
God files: 33 -> 33
No degradation detected
```
Exit 0.

Authoritative self-scan (`run execute`), `evidence.sentrux/sentrux-command-observation.json`:
```
gate:  exitCode 0, success true, "Quality: 3554 -> 3640 ... No degradation detected"
check: exitCode 0, success true, "All rules passed - Quality: 3640"
```
Exit 0 overall, `evidence.sentrux` node `status: succeeded, verdict: pass`.

Quality (3640), coupling (71.23), god files (33) and the pass/fail verdict
are identical between the standalone `sentrux check .` and the authoritative
self-scan's internal `gate`+`check` commands — the parity issue #106 asks
for.

## Gates

- `export CODE_INTEL_HOME="$(pwd)"`: set in every build/test/scan shell
  invocation (env does not persist across separate tool calls in this
  harness).
- `cargo build -p code-intel --release --locked`: green (`target/release/code-intel.exe` rebuilt).
- `cargo test -p code-intel --locked`: **all green**, 0 failed (~2843 tests
  across ~49 targets; includes both `#[path]`-duplicated copies of
  `sentrux_gate.rs`'s test module).
- `cargo fmt --check`: clean (one auto-fixed multi-line `let` in the new
  test, applied via `cargo fmt`, then reverified clean).
- Clippy on touched files: `sentrux.rs` zero warnings. `main.rs` zero
  warnings in touched regions (all flagged lines are pre-existing, outside
  my edits — verified by line number against the actual diff). `sentrux_gate.rs`:
  `run_check_aligned` triggers one "function never used" warning, but only
  in the `capability_inventory::builtin_provider_evidence::sentrux_gate`
  `#[path]`-duplicate copy of this file (that copy never calls it — by
  design, see "Deliberately NOT changed" above). This is the exact same
  pre-existing pattern already present for two untouched sibling functions,
  `scan_json` and `metrics_json`, in that same duplicate copy — confirmed by
  reading the clippy output directly. Not a new category of problem.
- Self-scan exit 0, parity proof: see above.
- No new god files: `god_file_count` unchanged at 33 (baseline and current,
  both pre- and post-fix).
- `repin`: found 10 stale sha256 pin sites across
  `orchestration/integrations.json` and
  `orchestration/internalization/sentrux.json` (caused by editing
  `sentrux.rs`/`sentrux_gate.rs`; this also fixed a real test failure,
  `ticket_r03_sentrux_record_blocks_shim_retirement_on_windows_and_plugin_gaps`
  in `tests/internalization_record.rs`, which recomputes and pins these
  exact file digests). Resynced via `./target/release/code-intel.exe repin
  --write`; re-verified 0 findings after write. Diff is those two JSON files
  only, mechanically rewritten, not hand-edited.

## Environment note (unrelated to the fix, for whoever runs this next)

This sandbox hit repeated, unrelated build/link instability while I worked:
a transient rustc ICE, an OOM-class rustc abort under full `-j16`
parallelism (only ~5.5GB free of 31GB RAM), and one corrupted `.pdb` for the
`understanding_quadrant` test binary (`LNK1285`) left over from an earlier
crash, fixed by deleting that binary's stale `.pdb`/`.exe`/`.d`. Also:
`rtk`'s cargo-build error summary silently reported "0 errors" once when the
real cargo output said "could not compile" — the reported test/build results
above are all taken from actual exit codes and raw log content (via `rtk
proxy` and/or direct log reads), not from `rtk`'s summarized counts. Settled
on `CARGO_INCREMENTAL=0` with reduced `--jobs` for the first recovery pass,
then plain incremental + `--jobs 4` once the corrupted caches were cleared,
which was stable for the rest of the session.
