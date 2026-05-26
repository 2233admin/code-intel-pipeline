param(
    [string]$ArtifactRoot = "",
    [string]$OutputPath = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Read-JsonFile {
    param([string]$Path)
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

if ([string]::IsNullOrWhiteSpace($ArtifactRoot)) {
    $fromEnv = [Environment]::GetEnvironmentVariable("CODE_INTEL_ARTIFACT_ROOT", "User")
    if (-not [string]::IsNullOrWhiteSpace($fromEnv)) {
        $ArtifactRoot = $fromEnv
    }
    elseif (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_ARTIFACT_ROOT)) {
        $ArtifactRoot = $env:CODE_INTEL_ARTIFACT_ROOT
    }
    else {
        $base = if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { $env:LOCALAPPDATA } else { (Join-Path $HOME ".code-intel") }
        $ArtifactRoot = Join-Path $base "code-intel\artifacts"
    }
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

    $report = Read-JsonFile $reportPath
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
