param(
    [string]$RepoPath = $PSScriptRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Write-RunFixture {
    param(
        [string]$RunPath,
        [ValidateSet("complete", "missing_marker", "bad_digest")]
        [string]$State
    )

    New-Item -ItemType Directory -Force -Path $RunPath | Out-Null
    $reportPath = Join-Path $RunPath "report.json"
    [ordered]@{
        schema = "code-intel-report.v1"
        summary = [ordered]@{
            failed = 0
            manualRequired = 0
            passed = 1
            skipped = 0
            failureCategories = [ordered]@{
                providerQuota = 0
                localToolError = 0
                graphMissing = 0
                sentruxFail = 0
            }
        }
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $reportPath -Encoding UTF8
    "# fixture summary" | Set-Content -LiteralPath (Join-Path $RunPath "summary.md") -Encoding UTF8

    if ($State -eq "missing_marker") { return }

    $digest = (Get-FileHash -LiteralPath $reportPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($State -eq "bad_digest") { $digest = "0" * 64 }
    [ordered]@{
        schema = "code-intel-run-commit.v1"
        generatedAt = "2026-07-13T00:00:00Z"
        report = "report.json"
        reportSha256 = $digest
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $RunPath "run-complete.json") -Encoding UTF8
}

$root = (Resolve-Path -LiteralPath $RepoPath).Path
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-publication-" + [guid]::NewGuid().ToString("N"))
$outputPath = Join-Path $fixtureRoot "index.md"

try {
    Write-RunFixture -RunPath (Join-Path $fixtureRoot "good-repo\20260713-120000") -State complete
    Write-RunFixture -RunPath (Join-Path $fixtureRoot "good-repo\20260713-130000") -State missing_marker
    Write-RunFixture -RunPath (Join-Path $fixtureRoot "bad-repo\20260713-140000") -State bad_digest
    Write-RunFixture -RunPath (Join-Path $fixtureRoot "legacy-repo\20260713-110000") -State missing_marker
    Write-RunFixture -RunPath (Join-Path $fixtureRoot "staged-repo\20260713-150000.staging-deadbeef") -State complete
    $badShapeRun = Join-Path $fixtureRoot "bad-shape-repo\20260713-160000"
    New-Item -ItemType Directory -Force -Path $badShapeRun | Out-Null
    $badShapeReport = Join-Path $badShapeRun "report.json"
    '{}' | Set-Content -LiteralPath $badShapeReport -Encoding UTF8
    $badShapeDigest = (Get-FileHash -LiteralPath $badShapeReport -Algorithm SHA256).Hash.ToLowerInvariant()
    [ordered]@{
        schema = "code-intel-run-commit.v1"
        generatedAt = "2026-07-13T00:00:00Z"
        report = "report.json"
        reportSha256 = $badShapeDigest
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $badShapeRun "run-complete.json") -Encoding UTF8

    $badTypeRun = Join-Path $fixtureRoot "bad-type-repo\20260713-170000"
    Write-RunFixture -RunPath $badTypeRun -State complete
    $badTypeReport = Join-Path $badTypeRun "report.json"
    $badType = Get-Content -LiteralPath $badTypeReport -Raw | ConvertFrom-Json
    $badType.summary.failed = "not-an-int"
    $badType | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $badTypeReport -Encoding UTF8
    $badTypeDigest = (Get-FileHash -LiteralPath $badTypeReport -Algorithm SHA256).Hash.ToLowerInvariant()
    $badTypeMarker = Get-Content -LiteralPath (Join-Path $badTypeRun "run-complete.json") -Raw | ConvertFrom-Json
    $badTypeMarker.reportSha256 = $badTypeDigest
    $badTypeMarker | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $badTypeRun "run-complete.json") -Encoding UTF8

    $badRangeRun = Join-Path $fixtureRoot "bad-range-repo\20260713-180000"
    Write-RunFixture -RunPath $badRangeRun -State complete
    $badRangeReport = Join-Path $badRangeRun "report.json"
    $badRange = Get-Content -LiteralPath $badRangeReport -Raw | ConvertFrom-Json
    $badRange.summary.failed = [long][int]::MaxValue + 1
    $badRange | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $badRangeReport -Encoding UTF8
    $badRangeDigest = (Get-FileHash -LiteralPath $badRangeReport -Algorithm SHA256).Hash.ToLowerInvariant()
    $badRangeMarker = Get-Content -LiteralPath (Join-Path $badRangeRun "run-complete.json") -Raw | ConvertFrom-Json
    $badRangeMarker.reportSha256 = $badRangeDigest
    $badRangeMarker | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $badRangeRun "run-complete.json") -Encoding UTF8

    # These fixtures exercise the pre-A07 marker/report contract. Keep that
    # compatibility surface explicit; the normal facade now admits only A07
    # code-intel-run-commit.v1 markers through the Rust engine.
    & (Join-Path $root "update-code-intel-index.ps1") -ArtifactRoot $fixtureRoot -OutputPath $outputPath -LegacyCompatibilityMode | Out-Null
    $indexPath = [System.IO.Path]::ChangeExtension($outputPath, ".json")
    $rows = @(Get-Content -LiteralPath $indexPath -Raw | ConvertFrom-Json)

    Assert-True ($rows.Count -eq 1) "Only one fully committed authoritative run should be indexed; actual=$($rows.Count)."
    Assert-True ($rows[0].repo -eq "good-repo") "The valid committed repository should be indexed."
    Assert-True ($rows[0].run -eq "20260713-120000") "The newest valid committed run should win over a newer incomplete run."
    Assert-True (-not (@($rows.repo) -contains "bad-repo")) "A marker with a mismatched report digest must be rejected."
    Assert-True (-not (@($rows.repo) -contains "bad-shape-repo")) "A self-consistent marker must not admit a report that lacks the minimum report contract."
    Assert-True (-not (@($rows.repo) -contains "bad-type-repo")) "A self-consistent marker must not admit non-numeric summary counts."
    Assert-True (-not (@($rows.repo) -contains "bad-range-repo")) "A self-consistent marker must not admit counts that overflow the index contract."
    Assert-True (-not (@($rows.repo) -contains "legacy-repo")) "Legacy runs without a commit marker are not authoritative."
    Assert-True (-not (@($rows.repo) -contains "staged-repo")) "Staging directories must never enter the authoritative index."

    $smokeRoot = Join-Path $fixtureRoot "smoke"
    & (Join-Path $root "run-code-intel.ps1") -RepoPath $root -Mode lite -ArtifactRoot $smokeRoot -SkipRepowise -SkipRepomix -SkipSentrux -SkipGitHubResearch -SkipOpenSpec | Out-Null
    Assert-True ($LASTEXITCODE -eq 0) "A real lite pipeline publication smoke run must succeed."
    $publishedRuns = @(Get-ChildItem -LiteralPath (Join-Path $smokeRoot (Split-Path -Leaf $root)) -Directory)
    Assert-True ($publishedRuns.Count -eq 1) "The smoke run must publish exactly one final directory."
    Assert-True ($publishedRuns[0].Name -notmatch '\.staging-') "The published directory must not retain a staging name."
    $staleReferences = @(Get-ChildItem -LiteralPath $publishedRuns[0].FullName -File -Recurse |
        Where-Object { $_.Extension -in @('.json', '.md', '.txt', '.yaml', '.yml', '.toml') } |
        Select-String -SimpleMatch '.staging-' -List)
    $stalePaths = @($staleReferences | ForEach-Object { $_.Path })
    Assert-True ($staleReferences.Count -eq 0) "Published text artifacts must not retain staging path references: $($stalePaths -join ', ')."

    $publishedMarkerPath = Join-Path $publishedRuns[0].FullName "run-complete.json"
    $publishedReportPath = Join-Path $publishedRuns[0].FullName "report.json"
    $publishedMarker = Get-Content -LiteralPath $publishedMarkerPath -Raw | ConvertFrom-Json
    $publishedDigest = (Get-FileHash -LiteralPath $publishedReportPath -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-True ($publishedMarker.reportSha256 -eq $publishedDigest) "The real publication marker must bind the published report bytes."

    [ordered]@{
        ok = $true
        schema = "code-intel-transactional-publication-test.v1"
        indexedRuns = $rows.Count
        selectedRun = $rows[0].run
        smokeRun = $publishedRuns[0].Name
    } | ConvertTo-Json -Depth 4
}
finally {
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}
