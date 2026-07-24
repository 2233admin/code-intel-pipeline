# Ponytail Governance Gate

`governance.ponytail-gate` is the executable C00 admission boundary for Agent-produced changes.
It validates necessity and implementation selection; it does not rank product work, judge code
style, or replace engineering review.

The checked contracts are:

- `orchestration/schemas/code-intel-ponytail-gate.v1.schema.json`
- `orchestration/ponytail-gate-policy.v1.json`
- `tests/fixtures/ponytail/c00-necessity-trace.json`

## Admission trace

Every declared artifact, dependency, abstraction, file, test, documentation, or process change
names exactly one current value source and its evidence. Allowed sources are an
operator-requested outcome, Committed Engineering Plan deliverable, verified defect or risk,
required contract or gate, evidence-closing spike, or approved debt reduction. `future_maybe` is
representable only so the deterministic gate can reject it; it is not a current value source.

Each change selects one first sufficient rung from the existing Implementation Minimalism
Benchmark. Every lower rung must appear once, in order, with a nonempty reason and known evidence.
This makes “smallest sufficient” reviewable without introducing a solver or policy platform.
`requiredEvidenceIds` closes the rest of the Necessity Trace, including evidence for the declared
protection boundary; value-source and lower-rung evidence do not replace it.

## Non-filterable engineering boundaries

The gate never admits a declaration that removes verification, evidence, safety, error handling,
accessibility, data-loss prevention, or artifact-contract requirements. An authority bypass cannot
override these boundaries. C00 governs implementation minimalism; it is not permission to
under-build.

## A05 authority bypass

A value-source or rung rejection may be temporarily bypassed only by an explicit
`code-intel-authority-event.v1` record scoped to the same change id. C00 reuses the A05 validator:
the event must be approved, have a named approver, cover the value-source evidence, every
lower-rung evidence ID, and every `requiredEvidenceIds` entry, be issued and
unexpired at evaluation time, and not appear in the consumed-event set or another branch. Missing
evidence, expiry, future dating, wrong scope, duplicate use, and replay fail closed.

## Modes and integration seam

`report_only` and `enforce` run the same rules and retain the same per-change trace. Report-only
sets `enforcedBlock=false` while preserving `wouldReject` and rejected branches. Enforce sets
`enforcedBlock=true` whenever a rejection remains. A bypassed branch remains visible with its
authority event id and the original rejection diagnostic.

The Rust module exposes the in-process `evaluate` and `policy_document` seams. Production callers
use `code-intel governance ponytail-gate --request <request.json|->`, registered as
`governance.ponytail-gate` in `orchestration/integrations.json`. A completed non-blocking result
exits 0; an enforce result with `enforcedBlock=true` remains schema-valid on stdout and exits 2.
Usage errors exit 64, contract-invalid input exits 65 without emitting a result, and host I/O
failures exit 74. The gate is not coupled to the A09 DAG runner.

Run the focused CI contract with:

```powershell
./scripts/tests/test-ponytail-gate-contract.ps1 -RepoPath .
```

No Ponytail package or runtime is installed or invoked. The source project remains a reference;
the semantic contract is owned by Code Intel Pipeline.
