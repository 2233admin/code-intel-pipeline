# Compatibility retirement ticket template (E01)

E01 freezes one small, reversible deletion ticket after E00 has approved the exact retirement
subject. It is a planner/validator boundary: the output remains `draft` and carries
`template_only_no_approval_or_deletion_authority`; it cannot approve, delete, reroute, disable, or
roll back a branch.

Each ticket names exactly one legacy capability, branch, and call path; one replacement capability
and its dependencies; affected files; golden, contract, effect, usage, rollback-rehearsal, and
deletion-diff evidence; distinct owner and verifier; and an observation expiry. The contract is
closed, so plural/extra branch fields are rejected rather than interpreted.

The A01 capability consumes A03-verified ticket, E00 manifest, E00 decision, and deletion-diff
artifacts. It requires an `approved` E00 decision, verifies the decision and manifest Artifact Ref
digests, recomputes the E00 approval-subject digest, and compares every projected evidence ref to
the consumed E00 manifest. It also requires the ticket's canonical call path and exact sorted,
unique affected-file set to equal the E00-approved values.

The deletion diff must name that same branch and exact file set. `deletionsOnly` and `summary` are
descriptive fields, not proof. The authoritative `replayable-delete-only-v1` patch binds every
file's base/result UTF-8 text to SHA-256, supplies exact deletion hunks, forbids added lines, and is
replayed by the runtime to prove that the result is obtained only by deletion. A forged summary,
replacement/addition, hidden touched path, hash mismatch, or non-replayable hunk fails closed with
exit 65. `options.evaluatedAt` is explicit and deterministic; expired observations fail.

Run the standalone completeness lint with:

```text
target/debug/code-intel.exe compatibility retirement-ticket lint --ticket <ticket.json> --evaluated-at <unix-seconds>
```

Run the content-bound A01 projection with `capability exec compatibility.retirement-ticket-template`.
Passing either command means only that the draft is complete and bound to E00; a separate verifier
and a separately authorized deletion executor are still required.
