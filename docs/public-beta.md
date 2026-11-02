# Public Beta Guide

## Supported surface

Releases up to and including v0.6.0 shipped a Windows ZIP only. Every release
from v0.7.0-beta.2 onward ships three ZIPs:
`code-intel-pipeline-<tag>-windows.zip`, `code-intel-pipeline-<tag>-macos.zip`,
and `code-intel-pipeline-<tag>-linux.zip`. The stable entrypoint is the
packaged `bin/code-intel.exe` on Windows and `bin/code-intel` on macOS/Linux.
Nothing in the stable path needs PowerShell. A release package must not require Cargo, a source-tree
`target/` directory, or a local Rust installation.

The beta core covers repository inventory, Sentrux structural evidence,
transactional artifact publication, failure classification, and the
Understanding/Hospital reports. Optional providers enrich those reports but do
not redefine whether the core pipeline is usable.

| Capability | Beta status | Missing-provider behavior |
| --- | --- | --- |
| Compiled `code-intel` entrypoint and doctor | Core | Fail with an actionable local error |
| `rg` inventory | Core | Fail with an actionable local error |
| Sentrux structural evidence | Core | Report real gate/check failure |
| Transactional run commit and reports | Core | Fail closed; incomplete runs are not indexed |
| Repowise semantic memory/docs | Optional, included by default | Record unavailable/skipped; `--no-require-repowise` relaxes the doctor check |
| Understand Anything graph | Optional | Record `graph_missing` / manual action |
| CodeNexus context | Optional compatibility adapter | Record note and continue |
| Repomix pack | Optional | Record unavailable/skipped and continue |
| Model assistance channels | Optional | Emit a request/dossier or explicit provider outcome |
| Runtime/CI and file-boundary evidence | Optional | Preserve the absence as evidence state |

`crates/code-nexus-lite` is incubated source and is not a compiled workspace
member or a promised binary in this beta package. The supported CodeNexus
surface is the optional compatibility adapter and its artifact contract.

## Quick start: stable v0.7.1 (download, verify, run locally)

This is the path for a first run. Every command below exists in the stable
release `v0.7.1` (verified against the `v0.7.1` tag, not executed on a
packaged binary for this guide). The stable package needs no install step: you
unzip it and run the binary in place. The audit of this path was run on Windows;
it has not been run on Linux or macOS.

Prerequisites for the native binary: `git` and `rg` (ripgrep) are required;
`python` and `repowise` are optional. Nothing here needs PowerShell or a Rust
toolchain.

