param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$adapter = Join-Path $root "Invoke-MultiAgentMergeQueue.ps1"
$policy = Join-Path $root "orchestration\multi-agent-merge-queue-policy.v1.json"
$statusSchema = Join-Path $root "orchestration\schemas\code-intel-multi-agent-merge-queue-status.v1.schema.json"
$activationConfig = Join-Path $root "claude-code-merge-queue.config.mjs"
$activationHook = Join-Path $root ".githooks\pre-push"
$activationInstaller = Join-Path $root "Install-MultiAgentMergeQueue.ps1"
$packageManifest = Join-Path $root "package.json"
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-merge-queue-" + [guid]::NewGuid().ToString("N"))

function Invoke-Adapter {
    param([string]$Repo, [string]$Queue, [string]$Action = "status", [switch]$AuthorizeLand)
    $arguments = @("-NoProfile", "-File", $adapter, "-Action", $Action, "-RepoPath", $Repo, "-QueueCommand", $Queue, "-Policy", $policy, "-Json")
    if ($AuthorizeLand) { $arguments += @("-AllowRepositoryMutation", "-AllowNetworkPush") }
    $output = @(& pwsh @arguments 2>&1 | ForEach-Object { $_.ToString() })
    [pscustomobject]@{ exitCode = $LASTEXITCODE; output = ($output -join "`n") }
}

try {
    foreach ($activationFile in @($activationConfig, $activationHook, $activationInstaller, $packageManifest)) {
        if (-not (Test-Path -LiteralPath $activationFile -PathType Leaf)) {
            throw "repository activation file is missing: $activationFile"
        }
    }
    $activationConfigText = Get-Content -Raw -LiteralPath $activationConfig
    if ($activationConfigText -notmatch 'Invoke-CodeIntelAcceptance\.ps1\s+-Stage\s+land' -or
        $activationConfigText -notmatch 'checksRequired\s*:\s*true') {
        throw "repository activation must bind queue landing to required three-stage acceptance"
    }
    $activationHookText = Get-Content -Raw -LiteralPath $activationHook
    if ($activationHookText -notmatch '(?m)^if ! "\$queue_command" check-push' -or
        $activationHookText -notmatch '(?m)^\s*/usr/bin/bash "\$previous_hook"') {
        throw "repository hook must enforce check-push and forward the shared hook through Git Bash"
    }
    $packageDocument = Get-Content -Raw -LiteralPath $packageManifest | ConvertFrom-Json
    if ([string]$packageDocument.devDependencies.'claude-code-merge-queue' -ne '0.5.1') {
        throw "repository provider dependency must be pinned exactly to 0.5.1"
    }

    New-Item -ItemType Directory -Force -Path $temp | Out-Null
    & git -C $temp init --quiet
    if ($LASTEXITCODE -ne 0) { throw "fixture git init failed" }
    New-Item -ItemType Directory -Force -Path (Join-Path $temp ".husky") | Out-Null
    & git -C $temp config core.hooksPath .husky
    if ($LASTEXITCODE -ne 0) { throw "fixture hooksPath setup failed" }

    @'
#!/bin/sh
npx --no-install claude-code-merge-queue check-push
'@ | Set-Content -LiteralPath (Join-Path (Join-Path $temp ".husky") "pre-push") -Encoding UTF8

    @'
export default {
  integrationBranch: "integration",
  productionBranch: "main",
  checkCommand: "pwsh -NoProfile -File ./project-acceptance.ps1",
  checksRequired: true,
};
'@ | Set-Content -LiteralPath (Join-Path $temp "claude-code-merge-queue.config.mjs") -Encoding UTF8

    $fakeQueue = Join-Path $temp "fake-merge-queue.ps1"
    @'
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest)
if ($Rest.Count -eq 1 -and $Rest[0] -eq "--version") {
    Write-Output "0.5.1"
    exit 0
}
if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_QUEUE_TEST_LOG)) {
    Add-Content -LiteralPath $env:CODE_INTEL_QUEUE_TEST_LOG -Value ($Rest -join " ")
}
exit 0
'@ | Set-Content -LiteralPath $fakeQueue -Encoding UTF8

    $ready = Invoke-Adapter -Repo $temp -Queue $fakeQueue
    if ($ready.exitCode -ne 0) { throw "status observation failed: $($ready.output)" }
    if (-not ($ready.output | Test-Json -SchemaFile $statusSchema)) {
        throw "ready status does not satisfy the checked status schema"
    }
    $readyResult = $ready.output | ConvertFrom-Json
    if (-not [bool]$readyResult.ready -or @($readyResult.failedGateIds).Count -ne 0) {
        throw "fully configured fixture must be landing-ready: $($ready.output)"
    }

    $log = Join-Path $temp "queue.log"
    $env:CODE_INTEL_QUEUE_TEST_LOG = $log
    $unauthorizedLand = Invoke-Adapter -Repo $temp -Queue $fakeQueue -Action "land"
    if ($unauthorizedLand.exitCode -ne 1 -or (Test-Path -LiteralPath $log)) {
        throw "landing without explicit repository-mutation and network-push authority must not invoke the provider"
    }

    $land = Invoke-Adapter -Repo $temp -Queue $fakeQueue -Action "land" -AuthorizeLand
    if ($land.exitCode -ne 0 -or -not (Test-Path -LiteralPath $log) -or (Get-Content -Raw -LiteralPath $log).Trim() -ne "land") {
        throw "ready fixture must delegate exactly one land action: $($land.output)"
    }

    @'
export default {
  integrationBranch: "integration",
  productionBranch: "main",
  checkCommand: null,
  checksRequired: true,
};
'@ | Set-Content -LiteralPath (Join-Path $temp "claude-code-merge-queue.config.mjs") -Encoding UTF8
    $missingAcceptance = Invoke-Adapter -Repo $temp -Queue $fakeQueue
    $missingAcceptanceResult = $missingAcceptance.output | ConvertFrom-Json
    if ([bool]$missingAcceptanceResult.ready -or @($missingAcceptanceResult.failedGateIds) -notcontains "acceptance-check-configured") {
        throw "missing acceptance command must fail closed"
    }

    @'
export default {
  integrationBranch: "main",
  productionBranch: null,
  checkCommand: "pwsh -NoProfile -File ./project-acceptance.ps1",
  checksRequired: true,
};
'@ | Set-Content -LiteralPath (Join-Path $temp "claude-code-merge-queue.config.mjs") -Encoding UTF8
    $missingPromotion = Invoke-Adapter -Repo $temp -Queue $fakeQueue
    $missingPromotionResult = $missingPromotion.output | ConvertFrom-Json
    if ([bool]$missingPromotionResult.ready -or @($missingPromotionResult.failedGateIds) -notcontains "human-promotion-boundary") {
        throw "single-stage agent-to-production landing must fail the human promotion boundary"
    }

    $policyDocument = Get-Content -Raw -LiteralPath $policy | ConvertFrom-Json
    if (@($policyDocument.forbiddenActions) -notcontains "promote" -or [bool]$policyDocument.authority.agentsMayPromoteProduction) {
        throw "policy must keep production promotion outside agent authority"
    }

    Write-Host "PASS multi-agent merge queue: explicit-authority landing delegates; unauthorized, missing acceptance, and missing promotion boundary fail closed"
} finally {
    Remove-Item Env:CODE_INTEL_QUEUE_TEST_LOG -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
