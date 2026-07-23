# Project Orientation Benchmark

`project.orientation-benchmark` verifies D01 orientation cost and quality without changing D01's output semantics. It runs a fixed `small | medium | large` by `clean | dirty | provider_missing` corpus, twice or more for both cold and warm conditions, with one local child process at a time and no LLM or hosted service.

## Run

```powershell
cargo build -p code-intel
target/debug/code-intel.exe benchmark orientation --out artifacts/orientation-benchmark --repetitions 3
```

The output directory must not already exist. The runner writes:

- `observations.json`: measured samples under `code-intel-project-orientation-benchmark-observations.v1`.
- `report.json`: authoritative evaluation under `code-intel-project-orientation-benchmark.v1`.
- `report.md`: rebuildable human view.

Cold samples include fresh fixture and immutable Artifact Ref materialization. Warm samples reuse the immutable input corpus but always use a fresh output directory. Percentiles use nearest-rank over `std::time::Instant` wall times. `small` and `medium` across all conditions define the typical corpus; `large` is a stress corpus. Every sample records the exact orientation byte size and SHA-256. A pass requires typical p95 at or below 60 seconds, identical output digests for every replay of a fixture, exact measured-field correctness, exact unresolved-field coverage, exact unsupported-file coverage, and complete claim provenance.

## Quality and failure policy

Every evaluated orientation claim must retain non-empty provenance. The evaluator recomputes each orientation sample's byte size and digest before using its determinism metrics. A fast result with missing claim provenance or forged size/digest metadata is a contract failure and no report is published. Reports separate fixture-materialization and A01 process/orientation costs and expose typical/all artifact-size percentiles.

The representative benchmark is a blocking CI contract. Local and CI reports state `cleanMachine: false`; that field remains unchanged and must not be forged. The prior clean-machine record in [`orchestration/evidence/d02-clean-machine-verifier-attestation.v1.json`](../orchestration/evidence/d02-clean-machine-verifier-attestation.v1.json) is historical evidence for its bound source digest; after benchmark implementation changes it is stale until a new disposable clean-machine run binds the current digest. Its closed schema is [`code-intel-clean-machine-verifier-attestation.v1.schema.json`](../orchestration/schemas/code-intel-clean-machine-verifier-attestation.v1.schema.json). Cold does not mean an operating-system page-cache flush.

The benchmark consumes D01 through the registered A01 capability and A03-verified artifacts. It does not grant authority to benchmark observations, infer missing structure, or convert provider absence into a passing structural verdict.
