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
$sentruxFailures = [ordered]@{
    schema = "code-intel-sentrux-failures.v1"
    status = "failed"
    primary = [ordered]@{
        id = "check:max_cc:run-code-intel.ps1:Get-CodeEvidenceSymbols"
        kind = "max_cc"
        stdout_excerpt = "run-code-intel.ps1:Get-CodeEvidenceSymbols (cc=311)"
        target = [ordered]@{
            status = "resolved"
            file = "run-code-intel.ps1"
            symbol = "Get-CodeEvidenceSymbols"
        }
    }
}

$skipDir = Join-Path $base "skip"
$skipResult = & $helper -RepoPath $RepoPath -ArtifactDir $skipDir -FailedSteps $failedSteps -FailureClassifications $classifications -SentruxFailures $sentruxFailures -SkipGitHubResearch
if (-not [bool]$skipResult.required -or [string]$skipResult.status -ne "manual_required") {
    throw "-SkipGitHubResearch should produce required manual research."
}
if (-not (Test-Path -LiteralPath $skipResult.path -PathType Leaf) -or -not (Test-Path -LiteralPath $skipResult.markdown -PathType Leaf)) {
    throw "-SkipGitHubResearch should write JSON and markdown artifacts."
}
$skipArtifact = Read-JsonFile $skipResult.path
if ($null -eq $skipArtifact.sentruxFailures -or [string]$skipArtifact.sentruxFailures.schema -ne "code-intel-sentrux-failures.v1") {
    throw "GitHub research should preserve normalized Sentrux context."
}
if (@($skipArtifact.queries | Where-Object { [string]$_.step -eq "sentrux normalized failure" }).Count -ne 1) {
    throw "GitHub research should seed a normalized Sentrux query."
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
$ghLog = Join-Path $base "gh-args.log"
@'
@echo off
echo %*>>"%CODE_INTEL_GH_TEST_LOG%"
echo [{"title":"Known blocker","fullName":"owner/repo","path":"src/fix.rs","url":"https://github.com/owner/repo/issues/1","sha":"0123456789abcdef","state":"OPEN","stargazersCount":42,"language":"PowerShell","updatedAt":"2026-01-01T00:00:00Z","repository":{"fullName":"owner/repo"}}]
'@ | Set-Content -LiteralPath $fakeGh -Encoding ASCII

$oldPath = $env:PATH
try {
    $env:CODE_INTEL_GH_TEST_LOG = $ghLog
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
    if (@($artifact.invocations).Count -eq 0 -or [string]$artifact.invocations[0].argv[0] -ne "search") {
        throw "GitHub research artifact must bind actual argv and exit codes."
    }
    if ([string]$artifact.candidates[0].sourceRevision -ne "0123456789abcdef") {
        throw "GitHub research artifact must bind returned source revisions."
    }
    $invocations = @(Get-Content -LiteralPath $ghLog)
    if ($invocations.Count -eq 0 -or @($invocations | Where-Object { $_ -notmatch '^search ' }).Count -ne 0) {
        throw "GitHub research must invoke only read-only gh search commands."
    }
}
finally {
    Remove-Item Env:CODE_INTEL_GH_TEST_LOG -ErrorAction SilentlyContinue
    $env:PATH = $oldPath
}

$fakeToken = "ghp_" + "supersecret123456789"
@"
@echo off
echo HTTP 429 API rate limit token=$fakeToken 1>&2
exit /b 1
"@ | Set-Content -LiteralPath $fakeGh -Encoding ASCII
$oldPath = $env:PATH
try {
    $env:PATH = $fakeBin + [IO.Path]::PathSeparator + $oldPath
    $rateDir = Join-Path $base "rate-limited"
    $rateResult = & $helper -RepoPath $RepoPath -ArtifactDir $rateDir -FailedSteps $failedSteps -FailureClassifications $classifications
    if ([string]$rateResult.status -ne "manual_required" -or [string]$rateResult.reason -notmatch "429|rate limit") {
        throw "Rate-limited gh should fail closed to manual_required."
    }
    $rateText = Get-Content -LiteralPath $rateResult.path -Raw
    if ($rateText -match "ghp_supersecret") {
        throw "Credential-like token leaked into the research artifact."
    }
    if ($rateText -notmatch "REDACTED") {
        throw "Credential-like token was not visibly redacted."
    }
}
finally {
    $env:PATH = $oldPath
}

Write-Host "GitHub solution research tests passed: $base"
$global:LASTEXITCODE = 0
