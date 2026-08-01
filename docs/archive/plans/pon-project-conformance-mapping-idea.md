# Pon project-conformance mapping

## Problem

The first internalization pass added a parity floor and a component-level language-adapter gate,
but it did not define how Pon's conformance workflow governs acceptance of Code Intel Pipeline as
a project. Generic precision/recall thresholds cannot stand in for that missing project-level
contract.

## Selected mapping

Internalize the method, not Pon's compiler implementation:

| Pon conformance mechanism | Code Intel project acceptance |
| --- | --- |
| CPython as reference oracle | reviewed fixture/golden artifact oracle |
| conformance corpus | repository-state and multi-language fixture corpora |
| JIT/AOT parity | normalized output parity across supported evidence paths |
| committed passing floor | monotonic parity case set and count |
| expected divergences | reviewed, expiring known-divergence ledger |
| fuzzing | fail-closed mutation/property tests |
| free-threading stress | repeated-run determinism and concurrency stress |
| fast/full gates | local/PR fast profile and release full profile |
| benchmark floor | representative latency/resource regression ratchet |

## Delivery

Add one project-conformance policy and one executable runner. The fast profile must execute the
mechanisms already supported by the repository. The full profile must fail closed while mapped
mechanisms such as the active divergence ledger or performance ratchet remain unfinished. This
makes the mapping useful immediately without overstating completeness.

The existing language-adapter acceptance gate remains a component suite under project
conformance; it is not the project-level policy.

## Constraints

- Do not copy Pon source, fixtures, or prose because its license was not declared when inspected.
- Preserve existing tests and artifacts; compose them rather than introducing another scanner.
- A passing fast profile is not production/release acceptance.
- Policy observations cannot silently update floors or create divergence exceptions.
