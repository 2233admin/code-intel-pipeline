param(
    [Parameter(Mandatory)]
    [ValidateSet("competitive-intelligence", "react-diagnostics")]
    [string]$Feature,

    [Parameter(Mandatory)]
    [ValidateSet("prepare", "status", "run", "build")]
    [string]$Operation,

    [string]$RepoPath = "",
    [Parameter(Mandatory)]
    [string]$ArtifactDir,
    [string]$Request = "",
    [string]$RouteResult = "",
    [long]$EvaluatedAt = -1,
    [long]$MaxAgeSeconds = 86400
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Find-CodeIntelCli {
    $exe = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
    foreach ($candidate in @(
        (Join-Path $PSScriptRoot "target/debug/$exe"),
        (Join-Path $PSScriptRoot "target/release/$exe"),
        (Join-Path $PSScriptRoot "bin/$exe")
    )) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    $command = Get-Command code-intel -ErrorAction SilentlyContinue
    if ($null -ne $command) { return $command.Source }
    throw "code-intel CLI is unavailable; run cargo build -p code-intel"
}

function Invoke-Provider([string]$Provider, [string]$ProviderOperation) {
    $arguments = @{
        Provider = $Provider
        Operation = $ProviderOperation
        ArtifactDir = $ArtifactDir
        MaxAgeSeconds = $MaxAgeSeconds
    }
    if (-not [string]::IsNullOrWhiteSpace($RepoPath)) { $arguments.RepoPath = $RepoPath }
    if (-not [string]::IsNullOrWhiteSpace($Request)) { $arguments.Request = $Request }
    if ($EvaluatedAt -ge 0) { $arguments.EvaluatedAt = $EvaluatedAt }
    return & (Join-Path $PSScriptRoot "Invoke-EvidenceProvider.ps1") @arguments | ConvertFrom-Json
}

function Build-FeatureReport([string]$DefaultRoute) {
    $route = if ([string]::IsNullOrWhiteSpace($RouteResult)) { $DefaultRoute } else { $RouteResult }
    if ([string]::IsNullOrWhiteSpace($route) -or -not (Test-Path -LiteralPath $route -PathType Leaf)) {
        throw "A route result is required before building $Feature"
    }
    $cli = Find-CodeIntelCli
    $raw = @(
        & $cli feature --action Build --feature $Feature --request $route --artifact-root $ArtifactDir --json
    )
    if ($LASTEXITCODE -ne 0) { throw "$Feature report build failed" }
    return ($raw -join "`n") | ConvertFrom-Json
}

$valid = ($Feature -eq "competitive-intelligence" -and $Operation -in @("prepare", "status", "build")) -or
    ($Feature -eq "react-diagnostics" -and $Operation -in @("run", "build"))
if (-not $valid) { throw "Operation $Operation is not valid for Beta feature $Feature" }

$result = switch ("$Feature/$Operation") {
    "competitive-intelligence/prepare" { Invoke-Provider "compete" "prepare" }
    "competitive-intelligence/status" { Invoke-Provider "compete" "status" }
    "competitive-intelligence/build" {
        if ([string]::IsNullOrWhiteSpace($RouteResult)) {
            Invoke-Provider "compete" "adapt" | Out-Null
        }
        Build-FeatureReport (Join-Path $ArtifactDir "compete-route-result.json")
    }
    "react-diagnostics/run" {
        Invoke-Provider "react-doctor" "scan" | Out-Null
        Build-FeatureReport (Join-Path $ArtifactDir "react-doctor-route-result.json")
    }
    "react-diagnostics/build" {
        if ([string]::IsNullOrWhiteSpace($RouteResult)) {
            Invoke-Provider "react-doctor" "adapt" | Out-Null
        }
        Build-FeatureReport (Join-Path $ArtifactDir "react-doctor-route-result.json")
    }
}

$result | ConvertTo-Json -Depth 40
