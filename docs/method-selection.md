# Deterministic method selection contract

`method.select` is the C02 advisory matcher. It consumes A04 admissibility results, evidence-backed
engineering fact projections, explicit evidence gaps, the approved C01 Method Cards, and the
checked rule table at `orchestration/method-selection-rules.v1.json`.

The selector does not contain method definitions. Positive rule signals must resolve to a C01
card's `problemSignals`; contraindication rules must quote an existing C01 contraindication. One
sorted rule exists for every card. This keeps method meaning in C01 while making matching
deterministic and reviewable.

Each admission envelope carries the original A04 request and its claimed A04 result. C02 calls the
A04 runtime validator against the explicit artifact root, which re-reads and digest-verifies the
original payload, revalidates the complete observation contract and freshness policy, and
recomputes the admission identity. The claimed result must byte-for-value equal that recomputed
result and have the `admitted`/`observed` verdict. C02 does not maintain a weaker copy of A04's
validator.

Selectable facts must be present in the verified payload's `data.methodSelectionFacts` array.
The request projection must exactly match the payload fact's ID, signal IDs, and evidence kinds,
and cite the recomputed admission identity. Unknown, duplicate, stale, forged, relabeled, or
unreferenced admissions fail closed; every supplied admission must be cited by at least one bound
fact. Thus caller-supplied labels cannot manufacture selection evidence outside an A04-admitted
payload. A04 still emits no Engineering Facts; this payload binding is selection evidence only.

Results are sorted by Method Card ID and use only `proposal`, `unknown`, or `none`:

- `proposal`: a rule signal matched, all C01 required evidence is present, and no mapped C01
  contraindication is active.
- `unknown`: a signal matched but required evidence is missing.
- `none`: no rule matched, or a matched method is contraindicated.

Every match explains matched signals, missing evidence, C01 cost, declared/triggered
contraindications, C01 confidence rules, and deterministic selection confidence. Equal top match
scores set `tie: true`; ordering never resolves the tie. The result is explicitly advisory and
never claims method execution, an Engineering Fact, an Adoption Decision, or a Committed Plan.

The module is an independent API that requires an explicit artifact-root authority, pending
serialized production wiring after the shared entrypoint work completes. Removing that future
route rolls back C02 without changing C01 or A04.
