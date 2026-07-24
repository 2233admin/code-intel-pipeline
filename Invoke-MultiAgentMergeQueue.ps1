#requires -Version 7.2

param(
    [ValidateSet("status", "validate", "land", "reconcile", "history")]
    [string]$Action = "status",

    [string]$RepoPath = ".",

    [string]$QueueCommand = "",

    [string]$Policy = (Join-Path $PSScriptRoot "orchestration\multi-agent-merge-queue-policy.v1.json"),

    [switch]$AllowRepositoryMutation,

    [switch]$AllowNetworkPush,

    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$gates = [System.Collections.Generic.List[object]]::new()

function Add-Gate {
    param([string]$Id, [bool]$Passed, [string]$Detail)
    $gates.Add([pscustomobject]@{ id = $Id; passed = $Passed; detail = $Detail })
}

function Get-JsScalar {
    param([string]$Text, [string]$Name)
    $pattern = "(?m)^\s*" + [regex]::Escape($Name) + "\s*:\s*(?<value>[^,\r\n]+)"
    $match = [regex]::Match($Text, $pattern)
    if (-not $match.Success) { return $null }
    return $match.Groups["value"].Value.Trim()
}

function ConvertFrom-JsStringLiteral {
    param([AllowNull()][string]$Value)
    if ([string]::IsNullOrWhiteSpace($Value)) { return $null }
    $match = [regex]::Match($Value.Trim(), '^["''](?<value>.*)["'']$')
    if (-not $match.Success) { return $null }
    return $match.Groups["value"].Value
}

function Resolve-QueueCommand {
    param([string]$Repo, [string]$Explicit)
    if (-not [string]::IsNullOrWhiteSpace($Explicit)) {
        if (Test-Path -LiteralPath $Explicit -PathType Leaf) {
            return [System.IO.Path]::GetFullPath($Explicit)
        }
        $explicitCommand = Get-Command $Explicit -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($null -ne $explicitCommand) { return [string]$explicitCommand.Source }
        return $null
    }

    $candidates = if ($IsWindows) {
        @("node_modules\.bin\claude-code-merge-queue.cmd", "node_modules\.bin\claude-code-merge-queue.exe")
    } else {
        @("node_modules/.bin/claude-code-merge-queue")
    }
    foreach ($candidate in $candidates) {
        $path = Join-Path $Repo $candidate
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            return [System.IO.Path]::GetFullPath($path)
        }
    }
    return $null
}

function Invoke-QueueCapture {
    param([string]$Command, [string[]]$Arguments, [string]$WorkingDirectory)
    Push-Location $WorkingDirectory
    try {
        $output = if ([System.IO.Path]::GetExtension($Command) -ieq ".ps1") {
            @(& pwsh -NoProfile -File $Command @Arguments 2>&1 | ForEach-Object { $_.ToString() })
        } else {
            @(& $Command @Arguments 2>&1 | ForEach-Object { $_.ToString() })
        }
        return [pscustomobject]@{ exitCode = $LASTEXITCODE; output = $output }
    } finally {
        Pop-Location
    }
}

function Invoke-QueueStreaming {
    param([string]$Command, [string[]]$Arguments, [string]$WorkingDirectory)
    Push-Location $WorkingDirectory
    try {
        if ([System.IO.Path]::GetExtension($Command) -ieq ".ps1") {
            & pwsh -NoProfile -File $Command @Arguments
        } else {
            & $Command @Arguments
        }
        return $LASTEXITCODE
    } finally {
        Pop-Location
    }
}

try {
    $policyDocument = Get-Content -Raw -LiteralPath $Policy | ConvertFrom-Json -Depth 20
} catch {
    Write-Error "Invalid merge queue policy: $($_.Exception.Message)"
    exit 2
}

$expectedGates = @(
    "git-repository", "provider-local-install", "provider-version", "provider-config",
    "acceptance-check-configured", "checks-required", "direct-push-protection",
    "human-promotion-boundary"
)
$policyShapePass = [string]$policyDocument.schema -eq "code-intel-multi-agent-merge-queue-policy.v1" -and
    [string]$policyDocument.source.revision -match '^[0-9a-f]{40}$' -and
    @($policyDocument.requiredLandingGates).Count -eq $expectedGates.Count -and
    @(Compare-Object @($policyDocument.requiredLandingGates | Sort-Object) @($expectedGates | Sort-Object)).Count -eq 0 -and
    @($policyDocument.forbiddenActions) -contains "promote" -and
    -not [bool]$policyDocument.authority.agentsMayPromoteProduction -and
    [bool]$policyDocument.authority.landRequiresExplicitRepositoryMutation -and
    [bool]$policyDocument.authority.landRequiresExplicitNetworkPush
if (-not $policyShapePass) {
    Write-Error "Merge queue policy shape or authority boundary is invalid."
    exit 2
}

$repo = [System.IO.Path]::GetFullPath($RepoPath)
$isGitRepo = $false
if (Test-Path -LiteralPath $repo -PathType Container) {
    & git -C $repo rev-parse --is-inside-work-tree *> $null
    $isGitRepo = $LASTEXITCODE -eq 0
}
Add-Gate "git-repository" $isGitRepo "repo=$repo"

$resolvedCommand = Resolve-QueueCommand -Repo $repo -Explicit $QueueCommand
Add-Gate "provider-local-install" ($null -ne $resolvedCommand) $(if ($null -ne $resolvedCommand) { "command=$resolvedCommand" } else { "repository-local claude-code-merge-queue command not found" })

