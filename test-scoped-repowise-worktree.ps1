# Regression coverage for scoped worktree creation on Windows.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$scratch = Join-Path $env:TEMP ("cip-scoped-worktree-{0}" -f ([guid]::NewGuid().ToString("N").Substring(0, 8)))
$repo = Join-Path $scratch "repo"
$shadowRoot = Join-Path $scratch "shadow"
$fakeBin = Join-Path $scratch "bin"
$oldPath = $env:PATH

try {
    New-Item -ItemType Directory -Force -Path (Join-Path $repo "included"),$fakeBin | Out-Null
    Set-Content -LiteralPath (Join-Path $repo "included\kept.txt") -Value "kept" -Encoding ASCII

    & git -C $repo init -q
    & git -C $repo config user.email "test@example.invalid"
    & git -C $repo config user.name "Code Intel Test"
    & git -C $repo config core.longpaths false
    & git -C $repo add included/kept.txt

    $blob = ("excluded" | & git -C $repo hash-object -w --stdin).Trim()
    $segments = 1..6 | ForEach-Object { "segment-$($_)-$([string]'x' * 40)" }
    $excludedPath = (($segments + "excluded.txt") -join "/")
    "100644 $blob`t$excludedPath" | & git -C $repo update-index --add --index-info
    & git -C $repo commit -qm "fixture"
    if ($LASTEXITCODE -ne 0) { throw "Failed to create Git fixture" }

    @"
@echo off
if not exist .repowise mkdir .repowise
if not exist .repowise\state.json echo {}>.repowise\state.json
if "%1"=="status" echo fake repowise ready
exit /b 0
"@ | Set-Content -LiteralPath (Join-Path $fakeBin "repowise.cmd") -Encoding ASCII
    $env:PATH = "$fakeBin;$oldPath"

    & (Join-Path $root "Invoke-ScopedRepowise.ps1") `
        -RepoPath $repo `
        -ShadowRoot $shadowRoot `
        -Platform windows `
        -ScopePaths @("included") `
        -Provider mock `
        -AllowShadowWorktreeMutation `
        -TimeoutSeconds 30 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Invoke-ScopedRepowise.ps1 exited $LASTEXITCODE" }

    $shadow = Join-Path $shadowRoot "repo-included"
    if (-not (Test-Path -LiteralPath (Join-Path $shadow "included\kept.txt") -PathType Leaf)) {
        throw "Scoped file was not checked out"
    }
    if (Test-Path -LiteralPath (Join-Path $shadow ($excludedPath.Replace("/", "\")))) {
        throw "Excluded long path was materialized"
    }
    Write-Host "PASS: scoped worktree excludes long paths before checkout"
}
finally {
    $env:PATH = $oldPath
    if (Test-Path -LiteralPath $repo) {
        & git -C $repo worktree prune --expire now 2>$null
    }
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
