# Repository Layout

This repository is converging toward a smaller public surface.

## Public Root Entry Points

Keep these files at the repository root until a release explicitly changes the
install and CI contract:

- `invoke-code-intel.ps1`: stable operator entry point.
- `run-code-intel.ps1`: current artifact-producing pipeline orchestrator.
- `check-code-intel-tools.ps1`: environment doctor.
- `install-code-intel-pipeline.ps1`: installer and repair entry point.
- `Find-CodeIntelProjects.ps1`: local project discovery entry point.
- `README.md`, `CHANGELOG.md`, `CONTEXT.md`: operator-facing docs.
- `Cargo.toml`, `Cargo.lock`, `crates/code-intel-cli`: Rust CLI policy core.

## Internal Script Buckets

Future file moves should use these buckets:

- `scripts/tests/`: PowerShell contract tests and smoke tests.
- `scripts/benchmarks/`: benchmark and A/B scripts.
- `scripts/adapters/`: tool-specific helper wrappers.
- `scripts/incubator/`: experiments that are not in the shipped product path.

Do not move a root PowerShell file without one of these:

- a root compatibility shim with the old filename, or
- a simultaneous update to installer, CI, release packaging, README, skill docs,
  and tests.

## Rust Core Boundary

The Rust CLI owns pure policy and artifact-consumer logic first:

- artifact resume
- failure classification
- effective failure policy
- Sentrux failure normalization
- Sentrux debt register classification
- next protocol / GitHub research routing decisions

The current policy contract is documented in `docs/rust-policy-core.md`.

The PowerShell pipeline still owns local orchestration and tool invocation until
the Rust policy contract is stable enough to become the runner.

## Incubator Boundary

`crates/code-nexus-lite/` is currently an incubator note, not a Cargo package.
It must not re-enter the workspace until its dependency chain is security-clean
and the worker is part of the shipped product path.
