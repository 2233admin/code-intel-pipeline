#requires -Version 7.2

<#
.SYNOPSIS
Shared frozen-source digest helpers for the compatibility retirement packets.

.DESCRIPTION
Every retirement packet freezes a set of source inputs and hashes them into a
snapshot identity. Both the generator (New-*RetirementPacket.ps1) and the
verifier (Test-*RetirementPacket.ps1) must compute that identity byte for byte
identically, so the computation lives here once and is dot-sourced by both.
Duplicating it in the mirror pair is what lets the two silently drift.

Why a projection instead of the whole registry file
---------------------------------------------------
Packets used to freeze the whole of orchestration/integrations.json. That file
also carries the toolchainDigests arrays, which are re-pinned whenever ANY
pinned Rust source file changes. So editing one unrelated Rust file invalidated
six packets at once and forced all of them to be regenerated in the same change
- 112 files, past the review-tool ceiling, which meant the governance mechanism
was blocking review of the very code it governs.

That coupling was never meaningful. A packet's staleness claim is about the
registry state of the capability it retires - is the replacement still declared,
with the same implementation, contract, effects and status - not about the
digests of source files it does not name. Freezing a projection over exactly
the integration entries the retirement concerns, plus the manifest's policy
header, keeps the claim the packet actually makes and drops the coupling it
never needed.

Canonicalisation
----------------
Deterministic by construction, and identical on both sides of the mirror:
  1. Select the named integration entries. Missing or duplicated ids are a hard
     error - an empty projection would be a silent governance hole.
  2. Order them by id with an ordinal (culture-invariant) sort, so the digest
     does not depend on the registry's file order or on the host locale.
  3. Emit { schema, policy, integrations } via ConvertTo-Json -Depth 40
     -Compress. Depth 40 matches the deepest packet writer; -Compress removes
     insignificant whitespace so formatting-only edits to the registry do not
     move the digest.
  4. SHA-256 over the UTF8 bytes, lowercase hex - the same shape as the
     Get-FileHash digests it sits beside in a frozen set.

Frozen-set entries
------------------
An entry is either a repository-relative file path, or a projection token:

    manifest-projection:<manifest path>#<integration id>[,<integration id>...]

Root resolution is shared too: crates/ and orchestration/ paths resolve against
the pipeline repository root, everything else against the PowerShell root
(legacy/).
#>

Set-StrictMode -Version Latest

function Get-FrozenDigestSha256 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)
    return ([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Text)))).ToLowerInvariant()
}

function Resolve-FrozenSourcePath {
    param(
        [Parameter(Mandatory = $true)][string]$Relative,
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$PipelineRepoRoot
    )
    $root = if ($Relative.StartsWith('crates/') -or $Relative.StartsWith('orchestration/')) { $PipelineRepoRoot } else { $RepoRoot }
    return (Join-Path $root $Relative)
}

function Get-FrozenManifestProjection {
    param(
        [Parameter(Mandatory = $true)][string]$ManifestPath,
        [Parameter(Mandatory = $true)][string[]]$IntegrationIds
    )
    if ($IntegrationIds.Count -eq 0) { throw "manifest projection requires at least one integration id" }
    $manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
    $ordered = [string[]]@($IntegrationIds)
    [Array]::Sort($ordered, [StringComparer]::Ordinal)
    $selected = @(foreach ($id in $ordered) {
        $matched = @($manifest.integrations | Where-Object { $_.id -eq $id })
        if ($matched.Count -ne 1) {
            throw "manifest projection requires exactly one '$id' integration in $ManifestPath (found $($matched.Count))"
        }
        $matched[0]
    })
    $projection = [ordered]@{
        schema = "code-intel-frozen-manifest-projection.v1"
        policy = $manifest.policy
        integrations = $selected
    }
    return Get-FrozenDigestSha256 (ConvertTo-Json -InputObject $projection -Depth 40 -Compress)
}

function Get-FrozenSourceDigest {
    param(
        [Parameter(Mandatory = $true)][string]$Entry,
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$PipelineRepoRoot
    )
    if ($Entry.StartsWith('manifest-projection:')) {
        $spec = $Entry.Substring('manifest-projection:'.Length)
        $parts = @($spec -split '#', 2)
        if ($parts.Count -ne 2) { throw "malformed frozen projection entry: $Entry" }
        $ids = @($parts[1] -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_.Length -gt 0 })
        if ([string]::IsNullOrWhiteSpace($parts[0]) -or $ids.Count -eq 0) { throw "malformed frozen projection entry: $Entry" }
        return Get-FrozenManifestProjection -ManifestPath (Resolve-FrozenSourcePath $parts[0] $RepoRoot $PipelineRepoRoot) -IntegrationIds $ids
    }
    return (Get-FileHash -LiteralPath (Resolve-FrozenSourcePath $Entry $RepoRoot $PipelineRepoRoot) -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-FrozenSourceIdentity {
    param(
        [Parameter(Mandatory = $true)][string[]]$FrozenSet,
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$PipelineRepoRoot
    )
    return Get-FrozenDigestSha256 ((@($FrozenSet | ForEach-Object {
        Get-FrozenSourceDigest -Entry $_ -RepoRoot $RepoRoot -PipelineRepoRoot $PipelineRepoRoot
    })) -join "`n")
}
