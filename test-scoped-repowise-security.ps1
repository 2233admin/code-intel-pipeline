# Security regression coverage for scoped Repowise egress.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$script = Join-Path $root "Invoke-ScopedRepowise.ps1"
$scratch = Join-Path $env:TEMP ("cip-scoped-security-{0}" -f ([guid]::NewGuid().ToString("N").Substring(0, 8)))
$repo = Join-Path $scratch "repo"
$outside = Join-Path $scratch "outside"
$shadowRoot = Join-Path $scratch "shadow"
$fakeBin = Join-Path $scratch "bin"
$providerLog = Join-Path $scratch "provider.log"
$oldPath = $env:PATH
$oldProviderLog = $env:CIP_TEST_PROVIDER_LOG

$authorityError = $null
try {
    & $script -RepoPath $repo -ScopePaths @("included") | Out-Null
}
catch {
    $authorityError = $_
}
if ($null -eq $authorityError -or $authorityError.Exception.Message -notmatch "explicit -AllowShadowWorktreeMutation authority") {
    throw "Scoped Repowise did not fail closed before unapproved Git mutation"
}

$modelPinError = $null
try {
    & $script `
        -RepoPath $repo `
        -Provider anthropic `
        -ConsumptionConsent granted `
        -ExternalDataConsent granted `
        -PaidSpendConsent granted `
        -CostScope metered_api `
        -AllowShadowWorktreeMutation `
        -Docs | Out-Null
}
catch {
    $modelPinError = $_
}
if ($null -eq $modelPinError -or $modelPinError.Exception.Message -notmatch "explicitly pinned model") {
    throw "Provider-backed docs accepted an implicit environment/provider model"
}

function Assert-RejectedScope {
    param(
        [string[]]$ScopePaths = @(),
        [string[]]$RootFiles = @(),
        [string]$ExpectedMessage
    )

    $before = if (Test-Path -LiteralPath $providerLog) { @(Get-Content -LiteralPath $providerLog).Count } else { 0 }
    $caught = $null
    try {
        & $script `
            -RepoPath $repo `
            -ShadowRoot $shadowRoot `
            -Platform windows `
            -ScopePaths $ScopePaths `
            -RootFiles $RootFiles `
            -Provider mock `
            -AllowShadowWorktreeMutation `
            -TimeoutSeconds 30 | Out-Null
    }
    catch {
        $caught = $_
    }

    if ($null -eq $caught) {
        throw "Expected scoped invocation to reject: $ExpectedMessage"
    }
    if ($caught.Exception.Message -notmatch $ExpectedMessage) {
        throw "Unexpected rejection message: $($caught.Exception.Message)"
    }
    $after = if (Test-Path -LiteralPath $providerLog) { @(Get-Content -LiteralPath $providerLog).Count } else { 0 }
    if ($after -ne $before) {
        throw "Provider process ran after unsafe scope rejection"
    }
}

