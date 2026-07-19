param(
    [string]$RepoPath = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$rustCli = Join-Path $root "target\debug\code-intel.exe"

if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
    Push-Location $root
    try {
        & cargo build -p code-intel | Out-Host
    }
    finally {
        Pop-Location
    }
}
if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
    throw "Missing Rust orchestrator: $rustCli"
}

$validateRaw = & $rustCli orchestrate --action Validate --json
if ($LASTEXITCODE -ne 0) {
    throw "Orchestration validate failed"
}
$validate = $validateRaw | ConvertFrom-Json
if (-not [bool]$validate.ok) {
    throw "Orchestration manifest is not ok"
}

$planArgs = @("orchestrate", "--action", "Plan", "--capability", "semantic_memory", "--json")
if (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
    $planArgs += @("--repo", $RepoPath)
}
$planRaw = & $rustCli @planArgs
if ($LASTEXITCODE -ne 0) {
    throw "Orchestration plan failed"
}
$plan = $planRaw | ConvertFrom-Json
$repowise = @($plan.plan | Where-Object { $_.id -eq "memory.repowise" })
if ($repowise.Count -ne 1) {
    throw "Expected one memory.repowise integration"
}
if (-not [bool]$repowise[0].required) {
    throw "memory.repowise must be required"
}

$evidenceRaw = & $rustCli orchestrate --action Plan --capability advisory_evidence --json
if ($LASTEXITCODE -ne 0) {
    throw "Advisory evidence orchestration plan failed"
}
$evidence = $evidenceRaw | ConvertFrom-Json
$evidenceIds = @($evidence.plan | ForEach-Object { $_.id })
if ($evidenceIds.Count -ne 4 -or
    $evidenceIds -notcontains "evidence.compete" -or
    $evidenceIds -notcontains "evidence.react-doctor" -or
    $evidenceIds -notcontains "feature.competitive-intelligence" -or
    $evidenceIds -notcontains "feature.react-diagnostics") {
    throw "Expected two providers and two first-party Beta feature integrations"
}
if (@($evidence.plan | Where-Object { [bool]$_.required }).Count -ne 0) {
    throw "Advisory evidence integrations must remain optional"
}

$features = & $rustCli orchestrate --action Plan --capability beta_features --json | ConvertFrom-Json
if (@($features.plan).Count -ne 2) {
    throw "Expected two first-party Beta feature integrations"
}

Write-Host "Integration orchestration smoke passed"
