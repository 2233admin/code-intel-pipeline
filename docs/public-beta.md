# Public Beta Guide

## Supported surface

Releases up to and including v0.6.0 shipped a Windows ZIP only. Every release
from v0.7.0-beta.2 onward ships three ZIPs:
`code-intel-pipeline-<tag>-windows.zip`, `code-intel-pipeline-<tag>-macos.zip`,
and `code-intel-pipeline-<tag>-linux.zip`. The stable entrypoint is the
packaged `bin/code-intel.exe` on Windows and `bin/code-intel` on macOS/Linux.
Installation needs no PowerShell: `code-intel setup` finishes the install on
every platform. A release package must not require Cargo, a source-tree
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

## Install and verify

Recommended: let the agent Skill do it. From `skills/code-intel-pipeline/`:

```bash
python scripts/bootstrap.py --repo-path "<repo>" --dry-run --json   # review the plan
python scripts/bootstrap.py --repo-path "<repo>" --json
```

Options: `--version <tag>`, `--channel prerelease`. The script downloads the
release ZIP for your platform, verifies its SHA-256 and build provenance
(`gh attestation verify` when `gh` 2.49+ is present), extracts it, and runs the
release's own `bin/code-intel setup`.

Manual install:

1. Download the release ZIP for your platform (`windows`, `macos`, or `linux`)
   and its `.sha256` file from GitHub Releases.
2. Verify the checksum (`sha256sum -c`, `shasum -a 256 -c`, or
   `certutil -hashfile <zip> SHA256` on Windows).
3. Verify the build provenance attestation (requires `gh` 2.49+). The same
   check applies to all three platform ZIPs:

```bash
gh attestation verify ./code-intel-pipeline-<tag>-<platform>.zip --repo 2233admin/code-intel-pipeline
```

   The release workflow signs every ZIP with GitHub Artifact Attestations
   (`actions/attest-build-provenance`); a failed verification means the asset
   was not produced by this repository's release workflow and must not be run.
   Official (non-beta, non-rc) tags are additionally SSH-signed; run
   `git verify-tag <tag>` against `.github/allowed_signers` — see
   [RELEASE_SIGNING.md](RELEASE_SIGNING.md).
4. Extract the ZIP to a writable directory.
5. Run setup from the extracted package (`code-intel.exe` on Windows):

```bash
<extracted>/code-intel-pipeline/bin/code-intel setup --repo-path <repo>
```

   `setup` copies the binary into the per-user data bin directory, persists
   `PATH` and `CODE_INTEL_HOME`, installs the Skill into `~/.agents/skills` and
   links it into `~/.claude/skills` and `~/.codex/skills`, then runs doctor.
   `--no-modify-path`, `--no-skills` and `--skip-doctor` skip those steps; add
   `--language zh|en` to record the repo's report language. Missing external
   tools are reported with an install command; setup never installs system
   tools. Re-running it is idempotent.

Then, in a new terminal:

```bash
code-intel ~/path/to/repo
```

Use `--mode lite` or `--mode full` only when the default `normal` profile is
not appropriate. Optional providers remain in the orchestration plan and are
used when available.

## Known limits

- Release packages published up to and including v0.6.0 are Windows-only;
  installing them on macOS/Linux is not supported. Releases from
  v0.7.0-beta.2 onward ship windows/macos/linux ZIPs.
- External providers can be unavailable, rate-limited, or unconfigured. Their
  outcomes are reported rather than rewritten as local success.
- Understand Anything graph generation still depends on its host integration.
- The beta does not promise the incubated CodeNexus Rust worker binary.

## Upgrade and rollback

Release ZIPs are self-contained. To upgrade, run the Skill bootstrap again (or
download the newer ZIP and re-run its `bin/code-intel setup`); setup is
idempotent. Manual rollback means re-running `setup` from the previous
extracted directory. Generated
artifacts live outside the package under the platform Code Intel data root.
