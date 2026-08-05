---
status: accepted
date: 2026-08-04
---

# Anchor verification: precision over recall

Several `run execute` products carry location claims about the repository:
`code_evidence.agent_slice` (`ranking.json`) claims files exist,
`code_evidence.symbols` claims a named symbol lives at a specific
`file:startLine`, and `diagnosis.surgery-plan` claims a primary target file.
Before issue #151, nothing re-checked any of these claims against the
repository between the moment a scan produced them and the moment they were
published. A claim could point at a file already renamed, or a symbol
already moved or deleted, and ship unchanged. An agent that trusts the
claim and opens the path finds nothing, or finds the wrong thing, and either
wastes a turn or reasons from a wrong premise.

## Decision

Code Intel verifies every file, line-range, and symbol anchor it is about
to publish, and would rather report an anchor as dropped than let a claim
that no longer resolves ship as if it still did. Three states, not two:
`verified` (the claim still holds exactly), `approximate` (a symbol drifted
to another line in the same file but is still there, and the corrected line
ships alongside the claim), and `dropped` (the claim does not resolve in
its own file, with a reason). Dropped is never silent: its count sits
alongside `verified`/`approximate` at the top level of the `run execute`
CLI summary, not buried inside a manifest a caller has to know to open.

Verification never expands into a repository-wide search. A symbol
re-resolution is bounded to the one file the original claim named; if it is
not there and not found anywhere else in that same file, it is dropped, not
chased across the repository. Widening the search would trade a
false-negative-shaped failure (dropping a claim that a broader search might
have rescued) for a worse one: a "corrected" location an agent did not ask
about and that may not be the same symbol at all. Precision is the point,
not maximal recovery.

This mirrors the same precision-over-recall call
[alibaba/open-code-review](https://github.com/alibaba/open-code-review)
makes publicly for its own line-level review comments: it accepts lower
recall in exchange for higher precision, describing it as
"a deliberate trade-off favoring precision over noise," and leans on
deterministic, engineering-logic positioning rather than a model's best
guess for exactly the steps that must not go wrong. Code Intel's anchor gate
takes the same position for a different artifact shape: a location claim,
like a review comment's line anchor, is worse than useless if it is wrong,
so an unresolvable one is excluded and counted rather than shipped anyway.

The three-state design itself follows gate G1's `EvidenceOutcome` pattern
(ADR-consuming issue #141): `approximate` cannot be constructed without the
corrected line it found, `dropped` cannot be constructed without a reason,
and the JSON encoding of each state is closed over its own exact key set so
relabeling one state as another by touching only the state tag is rejected,
not silently accepted. See `crates/code-intel-cli/src/anchor_verification.rs`
for the implementation and its own, more detailed rationale, including why
dropped anchors are excluded from a new companion artifact
(`verification.anchors`) rather than rewritten out of the original products
in place.

## Alternatives considered

### Best-effort repository-wide symbol search

Searching the whole repository for a moved symbol would rescue more claims
(higher recall) but at the cost of returning a location the original claim
never pointed at and that may belong to an unrelated declaration of the
same name. Given the choice between reporting "dropped, with a reason" and
guessing at a repository-wide match, the gate reports dropped.

### Silent drop, no count

Removing an unresolvable claim without recording that anything was removed
would look identical, from the outside, to a repository with nothing ever
wrong to report. Issue #139's charter rule applies here as much as it did
to gate G1: a check that always appears to pass is worse than no check at
all. `dropped` is always a visible, computed count, never an implicit zero.

## Consequences

- A product's location claims are only as trustworthy as this gate's last
  run against the same repository; claims made and never re-verified (the
  `change_impact` command's own committed-evidence path, and
  `audit_report/`'s free-text `surface_evidence`, neither of which currently
  carries a path/line/symbol-shaped claim) are explicitly out of this gate's
  scope, not silently covered by it.
- `approximate` gives a reader a corrected line for free when one exists,
  without requiring a second, broader tool call to find it.
- A repository that renames files or moves symbols frequently between scans
  will show a nonzero `dropped` count on its next `run execute`, by design.
