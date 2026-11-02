# Code Intel Pipeline

> English landing page. The full operator manual is currently Chinese-first: see [README.md](README.md).

**Hand your coding agent a map of the repo before it edits.**

Code Intel Pipeline is a local code-intelligence pipeline for AI coding agents.
One command runs a DAG of evidence collectors against a repository and commits
the results as a content-addressed, transactional artifact run:

```text
code-intel .
```

```text
[PASS] your-repo
  Outcome: completed
  Run evidence: <data-root>/artifacts/<repo>/<run-id>/run-complete.json
```

## What one run produces

- **Agent code map** — every file ranked by entrypoint / symbol / import
  evidence, with per-file symbol lists, so an agent reads the important code
  first instead of alphabetically.
- **Native code evidence** — files, symbols, imports, chunks, and coverage
  scorecards extracted without any network or model calls.
- **Structural gate** — Sentrux rules and baseline comparison; structural
  regressions fail the run instead of hiding in a diff.
- **Hospital diagnosis** — a governance verdict (green / observe / surgery
  plan) computed from the admitted evidence, not from vibes.
- **Transactional artifacts** — every node's output lands in a
  content-addressed store, committed atomically; a run is either fully
  published or not indexed at all. Read runs back with
  `code-intel artifact query` (see [Install](#install)).

See a real run against a fresh `expressjs/express` clone:
[live demo report](https://2233admin.github.io/code-intel-pipeline/demo/).

## Install

Same on windows / macos / linux. **No PowerShell and no Rust toolchain needed.**
The stable release v0.7.1 has no install step. Full guide:
[docs/public-beta.md](docs/public-beta.md).

**Stable v0.7.1: download, verify, run in place.** Download
`code-intel-pipeline-v0.7.1-<platform>.zip` and its `.sha256` from
[GitHub Releases](https://github.com/2233admin/code-intel-pipeline/releases),
verify both (`gh attestation verify <zip> --repo 2233admin/code-intel-pipeline`;
see [docs/release-provenance-runbook.md](docs/release-provenance-runbook.md)),
extract, then run the binary directly (`code-intel.exe` on Windows) and read the
output back:

```bash
<extracted>/code-intel-pipeline/bin/code-intel <repo> --mode lite --artifact-root <root> --json
<extracted>/code-intel-pipeline/bin/code-intel artifact query --artifact-root <root> --repo <published-repo-key> --type code_evidence.agent_slice
```

Prerequisites: `git` and `rg` are required; `python` and `repowise` are
optional. `<published-repo-key>` and `<run-id>` come from `publication.path`
in the `--json` output (`<root>/<published-repo-key>/<run-id>`; the run is
committed when `run-complete.json` exists there). `lite` does not publish
`diagnosis.*`; use `--mode normal` to read `diagnosis.hospital-view`. A
committed run directory holds only `run-complete.json` and
`objects/sha256/<hash>`; there is no `report.json`. Exit codes and outcomes are
explained in the guide. This path was audited on Windows; it has not been run
on Linux or macOS.

**Current source / prerelease only (not in v0.7.1): `code-intel setup` and the
Skill bootstrap.** They need `v0.7.2-beta.7` or newer: pass
`--version v0.7.2-beta.7 --channel prerelease` (the bootstrap checks `--channel` even with `--version`, so pass both) or just `--channel prerelease`, and only when the user
explicitly asks for it (#405). The default stable channel is now rejected with
"This release predates `code-intel setup`". `code-intel status <repo>` and
`code-intel query <repo>` are likewise absent from v0.7.1. Review the plan
first, then run it, from `skills/code-intel-pipeline/`:

```bash
python scripts/bootstrap.py --repo-path "<repo>" --version v0.7.2-beta.7 --channel prerelease --dry-run --json
python scripts/bootstrap.py --repo-path "<repo>" --version v0.7.2-beta.7 --channel prerelease --json
```

The script downloads the GitHub Release ZIP for your platform, verifies its
SHA-256 and build provenance (`gh attestation verify` when `gh` 2.49+ is
present; otherwise it prints an explicit degradation notice instead of silently
skipping), extracts it, then runs the release's own `bin/code-intel setup`.
From an extracted package of that version you can run it by hand
(`--language zh|en` records the repo's report language):

```bash
<extracted>/code-intel-pipeline/bin/code-intel setup --repo-path <repo>
```

`code-intel setup` copies the binary to the data bin dir (Windows
`$env:LOCALAPPDATA\code-intel\bin`, macOS `~/Library/Application Support/code-intel/bin`,
Linux `${XDG_DATA_HOME:-~/.local/share}/code-intel/bin`); persists `PATH` and
`CODE_INTEL_HOME` (Windows: user environment; macOS/Linux: `~/.config/code-intel/env.sh`
sourced from your shell profile, skipped with `--no-modify-path`); installs the
Skill to `~/.agents/skills/code-intel-pipeline` and links it into
`~/.claude/skills` and `~/.codex/skills` (`--no-skills` to skip); then runs
doctor (`--skip-doctor` to skip). Missing external tools (`rg`, `git`, `python`,
`repowise`) are reported with an install command; setup never installs system
tools itself. Re-running it is idempotent.

**Build from source** (needs a Rust toolchain; current source, includes `setup`):

```bash
git clone https://github.com/2233admin/code-intel-pipeline.git
cd code-intel-pipeline
cargo build --release -p code-intel
mkdir -p bin && cp target/release/code-intel bin/    # Windows: code-intel.exe
bin/code-intel setup --repo-path ~/src/your-repo
```

Note: the pipeline needs full git history for lineage identity — run
`git fetch --unshallow` first if you cloned with `--depth 1`.

## Who this is for

- You run Claude Code / Codex / another coding agent on repositories it has
  never seen, and want it oriented before it edits.
- You want structural regressions (god files, complexity, coupling) to fail
  loudly during agent sessions instead of accumulating silently.
- You want evidence and diagnosis artifacts that survive the session, pinned
  by digest, instead of chat-scoped context.

## Status

Public beta. All production logic lives in the compiled Rust `code-intel`
binary, the primary entry point; PowerShell has been removed from the product.
Since v0.7.0 all three platforms ship prebuilt release ZIPs. Enhancement providers
(Repowise semantic docs, Understand Anything graphs, CodeNexus context) are
optional: when absent the run records them as skipped instead of faking
success.

The optional [CodeGraph adapter](docs/research/codegraph-adoption.md) adds snapshot-bound semantic queries, call relationships and advisory test candidates through `code-intel provider codegraph` and the MCP `get_code_context` tool. Explicit indexing/synchronization is required; partial graph results never authorize gates, and the native CodeNexus baseline remains independent.

## More

- [Full manual (中文)](README.md)
- [Public beta guide](docs/public-beta.md)
- [Repository layout & governance](docs/repository-layout.md)
- [Artifact data contract](docs/artifact-data-contract.md)

MIT © 2233admin
