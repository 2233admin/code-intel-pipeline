#requires -Version 7.2

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))

& cargo test -p code-intel --test survival_scan --quiet
if ($LASTEXITCODE -ne 0) { throw "B05 Rust contract suite failed" }

$manifest = Get-Content -LiteralPath (Join-Path $root "orchestration\integrations.json") -Raw | ConvertFrom-Json -Depth 100
$entry = @($manifest.integrations | Where-Object { $_.id -eq "repository.survival-scan" })
if ($entry.Count -ne 1) { throw "repository.survival-scan must have exactly one integration entry" }
if (-not $entry[0].required) { throw "repository.survival-scan must be required" }
if ([string]$entry[0].commands.scan -notmatch "repository survival-scan") { throw "survival scan production command is missing" }

foreach ($relative in @(
    "orchestration\schemas\code-intel-repository-survival-scan-request.v1.schema.json",
    "orchestration\schemas\code-intel-repository-survival-scan-result.v1.schema.json",
    "docs\repository-survival-scan.md"
)) {
    if (-not (Test-Path -LiteralPath (Join-Path $root $relative) -PathType Leaf)) { throw "B05 contract file missing: $relative" }
}

$facade = Get-Content -LiteralPath (Join-Path $root "run-code-intel.ps1") -Raw
if ($facade -notmatch "repository survival-scan" -or $facade -notmatch "SurvivalScanArtifactRoot") {
    throw "B05 PowerShell facade route is missing"
}

Write-Output "B05 repository survival scan contract passed"
