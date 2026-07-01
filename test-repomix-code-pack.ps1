Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$helper = Join-Path $root "Invoke-RepomixCodePack.ps1"
$base = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-repomix-test-" + [guid]::NewGuid().ToString("N"))
$fakeBin = Join-Path $base "bin"
$artifactDir = Join-Path $base "artifacts"
$repoDir = Join-Path $base "repo"
New-Item -ItemType Directory -Force -Path $fakeBin, $artifactDir, $repoDir | Out-Null

$fakeRepomix = Join-Path $fakeBin "repomix.cmd"
@'
@echo off
setlocal enabledelayedexpansion
set out=
:loop
if "%~1"=="" goto done
if "%~1"=="-o" set "out=%~2"
shift
goto loop
:done
if "%out%"=="" exit /b 2
echo repomix fake output > "%out%"
echo packed %out%
exit /b 0
'@ | Set-Content -LiteralPath $fakeRepomix -Encoding ASCII

$oldPath = $env:PATH
try {
    $env:PATH = $fakeBin + [IO.Path]::PathSeparator + $env:PATH

    $local = & $helper -RepoPath $repoDir -ArtifactDir $artifactDir -Style markdown
    if ([string]$local.status -ne "ok") { throw "Local Repomix pack should be ok." }
    if ([string]$local.style -ne "markdown") { throw "Local Repomix style should be markdown." }
    if (-not (Test-Path -LiteralPath $local.path -PathType Leaf)) { throw "Local Repomix output missing." }
    $localSummary = Get-Content -LiteralPath $local.summaryPath -Raw | ConvertFrom-Json
    if ([string]$localSummary.schema -ne "code-intel-repomix-pack.v1") { throw "Local Repomix summary schema mismatch." }

    $remoteDir = Join-Path $base "remote-artifacts"
    $remote = & $helper -Remote "yamadashy/repomix" -ArtifactDir $remoteDir -Style xml -Compress
    if ([string]$remote.status -ne "ok") { throw "Remote Repomix pack should be ok." }
    if ([string]$remote.remote -ne "yamadashy/repomix") { throw "Remote target not preserved." }
    if (@($remote.command | Where-Object { [string]$_ -eq "--remote" }).Count -ne 1) { throw "Remote command should include --remote." }
    if (@($remote.command | Where-Object { [string]$_ -eq "--compress" }).Count -ne 1) { throw "Compressed command should include --compress." }

    Write-Host "Repomix code pack tests passed."
}
finally {
    $env:PATH = $oldPath
    if (Test-Path -LiteralPath $base) {
        Remove-Item -LiteralPath $base -Recurse -Force
    }
}
