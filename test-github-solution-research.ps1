param(
    [string]$RepoPath = $PSScriptRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Read-JsonFile {
    param([string]$Path)
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function New-TestStep {
    [ordered]@{
        name = "sentrux gate"
        status = "failed"
        error = "sentrux.exe failed error code: 1"
        output = ""
    }
}

function New-TestClassification {
    [ordered]@{
        category = "sentrux_fail"
        name = "sentrux gate"
    }
}

$root = Split-Path -Parent $PSCommandPath
$helper = Join-Path $root "Invoke-GitHubSolutionResearch.ps1"
$base = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-gh-research-test-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $base | Out-Null

$failedSteps = @(New-TestStep)
$classifications = @(New-TestClassification)

$skipDir = Join-Path $base "skip"
$skipResult = & $helper -RepoPath $RepoPath -ArtifactDir $skipDir -FailedSteps $failedSteps -FailureClassifications $classifications -SkipGitHubResearch
if (-not [bool]$skipResult.required -or [string]$skipResult.status -ne "manual_required") {
    throw "-SkipGitHubResearch should produce required manual research."
}
if (-not (Test-Path -LiteralPath $skipResult.path -PathType Leaf) -or -not (Test-Path -LiteralPath $skipResult.markdown -PathType Leaf)) {
    throw "-SkipGitHubResearch should write JSON and markdown artifacts."
}

$oldPath = $env:PATH
try {
    $emptyPath = Join-Path $base "empty-path"
    New-Item -ItemType Directory -Force -Path $emptyPath | Out-Null
    $env:PATH = $emptyPath
    $missingGhDir = Join-Path $base "missing-gh"
    $missingGhResult = & $helper -RepoPath $RepoPath -ArtifactDir $missingGhDir -FailedSteps $failedSteps -FailureClassifications $classifications
    if ([string]$missingGhResult.status -ne "manual_required" -or [string]$missingGhResult.reason -notmatch "gh") {
        throw "Missing gh should produce manual_required with a gh reason."
    }
}
finally {
    $env:PATH = $oldPath
}

$fakeBin = Join-Path $base "fake-gh"
New-Item -ItemType Directory -Force -Path $fakeBin | Out-Null
$fakeGh = Join-Path $fakeBin "gh.cmd"
@'
@echo off
echo [{"title":"Known blocker","fullName":"owner/repo","path":"src/fix.rs","url":"https://github.com/owner/repo/issues/1","state":"OPEN","stargazersCount":42,"language":"PowerShell","updatedAt":"2026-01-01T00:00:00Z","repository":{"fullName":"owner/repo"}}]
'@ | Set-Content -LiteralPath $fakeGh -Encoding ASCII

$oldPath = $env:PATH
try {
    $env:PATH = $fakeBin + [IO.Path]::PathSeparator + $oldPath
    $fakeGhDir = Join-Path $base "fake-gh-run"
    $fakeGhResult = & $helper -RepoPath $RepoPath -ArtifactDir $fakeGhDir -FailedSteps $failedSteps -FailureClassifications $classifications
    if ([string]$fakeGhResult.status -ne "auto_generated") {
        throw "Fake gh should produce auto_generated research."
    }
    if ([int]$fakeGhResult.candidates -le 0 -or @($fakeGhResult.evidenceLinks).Count -le 0) {
        throw "Fake gh should produce candidates and evidence links."
    }
    $artifact = Read-JsonFile $fakeGhResult.path
    if ([string]$artifact.schema -ne "github-solution-research.v1") {
        throw "GitHub research artifact schema mismatch."
    }
}
finally {
    $env:PATH = $oldPath
}

Write-Host "GitHub solution research tests passed: $base"
