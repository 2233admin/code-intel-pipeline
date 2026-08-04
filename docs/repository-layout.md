# Repository Layout

This repository is converging toward a smaller public surface.

## Repository Root

- `README.md`, `CHANGELOG.md`, `CONTEXT.md`: operator-facing docs.
- `changelog.d/`: per-PR changelog fragments (see `changelog.d/README.md`). Do not
  edit `CHANGELOG.md`'s `[Unreleased]` section from ordinary PRs — write a
  fragment instead. Release aggregation: `tools/aggregate_changelog.py`.
- `tools/`: small maintainer scripts (Python). `aggregate_changelog.py` folds
  `changelog.d/` fragments into `CHANGELOG.md` at release time (`--dry-run`
  previews; `--check-pr` is the CI advisory for missing fragments).
- `Cargo.toml`, `Cargo.lock`, `crates/code-intel-cli`: primary compiled CLI and execution kernel.

No PowerShell entry point lives at the repository root. Every `.ps1` surface is
under `legacy/`.

## The `legacy/` Directory

`legacy/` holds the whole PowerShell surface. The name records *which language
tier a file belongs to*, not whether it is retired — the directory mixes live
entry points with facades awaiting retirement. Read the per-file status below;
do not infer it from the directory name.

Still live, with no replacement today:

- `legacy/install-code-intel-pipeline.ps1`: installer and repair entry point. The
  only source-install path, including macOS and Linux.
- `legacy/code-intel.ps1`: recovery and update launcher for the compiled CLI.
- `legacy/Invoke-SentruxAgentTool.ps1`: coding-session gate entry point
  (`session_start` / `session_end`), mandated by AGENTS.md. Teardown: #50.
- `legacy/Find-CodeIntelProjects.ps1`: local project discovery entry point.
- `legacy/bootstrap-new-machine.ps1`: new-machine bootstrap entry point.

Superseded or retiring — do not add new callers:

- `legacy/check-code-intel-tools.ps1`: superseded by the native Rust doctor
  (#48). Use `code-intel doctor`.
- `legacy/run-code-intel.ps1`: compatibility adapter host for capabilities not yet
  internalized. Absorbed into `code-intel run` by #47.
- `legacy/invoke-code-intel.ps1`: v0.x compatibility forwarder.

## Internal Script Buckets

`legacy/scripts/tests/` holds the PowerShell contract tests and smoke tests. It
is the only bucket that exists today; `benchmarks/`, `adapters/` and
`incubator/` are the reserved names for their categories, to be created under
`legacy/scripts/` when such a script first appears.

Do not move a PowerShell file out of `legacy/` without one of these:

- a compatibility shim at the old path with the old filename, or
- a simultaneous update to installer, CI, release packaging, README, skill docs,
  and tests — plus a re-freeze of every retirement packet that pins a file you
  touched, and a digest-pin re-sync afterwards.

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
