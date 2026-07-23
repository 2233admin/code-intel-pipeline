# Compatibility Retirement Gate (E00)

`compatibility.retirement-gate` decides whether one legacy branch has enough evidence to enter a
separate retirement ticket. It cannot delete, rewrite, disable, or reroute that branch. Its sole
effect is publishing a local, snapshot-bound decision artifact and a Gain Ledger projection.

The A01 capability consumes one A03-verified closed retirement manifest plus the exact A03-verified
evidence artifacts referenced by that manifest. Extra evidence, missing evidence, reused evidence,
digest mismatches, snapshot mismatches, and unregistered artifact types fail closed.
The request must provide exactly one explicit `options.evaluatedAt` integer; this keeps freshness
evaluation deterministic and replayable instead of reading wall-clock time implicitly.

Approval requires independently content-bound evidence for:

- a production-ready replacement atom with a non-cyclic dependency set;
- golden, contract, and effect parity with at least one passing assertion each;
- B07 registry reconciliation for the legacy participant and replacement capability;
- a completed compatibility observation window whose `checkedAt <= evaluatedAt <= expiresAt`;
- an owned rollback command and a real, matching execution with exit code 0;
- production usage observation covering the same compatibility window, with internally reconciled
  totals, positive replacement traffic, and zero legacy invocations;
- an admitted C00 necessity decision whose change ID and trace digest bind this retirement record;
- one matching approved dependency state for every replacement dependency, including D02 when it
  is a dependency; and
- an approval bound to the SHA-256 of the complete approval subject and a current repository-trusted
  authority event. The reviewer must match the trusted event approver and differ from the legacy
  owner; `authorIndependent: true` alone has no authority.

The complete approval subject includes a canonical `<portable-path>::<branch-id>` call path and the
exact sorted, unique set of affected files. Those fields are therefore covered by both the subject
SHA-256 and the independent approval. A later ticket cannot broaden the call path or add a second
file/branch under the same E00 approval.

`pending` or `rejected` dependency evidence remains a visible blocker. The gate never fabricates a
D02 approval. A replacement that names itself or the legacy capability as a dependency is cyclic
and blocked. Line reduction is explicitly fixed to `false` in the manifest contract because fewer
lines are not correctness evidence.

The output is `approved` or `blocked`, retains sorted blocker codes, fixes the authority boundary to
`approval_only_no_deletion_authority`, and projects an `approved-for-ticket` or `blocked` Gain
Ledger item. Actual deletion remains an E01+ branch-specific action with its own authority.
