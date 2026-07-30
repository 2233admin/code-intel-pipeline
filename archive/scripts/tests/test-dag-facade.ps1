#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Contract {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

# Set-StrictMode turns an absent property into a terminating error, and manifest
# nodes only carry `diagnostic` when they failed.
function Get-Prop {
    param([object]$Object, [string]$Name)
    if ($null -eq $Object) { return $null }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
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
        # The facade throws on any nonzero coordinator exit, which hides the run
        # manifest. This test is about artifact routing and inventory parity, and
        # the `doctor` node it cannot configure probes the host toolchain, so
        # tolerate the throw here and judge the run from the manifest on disk.
        try {
            if ($Explicit) {
                & $runner -RepoPath $RepoPath -ArtifactRoot $ArtifactBase -DagCoordinate | Out-Null
            }
            else {
                & $runner -RepoPath $RepoPath -DagCoordinate | Out-Null
            }
        }
        catch {
            # Write-Host, not Write-Output: this function returns a value, and
            # anything on the output stream would be appended to it.
            Write-Host "DAG facade reported: $($_.Exception.Message)"
        }

        $repoName = Split-Path -Leaf $RepoPath
        $repoArtifactRoot = Join-Path $ArtifactBase $repoName
        $direct = @(Get-ChildItem -LiteralPath $repoArtifactRoot -Directory -Filter "*.dag-staging-*" -ErrorAction SilentlyContinue)
        Assert-Contract ($direct.Count -eq 1) "DAG run must be a direct child of the legacy repo artifact root."
        Assert-Contract (-not (Test-Path -LiteralPath (Join-Path $repoArtifactRoot $repoName))) "DAG facade duplicated the repository name in the artifact path."

        $manifestPath = Join-Path $direct[0].FullName "run-manifest.json"
        Assert-Contract (Test-Path -LiteralPath $manifestPath -PathType Leaf) "DAG facade produced no run manifest."
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json -ErrorAction Stop
        Assert-Contract ($manifest.schema -eq "code-intel-run-manifest.v1") "DAG facade emitted the wrong schema."

        # A structural gate failure on a fixture holding one README is the
        # regression this lane exists to catch, so it is asserted unconditionally.
        # The whole-run outcome additionally depends on `doctor`, which fails on a
        # runner without the full toolchain; `Authoritative self-scan (release
        # gate parity)` covers run-level exit 0 with the doctor configured.
        $hospital = Get-Prop $manifest.nodes "diagnosis.hospital"
        Assert-Contract ($null -ne $hospital) "DAG facade ran no diagnosis.hospital node."
        Assert-Contract ((Get-Prop $hospital "status") -eq "succeeded") "diagnosis.hospital did not succeed: $(Get-Prop $hospital 'status') $(Get-Prop $hospital 'diagnostic')"
        Assert-Contract ((Get-Prop $hospital "verdict") -eq "pass") "diagnosis.hospital did not pass: $(Get-Prop $hospital 'diagnostic')"

        if ((Get-Prop (Get-Prop $manifest.nodes "doctor") "status") -eq "succeeded") {
            Assert-Contract ($manifest.outcome -eq "completed") "DAG facade did not complete: $($manifest.outcome)"
        }
        else {
            $nonGreen = @($manifest.nodes.PSObject.Properties |
                Where-Object { (Get-Prop $_.Value "status") -ne "succeeded" } |
                ForEach-Object { "$($_.Name)=$(Get-Prop $_.Value 'status')/$(Get-Prop $_.Value 'diagnostic')" }) -join "; "
            Write-Host "Host toolchain is incomplete, so run-level outcome is not asserted. Non-green nodes: $nonGreen"
        }

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

# $LASTEXITCODE still carries the coordinator's exit code, and the CI `pwsh`
# shell exits with it. Every assertion above passed, so say so explicitly rather
# than failing the step on a tolerated node's exit code.
exit 0
