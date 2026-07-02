#requires -Version 7.2

param(
    [string]$ArtifactRoot = "",
    [string]$OutputPath = "",
    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$platformModule = Join-Path (Join-Path $PSScriptRoot "tools") "code-intel-platform.psm1"
Import-Module $platformModule -Force
$effectivePlatform = Get-CodeIntelPlatform -Platform $Platform

function Read-JsonFile {
    param([string]$Path)
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

if ([string]::IsNullOrWhiteSpace($ArtifactRoot)) {
    $ArtifactRoot = Get-CodeIntelArtifactRoot -Platform $effectivePlatform
}

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $ArtifactRoot "index.md"
}

New-Item -ItemType Directory -Force -Path $ArtifactRoot | Out-Null

$rows = New-Object System.Collections.Generic.List[object]
$repoDirs = @(Get-ChildItem -LiteralPath $ArtifactRoot -Directory -ErrorAction SilentlyContinue)
foreach ($repoDir in $repoDirs) {
    $latestRun = Get-ChildItem -LiteralPath $repoDir.FullName -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending |
        Select-Object -First 1
    if ($null -eq $latestRun) { continue }

    $reportPath = Join-Path $latestRun.FullName "report.json"
    $summaryPath = Join-Path $latestRun.FullName "summary.md"
    if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) { continue }

    try {
        $report = Read-JsonFile $reportPath
    }
    catch {
        Write-Warning "Skipping unparseable report.json at $reportPath : $($_.Exception.Message)"
        continue
    }
    $cats = $report.summary.failureCategories
    $category = "clean"
    if ($cats.providerQuota -gt 0) { $category = "provider_quota" }
    elseif ($cats.localToolError -gt 0) { $category = "local_tool_error" }
    elseif ($cats.graphMissing -gt 0) { $category = "graph_missing" }
    elseif ($cats.sentruxFail -gt 0) { $category = "sentrux_fail" }
    elseif ($report.summary.manualRequired -gt 0) { $category = "manual_required" }
    elseif ($report.summary.failed -gt 0) { $category = "failed" }

    $rows.Add([pscustomobject][ordered]@{
        repo = $repoDir.Name
        run = $latestRun.Name
        category = $category
        failed = [int]$report.summary.failed
        manualRequired = [int]$report.summary.manualRequired
        passed = [int]$report.summary.passed
        skipped = [int]$report.summary.skipped
        summary = $summaryPath
        report = $reportPath
    })
}

$jsonPath = [System.IO.Path]::ChangeExtension($OutputPath, ".json")
$rows | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $jsonPath -Encoding UTF8

$lines = @(
    "# Code Intel Artifact Index",
    "",
    "- Updated: $((Get-Date).ToString("o"))",
    "- Artifact root: $ArtifactRoot",
    "",
    "| Repo | Latest Run | Category | Passed | Failed | Manual | Skipped | Summary |",
    "|---|---:|---|---:|---:|---:|---:|---|"
)

foreach ($row in @($rows | Sort-Object repo)) {
    $summaryLink = if (Test-Path -LiteralPath $row.summary -PathType Leaf) { "[summary]($($row.summary))" } else { "" }
    $lines += "| $($row.repo) | $($row.run) | $($row.category) | $($row.passed) | $($row.failed) | $($row.manualRequired) | $($row.skipped) | $summaryLink |"
}

if ($rows.Count -eq 0) {
    $lines += ""
    $lines += "No code intel artifacts found yet."
}

$lines | Set-Content -LiteralPath $OutputPath -Encoding UTF8

[pscustomobject][ordered]@{
    ok = $true
    output = $OutputPath
    json = $jsonPath
    repos = $rows.Count
} | ConvertTo-Json -Depth 4

# This script never invokes a native command itself, so $LASTEXITCODE at this
# point is stale from whatever ran earlier in the same process/session (e.g.
# a preceding tool probe). Reset explicitly so callers checking $LASTEXITCODE
# after `& update-code-intel-index.ps1` see this script's own outcome, not
# leftover state from an unrelated prior command.
exit 0
