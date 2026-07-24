# Code Intel project conformance

This is the project-level acceptance contract selectively mapped from Pon's conformance method.
It evaluates Code Intel Pipeline through executable evidence. It does not reproduce or claim
compatibility with Pon's compiler/runtime features.

## Mapping

| Upstream method | Local acceptance meaning | Current state |
| --- | --- | --- |
| CPython differential oracle | reviewed canonical artifact oracle | implemented |
| conformance corpora | repository-state and multi-language fixture corpora | implemented |
| JIT/AOT output agreement | normalized artifact-contract parity | implemented |
| passing floor | monotonic fixture set and count | implemented |
| divergence ledger | scoped, owned, expiring known differences | designed, not active |
| fuzzing | fail-closed negative mutations | implemented |
| free-threading stress | repeated/fresh-root determinism | partial |
| benchmark ratchet | representative latency/resource floor | deferred |

The machine-owned mapping and profile definitions live in
`orchestration/code-intel-project-conformance-policy.v1.json`.

## Profiles

`fast` is the currently usable local/PR profile. It executes the parity floor, component adapter
contract, multi-language corpus test, CPython 3.14 development lane, and fail-closed multi-Agent
merge-queue contract. It accepts the
explicitly partial determinism mechanism, so a fast pass is not release acceptance.

The merge-queue suite uses isolated fixtures; it verifies readiness and authority semantics without
installing the provider or pushing a remote. `full` is the release target. It requires every mapped mechanism to be `implemented`. It therefore
fails closed until the divergence ledger participates in comparisons, determinism stress is complete,
and a representative performance ratchet exists.

Run:

```powershell
./scripts/tests/Test-CodeIntelProjectConformance.ps1 -Profile fast
./scripts/tests/Test-CodeIntelProjectConformance.ps1 -Profile full -Json
```

Exit code `0` means that the selected profile's mechanism readiness and every executable suite pass.
Exit code `1` means conformance was rejected. Malformed or unpinned policy input exits `2`.

## Authority boundary

- A suite observation cannot update a floor or add a divergence exception.
- The component language-adapter gate is evidence consumed by this project gate; it is not the
  project-level policy.
- `full` may be promoted only by implementing and testing the missing mapped mechanisms, not by
  deleting them from the policy or weakening accepted statuses.
- The upstream reference is method provenance only. No Pon code or fixtures are copied.
