#requires -Version 7.2

param(
    [string]$ArtifactRoot = "",
    [string]$OutputPath = "",
    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto",
    [ValidateSet("rebuild", "incremental")]
    [string]$Operation = "rebuild",
    [string]$ExistingIndex = "",
    [switch]$LegacyCompatibilityMode
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$platformModule = Join-Path (Join-Path $PSScriptRoot "tools") "code-intel-platform.psm1"
Import-Module $platformModule -Force
$effectivePlatform = Get-CodeIntelPlatform -Platform $Platform

if ([string]::IsNullOrWhiteSpace($ArtifactRoot)) {
    $ArtifactRoot = Get-CodeIntelArtifactRoot -Platform $effectivePlatform
}
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $ArtifactRoot "index.md"
}

if (-not $LegacyCompatibilityMode) {
    New-Item -ItemType Directory -Force -Path $ArtifactRoot | Out-Null
    $rustCli = Join-Path $PSScriptRoot "target\debug\code-intel.exe"
    if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
        throw "Artifact index binary is missing: $rustCli"
    }
    $jsonPath = [System.IO.Path]::ChangeExtension($OutputPath, ".json")
    $arguments = @(
        "artifact", "index",
        "--artifact-root", $ArtifactRoot,
        "--output", $jsonPath,
        "--operation", $Operation
    )
    if ($Operation -eq "incremental") {
        if ([string]::IsNullOrWhiteSpace($ExistingIndex)) { $ExistingIndex = $jsonPath }
        if (-not (Test-Path -LiteralPath $ExistingIndex -PathType Leaf)) {
            throw "Incremental artifact index requires an existing index: $ExistingIndex"
        }
        $arguments += @("--existing", $ExistingIndex)
    }
    $rawResult = & $rustCli @arguments
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $index = $rawResult | ConvertFrom-Json
    $lines = @(
        "# Code Intel Artifact Index",
        "",
        "- Schema: $($index.schema)",
        "- Admission: A07 committed runs only",
        "",
        "| Repo | Latest Run | Outcome | Run Identity |",
        "|---|---|---|---|"
    )
    foreach ($entry in @($index.entries)) {
        $lines += "| $($entry.repo) | $($entry.run) | $($entry.outcome) | $($entry.runIdentity) |"
    }
    if (@($index.entries).Count -eq 0) {
        $lines += ""
        $lines += "No committed code intel artifacts found yet."
    }
    $lines | Set-Content -LiteralPath $OutputPath -Encoding UTF8
    [pscustomobject][ordered]@{
        ok = $true
        schema = $index.schema
        mode = "committed-only"
        output = $OutputPath
        json = $jsonPath
        repos = @($index.entries).Count
        diagnostics = @($index.diagnostics).Count
    } | ConvertTo-Json -Depth 4
    exit 0
}

function Read-JsonFile {
    param([string]$Path)
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Test-JsonProperty {
    param(
        [object]$InputObject,
        [string]$Name
    )

    return $null -ne $InputObject -and $null -ne $InputObject.PSObject.Properties[$Name]
}

function Test-JsonCount {
    param([object]$Value)

    if ($null -eq $Value) { return $false }
    $typeCode = [System.Type]::GetTypeCode($Value.GetType())
    if ($typeCode -notin @(
        [System.TypeCode]::Byte,
        [System.TypeCode]::SByte,
        [System.TypeCode]::Int16,
        [System.TypeCode]::UInt16,
        [System.TypeCode]::Int32,
        [System.TypeCode]::UInt32,
        [System.TypeCode]::Int64,
        [System.TypeCode]::UInt64
    )) {
        return $false
    }
    return [decimal]$Value -ge 0 -and [decimal]$Value -le [int]::MaxValue
}

function Test-RunCommit {
    param([string]$RunPath)

    if ((Split-Path -Leaf $RunPath) -match '\.staging-') { return $false }

    $markerPath = Join-Path $RunPath "run-complete.json"
    $reportPath = Join-Path $RunPath "report.json"
    if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        return $false
    }

    try {
        $marker = Read-JsonFile $markerPath
        if ($marker.schema -ne "code-intel-run-commit.v1" -or
            $marker.report -ne "report.json" -or
            [string]$marker.reportSha256 -notmatch '^[a-f0-9]{64}$') {
            return $false
        }
        $actualDigest = (Get-FileHash -LiteralPath $reportPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualDigest -ne [string]$marker.reportSha256) { return $false }

        $report = Read-JsonFile $reportPath
        if (-not (Test-JsonProperty -InputObject $report -Name "summary")) { return $false }
        $summary = $report.summary
        if ($null -eq $summary) { return $false }
        foreach ($name in @("failed", "manualRequired", "passed", "skipped", "failureCategories")) {
            if (-not (Test-JsonProperty -InputObject $summary -Name $name)) { return $false }
        }
        foreach ($name in @("failed", "manualRequired", "passed", "skipped")) {
            if (-not (Test-JsonCount -Value $summary.$name)) { return $false }
        }
        $categories = $summary.failureCategories
        if ($null -eq $categories) { return $false }
        foreach ($name in @("providerQuota", "localToolError", "graphMissing", "sentruxFail")) {
            if (-not (Test-JsonProperty -InputObject $categories -Name $name)) { return $false }
            if (-not (Test-JsonCount -Value $categories.$name)) { return $false }
        }
        return $true
    }
    catch {
        Write-Warning "Skipping invalid run commit at $markerPath : $($_.Exception.Message)"
        return $false
    }
}

New-Item -ItemType Directory -Force -Path $ArtifactRoot | Out-Null

$rows = New-Object System.Collections.Generic.List[object]
$repoDirs = @(Get-ChildItem -LiteralPath $ArtifactRoot -Directory -ErrorAction SilentlyContinue)
foreach ($repoDir in $repoDirs) {
    $latestRun = Get-ChildItem -LiteralPath $repoDir.FullName -Directory -ErrorAction SilentlyContinue |
        Where-Object { Test-RunCommit -RunPath $_.FullName } |
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
