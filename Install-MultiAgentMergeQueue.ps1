#requires -Version 7.2

param(
    [string]$RepoPath = $PSScriptRoot,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repo = [System.IO.Path]::GetFullPath($RepoPath)
$adapter = Join-Path $repo "Invoke-MultiAgentMergeQueue.ps1"
$hook = Join-Path $repo ".githooks\pre-push"
$lockFile = Join-Path $repo "package-lock.json"

foreach ($required in @($adapter, $hook, $lockFile)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Merge queue activation requires repository file: $required"
    }
}

& git -C $repo rev-parse --is-inside-work-tree *> $null
if ($LASTEXITCODE -ne 0) { throw "Not a Git repository: $repo" }

$npmName = if ($IsWindows) { "npm.cmd" } else { "npm" }
$npm = Get-Command $npmName -CommandType Application -ErrorAction Stop | Select-Object -First 1
Push-Location $repo
try {
    & $npm.Source ci --ignore-scripts --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) { throw "npm ci failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}

$localHooksPath = @(& git -C $repo config --local --get core.hooksPath 2>$null) | Select-Object -Last 1
if ([string]::IsNullOrWhiteSpace($localHooksPath) -or $localHooksPath -ne ".githooks") {
    $effectiveHooksPath = @(& git -C $repo config --get core.hooksPath 2>$null) | Select-Object -Last 1
    if (-not [string]::IsNullOrWhiteSpace($effectiveHooksPath) -and $effectiveHooksPath -ne ".githooks") {
        & git -C $repo config --local codeIntel.mergeQueue.previousHooksPath $effectiveHooksPath
        if ($LASTEXITCODE -ne 0) { throw "Could not preserve the previous hooks path." }
    }
    & git -C $repo config --local core.hooksPath .githooks
    if ($LASTEXITCODE -ne 0) { throw "Could not activate the repository hooks path." }
}

if (-not $IsWindows) {
    & chmod +x $hook
    if ($LASTEXITCODE -ne 0) { throw "Could not make the pre-push hook executable." }
}

$statusJson = @(& pwsh -NoProfile -File $adapter -Action validate -RepoPath $repo -Json 2>&1) -join "`n"
if ($LASTEXITCODE -ne 0) { throw "Merge queue validation failed: $statusJson" }
$status = $statusJson | ConvertFrom-Json
if (-not [bool]$status.ready) { throw "Merge queue activation did not reach readiness." }

if ($Json) {
    $status | ConvertTo-Json -Depth 10
} else {
    Write-Host "Multi-Agent Merge Queue activated: readiness $(@($status.gates | Where-Object passed).Count)/$(@($status.gates).Count)"
    Write-Host "Provider: $($status.command)"
    Write-Host "Hooks: .githooks (previous path preserved in local Git config when present)"
}
