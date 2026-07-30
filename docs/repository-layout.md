# Repository Layout

This repository is converging toward a smaller public surface.

## Public Root Entry Points

Keep these files at the repository root until a release explicitly changes the
install and CI contract:

- `archive/code-intel.ps1`: recovery and update launcher for the compiled CLI.
- `archive/invoke-code-intel.ps1`: v0.x compatibility forwarder.
- `archive/run-code-intel.ps1`: compatibility adapter host for capabilities not yet internalized.
- `archive/check-code-intel-tools.ps1`: environment doctor.
- `archive/install-code-intel-pipeline.ps1`: installer and repair entry point.
- `archive/Find-CodeIntelProjects.ps1`: local project discovery entry point.
- `archive/bootstrap-new-machine.ps1`: new-machine bootstrap entry point.
- `archive/Invoke-SentruxAgentTool.ps1`: Sentrux compatibility entry point.
- `README.md`, `CHANGELOG.md`, `CONTEXT.md`: operator-facing docs.
- `Cargo.toml`, `Cargo.lock`, `crates/code-intel-cli`: primary compiled CLI and execution kernel.

## Internal Script Buckets

Internal scripts use these buckets:

- `scripts/tests/`: PowerShell contract tests and smoke tests.
- `scripts/benchmarks/`: benchmark and A/B scripts.
- `scripts/adapters/`: tool-specific helper wrappers.
- `scripts/incubator/`: experiments that are not in the shipped product path.

The public PowerShell compatibility and recovery entry points stay at the repository root. Test
scripts are internal and must remain under `scripts/tests/`.

Do not move a root PowerShell file without one of these:

- a root compatibility shim with the old filename, or
- a simultaneous update to installer, CI, release packaging, README, skill docs,
  and tests.

## Rust Core Boundary

The Rust CLI owns the primary operator entry, execution kernel, policy, and artifact contracts:

- artifact resume
- failure classification
- effective failure policy
- Sentrux failure normalization
- Sentrux debt register classification
- next protocol / GitHub research routing decisions

The current policy contract is documented in `docs/rust-policy-core.md`.

PowerShell remains for recovery, compatibility, and adapters that have not yet
been internalized.

## Incubator Boundary

`crates/code-nexus-lite/` is currently an incubator note, not a Cargo package.
It must not re-enter the workspace until its dependency chain is security-clean
and the worker is part of the shipped product path.
