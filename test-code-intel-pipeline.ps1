param(
    [string]$Repo = "",
    [string]$RepoPath = "",

    [string]$Config = "",

    [ValidateSet("lite", "normal", "full")]
    [string]$Mode = "normal",

    [switch]$RepowiseDocs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Read-JsonFile {
    param([string]$Path)
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

$root = Split-Path -Parent $PSCommandPath
if ([string]::IsNullOrWhiteSpace($Config)) {
    $Config = Join-Path $root "pipeline.config.json"
}
$doctor = Join-Path $root "check-code-intel-tools.ps1"
$runner = Join-Path $root "run-code-intel.ps1"

$label = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) { $RepoPath } else { $Repo }
if ([string]::IsNullOrWhiteSpace($label)) {
    throw "Specify -Repo <alias-or-path> or -RepoPath <path>."
}

$doctorJson = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
    & $doctor -Config $Config -RepoPath $RepoPath -Json | ConvertFrom-Json
}
else {
    & $doctor -Config $Config -Repo $Repo -Json | ConvertFrom-Json
}
if (-not $doctorJson.ok) {
    throw "Doctor failed: $($doctorJson.missing -join ', ')"
}

if ($RepowiseDocs) {
    if (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
        & $runner -Config $Config -RepoPath $RepoPath -Mode $Mode -RepowiseDocs
    }
    else {
        & $runner -Config $Config -Repo $Repo -Mode $Mode -RepowiseDocs
    }
}
else {
    if (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
        & $runner -Config $Config -RepoPath $RepoPath -Mode $Mode
    }
    else {
        & $runner -Config $Config -Repo $Repo -Mode $Mode
    }
}
if ($LASTEXITCODE -ne 0) {
    throw "Pipeline run failed for repo: $label"
}

$repoName = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) { Split-Path -Leaf (Get-Item -LiteralPath $RepoPath).FullName } else { $Repo }
$artifactRoot = if ($doctorJson.checks -and $doctorJson.checks.config -and (Test-Path -LiteralPath $Config -PathType Leaf)) {
    $configData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
    if ($configData.PSObject.Properties["artifactRoot"] -and -not [string]::IsNullOrWhiteSpace([string]$configData.artifactRoot)) { [string]$configData.artifactRoot } else { "" }
}
else { "" }
if ([string]::IsNullOrWhiteSpace($artifactRoot)) {
    $base = if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { $env:LOCALAPPDATA } else { (Join-Path $HOME ".code-intel") }
    $artifactRoot = Join-Path $base "code-intel\artifacts"
}

$artifactDir = Get-ChildItem -Path (Join-Path $artifactRoot $repoName) -Directory |
    Sort-Object Name -Descending |
    Select-Object -First 1

if ($null -eq $artifactDir) {
    throw "No artifact directory found for repo: $Repo"
}

$reportPath = Join-Path $artifactDir.FullName "report.json"
$summaryPath = Join-Path $artifactDir.FullName "summary.md"
$understandingPath = Join-Path $artifactDir.FullName "understanding.md"
if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
    throw "Missing report.json: $reportPath"
}
if (-not (Test-Path -LiteralPath $summaryPath -PathType Leaf)) {
    throw "Missing summary.md: $summaryPath"
}
if (-not (Test-Path -LiteralPath $understandingPath -PathType Leaf)) {
    throw "Missing understanding.md: $understandingPath"
}

$report = Read-JsonFile $reportPath
$requiredCategories = @("providerQuota", "localToolError", "graphMissing", "sentruxFail")
$missingCategories = @()
foreach ($key in $requiredCategories) {
    if ($null -eq $report.summary.failureCategories.$key) {
        $missingCategories += $key
    }
}
if ($missingCategories.Count -gt 0) {
    throw "Missing failure category counters: $($missingCategories -join ', ')"
}

$result = [ordered]@{
    ok = $true
    repo = $label
    mode = $Mode
    artifactDir = $artifactDir.FullName
    report = $reportPath
    summary = $summaryPath
    understanding = $understandingPath
    steps = $report.steps.Count
    failed = $report.summary.failed
    manualRequired = $report.summary.manualRequired
    failureCategories = $report.summary.failureCategories
}

$result | ConvertTo-Json -Depth 6