1. Download `code-intel-pipeline-v0.7.1-<platform>.zip` and its `.sha256` from
   [GitHub Releases](https://github.com/2233admin/code-intel-pipeline/releases)
   (`<platform>` is `windows`, `macos`, or `linux`).
2. Verify the checksum (`sha256sum -c`, `shasum -a 256 -c`, or
   `certutil -hashfile <zip> SHA256` on Windows).
3. Verify the build provenance attestation (requires `gh` 2.49+). The same
   check applies to all three platform ZIPs:

```bash
gh attestation verify ./code-intel-pipeline-v0.7.1-<platform>.zip --repo 2233admin/code-intel-pipeline
```

   The release workflow signs every ZIP with GitHub Artifact Attestations
   (`actions/attest-build-provenance`); a failed verification means the asset
   was not produced by this repository's release workflow and must not be run.
   Official (non-beta, non-rc) tags are additionally SSH-signed; run
   `git verify-tag <tag>` against `.github/allowed_signers` — see
   [RELEASE_SIGNING.md](RELEASE_SIGNING.md).
4. Extract the ZIP to a writable directory. On macOS and Linux, some
   extractors drop the execute bit, so run
   `chmod +x <extracted>/code-intel-pipeline/bin/code-intel` first.
5. Run the analysis locally with the offline `lite` mode (`code-intel.exe` on
   Windows). `<repo>` must be a git checkout with full history (`git fetch
   --unshallow` if you cloned with `--depth 1`); the run only reads it:

```bash
<extracted>/code-intel-pipeline/bin/code-intel <repo> --mode lite --artifact-root <root> --json
```

6. Read the output back from the published run:

```bash
<extracted>/code-intel-pipeline/bin/code-intel artifact query --artifact-root <root> --repo <published-repo-key> --type code_evidence.agent_slice
```

### Reading the result

- `<published-repo-key>` and `<run-id>` come from `publication.path` in the
  `--json` output: the path is `<root>/<published-repo-key>/<run-id>`. Use them
  only when the run was committed. In v0.7.1 the primary `--json` output has no
  `publication.status` field: the run is committed when
  `publication.marker` (`run-complete.json` in that directory) exists. (The
  `run execute` result does carry `publication.status == "committed"`.)
- Exit code and `outcome`: `0` is `completed`; `10` is `domain_failed` (a
  verdict such as a structural gate failed); `20` is `domain_unknown` (a
  verdict could not be decided); `70` is `process_failed` or `incomplete` (a
  tool or the environment failed, or the run did not finish); `64` is a usage
  error such as a bad `--mode`. `failureNode` and `diagnostic` name the first
  failing node.
- A committed run directory contains only `run-complete.json` and
  `objects/sha256/<hash>`. There is no `report.json`; read evidence through
  `artifact query`, not by opening files.
- `lite` does not publish `diagnosis.*` artifacts. To read the hospital view,
  rerun with `--mode normal` and query it:

```bash
<extracted>/code-intel-pipeline/bin/code-intel artifact query --artifact-root <root> --repo <published-repo-key> --type diagnosis.hospital-view --limit 1
```

The field contract of `artifact query` results is in
[evidence-query.md](evidence-query.md).

### Current source / prerelease only (not in v0.7.1)

The commands in this block do not exist in the stable `v0.7.1` binary. Do not
use them with the stable ZIP.

- `code-intel setup` (copies the binary to the per-user data directory,
  persists `PATH` and `CODE_INTEL_HOME`, links the Skill, runs doctor) needs
  `v0.7.2-beta.7` or newer.
- The Skill's `scripts/bootstrap.py` ends by running the release's own
  `bin/code-intel setup`, so it has the same requirement. Its default stable
  channel is currently rejected with "This release predates `code-intel
  setup`". Use `--version v0.7.2-beta.7 --channel prerelease` (the bootstrap checks `--channel` even with `--version`, so pass both) or just `--channel prerelease`, and only
  when the user explicitly asks for it.
- `code-intel status <repo>` and `code-intel query <repo> ...` exist only in
  current source builds and newer prereleases.

With such a build, review the plan first, then install:

```bash
python scripts/bootstrap.py --repo-path "<repo>" --version v0.7.2-beta.7 --channel prerelease --dry-run --json   # review the plan
python scripts/bootstrap.py --repo-path "<repo>" --version v0.7.2-beta.7 --channel prerelease --json
```

or, from an extracted `v0.7.2-beta.7`-or-newer package:

```bash
<extracted>/code-intel-pipeline/bin/code-intel setup --repo-path <repo>
```

`setup` copies the binary into the per-user data bin directory, persists
`PATH` and `CODE_INTEL_HOME`, installs the Skill into `~/.agents/skills` and
links it into `~/.claude/skills` and `~/.codex/skills`, then runs doctor.
`--no-modify-path`, `--no-skills` and `--skip-doctor` skip those steps; add
`--language zh|en` to record the repo's report language. Missing external
tools are reported with an install command; setup never installs system
tools. Re-running it is idempotent. After `setup` and a new terminal,
`code-intel <repo>` works from any directory.

Use `--mode normal` or `--mode full` only when `lite` is not enough. Optional
providers remain in the orchestration plan and are used when available.

## Known limits

- Release packages published up to and including v0.6.0 are Windows-only;
  installing them on macOS/Linux is not supported. Releases from
  v0.7.0-beta.2 onward ship windows/macos/linux ZIPs.
- External providers can be unavailable, rate-limited, or unconfigured. Their
  outcomes are reported rather than rewritten as local success.
- Understand Anything graph generation still depends on its host integration.
- The beta does not promise the incubated CodeNexus Rust worker binary.

## Upgrade and rollback

Release ZIPs are self-contained. With the stable v0.7.1 path, upgrading means
downloading, verifying and extracting the newer ZIP and running its
`bin/code-intel` directly; rolling back means running the previous extracted
directory. Releases `v0.7.2-beta.7` or newer additionally support
`code-intel setup` and the Skill bootstrap (see the prerelease block above);
`setup` is idempotent and is re-run from the extracted directory you want to
use. Generated artifacts live outside the package, under the `--artifact-root`
you passed (or the platform Code Intel data root by default).
