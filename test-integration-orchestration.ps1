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

function Invoke-ProviderValidateProcess {
    param(
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [string]$CodeIntelHome = "",
        [string]$ExplicitManifest = ""
    )

    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $rustCli
    $start.WorkingDirectory = $WorkingDirectory
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.Arguments = "provider --action Validate --json"
    [void]$start.Environment.Remove("CODE_INTEL_HOME")
    [void]$start.Environment.Remove("CODE_INTEL_INTEGRATIONS_MANIFEST")
    if (-not [string]::IsNullOrWhiteSpace($CodeIntelHome)) {
        $start.Environment["CODE_INTEL_HOME"] = $CodeIntelHome
    }
    if (-not [string]::IsNullOrWhiteSpace($ExplicitManifest)) {
        $start.Environment["CODE_INTEL_INTEGRATIONS_MANIFEST"] = $ExplicitManifest
    }

    $process = [System.Diagnostics.Process]::Start($start)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [pscustomobject]@{
        exitCode = $process.ExitCode
        stdout = $stdout
        stderr = $stderr
    }
}

$validateRaw = & $rustCli orchestrate --action Validate --json
if ($LASTEXITCODE -ne 0) {
    throw "Orchestration validate failed"
}
$validate = $validateRaw | ConvertFrom-Json
if (-not [bool]$validate.ok) {
    throw "Orchestration manifest is not ok"
}
$semanticMemoryStage = @($validate.stages | Where-Object { $_.id -eq "semantic_memory" })
if ($semanticMemoryStage.Count -ne 1) {
    throw "Expected one semantic_memory stage"
}
if ([bool]$semanticMemoryStage[0].required) {
    throw "semantic_memory must be optional for external beta release"
}
if ($semanticMemoryStage[0].description -notmatch "(?i)optional|non-blocking") {
    throw "semantic_memory must explicitly document its optional, non-blocking release behavior"
}

$landingStage = @($validate.stages | Where-Object { $_.id -eq "landing_coordination" })
if ($landingStage.Count -ne 1 -or [bool]$landingStage[0].required -or [int]$landingStage[0].order -ne 77) {
    throw "Expected one optional landing_coordination stage at order 77"
}

$workspaceRepo = Join-Path ([System.IO.Path]::GetTempPath()) "multi agent workspace with spaces"
$workspacePlanRaw = & $rustCli orchestrate --action Plan --capability development.multi-agent-workspace-preflight --repo $workspaceRepo --json
if ($LASTEXITCODE -ne 0) {
    throw "Multi-agent workspace preflight orchestration plan failed"
}
$workspacePlan = $workspacePlanRaw | ConvertFrom-Json
$workspacePreflight = @($workspacePlan.plan | Where-Object { $_.id -eq "development.multi-agent-workspace-preflight" })
if ($workspacePreflight.Count -ne 1 -or [bool]$workspacePreflight[0].required -or $workspacePreflight[0].stage -ne "preflight") {
    throw "Expected one optional development.multi-agent-workspace-preflight integration"
}
if ($workspacePreflight[0].commands.admitMutation -notmatch "-Intent mutation" -or
    $workspacePreflight[0].commands.observe -notmatch "-Intent observation" -or
    $workspacePreflight[0].commands.admitMutation -notmatch "'[^']*multi agent workspace with spaces'") {
    throw "Workspace preflight plan must separate observation from mutation and quote spaced paths"
}

$acceptancePlanRaw = & $rustCli orchestrate --action Plan --capability verification.code-intel-acceptance --json
if ($LASTEXITCODE -ne 0) {
    throw "Three-stage acceptance orchestration plan failed"
}
$acceptancePlan = $acceptancePlanRaw | ConvertFrom-Json
$acceptance = @($acceptancePlan.plan | Where-Object { $_.id -eq "verification.code-intel-acceptance" })
if ($acceptance.Count -ne 1 -or [bool]$acceptance[0].required -or $acceptance[0].stage -ne "verification") {
    throw "Expected one optional verification.code-intel-acceptance integration"
}
if ($acceptance[0].commands.agent -notmatch "-Stage agent" -or
    $acceptance[0].commands.land -notmatch "-Stage land" -or
    $acceptance[0].commands.promote -notmatch "-Stage promote") {
    throw "Acceptance plan must preserve agent, land, and promote stage mappings"
}

