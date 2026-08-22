param(
    [string]$RepoPath = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../../.."))
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

$root = (Resolve-Path -LiteralPath $RepoPath).Path
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-publication-" + [guid]::NewGuid().ToString("N"))

try {
    $smokeRoot = Join-Path $fixtureRoot "smoke"
    & (Join-Path $root "legacy/run-code-intel.ps1") -RepoPath $root -Mode lite -ArtifactRoot $smokeRoot -SkipRepowise -SkipSentrux -SkipOpenSpec | Out-Null
    Assert-True ($LASTEXITCODE -eq 0) "A real lite pipeline publication smoke run must succeed."
    $publishedRuns = @(Get-ChildItem -LiteralPath (Join-Path $smokeRoot (Split-Path -Leaf $root)) -Directory)
    Assert-True ($publishedRuns.Count -eq 1) "The smoke run must publish exactly one final directory."
    Assert-True ($publishedRuns[0].Name -notmatch '\.staging-') "The published directory must not retain a staging name."
    $staleReferences = @(Get-ChildItem -LiteralPath $publishedRuns[0].FullName -File -Recurse |
        Where-Object { $_.Extension -in @('.json', '.md', '.txt', '.yaml', '.yml', '.toml') } |
        Select-String -SimpleMatch '.staging-' -List)
    $stalePaths = @($staleReferences | ForEach-Object { $_.Path })
    Assert-True ($staleReferences.Count -eq 0) "Published text artifacts must not retain staging path references: $($stalePaths -join ', ')."

    $publishedReportPath = Join-Path $publishedRuns[0].FullName "report.json"
    $publishedSummaryPath = Join-Path $publishedRuns[0].FullName "summary.md"
    Assert-True (Test-Path -LiteralPath $publishedReportPath -PathType Leaf) "The promoted legacy report must contain report.json."
    Assert-True (Test-Path -LiteralPath $publishedSummaryPath -PathType Leaf) "The promoted legacy report must contain summary.md."
    $publishedReport = Get-Content -LiteralPath $publishedReportPath -Raw | ConvertFrom-Json
    Assert-True ($publishedReport.repo -eq $root) "The promoted compatibility report must retain its repository binding."
    Assert-True ($null -ne $publishedReport.PSObject.Properties["steps"]) "The promoted compatibility report must retain advisory step results."
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $publishedRuns[0].FullName "run-complete.json"))) "The legacy report path must not claim canonical Run Commit authority."
    $remainingStaging = @(Get-ChildItem -LiteralPath $smokeRoot -Directory -Recurse |
        Where-Object { $_.Name -match '\.staging-' })
    Assert-True ($remainingStaging.Count -eq 0) "Successful promotion must leave no staging directory behind."

    [ordered]@{
        ok = $true
        schema = "code-intel-transactional-publication-test.v1"
        smokeRun = $publishedRuns[0].Name
        authority = "non_authoritative_legacy_report"
        canonicalMarkerPresent = $false
    } | ConvertTo-Json -Depth 4
}
finally {
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}
