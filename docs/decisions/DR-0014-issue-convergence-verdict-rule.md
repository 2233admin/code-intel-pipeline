# DR-0014 issue convergence verdict rule

Status: active
Date: 2026-09-30

## Decision

Convergence of this repository's issue queue means: **every open issue carries
an explicit verdict of `do`, `freeze`, or `close`.** It does not mean the
backlog reaches zero, and it does not mean open issues reach zero.

An issue is converged when it is either:

- **done** — the work shipped, or
- **frozen** — `backlog`, with a written reason for why it is out of the current
  scope, or
- **closed** — it is obsolete, superseded, or already satisfied by merged work,
  with a comment saying which.

A session that opens implementation work on a `backlog` issue without first
recording why it is no longer frozen is violating this record. DR-0007 already
makes GitHub Issues the delivery source of truth, so the verdict lives in the
issue, never in a planning document.

## Why

On 2026-09-30 the open queue held **96 issues, 65 of them `backlog`**. The
`backlog` label's own definition reads `Frozen: not in the v1 convergence
scope` — so those 65 are decisions *not* to build, not unfinished work. The
oldest, #14, has sat since 2026-07-24, 68 days.

A session told to "do all the outstanding work" reads 96 and attempts 96. That
session dies before its first PR, and the queue it leaves behind is no better
than the one it inherited. The failure is not stamina; it is that the input
number and the actual obligation are different numbers, and only the label
distinguishes them.

The same day produced two concrete instances of the cost. Issue #383 was still
open and still `claimed` although PR #388 had already merged its fix — a
ticket that looks like work and is not. Issue #302 held a claim from
2026-08-21 whose branch existed in neither local heads nor origin refs; a claim
that looks like an active session and is not. Both were invisible in a raw
count of open issues.

Issue #267 compounds it. It was unparked on 2026-09-14 and named #269 as its
first frontier, but #270 through #273 remained `backlog` with no matching
unpark record. The precondition was met and nothing moved, which no count of
open issues can reveal.

This record changes no runtime behavior. It changes what a session is required
to be able to show when it says the queue is handled.

## Enforcement

- Convention only: no gate inspects issue labels. The cost of a mechanical check
  is higher than the cost of the rule while the queue is being reduced by hand.
- A session claiming convergence cites per-issue verdicts, or it claims nothing.
- The survey that motivated this record is `docs/problem-inventory-2026-09-30.md`;
  the open handoff is `.superpowers/sdd/2026-09-30-issue-convergence-handoff.md`.
- This record does **not** retract DR-0005. Being under the open-PR ceiling
  permits starting new work; it does not oblige a session to do backlog items,
  and a session that unfreezes a `backlog` issue does so explicitly.
