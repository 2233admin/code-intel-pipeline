# Public Beta Guide

## Supported surface

Existing public beta releases (v0.6.0 and earlier) ship a Windows ZIP only.
From the next tag onward, every release ships three ZIPs:
`code-intel-pipeline-<tag>-windows.zip`, `code-intel-pipeline-<tag>-macos.zip`,
and `code-intel-pipeline-<tag>-linux.zip`. The stable entrypoint is the
packaged `bin/code-intel.exe` on Windows and `bin/code-intel` on macOS/Linux;
`archive/code-intel.ps1` is the PowerShell recovery launcher and
`archive/invoke-code-intel.ps1` is a v0.x compatibility forwarder. PowerShell 7.2+
(`pwsh`) is required on every platform, including macOS and Linux, because the
installer and launchers are implemented in PowerShell. A release package must
not require Cargo, a source-tree `target/` directory, or a local Rust
installation.

The beta core covers repository inventory, Sentrux structural evidence,
transactional artifact publication, failure classification, and the
Understanding/Hospital reports. Optional providers enrich those reports but do
not redefine whether the core pipeline is usable.

| Capability | Beta status | Missing-provider behavior |
| --- | --- | --- |
| Compiled `code-intel` entrypoint and doctor | Core | Fail with an actionable local error |
| PowerShell recovery launcher | Recovery | Verify and repair from the official GitHub release |
| `rg` inventory | Core | Fail with an actionable local error |
| Sentrux structural evidence | Core | Report real gate/check failure |
| Transactional run commit and reports | Core | Fail closed; incomplete runs are not indexed |
| Repowise semantic memory/docs | Optional, included by default | Record unavailable/skipped; `-SkipRepowise` bypasses it |
| Understand Anything graph | Optional | Record `graph_missing` / manual action |
| CodeNexus context | Optional compatibility adapter | Record note and continue |
| Repomix pack | Optional | Record unavailable/skipped and continue |
| Model assistance channels | Optional | Emit a request/dossier or explicit provider outcome |
| Runtime/CI and file-boundary evidence | Optional | Preserve the absence as evidence state |

`crates/code-nexus-lite` is incubated source and is not a compiled workspace
member or a promised binary in this beta package. The supported CodeNexus
surface is the optional compatibility adapter and its artifact contract.

## Install and verify

1. Download the release ZIP for your platform (`windows`, `macos`, or `linux`)
   and its `.sha256` file.
2. Verify the checksum with `Get-FileHash -Algorithm SHA256`.
3. Verify the build provenance attestation (requires `gh` 2.49+). The same
   check applies to all three platform ZIPs:

```powershell
gh attestation verify .\code-intel-pipeline-<tag>-<platform>.zip --repo 2233admin/code-intel-pipeline
```

   The release workflow signs every ZIP with GitHub Artifact Attestations
   (`actions/attest-build-provenance`); a failed verification means the asset
   was not produced by this repository's release workflow and must not be run.
4. Extract the ZIP to a writable directory.
5. Run (Windows):

```powershell
.\bin\code-intel.exe C:\path\to\repo
```

   On macOS/Linux (`chmod` is only needed if your unzip tool did not preserve
   the execute bit; `bootstrap.py` restores it automatically):

```bash
chmod +x ./bin/code-intel
./bin/code-intel ~/path/to/repo
```

Use `--mode lite` or `--mode full` only when the default `normal` profile is
not appropriate. Optional providers remain in the orchestration plan and are
used when available.

## Known limits

- Release packages published up to and including v0.6.0 are Windows-only;
  installing them on macOS/Linux is not supported. Releases from the next tag
  onward ship windows/macos/linux ZIPs, and macOS/Linux still require
  PowerShell 7.2+ (`pwsh`) for the installer and launchers.
- External providers can be unavailable, rate-limited, or unconfigured. Their
  outcomes are reported rather than rewritten as local success.
- Understand Anything graph generation still depends on its host integration.
- Compatibility facades remain shipped while retirement evidence and approval
  chains are incomplete; retirement is not a beta-core prerequisite.
- The beta does not promise the incubated CodeNexus Rust worker binary.

## Upgrade and rollback

Release ZIPs are self-contained. `archive/code-intel.ps1 -Update` installs a verified
official stable release while retaining a verified local fallback. Manual
rollback means switching back to the previous extracted directory. Generated
artifacts live outside the package under the platform Code Intel data root.
