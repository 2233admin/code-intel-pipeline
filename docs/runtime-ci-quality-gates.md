# Runtime/CI quality-gate evidence

Code Intel structural evidence does not prove that a repository is tested,
secure, or tested against realistic failures. Projects may publish a
snapshot-bound `code-intel-runtime-ci-observation.v1` artifact and include an
optional `signals.quality` signal:

```json
{
  "status": "passed",
  "observed": true,
  "summary": "nextest, clippy, audit, deny, coverage and mutation gates passed"
}
```

The signal is provider-neutral. A project may use `cargo-nextest`,
`cargo-clippy`, `cargo-audit`, `cargo-deny`, `cargo-llvm-cov`, `cargo-mutants`,
`cargo-fuzz`, or equivalent tools for another language. The collector owns the
tool commands and log artifacts; Code Intel only ingests the digest-bound,
snapshot-bound result.

Semantics:

- `passed`: the quality gate completed successfully.
- `failed`: at least one required quality check failed.
- `cancelled`: the run did not complete and is not evidence of health.
- `unknown`: the quality layer was not observed.

An observed `failed` or `cancelled` quality signal makes the normalized
runtime/CI summary `red`. A passed quality signal adds
`quality_gates_observed_passed` to the facts. Omitting the optional signal is
backward-compatible, but does not claim that quality gates ran.

Code Intel does not execute project tests, install dependencies, create
commits, open pull requests, or push changes as a consequence of this signal.
