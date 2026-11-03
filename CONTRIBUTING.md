# Contributing

Issues, reports, and pull requests may be written in English or Chinese.

## Workflow

1. Find an existing issue or open a new one. Labels are described in [docs/agents/triage-labels.md](docs/agents/triage-labels.md) and the tracker conventions in [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md).
2. Claim it before writing code (DR-0004): add the `claimed` label and comment with your branch name and a one-line approach. If you cannot add labels, leave the same comment and ask a maintainer to add `claimed` for you before you start work that may overlap with others.
3. Read [AGENTS.md](AGENTS.md) for the repository rules and [docs/decisions/README.md](docs/decisions/README.md) for the active decision records.

## Toolchain

Use the toolchain pinned in `rust-toolchain.toml`; rustup selects it automatically.

## Build and verify

```bash
cargo build --locked -p code-intel --bin code-intel
target/debug/code-intel --version
cargo test -p code-intel --locked
cargo test -p code-intel --locked --test <name>   # focused tests for what you changed
```

Before pushing, run `code-intel sentrux gate <scope>` and `code-intel lint hardcoded-paths`.

## Install-class bugs

If the bug is that users cannot install, or the installed binary fails, the reproduction must be added to the `install_smoke` test in the same pull request (DR-0001).

## Compiler warnings

Do not add `-D warnings` and do not bulk-delete "dead code": the warning count is not a debt metric. See "Reading the crate's warning count" in [AGENTS.md](AGENTS.md).

## Pull requests

Write `Closes #N` in the description and list the commands you actually ran.
