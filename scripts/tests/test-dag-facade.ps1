#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Contract {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Invoke-DagFacadeCase {
    param(
        [string]$RepoPath,
        [string]$ArtifactBase,
        [bool]$Explicit
    )

    $runner = Join-Path ([System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))) "run-code-intel.ps1"
    $previous = $env:CODE_INTEL_ARTIFACT_ROOT
    try {
        $env:CODE_INTEL_ARTIFACT_ROOT = $ArtifactBase
        if ($Explicit) {
            $raw = & $runner -RepoPath $RepoPath -ArtifactRoot $ArtifactBase -DagCoordinate
        }
        else {
            $raw = & $runner -RepoPath $RepoPath -DagCoordinate
        }
        Assert-Contract ($LASTEXITCODE -eq 0) "DAG facade exited nonzero."
        $manifest = ($raw -join "`n") | ConvertFrom-Json -ErrorAction Stop
        Assert-Contract ($manifest.schema -eq "code-intel-run-manifest.v1") "DAG facade emitted the wrong schema."
        Assert-Contract ($manifest.outcome -eq "completed") "DAG facade did not complete."

        $repoName = Split-Path -Leaf $RepoPath
        $repoArtifactRoot = Join-Path $ArtifactBase $repoName
        $direct = @(Get-ChildItem -LiteralPath $repoArtifactRoot -Directory -Filter "*.dag-staging-*" -ErrorAction SilentlyContinue)
        Assert-Contract ($direct.Count -eq 1) "DAG run must be a direct child of the legacy repo artifact root."
        Assert-Contract (-not (Test-Path -LiteralPath (Join-Path $repoArtifactRoot $repoName))) "DAG facade duplicated the repository name in the artifact path."
        return [pscustomobject]@{
            manifest = $manifest
            files = [System.IO.File]::ReadAllBytes((Join-Path $direct[0].FullName "inventory.rg\files.txt"))
        }
    }
    finally {
        $env:CODE_INTEL_ARTIFACT_ROOT = $previous
    }
}

$root = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-dag-facade-" + [guid]::NewGuid().ToString("N"))
$repo = Join-Path $root "repo & 文"
$explicitRoot = Join-Path $root "explicit artifacts"
$defaultRoot = Join-Path $root "default artifacts"
New-Item -ItemType Directory -Path $repo | Out-Null
Set-Content -LiteralPath (Join-Path $repo "README & 文.md") -Value "fixture" -NoNewline -Encoding utf8
try {
    $explicit = Invoke-DagFacadeCase -RepoPath $repo -ArtifactBase $explicitRoot -Explicit $true
    $default = Invoke-DagFacadeCase -RepoPath $repo -ArtifactBase $defaultRoot -Explicit $false
    Assert-Contract ([System.Linq.Enumerable]::SequenceEqual([byte[]]$explicit.files, [byte[]]$default.files)) "Explicit/default facade routes changed inventory bytes."
    Write-Output "DAG facade path/parity passed: explicit, default, special repository name"
}
finally {
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