try {
    New-Item -ItemType Directory -Force -Path (Join-Path $repo "included"),$outside,$fakeBin | Out-Null
    Set-Content -LiteralPath (Join-Path $repo "included\kept.txt") -Value "clean" -Encoding ASCII
    Set-Content -LiteralPath (Join-Path $repo "root.txt") -Value "root" -Encoding ASCII
    Set-Content -LiteralPath (Join-Path $repo ".gitignore") -Value "included/ignored.txt" -Encoding ASCII
    Set-Content -LiteralPath (Join-Path $outside "secret.txt") -Value "secret" -Encoding ASCII

    & git -C $repo init -q
    & git -C $repo config user.email "test@example.invalid"
    & git -C $repo config user.name "Code Intel Test"
    & git -C $repo add included/kept.txt root.txt .gitignore
    & git -C $repo commit -qm "fixture"
    if ($LASTEXITCODE -ne 0) { throw "Failed to create Git fixture" }

    New-Item -ItemType Junction -Path (Join-Path $repo "escape-link") -Target $outside | Out-Null

    @"
@echo off
if not exist .repowise\egress-manifest.json exit /b 91
echo %*>> "%CIP_TEST_PROVIDER_LOG%"
if not exist .repowise mkdir .repowise
if not exist .repowise\state.json echo {}>.repowise\state.json
if "%1"=="status" echo fake repowise ready
exit /b 0
"@ | Set-Content -LiteralPath (Join-Path $fakeBin "repowise.cmd") -Encoding ASCII
    $env:CIP_TEST_PROVIDER_LOG = $providerLog
    $env:PATH = "$fakeBin;$oldPath"

    Assert-RejectedScope -ScopePaths @((Join-Path $repo "included")) -ExpectedMessage "relative"
    Assert-RejectedScope -ScopePaths @("..\outside") -ExpectedMessage "traversal"
    Assert-RejectedScope -RootFiles @((Join-Path $repo "root.txt")) -ExpectedMessage "relative"
    Assert-RejectedScope -RootFiles @("included\..\root.txt") -ExpectedMessage "traversal"
    Assert-RejectedScope -ScopePaths @("escape-link") -ExpectedMessage "escape"
    Assert-RejectedScope -RootFiles @("escape-link\secret.txt") -ExpectedMessage "escape"

    Set-Content -LiteralPath (Join-Path $repo "included\kept.txt") -Value "dirty" -Encoding ASCII
    Set-Content -LiteralPath (Join-Path $repo "included\untracked.txt") -Value "untracked" -Encoding ASCII
    Set-Content -LiteralPath (Join-Path $repo "included\ignored.txt") -Value "ignored" -Encoding ASCII

    & $script `
        -RepoPath $repo `
        -ShadowRoot $shadowRoot `
        -Platform windows `
        -ScopePaths @("included") `
        -Provider mock `
        -AllowShadowWorktreeMutation `
        -TimeoutSeconds 30 | Out-Null

    $shadow = Join-Path $shadowRoot "repo-included"
    $trackedValue = (Get-Content -LiteralPath (Join-Path $shadow "included\kept.txt") -Raw).Trim()
    if ($trackedValue -ne "clean") {
        throw "Default policy leaked dirty tracked content: $trackedValue"
    }
    if (Test-Path -LiteralPath (Join-Path $shadow "included\untracked.txt")) {
        throw "Default policy leaked an untracked file"
    }
    if (Test-Path -LiteralPath (Join-Path $shadow "included\ignored.txt")) {
        throw "Default policy leaked an ignored file"
    }

    $manifestPath = Join-Path $shadow ".repowise\egress-manifest.json"
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    $head = (& git -C $repo rev-parse HEAD).Trim()
    if ($manifest.head -ne $head) { throw "Manifest HEAD mismatch" }
    if ($manifest.provider -ne "mock") { throw "Manifest provider mismatch" }
    if ($manifest.working_tree_policy -ne "head-tracked-only") { throw "Default policy missing from manifest" }
    if (@($manifest.scope.paths) -notcontains "included") { throw "Scope path missing from manifest" }
    if ($manifest.schema_version -ne 2) { throw "Manifest schema version mismatch" }
    if ($manifest.provider_payload_state -ne "pending" -or @($manifest.provider_payload).Count -ne 0) {
        throw "PowerShell manifest must leave provider payload pending for Python traversal"
    }
    $manifestPaths = @($manifest.scope_inventory.path)
    $sortedManifestPaths = [string[]]@($manifestPaths)
    [Array]::Sort($sortedManifestPaths, [StringComparer]::Ordinal)
    if (($manifestPaths -join "`n") -ne ($sortedManifestPaths -join "`n")) {
        throw "Manifest files are not sorted by path"
    }
    $keptEntry = @($manifest.scope_inventory | Where-Object { $_.path -eq "included/kept.txt" })
    if ($keptEntry.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$keptEntry[0].sha256)) {
        throw "Manifest does not contain the scoped file hash"
    }
    $expectedHash = (Get-FileHash -LiteralPath (Join-Path $shadow "included\kept.txt") -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($keptEntry[0].sha256 -ne $expectedHash) { throw "Manifest file hash mismatch" }

    & $script `
        -RepoPath $repo `
        -ShadowRoot $shadowRoot `
        -Platform windows `
        -ScopePaths @("included") `
        -Provider mock `
        -AllowShadowWorktreeMutation `
        -IncludeWorkingTree `
        -TimeoutSeconds 30 | Out-Null

    if ((Get-Content -LiteralPath (Join-Path $shadow "included\kept.txt") -Raw).Trim() -ne "dirty") {
        throw "IncludeWorkingTree did not preserve dirty tracked content"
    }
    foreach ($name in @("untracked.txt", "ignored.txt")) {
        if (-not (Test-Path -LiteralPath (Join-Path $shadow "included\$name") -PathType Leaf)) {
            throw "IncludeWorkingTree did not preserve $name"
        }
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.working_tree_policy -ne "include-working-tree") {
        throw "Explicit working-tree policy missing from manifest"
    }
    foreach ($path in @("included/kept.txt", "included/untracked.txt", "included/ignored.txt")) {
        if (@($manifest.scope_inventory.path) -notcontains $path) { throw "Manifest missing $path" }
    }

    Write-Host "PASS: scoped Repowise rejects path escape and makes working-tree egress explicit"
}
finally {
    $env:PATH = $oldPath
    $env:CIP_TEST_PROVIDER_LOG = $oldProviderLog
    if (Test-Path -LiteralPath $repo) {
        & git -C $repo worktree prune --expire now 2>$null
    }
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
