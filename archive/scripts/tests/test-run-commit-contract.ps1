#requires -Version 7.2

param([string]$Root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../..")))

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$temp = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-a07-facade-" + [guid]::NewGuid().ToString("N"))
try {
    $source = Join-Path $temp "source"
    $artifactRoot = Join-Path $temp "artifacts"
    $authority = Join-Path $artifactRoot "repo"
    New-Item -ItemType Directory -Path $source, $authority | Out-Null

    $snapshot = "a" * 64
    $inventoryBytes = [Text.UTF8Encoding]::new($false).GetBytes("portable evidence`n")
    $inventoryDigest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($inventoryBytes)).ToLowerInvariant()
    $inventoryRelative = "objects/sha256/$inventoryDigest"
    $inventoryPath = Join-Path $source ($inventoryRelative -replace '/', [IO.Path]::DirectorySeparatorChar)
    New-Item -ItemType Directory -Path (Split-Path -Parent $inventoryPath) -Force | Out-Null
    [IO.File]::WriteAllBytes($inventoryPath, $inventoryBytes)

    $inventoryRef = [ordered]@{
        schema = "code-intel-artifact-ref.v1"
        artifactSchema = "code-intel-file-inventory.v1"
        type = "inventory.files"
        path = $inventoryRelative
        sha256 = $inventoryDigest
        consumedSnapshotIdentity = $snapshot
    }
    $manifest = [ordered]@{
        schema = "code-intel-run-manifest.v1"
        runIdentity = "dag-v1:aabb"
        snapshotIdentity = $snapshot
        outcome = "completed"
        nodes = [ordered]@{
            inventory = [ordered]@{
                status = "succeeded"
                verdict = "pass"
                artifacts = @($inventoryRef)
            }
        }
    }
    $manifestBytes = [Text.UTF8Encoding]::new($false).GetBytes(($manifest | ConvertTo-Json -Depth 12 -Compress))
    $manifestDigest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($manifestBytes)).ToLowerInvariant()
    $manifestRelative = "objects/sha256/$manifestDigest"
    $manifestPath = Join-Path $source ($manifestRelative -replace '/', [IO.Path]::DirectorySeparatorChar)
    [IO.File]::WriteAllBytes($manifestPath, $manifestBytes)
    $manifestRef = [ordered]@{
        schema = "code-intel-artifact-ref.v1"
        artifactSchema = "code-intel-run-manifest.v1"
        type = "run.manifest"
        path = $manifestRelative
        sha256 = $manifestDigest
        consumedSnapshotIdentity = $snapshot
    }
    $manifestRefPath = Join-Path $temp "manifest-ref.json"
    [IO.File]::WriteAllText($manifestRefPath, ($manifestRef | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))

    & (Join-Path $Root "run-code-intel.ps1") `
        -RunCommitSourceRoot $source `
        -RunCommitAuthorityRoot $authority `
        -RunCommitManifestRef $manifestRefPath `
        -RunCommitFinalName "published" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "run commit facade exited $LASTEXITCODE" }

    $markerPath = Join-Path $authority "published/run-complete.json"
    $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
    if ($marker.schema -ne "code-intel-run-commit.v1" -or
        $marker.runIdentity -ne "dag-v1:aabb" -or
        $marker.snapshotIdentity -ne $snapshot -or
        $marker.manifest.sha256 -ne $manifestDigest) {
        throw "facade published an incoherent A07 marker"
    }
    if ($null -ne $marker.PSObject.Properties["generatedAt"] -or
        $null -ne $marker.PSObject.Properties["reportSha256"]) {
        throw "facade published the legacy marker shape"
    }

    $index = Join-Path $artifactRoot "index.md"
    & (Join-Path $Root "update-code-intel-index.ps1") -ArtifactRoot $artifactRoot -OutputPath $index | Out-Null
    if (-not (Get-Content -LiteralPath $index -Raw).Contains("published")) {
        throw "A08 index did not admit a valid new A07 marker"
    }
    Write-Output "run.commit facade contract: pass; A08 committed-only admission: pass"
}
finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
