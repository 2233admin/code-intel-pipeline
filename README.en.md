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
  published or not indexed at all. Read runs back with `code-intel resume
  --repo <path>`.

See a real run against a fresh `expressjs/express` clone:
[live demo report](https://2233admin.github.io/code-intel-pipeline/demo/).

## Install

**Windows** — download the ZIP from the
[latest release](https://github.com/2233admin/code-intel-pipeline/releases/latest)
and follow its notes, or install from source:

```powershell
git clone https://github.com/2233admin/code-intel-pipeline.git
cd code-intel-pipeline
.\legacy\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo -RepairSkillLinks -InstallMissing
code-intel C:\path\to\your\repo
```

**Coding agents (Codex / Claude Code)** — install the skill package with
`$skill-installer` from
[`skills/code-intel-pipeline`](skills/code-intel-pipeline); it downloads the
stable release, and when `gh` 2.49+ is present first verifies the release
ZIP's [GitHub Artifact Attestation](https://cli.github.com/manual/gh_attestation_verify)
(proof the workflow of this repository produced it, not just that the bytes
arrived intact) before falling back to the published SHA-256 checksum. When
`gh` is missing or too old, the installer prints an explicit degradation
notice instead of silently skipping the attestation check. Verify the same
guarantee by hand with
`gh attestation verify <zip> --repo 2233admin/code-intel-pipeline` — see
[docs/release-provenance-runbook.md](docs/release-provenance-runbook.md) for
the full command and a recorded verification run.

**macOS / Linux (source build; requires PowerShell 7.2+, Rust toolchain, ripgrep):**

```bash
git clone https://github.com/2233admin/code-intel-pipeline.git
cd code-intel-pipeline
pwsh ./legacy/install-code-intel-pipeline.ps1 -RepoPath ~/src/your-repo -InstallMissing
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

Windows PowerShell surface is in public beta; production logic is moving to
Rust (the compiled `code-intel` binary is the primary entry point). macOS /
Linux are source-build only until the next release. Enhancement providers
(Repowise semantic docs, Understand Anything graphs, CodeNexus context) are
optional: when absent the run records them as skipped instead of faking
success.

## More

- [Full manual (中文)](README.md)
- [Public beta guide](docs/public-beta.md)
- [Repository layout & governance](docs/repository-layout.md)
- [Artifact data contract](docs/artifact-data-contract.md)

MIT © 2233admin
