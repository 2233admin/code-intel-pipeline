[CmdletBinding()]
param([string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent))

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$runPath = Join-Path $RepoRoot "run-code-intel.ps1"
$run = Get-Content -LiteralPath $runPath -Raw
$legacyProductionCalls = @([regex]::Matches($run, 'Join-Path \$PSScriptRoot "test-code-intel-provider\.ps1"')).Count
$productionProbeCalls = @([regex]::Matches($run, 'Join-Path \$PSScriptRoot "Invoke-RepowiseProviderProbe\.ps1"')).Count
if ($legacyProductionCalls -ne 0) { throw "run-code-intel.ps1 still invokes the test provider wrapper" }
if ($productionProbeCalls -ne 1) { throw "run-code-intel.ps1 must contain exactly one production provider probe route" }
if ($run -notmatch 'Index-only repowise will still run\.') {
    throw "provider quota/failure no longer preserves the index-only route"
}

$testWrapper = Get-Content -LiteralPath (Join-Path $RepoRoot "test-code-intel-provider.ps1") -Raw
if ($testWrapper -notmatch 'Invoke-RepowiseProviderProbe\.ps1') {
    throw "test-only compatibility wrapper no longer delegates to the production probe"
}
$historical = @(& git -C $RepoRoot log --format=%H -S 'test-code-intel-provider.ps1' -- run-code-intel.ps1)
if ($LASTEXITCODE -ne 0 -or $historical.Count -eq 0) {
    throw "historical legacy provider-preflight branch cannot be located for rollback provenance"
}

[ordered]@{
    ok = $true
    schema = "code-intel-provider-preflight-retirement-boundary.v1"
    branchId = "run-code-intel.provider-preflight.test-wrapper"
    affectedFiles = @("run-code-intel.ps1")
    legacyProductionCalls = $legacyProductionCalls
    productionProbeCalls = $productionProbeCalls
    testWrapperDisposition = "test_only_compatibility_wrapper"
    historicalSourceRevision = $historical[0]
    installerCheckProviderCall = "out_of_scope_not_authorized_for_deletion"
    deletionExecuted = $false
    retired = $false
} | ConvertTo-Json -Depth 5 -Compress
