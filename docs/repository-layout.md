# Repository Layout

This repository is converging toward a smaller public surface.

## Repository Root

- `README.md`, `CHANGELOG.md`, `CONTEXT.md`: operator-facing docs.
- `Cargo.toml`, `Cargo.lock`, `crates/code-intel-cli`: primary compiled CLI and execution kernel.

PowerShell has been removed from the product (DR-0018): the repository ships no
`.ps1` entry point, and the install path is the Skill bootstrap plus
`code-intel setup`. Install, environment checks and the structure gate are
subcommands of the compiled CLI: `code-intel setup`, `code-intel doctor`,
`code-intel doctor bootstrap`, `code-intel sentrux gate`.

## Rust Core Boundary

The Rust CLI owns the primary operator entry, execution kernel, policy, and artifact contracts:

- artifact resume
- failure classification
- effective failure policy
- Sentrux failure normalization
- Sentrux debt register classification
- next protocol / GitHub research routing decisions

The current policy contract is documented in `docs/rust-policy-core.md`.

## Incubator Boundary

`crates/code-nexus-lite/` is currently an incubator note, not a Cargo package.
It must not re-enter the workspace until its dependency chain is security-clean
and the worker is part of the shipped product path.
