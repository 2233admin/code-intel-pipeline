[CmdletBinding()]
param(
    [string]$PluginRoot = (Join-Path $env:USERPROFILE ".sentrux\plugins"),
    [switch]$NoReadOnlyLock,
    [switch]$SkipValidate
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$overlayRoot = Join-Path $PSScriptRoot "overlays\sentrux\vlang"
$targetRoot = Join-Path $PluginRoot "vlang"
$requiredFiles = @(
    "plugin.toml",
    "queries\tags.scm",
    "grammars\windows-x86_64.dll"
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
        & attrib -R $targetPath
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
        & attrib +R $targetPath
    }
}

$validationStatus = "skipped"
$validationDetail = ""

if (-not $SkipValidate) {
    if (-not (Get-Command sentrux -ErrorAction SilentlyContinue)) {
        throw "sentrux CLI not found in PATH"
    }
    $validateOutput = & sentrux plugin validate $targetRoot 2>&1
    $validationDetail = ($validateOutput | ForEach-Object { $_.ToString() } | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        if ($validationDetail -match "unknown command 'plugin'" -or $validationDetail -match "unknown command `"plugin`"") {
            $validationStatus = "skipped"
            $validationDetail = "current sentrux is lite/shim and has no plugin command"
        }
        else {
        throw "sentrux plugin validate failed for $targetRoot"
        }
    }
    else {
        $validationStatus = "passed"
    }
}

$global:LASTEXITCODE = 0

[pscustomobject][ordered]@{
    status = "installed"
    plugin = "vlang"
    target = $targetRoot
    backup = if (Test-Path -LiteralPath $backupPath) { $backupPath } else { $null }
    readOnlyLock = -not $NoReadOnlyLock
    validation = $validationStatus
    validationDetail = $validationDetail
} | ConvertTo-Json -Depth 4