$mergeQueuePlanRaw = & $rustCli orchestrate --action Plan --capability delivery.multi-agent-merge-queue --repo (Join-Path ([System.IO.Path]::GetTempPath()) "merge queue repo with spaces") --json
if ($LASTEXITCODE -ne 0) {
    throw "Multi-agent merge queue orchestration plan failed"
}
$mergeQueuePlan = $mergeQueuePlanRaw | ConvertFrom-Json
$mergeQueue = @($mergeQueuePlan.plan | Where-Object { $_.id -eq "delivery.multi-agent-merge-queue" })
if ($mergeQueue.Count -ne 1 -or [bool]$mergeQueue[0].required -or $mergeQueue[0].stage -ne "landing_coordination") {
    throw "Expected one optional delivery.multi-agent-merge-queue integration"
}
if ($mergeQueue[0].entrypoint -ne "Invoke-MultiAgentMergeQueue.ps1" -or @($mergeQueue[0].capabilities).Count -lt 4) {
    throw "Merge queue adapter entrypoint and capabilities are incomplete"
}
$promoteCommand = $mergeQueue[0].commands.PSObject.Properties["promote"]
if ($null -ne $promoteCommand -or $mergeQueue[0].commands.land -notmatch "AllowRepositoryMutation" -or $mergeQueue[0].commands.land -notmatch "AllowNetworkPush") {
    throw "Merge queue plan must exclude promote and require explicit land authority"
}
if ($mergeQueue[0].commands.status -notmatch "'[^']*merge queue repo with spaces'") {
    throw "Merge queue plan did not safely quote a repo path containing spaces"
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
if ([bool]$repowise[0].required) {
    throw "memory.repowise must be optional"
}
if ($repowise[0].extensionPoint -notmatch "(?i)default" -or $repowise[0].extensionPoint -notmatch "(?i)optional|non-blocking") {
    throw "memory.repowise must document that it remains in the default plan while failures are non-blocking"
}

$providerValidateRaw = & $rustCli provider --action Validate --json
if ($LASTEXITCODE -ne 0) {
    throw "Provider registry validate failed"
}
$providerValidate = $providerValidateRaw | ConvertFrom-Json
if (-not [bool]$providerValidate.ok) {
    throw "Provider registry is not coherent: $($providerValidate.errors -join '; ')"
}

$providerListRaw = & $rustCli provider --action List --json
if ($LASTEXITCODE -ne 0) {
    throw "Provider registry list failed"
}
$providerList = $providerListRaw | ConvertFrom-Json
$canonicalCodeNexus = @($providerList.operations | Where-Object {
    $_.provider -eq "codenexus" -and $_.operation -eq "lite"
})
if ($canonicalCodeNexus.Count -ne 1) {
    throw "Expected one canonical codenexus/lite provider operation"
}
if ([bool]$canonicalCodeNexus[0].required -or $canonicalCodeNexus[0].status -ne "compatibility") {
    throw "Canonical codenexus/lite provider must reflect non-blocking compatibility runtime policy"
}
if ($canonicalCodeNexus[0].notes -notmatch "non-blocking") {
    throw "Canonical codenexus/lite provider must explicitly document non-blocking behavior"
}
if ($canonicalCodeNexus[0].commandTemplate -ne 'pwsh -NoProfile -File "$env:CODE_INTEL_HOME\Invoke-CodeNexusLite.ps1" -RepoPath ''<repo-path>''') {
    throw "Canonical codenexus/lite provider must use an executable CODE_INTEL_HOME PowerShell entrypoint"
}

$legacyLite = @($providerList.operations | Where-Object {
    $_.provider -eq "repowise" -and $_.operation -eq "lite"
})
if ($legacyLite.Count -ne 1) {
    throw "Expected one legacy repowise/lite compatibility operation"
}
if ([bool]$legacyLite[0].required -or $legacyLite[0].status -ne "compatibility" -or $legacyLite[0].notes -notmatch "(?i)deprecated") {
    throw "Legacy repowise/lite alias must be optional and explicitly deprecated compatibility"
}
if (@($providerList.operations | Where-Object {
    ($_.required -or $_.status -eq "active") -and $_.commandTemplate -match "code-nexus-lite\.exe"
}).Count -ne 0) {
    throw "Missing code-nexus-lite.exe must not be active or required"
}

$localizationPlanArgs = @("orchestrate", "--action", "Plan", "--capability", "localization.codenexus-lite", "--json")
$localizationPlanRaw = & $rustCli @localizationPlanArgs
if ($LASTEXITCODE -ne 0) {
    throw "CodeNexus localization orchestration plan failed"
}
$localizationPlan = $localizationPlanRaw | ConvertFrom-Json
$localization = @($localizationPlan.plan | Where-Object { $_.id -eq "localization.codenexus-lite" })
if ($localization.Count -ne 1 -or [bool]$localization[0].required -or $localization[0].kind -ne "compatibility-adapter") {
    throw "CodeNexus localization integration must be optional compatibility, matching non-blocking runner behavior"
}

$repoWithSpaces = Join-Path ([System.IO.Path]::GetTempPath()) "code intel provider plan"
$providerPlanRaw = & $rustCli provider --action Plan --provider codenexus --operation lite --repo $repoWithSpaces --json
if ($LASTEXITCODE -ne 0) {
    throw "Canonical CodeNexus provider plan failed"
}
$providerPlan = $providerPlanRaw | ConvertFrom-Json
if ($providerPlan.command -notmatch '^pwsh -NoProfile -File "\$env:CODE_INTEL_HOME\\Invoke-CodeNexusLite\.ps1" -RepoPath ''.+code intel provider plan''$') {
    throw "Canonical CodeNexus provider plan did not safely quote the repo path"
}
$parseTokens = $null
$parseErrors = $null
[void][System.Management.Automation.Language.Parser]::ParseInput(
    [string]$providerPlan.command,
    [ref]$parseTokens,
    [ref]$parseErrors
)
if (@($parseErrors).Count -ne 0) {
    throw "Canonical CodeNexus provider plan is not valid PowerShell: $($parseErrors -join '; ')"
}

$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-provider-resolution-" + [guid]::NewGuid().ToString("N"))
try {
    $fixtureManifestDir = Join-Path $fixtureRoot "orchestration"
    New-Item -ItemType Directory -Force -Path $fixtureManifestDir | Out-Null
    '{"policy":{"name":"unrelated"},"integrations":[]}' | Set-Content -LiteralPath (Join-Path $fixtureManifestDir "integrations.json") -Encoding utf8

    $homeResolution = Invoke-ProviderValidateProcess -WorkingDirectory $fixtureRoot -CodeIntelHome $root
    if ($homeResolution.exitCode -ne 0) {
        throw "CODE_INTEL_HOME provider validation process failed: $($homeResolution.stderr)"
    }
    $homeValidation = $homeResolution.stdout | ConvertFrom-Json
    if (-not [bool]$homeValidation.ok) {
        throw "CODE_INTEL_HOME was shadowed by unrelated cwd manifest: $($homeValidation.errors -join '; ')"
    }

    $manifestPath = Join-Path $root "orchestration\integrations.json"
    $explicitResolution = Invoke-ProviderValidateProcess -WorkingDirectory $fixtureRoot -ExplicitManifest $manifestPath
    if ($explicitResolution.exitCode -ne 0) {
        throw "Explicit manifest provider validation process failed: $($explicitResolution.stderr)"
    }
    $explicitValidation = $explicitResolution.stdout | ConvertFrom-Json
    if (-not [bool]$explicitValidation.ok) {
        throw "Explicit manifest was not honored from arbitrary cwd: $($explicitValidation.errors -join '; ')"
    }
}
finally {
    $resolvedFixture = [System.IO.Path]::GetFullPath($fixtureRoot)
    $resolvedTemp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
    if ($resolvedFixture.StartsWith($resolvedTemp, [System.StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $resolvedFixture)) {
        Remove-Item -LiteralPath $resolvedFixture -Recurse -Force
    }
}

$parityBaseline = Join-Path $root "test-parity-baseline.ps1"
if (-not (Test-Path -LiteralPath $parityBaseline -PathType Leaf)) {
    throw "Missing A00 parity baseline test: $parityBaseline"
}
& $parityBaseline

Write-Host "Integration orchestration smoke passed"
