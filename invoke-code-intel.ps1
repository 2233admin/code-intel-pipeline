param(
    [string]$Repo = "",
    [string[]]$Repos = @(),
    [switch]$All,

    [string]$Config = "D:\projects\_tools\code-intel-pipeline\pipeline.config.json",

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
    param([string]$RepoName)

    Write-Host "Code intel invoke: doctor $RepoName"
    & $doctor -Config $Config -Repo $RepoName
    if ($LASTEXITCODE -ne 0) {
        return [pscustomobject][ordered]@{
            repo = $RepoName
            ok = $false
            stage = "doctor"
            exitCode = $LASTEXITCODE
        }
    }

    Write-Host "Code intel invoke: pipeline $RepoName"
    if ($RepowiseDocs -or $SaveSentruxBaseline -or $AutoSaveMissingSentruxBaseline -or $RequireUnderstandGraph) {
        $invokeParams = @{
            Config = $Config
            Repo = $RepoName
            Mode = $Mode
        }
        if ($RepowiseDocs) { $invokeParams.RepowiseDocs = $true }
        if ($SaveSentruxBaseline) { $invokeParams.SaveSentruxBaseline = $true }
        if ($AutoSaveMissingSentruxBaseline) { $invokeParams.AutoSaveMissingSentruxBaseline = $true }
        if ($RequireUnderstandGraph) { $invokeParams.RequireUnderstandGraph = $true }
        & $runner @invokeParams
    }
    else {
        & $runner -Config $Config -Repo $RepoName -Mode $Mode
    }

    $code = $LASTEXITCODE
    return [pscustomobject][ordered]@{
        repo = $RepoName
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
elseif (-not [string]::IsNullOrWhiteSpace($Repo)) {
    $targetRepos = @($Repo)
}
else {
    throw "Specify -Repo <alias>, -Repos <alias[]> or -All."
}

$results = New-Object System.Collections.Generic.List[object]
foreach ($target in $targetRepos) {
    $results.Add((Invoke-OneRepo $target))
}

if (-not $NoIndexUpdate -and (Test-Path -LiteralPath $indexer -PathType Leaf)) {
    Write-Host "Code intel invoke: update artifact index"
    & $indexer | Out-Host
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
