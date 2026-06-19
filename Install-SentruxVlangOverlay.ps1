#requires -Version 7.2

[CmdletBinding()]
param(
    [string]$PluginRoot = "",
    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto",
    [switch]$NoReadOnlyLock,
    [switch]$SkipValidate
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$platformModule = Join-Path (Join-Path $PSScriptRoot "tools") "code-intel-platform.psm1"
Import-Module $platformModule -Force
$effectivePlatform = Get-CodeIntelPlatform -Platform $Platform

if ([string]::IsNullOrWhiteSpace($PluginRoot)) {
    $PluginRoot = Join-Path (Join-Path (Get-CodeIntelHomeDirectory) ".sentrux") "plugins"
}

$overlayRoot = Join-Path (Join-Path (Join-Path $PSScriptRoot "overlays") "sentrux") "vlang"
$targetRoot = Join-Path $PluginRoot "vlang"
$grammarName = switch ($effectivePlatform) {
    "windows" { "windows-x86_64.dll" }
    "macos" { "darwin-arm64.dylib" }
    "linux" { "linux-x86_64.so" }
}
$requiredFiles = @(
    "plugin.toml",
    (Join-Path "queries" "tags.scm"),
    (Join-Path "grammars" $grammarName)
)

function Test-SameOverlayFile {
    param(
        [string]$SourcePath,
        [string]$TargetPath
    )

    if (-not (Test-Path -LiteralPath $TargetPath -PathType Leaf)) {
        return $false
    }

    $sourceItem = Get-Item -LiteralPath $SourcePath
    $targetItem = Get-Item -LiteralPath $TargetPath
    if ($sourceItem.Length -ne $targetItem.Length) {
        return $false
    }

    try {
        $sourceBytes = [System.IO.File]::ReadAllBytes($SourcePath)
        $targetBytes = [System.IO.File]::ReadAllBytes($TargetPath)
        for ($i = 0; $i -lt $sourceBytes.Length; $i++) {
            if ($sourceBytes[$i] -ne $targetBytes[$i]) {
                return $false
            }
        }
        return $true
    }
    catch {
        return $true
    }
}

foreach ($relativePath in $requiredFiles) {
    $sourcePath = Join-Path $overlayRoot $relativePath
    if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        if ($relativePath -like (Join-Path "grammars" "*")) {
            [pscustomobject][ordered]@{
                status = "manual_required"
                plugin = "vlang"
                platform = $effectivePlatform
                missing = $sourcePath
                message = "No vlang grammar artifact is bundled for this platform; skipping overlay install."
            } | ConvertTo-Json -Depth 4
            exit 0
        }
        throw "Overlay file missing: $sourcePath"
    }
}

$backupRoot = Join-Path ([System.IO.Path]::GetTempPath()) "sentrux-plugin-backup"
$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$backupPath = Join-Path $backupRoot "vlang-$timestamp"
New-Item -ItemType Directory -Force -Path $backupRoot | Out-Null

if (Test-Path -LiteralPath $targetRoot) {
    Copy-Item -LiteralPath $targetRoot -Destination $backupPath -Recurse -Force
}

New-Item -ItemType Directory -Force -Path $PluginRoot | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $targetRoot "queries") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $targetRoot "grammars") | Out-Null

foreach ($relativePath in $requiredFiles) {
    $targetPath = Join-Path $targetRoot $relativePath
    if (Test-Path -LiteralPath $targetPath) {
        if ($effectivePlatform -eq "windows" -and (Get-Command attrib -ErrorAction SilentlyContinue)) {
            & attrib -R $targetPath
        }
        else {
            (Get-Item -LiteralPath $targetPath).IsReadOnly = $false
        }
    }
}

foreach ($relativePath in $requiredFiles) {
    $sourcePath = Join-Path $overlayRoot $relativePath
    $targetPath = Join-Path $targetRoot $relativePath
    if (Test-SameOverlayFile $sourcePath $targetPath) {
        continue
    }
    Copy-Item -LiteralPath $sourcePath -Destination $targetPath -Force
}

if (-not $NoReadOnlyLock) {
    foreach ($relativePath in $requiredFiles) {
        $targetPath = Join-Path $targetRoot $relativePath
        if ($effectivePlatform -eq "windows" -and (Get-Command attrib -ErrorAction SilentlyContinue)) {
            & attrib +R $targetPath
        }
        else {
            (Get-Item -LiteralPath $targetPath).IsReadOnly = $true
        }
    }
}

if (-not $SkipValidate) {
    if (-not (Get-Command sentrux -ErrorAction SilentlyContinue)) {
        throw "sentrux CLI not found in PATH"
    }
    & sentrux plugin validate $targetRoot
    if ($LASTEXITCODE -ne 0) {
        throw "sentrux plugin validate failed for $targetRoot"
    }
}

[pscustomobject][ordered]@{
    status = "installed"
    plugin = "vlang"
    platform = $effectivePlatform
    target = $targetRoot
    backup = if (Test-Path -LiteralPath $backupPath) { $backupPath } else { $null }
    readOnlyLock = -not $NoReadOnlyLock
} | ConvertTo-Json -Depth 4
