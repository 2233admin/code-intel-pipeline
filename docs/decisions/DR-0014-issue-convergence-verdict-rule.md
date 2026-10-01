# DR-0014 issue convergence verdict rule

Status: active
Date: 2026-09-30
Amended: 2026-09-30 (convergence pass; see "Amendment" below)

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

## Amendment (2026-09-30 convergence pass)

The original rule defined the three verdicts but said nothing about **the
act of auditing them**, and the pass found three ways a queue can look
converged while it is not. These are now part of the rule.

### 1. A label is not a verdict

`backlog` was on 65 issues while 27 of them carried **no comment at all**. The
rule already required "a written reason"; enforcement did not say what to do
about labels applied in bulk on 2026-08-03 that never got one. Those 27 now
carry reasons.

**A verdict is a comment carrying evidence, not a label.** A label may be
applied in bulk; the reason may not.

### 2. `claimed` is a claim about a session, and it goes stale silently

The pass found **four** zombie claims — #302, #47, #193, and
#395/#396/#397 — all with the same shape: a claim comment naming a branch, no
push, no PR. Three of them (DR-0004 names the 48h bar) had been open for 37-40
days. Two of them additionally contradicted themselves: #47 was both `backlog`
and `claimed` at once, which the two label definitions make impossible.

DR-0004 says a stale claim is releasable after 48h. This record adds the part
DR-0004 cannot enforce: **`claimed` is a statement about a live session, so a
reviewer must treat a claim as unverified until the branch is confirmed to
exist.** A claim comment is an assertion, not evidence.

**Verifying a claim means checking the named ref exists** — in local heads, in
origin refs, and in any other ref. A comment that names a branch is not a
branch.

### 3. "Work exists" is not "work delivered"

The pass found four distinct states that a raw open-issue count cannot
distinguish:

| State | Case found |
|---|---|
| Shipped but ticket open | #383 — fix merged via PR #388 |
| Implemented, in remote, never PR'd | #123 — 2 commits on `origin/issue-307-bounds-oversize-input`, no PR |
| Implemented, in local branch, never pushed to `main` | #363 — commit `e923a70`, PR #364 closed as misrouted |
| Implemented, uncommitted, evidence gone | #393 — code on disk, `target/issue-393-verification/` deleted |
| Claimed, implemented, **worktree deleted** | #395/#396/#397 — branch has 0 commits ahead of main, directory gone |

The last row is the reason the other four matter: the same missing step — a
push — that would have saved #395/#396/#397 also separates "delivered" from
"written down" in every other row.

**Do not close an issue because the code looks done.** Check where the work
actually is: merged into `main`, on a remote branch, on a local branch,
uncommitted, or gone. Each state has a different correct verdict.

## Why

On 2026-09-30 the open queue held **96 issues, 65 of them `backlog`**. The
`backlog` label's own definition reads `Frozen: not in the v1 convergence scope` — so those 65 are decisions *not* to build, not unfinished work. The oldest, #14, has sat since 2026-07-24, 68 days.

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

The convergence pass turned both of those into patterns: a bulk-applied label
with no reason behind it, and a claim that outlived its branch by weeks. The
three amendments above exist so that the next pass does not rediscover them
from scratch.

## Enforcement

- Convention only: no gate inspects issue labels. The cost of a mechanical check
  is higher than the cost of the rule while the queue is being reduced by hand.
- A session claiming convergence cites per-issue verdicts, or it claims nothing.
- When citing a `do` verdict, state **where the work is**, per amendment §3. A
  verdict without that is incomplete.
- When auditing `claimed`, confirm the named ref exists, per amendment §2.
- The survey that motivated this record is `docs/problem-inventory-2026-09-30.md`;
  the open handoff is `docs/handoff-2026-09-30-issue-convergence.md` (it was
  first written into `.superpowers/sdd/`, a directory gitignored with `*`, and
  moved here so a worktree can see it).
- This record does **not** retract DR-0005. Being under the open-PR ceiling
  permits starting new work; it does not oblige a session to do backlog items,
  and a session that unfreezes a `backlog` issue does so explicitly.
