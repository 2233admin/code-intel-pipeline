#requires -Version 7.2

param(
    [string]$Repo = "",
    [string]$RepoPath = "",
    [string[]]$Repos = @(),
    [switch]$All,

    [string]$Config = "",

    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto",

    [ValidateSet("lite", "normal", "full")]
    [string]$Mode = "normal",

    [switch]$RepowiseDocs,
    [switch]$SaveSentruxBaseline,
    [switch]$AutoSaveMissingSentruxBaseline,
    [switch]$RequireUnderstandGraph,
    [switch]$NoIndexUpdate
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
if ([string]::IsNullOrWhiteSpace($Config)) {
    $Config = Join-Path $root "pipeline.config.json"
}
$doctor = Join-Path $root "check-code-intel-tools.ps1"
$runner = Join-Path $root "run-code-intel.ps1"
$indexer = Join-Path $root "update-code-intel-index.ps1"

function Get-JsonProperty {
    param(
        [object]$Object,
        [string]$Name
    )

    if ($null -eq $Object) { return $null }
    $prop = $Object.PSObject.Properties[$Name]
    if ($null -eq $prop) { return $null }
    return $prop.Value
}

function Invoke-OneRepo {
    param(
        [string]$RepoName,
        [string]$DirectRepoPath = ""
    )

    $label = if (-not [string]::IsNullOrWhiteSpace($DirectRepoPath)) { $DirectRepoPath } else { $RepoName }
    Write-Host "Code intel invoke: doctor $label"
    $global:LASTEXITCODE = 0
    if (-not [string]::IsNullOrWhiteSpace($DirectRepoPath)) {
        & $doctor -Config $Config -RepoPath $DirectRepoPath -Platform $Platform
    }
    else {
        & $doctor -Config $Config -Repo $RepoName -Platform $Platform
    }
    if ($LASTEXITCODE -ne 0) {
        return [pscustomobject][ordered]@{
            repo = $label
            ok = $false
            stage = "doctor"
            exitCode = $LASTEXITCODE
        }
    }

    Write-Host "Code intel invoke: pipeline $label"
    if ($RepowiseDocs -or $SaveSentruxBaseline -or $AutoSaveMissingSentruxBaseline -or $RequireUnderstandGraph) {
        $invokeParams = @{
            Config = $Config
            Mode = $Mode
            Platform = $Platform
        }
        if (-not [string]::IsNullOrWhiteSpace($DirectRepoPath)) { $invokeParams.RepoPath = $DirectRepoPath } else { $invokeParams.Repo = $RepoName }
        if ($RepowiseDocs) { $invokeParams.RepowiseDocs = $true }
        if ($SaveSentruxBaseline) { $invokeParams.SaveSentruxBaseline = $true }
        if ($AutoSaveMissingSentruxBaseline) { $invokeParams.AutoSaveMissingSentruxBaseline = $true }
        if ($RequireUnderstandGraph) { $invokeParams.RequireUnderstandGraph = $true }
        & $runner @invokeParams
    }
    else {
        if (-not [string]::IsNullOrWhiteSpace($DirectRepoPath)) {
            & $runner -Config $Config -RepoPath $DirectRepoPath -Mode $Mode -Platform $Platform
        }
        else {
            & $runner -Config $Config -Repo $RepoName -Mode $Mode -Platform $Platform
        }
    }

    $code = $LASTEXITCODE
    return [pscustomobject][ordered]@{
        repo = $label
        ok = $code -eq 0
        stage = "pipeline"
        exitCode = $code
    }
}

if (-not (Test-Path -LiteralPath $doctor -PathType Leaf)) {
    throw "Doctor script missing: $doctor"
}
if (-not (Test-Path -LiteralPath $runner -PathType Leaf)) {
    throw "Pipeline script missing: $runner"
}

$targetRepos = @()
if ($All) {
    $configData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
    $reposConfig = Get-JsonProperty $configData "repos"
    if ($null -eq $reposConfig) {
        throw "No repos configured in: $Config"
    }
    $targetRepos = @($reposConfig.PSObject.Properties.Name)
}
elseif ($Repos.Count -gt 0) {
    $targetRepos = @($Repos)
}
elseif (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
    $targetRepos = @([pscustomobject]@{ repo = ""; path = $RepoPath })
}
elseif (-not [string]::IsNullOrWhiteSpace($Repo)) {
    $targetRepos = @($Repo)
}
else {
    throw "Specify -Repo <alias>, -RepoPath <path>, -Repos <alias[]> or -All."
}

$results = New-Object System.Collections.Generic.List[object]
foreach ($target in $targetRepos) {
    if ($target -is [pscustomobject]) {
        $results.Add((Invoke-OneRepo $target.repo $target.path))
    }
    else {
        $results.Add((Invoke-OneRepo $target))
    }
}

if (-not $NoIndexUpdate -and (Test-Path -LiteralPath $indexer -PathType Leaf)) {
    Write-Host "Code intel invoke: update artifact index"
    $indexParams = @{}
    if (Test-Path -LiteralPath $Config -PathType Leaf) {
        $indexConfigData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
        $configuredArtifactRoot = Get-JsonProperty $indexConfigData "artifactRoot"
        if (-not [string]::IsNullOrWhiteSpace([string]$configuredArtifactRoot)) {
            $indexParams.ArtifactRoot = [string]$configuredArtifactRoot
        }
    }
    & $indexer @indexParams | Out-Host
}

Write-Host "Code intel invoke: batch summary"
foreach ($result in $results) {
    $mark = if ($result.ok) { "OK" } else { "FAILED" }
    Write-Host "$mark $($result.repo) stage=$($result.stage) exit=$($result.exitCode)"
}

$failed = @($results | Where-Object { -not $_.ok })
if ($failed.Count -gt 0) {
    exit 1
}
exit 0
