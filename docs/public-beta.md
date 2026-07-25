# Public Beta Guide

## Supported surface

The public beta ships as a Windows ZIP and uses PowerShell 7.2 or newer. The
stable entrypoint is `invoke-code-intel.ps1`; the packaged Rust core is
`bin/code-intel.exe`. A release package must not require Cargo, a source-tree
`target/` directory, or a local Rust installation.

The beta core covers repository inventory, Sentrux structural evidence,
transactional artifact publication, failure classification, and the
Understanding/Hospital reports. Optional providers enrich those reports but do
not redefine whether the core pipeline is usable.

| Capability | Beta status | Missing-provider behavior |
| --- | --- | --- |
| Stable PowerShell entrypoint and doctor | Core | Fail with an actionable local error |
| `code-intel.exe` policy/artifact core | Core | Fail; packaged binary is required |
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

1. Download the release ZIP and its `.sha256` file.
2. Verify the checksum with `Get-FileHash -Algorithm SHA256`.
3. Extract the ZIP to a writable directory.
4. Run:

```powershell
.\invoke-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal -SkipRepowise
```

Remove `-SkipRepowise` when Repowise is installed and semantic memory is
desired. Optional providers remain in the default orchestration plan so a
configured machine gets the richer result without using a different product
path.

## Known limits

- The beta release package is Windows-only.
- External providers can be unavailable, rate-limited, or unconfigured. Their
  outcomes are reported rather than rewritten as local success.
- Understand Anything graph generation still depends on its host integration.
- Compatibility facades remain shipped while retirement evidence and approval
  chains are incomplete; retirement is not a beta-core prerequisite.
- The beta does not promise the incubated CodeNexus Rust worker binary.

## Upgrade and rollback

Release ZIPs are self-contained. Extract a new beta beside the previous one,
run the package smoke test, and then switch the caller's path. Rollback means
switching the path back to the previous extracted directory; do not overwrite
the old directory in place. Generated artifacts live outside the package under
the platform Code Intel data root.
