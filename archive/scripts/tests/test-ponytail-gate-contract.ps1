#requires -Version 7.2

param(
    [string]$RepoPath = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = (Resolve-Path -LiteralPath $RepoPath).Path
$schemaPath = Join-Path $root "orchestration/schemas/code-intel-ponytail-gate.v1.schema.json"
$policyPath = Join-Path $root "orchestration/ponytail-gate-policy.v1.json"
$fixturePath = Join-Path $root "tests/fixtures/ponytail/c00-necessity-trace.json"
$docPath = Join-Path $root "docs/ponytail-governance-gate.md"
$registryPath = Join-Path $root "orchestration/integrations.json"

$schema = Get-Content -Raw -LiteralPath $schemaPath | ConvertFrom-Json
$policy = Get-Content -Raw -LiteralPath $policyPath | ConvertFrom-Json
$fixtureRaw = Get-Content -Raw -LiteralPath $fixturePath
$fixture = $fixtureRaw | ConvertFrom-Json

if ($schema.'$id' -ne "code-intel-ponytail-gate.v1") {
    throw "Ponytail gate schema id drifted"
}
if ($schema.'$defs'.request.additionalProperties -ne $false -or
    $schema.'$defs'.change.additionalProperties -ne $false -or
    $schema.'$defs'.result.additionalProperties -ne $false) {
    throw "Ponytail gate schema must remain closed"
}
if (@($schema.'$defs'.change.required) -notcontains "requiredEvidenceIds") {
    throw "Ponytail gate schema must require complete Necessity Trace evidence"
}
if ($policy.schema -ne "code-intel-ponytail-gate-policy.v1") {
    throw "Ponytail gate policy schema drifted"
}
if (@($policy.allowedCurrentValueSources).Count -ne 6 -or
    @($policy.forbiddenValueSources) -notcontains "future_maybe") {
    throw "Ponytail current-value rule table drifted"
}
foreach ($required in @("safety", "evidence", "verification")) {
    if (@($policy.nonFilterableRequirements) -notcontains $required) {
        throw "Ponytail protected requirement missing: $required"
    }
}
if ($fixture.schema -ne "code-intel-ponytail-gate-request.v1" -or
    @($fixture.changes).Count -lt 1) {
    throw "C00 self Necessity Trace is missing"
}
if (-not ($fixtureRaw | Test-Json -SchemaFile $schemaPath)) {
    throw "C00 self Necessity Trace does not satisfy the checked schema"
}
$doc = Get-Content -Raw -LiteralPath $docPath
foreach ($term in @("report_only", "enforce", "code-intel-authority-event.v1", "future_maybe", "governance ponytail-gate")) {
    if (-not $doc.Contains($term)) {
        throw "Ponytail gate documentation is missing: $term"
    }
}

Push-Location $root
try {
    & cargo test -p code-intel --test ponytail_gate -q
    if ($LASTEXITCODE -ne 0) {
        throw "Ponytail gate targeted Rust tests failed with exit code $LASTEXITCODE"
    }
    $binary = Join-Path $root "target/debug/code-intel.exe"
    $resultRaw = & $binary governance ponytail-gate --request $fixturePath
    if ($LASTEXITCODE -ne 0) {
        throw "Ponytail gate production CLI rejected the admitted self trace: $LASTEXITCODE"
    }
    if (-not (($resultRaw -join [Environment]::NewLine) | Test-Json -SchemaFile $schemaPath)) {
        throw "Ponytail gate production CLI result does not satisfy the checked result schema"
    }
}
finally {
    Pop-Location
}

$registry = Get-Content -Raw -LiteralPath $registryPath | ConvertFrom-Json
$registered = @($registry.integrations | Where-Object { $_.id -eq "governance.ponytail-gate" })
if ($registered.Count -ne 1 -or
    $registered[0].commands.evaluate -ne "target/debug/code-intel.exe governance ponytail-gate --request <request.json|->") {
    throw "Ponytail gate production registry declaration is missing or drifted"
}

[ordered]@{
    ok = $true
    schema = "code-intel-ponytail-gate-ci-contract.v1"
    changes = @($fixture.changes).Count
    allowedCurrentValueSources = @($policy.allowedCurrentValueSources).Count
    firstSufficientRungs = @($policy.firstSufficientSolutionRungs).Count
    dependencyAdditions = @($fixture.changes | Where-Object { $_.kind -eq "dependency" -and $_.operation -eq "add" }).Count
} | ConvertTo-Json
