#requires -Version 7.2

param([string]$RepoPath = $PSScriptRoot)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = (Resolve-Path -LiteralPath $RepoPath).Path

$facade = Get-Content -Raw -LiteralPath (Join-Path $root "run-code-intel.ps1")
if ($facade.Contains('Join-Path $PSScriptRoot "test-code-intel-provider.ps1"')) {
    throw "Production facade must not execute the test provider script"
}
if (-not $facade.Contains('& $rustCli provider repowise-adapt')) {
    throw "Production facade does not expose provider.repowise-adapt"
}
if (-not $facade.Contains('Join-Path $PSScriptRoot "Invoke-RepowiseProviderProbe.ps1"')) {
    throw "Production facade does not use the Repowise provider health adapter"
}
if (-not $facade.Contains("Index-only repowise will still run.")) {
    throw "Repowise docs health failure no longer preserves index-only execution"
}

$probe = Join-Path $root "Invoke-RepowiseProviderProbe.ps1"
try {
    $env:CODE_INTEL_API_KEY = "sk-contract-secret-123456"
    $raw = & $probe -Provider mock -Json
    if ($LASTEXITCODE -ne 0) { throw "Mock Repowise provider health probe failed" }
    if ([string]$raw -match "contract-secret") { throw "Repowise health output leaked a secret" }
    $health = $raw | ConvertFrom-Json
    if ($health.schema -ne "code-intel-repowise-provider-health.v1" -or
        $health.kind -ne "health" -or $health.evidence -ne $false -or -not $health.ok) {
        throw "Repowise health output contract is invalid"
    }
}
finally {
    Remove-Item Env:CODE_INTEL_API_KEY -ErrorAction SilentlyContinue
}

Push-Location $root
try {
    & cargo test -p code-intel --test repowise_adapter -q
    if ($LASTEXITCODE -ne 0) { throw "Repowise Rust adapter contract failed" }
    & cargo test -p code-intel --test repowise_route -q
    if ($LASTEXITCODE -ne 0) { throw "Repowise production route contract failed" }

    $fixtureRoot = Join-Path $root "tests/fixtures/repowise-adapter"
    $facadeOutput = & pwsh -NoProfile -File (Join-Path $root "run-code-intel.ps1") `
        -RepowiseAdapterRequest (Join-Path $fixtureRoot "success.json") `
        -RepowiseAdapterArtifactRoot $fixtureRoot `
        -RepowiseAdapterEvaluatedAt 1700000100 `
        -RepowiseAdapterMaxAgeSeconds 300
    if ($LASTEXITCODE -ne 0) { throw "Repowise production facade route failed" }
    $facadeJson = [string]::Join([Environment]::NewLine, @($facadeOutput))
    if (-not ($facadeJson | Test-Json -SchemaFile (Join-Path $root "orchestration/schemas/code-intel-repowise-route-result.v1.schema.json"))) {
        throw "Repowise production facade output failed its checked schema"
    }
    $facadeResult = $facadeJson | ConvertFrom-Json
    if ($facadeResult.status -ne "completed" -or @($facadeResult.admissions).Count -ne 2) {
        throw "Repowise production facade did not submit both success channels through A04"
    }
}
finally { Pop-Location }

[ordered]@{
    ok = $true
    schema = "code-intel-repowise-adapter-ci-contract.v1"
    productionRoute = "code-intel provider repowise-adapt"
    productionProbe = "Invoke-RepowiseProviderProbe.ps1"
    testScriptProductionReferences = 0
    healthIsEvidence = $false
} | ConvertTo-Json
