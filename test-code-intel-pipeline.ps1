param(
    [Parameter(Mandatory = $true)]
    [string]$Repo,

    [string]$Config = "D:\projects\_tools\code-intel-pipeline\pipeline.config.json",

    [switch]$RepowiseDocs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Read-JsonFile {
    param([string]$Path)
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

$root = Split-Path -Parent $PSCommandPath
$doctor = Join-Path $root "check-code-intel-tools.ps1"
$runner = Join-Path $root "run-code-intel.ps1"

$doctorJson = & $doctor -Config $Config -Repo $Repo -Json | ConvertFrom-Json
if (-not $doctorJson.ok) {
    throw "Doctor failed: $($doctorJson.missing -join ', ')"
}

if ($RepowiseDocs) {
    & $runner -Config $Config -Repo $Repo -Mode normal -RepowiseDocs
}
else {
    & $runner -Config $Config -Repo $Repo -Mode normal
}
if ($LASTEXITCODE -ne 0) {
    throw "Pipeline run failed for repo: $Repo"
}

$artifactDir = Get-ChildItem -Path "D:\projects\_artifacts\code-intel\$Repo" -Directory |
    Sort-Object Name -Descending |
    Select-Object -First 1

if ($null -eq $artifactDir) {
    throw "No artifact directory found for repo: $Repo"
}

$reportPath = Join-Path $artifactDir.FullName "report.json"
$summaryPath = Join-Path $artifactDir.FullName "summary.md"
if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
    throw "Missing report.json: $reportPath"
}
if (-not (Test-Path -LiteralPath $summaryPath -PathType Leaf)) {
    throw "Missing summary.md: $summaryPath"
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
    repo = $Repo
    artifactDir = $artifactDir.FullName
    report = $reportPath
    summary = $summaryPath
    steps = $report.steps.Count
    failed = $report.summary.failed
    manualRequired = $report.summary.manualRequired
    failureCategories = $report.summary.failureCategories
}

$result | ConvertTo-Json -Depth 6
