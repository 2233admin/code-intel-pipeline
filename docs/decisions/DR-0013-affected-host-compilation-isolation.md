# DR-0013 affected-host compilation isolation

Status: active
Date: 2026-09-30

## Decision

The development host recorded in issue #403 (Windows, `Win11ProW X64`, the
machine that owns this repository's checkout) is an **isolation-excluded
host**. While it is excluded:

- Do not run `cargo build`, `cargo test`, `cargo check`, `cargo clippy`,
  `cargo run`, or `cargo nextest` at the workspace root. The repository's
  verification policy (the user-level agent instructions file, outside this
  checkout) requires `cargo test --workspace --no-fail-fast` after a change;
  that requirement is **suspended on this host** and must be satisfied on
  another host or in CI.
- Do not run `cargo clean` or otherwise mutate `target/`.
- Any repair whose acceptance depends on running the suite executes on a
  different host, and its evidence is the CI run, not this machine.
- Read-only work is unaffected: source reading, `gh` queries, `git` queries,
  and static file measurement are allowed.

The exclusion is lifted only by a new decision record that cites the
authorization the #403 report says it is waiting on. It is not lifted by
passing tests, by elapsed time, or by the absence of new crashes.

## Why

Issue #403 was filed 2026-09-29 and states the constraint directly: "Do not
reproduce, build, or run the full test suite on the affected Windows host."
This host is that machine. A session on 2026-09-30 started
`cargo test --workspace --no-fail-fast` here and had to be terminated after
several test binaries had already run; the exclusion would have been honored
by any session that read the open `bug` issues first, and none should depend
on that.

The host does not fail from resource exhaustion, and the record must not
imply otherwise. Measured on 2026-09-30: 32 logical processors, 93.7 GB RAM,
pagefile allocated 68 GB with a 1.4 GB peak usage, all four physical
disks report `Healthy`, and the System event log holds **zero** WHEA-Logger
records in the preceding 7 days. Six `Kernel-Power 41` restarts fall on
2026-09-29 between 03:55 and 19:17; three carry `BugCheckCode` 0, and the
other three carry `BugCheckCode` 26 (`0x1A`, `MEMORY_MANAGEMENT`) with
`BugcheckParameter1` 63 (`0x3F`, pagefile inpage error). `Ntfs` event 98
entries in the same window read "volume is healthy; no action needed" and are
health-check records, not corruption reports.

So the crash is real and repeatable, and "compilation exhausted memory" is
**not** a supported explanation. #403 itself holds causation open pending an
authorized dump analysis that this account cannot perform. This record does
not settle causation; it isolates the host so that a repair can proceed
without betting the machine on an unproven theory.

`docs/decisions/README.md` exists because a scope rule decided in one
session's chat is invisible to the next session, and #208 is the recorded
case: an exact-lock PR was produced in parallel with a floor-pinned session.
An unstated "don't run the suite on this box" rule fails the same way, at a
higher cost, because the failure mode is a crash rather than a wrong PR.

## Enforcement

- This record, plus the #403 body, are the two places a session must be able
  to read before running a build or test command. The cost is one `gh issue
  view 403` and one listing of `docs/decisions/`.
- Agent instructions in `AGENTS.md` carry the "check #403 before compiling"
  line for this repository; that file is the surface a new agent reads first.
- Convention only: no gate mechanically blocks `cargo test` on this host. The
  cost of a mechanical guard (a wrapper that refuses, shadowing cargo) exceeds
  the cost of the rule until a second host needs the same protection.
- The verified resource-pressure defect in #403 is a **separate** matter and is
  not blocked by this record: `crates/code-intel-cli/src/snapshot.rs`
  `digest_worktree` retains every scoped file in `records` and
  `hash_records` concatenates them into a second `canonical` buffer before
  hashing, so peak allocation scales with whole-tree content times two. It is
  repairable, and repairing it does not require compiling here.