$version = $null
$versionPass = $false
if ($null -ne $resolvedCommand) {
    $versionResult = Invoke-QueueCapture -Command $resolvedCommand -Arguments @("--version") -WorkingDirectory $repo
    if ($versionResult.exitCode -eq 0 -and @($versionResult.output).Count -gt 0) {
        $version = [string]@($versionResult.output)[-1]
        try {
            $versionPass = [version]$version -ge [version]([string]$policyDocument.provider.minimumVersion)
        } catch {
            $versionPass = $false
        }
    }
}
Add-Gate "provider-version" $versionPass $(if ($versionPass) { "version=$version" } else { "minimum=$($policyDocument.provider.minimumVersion); observed=$version" })

$configPath = @(
    (Join-Path $repo "claude-code-merge-queue.config.mjs"),
    (Join-Path $repo "claude-code-merge-queue.config.js")
) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
$configPass = $null -ne $configPath
Add-Gate "provider-config" $configPass $(if ($configPass) { "config=$configPath" } else { "queue config is missing" })

$configText = if ($configPass) { Get-Content -Raw -LiteralPath $configPath } else { "" }
$checkCommandValue = Get-JsScalar -Text $configText -Name "checkCommand"
$acceptanceCommand = ConvertFrom-JsStringLiteral $checkCommandValue
$acceptancePass = -not [string]::IsNullOrWhiteSpace($acceptanceCommand)
Add-Gate "acceptance-check-configured" $acceptancePass $(if ($acceptancePass) { "checkCommand is explicit" } else { "checkCommand must be a static non-empty string" })

$checksRequiredValue = Get-JsScalar -Text $configText -Name "checksRequired"
$checksRequiredPass = $configPass -and ([string]::IsNullOrWhiteSpace($checksRequiredValue) -or $checksRequiredValue -notmatch '(?i)^false$')
Add-Gate "checks-required" $checksRequiredPass $(if ($checksRequiredPass) { "checksRequired defaults to or is true" } else { "checksRequired:false is not admitted" })

$hookPath = $null
if ($isGitRepo) {
    $gitHookOutput = @(& git -C $repo rev-parse --git-path hooks 2>$null)
    if ($LASTEXITCODE -eq 0 -and $gitHookOutput.Count -gt 0) {
        $hooksRoot = [string]$gitHookOutput[-1]
        if (-not [System.IO.Path]::IsPathRooted($hooksRoot)) { $hooksRoot = Join-Path $repo $hooksRoot }
        $hookPath = Join-Path ([System.IO.Path]::GetFullPath($hooksRoot)) "pre-push"
    }
}
$hookPass = $null -ne $hookPath -and (Test-Path -LiteralPath $hookPath -PathType Leaf) -and
    (Get-Content -Raw -LiteralPath $hookPath) -match 'claude-code-merge-queue\s+check-push'
Add-Gate "direct-push-protection" $hookPass $(if ($hookPass) { "active hook=$hookPath" } else { "active pre-push hook does not enforce queue check-push" })

$integrationBranch = ConvertFrom-JsStringLiteral (Get-JsScalar -Text $configText -Name "integrationBranch")
$productionBranch = ConvertFrom-JsStringLiteral (Get-JsScalar -Text $configText -Name "productionBranch")
$promotionPass = -not [string]::IsNullOrWhiteSpace($integrationBranch) -and
    -not [string]::IsNullOrWhiteSpace($productionBranch) -and
    $integrationBranch -ne $productionBranch
Add-Gate "human-promotion-boundary" $promotionPass $(if ($promotionPass) { "integration=$integrationBranch; production=$productionBranch" } else { "distinct static integrationBranch and productionBranch are required" })

$failed = @($gates | Where-Object { -not $_.passed })
$status = [ordered]@{
    schema = "code-intel-multi-agent-merge-queue-status.v1"
    provider = "claude-code-merge-queue"
    sourceRevision = [string]$policyDocument.source.revision
    repo = $repo
    ready = $failed.Count -eq 0
    command = $resolvedCommand
    config = $configPath
    version = $version
    gates = @($gates)
    failedGateIds = @($failed | ForEach-Object id)
}

if ($Action -in @("status", "validate")) {
    if ($Json) {
        $status | ConvertTo-Json -Depth 10
    } else {
        Write-Host "Multi-Agent Merge Queue: $(if ($status.ready) { 'ready' } else { 'not-ready' })"
        foreach ($gate in $gates) {
            Write-Host "$(if ($gate.passed) { 'PASS' } else { 'FAIL' }) $($gate.id): $($gate.detail)"
        }
    }
    if ($Action -eq "validate" -and -not $status.ready) { exit 1 }
    exit 0
}

if ($Action -eq "land" -and (-not $AllowRepositoryMutation -or -not $AllowNetworkPush)) {
    Write-Error "land requires both -AllowRepositoryMutation and -AllowNetworkPush; readiness alone is not mutation authority."
    exit 1
}

$requiredForAction = if ($Action -eq "land") {
    $expectedGates
} else {
    @("git-repository", "provider-local-install", "provider-version", "provider-config")
}
$actionFailures = @($gates | Where-Object { $_.id -in $requiredForAction -and -not $_.passed })
if ($actionFailures.Count -gt 0) {
    Write-Error "$Action rejected by merge queue readiness: $($actionFailures.id -join ', ')"
    exit 1
}

$queueArguments = switch ($Action) {
    "land" { @("land") }
    "reconcile" { @("reconcile") }
    "history" { @("land-history") }
}
Write-Host "Multi-Agent Merge Queue: delegating '$Action' to the repository-local provider."
$queueExit = Invoke-QueueStreaming -Command $resolvedCommand -Arguments $queueArguments -WorkingDirectory $repo
exit $queueExit
