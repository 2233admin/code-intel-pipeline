[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArgs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$RemainingArgs = @($RemainingArgs)

$Features = @(
    "dsm_export",
    "file_detail_panel",
    "evolution_details",
    "what_if_analysis",
    "agent_mcp",
    "rule_gates",
    "nine_color_modes"
)

function Get-LicensePath {
    if (-not [string]::IsNullOrWhiteSpace($env:SENTRUX_LICENSE_FILE)) {
        return $env:SENTRUX_LICENSE_FILE
    }
    if (-not [string]::IsNullOrWhiteSpace($env:APPDATA)) {
        return (Join-Path $env:APPDATA "sentrux\license.json")
    }
    return (Join-Path $HOME ".sentrux\license.json")
}

function Get-AutoDisabledPath {
    $licensePath = Get-LicensePath
    $licenseDir = Split-Path -Parent $licensePath
    return (Join-Path $licenseDir "auto-pro.disabled")
}

function Get-KeyPreview {
    param([string]$Key)
    if ($Key.Length -le 8) { return "********" }
    return ("{0}...{1}" -f $Key.Substring(0, 4), $Key.Substring($Key.Length - 4))
}

function Get-KeyFingerprint {
    param([string]$Key)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Key)
        $hash = $sha.ComputeHash($bytes)
        return (($hash | Select-Object -First 8 | ForEach-Object { $_.ToString("x2") }) -join "")
    }
    finally {
        $sha.Dispose()
    }
}

function Write-License {
    param(
        [string]$Key,
        [string]$Source
    )

    if ([string]::IsNullOrWhiteSpace($Key)) {
        throw "license key cannot be empty"
    }

    $licensePath = Get-LicensePath
    $licenseDir = Split-Path -Parent $licensePath
    New-Item -ItemType Directory -Force -Path $licenseDir | Out-Null

    $license = [ordered]@{
        tier = "pro"
        status = "active"
        source = $Source
        key_preview = Get-KeyPreview $Key
        key_fingerprint = Get-KeyFingerprint $Key
        activated_at = (Get-Date).ToUniversalTime().ToString("o")
        features = $Features
    }
    $license | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $licensePath -Encoding UTF8
}

function Clear-AutoDisabled {
    $path = Get-AutoDisabledPath
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        Remove-Item -LiteralPath $path -Force
    }
}

function Ensure-AutoActivation {
    if ($env:SENTRUX_AUTO_PRO -in @("0", "false", "False", "FALSE")) {
        return
    }

    $licensePath = Get-LicensePath
    $disabledPath = Get-AutoDisabledPath
    if ((Test-Path -LiteralPath $licensePath -PathType Leaf) -or (Test-Path -LiteralPath $disabledPath -PathType Leaf)) {
        return
    }

    Write-License "OSS-AUTO-PRO" "auto-open-source"
}

function Show-ProStatus {
    Ensure-AutoActivation
    $licensePath = Get-LicensePath
    if (-not (Test-Path -LiteralPath $licensePath -PathType Leaf)) {
        Write-Output "Tier: free"
        Write-Output "Status: inactive"
        Write-Output "License: $licensePath"
        Write-Output "Features: check, gate, scan, mcp, plugin, analytics"
        return
    }

    $license = Get-Content -LiteralPath $licensePath -Raw | ConvertFrom-Json
    if ($license.tier -eq "pro" -and $license.status -eq "active") {
        Write-Output "Tier: pro"
        Write-Output "Status: active"
        Write-Output "License: $licensePath"
        if ($license.PSObject.Properties["key_preview"]) {
            Write-Output "Key: $($license.key_preview)"
        }
        Write-Output "Features: $($Features -join ', ')"
        return
    }

    Write-Output "Tier: free"
    Write-Output "Status: inactive"
    Write-Output "License: $licensePath"
}

function Deactivate-Pro {
    $licensePath = Get-LicensePath
    if (Test-Path -LiteralPath $licensePath -PathType Leaf) {
        Remove-Item -LiteralPath $licensePath -Force
    }

    $disabledPath = Get-AutoDisabledPath
    $disabledDir = Split-Path -Parent $disabledPath
    New-Item -ItemType Directory -Force -Path $disabledDir | Out-Null
    "disabled" | Set-Content -LiteralPath $disabledPath -Encoding UTF8

    Write-Output "Sentrux Pro deactivated"
    Write-Output "Tier: free"
    Write-Output "Auto activation: disabled until sentrux pro activate <key>"
}

function Show-ProHelp {
    Write-Output "Manage local open-source Sentrux Pro activation"
    Write-Output ""
    Write-Output "Usage: sentrux pro <COMMAND>"
    Write-Output ""
    Write-Output "Commands:"
    Write-Output "  activate <key>  Save license and enable Pro features"
    Write-Output "  status          Show tier, status, license path, and features"
    Write-Output "  deactivate      Remove local license and return to free tier"
}

function Resolve-Core {
    if (-not [string]::IsNullOrWhiteSpace($env:SENTRUX_CORE_EXE) -and (Test-Path -LiteralPath $env:SENTRUX_CORE_EXE -PathType Leaf)) {
        return (Get-Item -LiteralPath $env:SENTRUX_CORE_EXE).FullName
    }

    $shimDir = Split-Path -Parent $PSCommandPath
    $parent = Split-Path -Parent $shimDir
    $candidates = New-Object System.Collections.Generic.List[string]
    foreach ($path in @(
        (Join-Path $shimDir "sentrux-core.exe"),
        (Join-Path $parent "sentrux.exe"),
        (Join-Path $parent "sentrux-core.exe")
    )) {
        $candidates.Add($path)
    }

    $pathEntries = @($env:PATH -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    foreach ($entry in $pathEntries) {
        $fullEntry = try { (Get-Item -LiteralPath $entry -ErrorAction Stop).FullName } catch { $entry }
        if ([string]::Equals($fullEntry.TrimEnd('\'), $shimDir.TrimEnd('\'), [System.StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        $candidates.Add((Join-Path $entry "sentrux.exe"))
        $candidates.Add((Join-Path $entry "sentrux-core.exe"))
    }

    $selfCmd = Join-Path $shimDir "sentrux.cmd"
    foreach ($candidate in $candidates) {
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) { continue }
        $full = (Get-Item -LiteralPath $candidate).FullName
        if ([string]::Equals($full, $selfCmd, [System.StringComparison]::OrdinalIgnoreCase)) { continue }
        if ([string]::Equals($full, $PSCommandPath, [System.StringComparison]::OrdinalIgnoreCase)) { continue }
        return $full
    }

    throw "Sentrux core executable not found. Install sentrux.exe or set SENTRUX_CORE_EXE."
}

function Inject-ProHelp {
    param([string]$Text)
    if ($Text -match "(?m)^\s+pro\s+") { return $Text }

    $lines = @($Text -split "\r?\n")
    $out = New-Object System.Collections.Generic.List[string]
    $inserted = $false
    foreach ($line in $lines) {
        if (-not $inserted -and $line.TrimStart().StartsWith("help")) {
            $out.Add("  pro        Manage local open-source Pro activation")
            $inserted = $true
        }
        $out.Add($line)
    }
    if (-not $inserted) {
        $out.Add("  pro        Manage local open-source Pro activation")
    }
    return ($out -join [Environment]::NewLine)
}

function Invoke-Core {
    param([string[]]$CoreArgs)
    Ensure-AutoActivation
    try {
        $core = Resolve-Core
        & $core @CoreArgs
    }
    catch {
        $shimDir = Split-Path -Parent $PSCommandPath
        $liteCore = Join-Path $shimDir "sentrux-lite-core.ps1"
        if (-not (Test-Path -LiteralPath $liteCore -PathType Leaf)) {
            throw "Sentrux core executable not found and lite core is missing. Install sentrux.exe or restore sentrux-lite-core.ps1."
        }
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $liteCore @CoreArgs
    }
    exit $LASTEXITCODE
}

if ($RemainingArgs.Count -gt 0 -and $RemainingArgs[0] -eq "pro") {
    $proArgs = @($RemainingArgs | Select-Object -Skip 1)
    if ($proArgs.Count -eq 0 -or $proArgs[0] -in @("-h", "--help", "help")) {
        Show-ProHelp
        exit 0
    }

    switch ($proArgs[0]) {
        "activate" {
            if ($proArgs.Count -lt 2) { throw "missing license key: sentrux pro activate <key>" }
            Clear-AutoDisabled
            Write-License $proArgs[1] "local-open-source"
            Write-Output "Sentrux Pro activated"
            Show-ProStatus
            exit 0
        }
        "status" {
            Show-ProStatus
            exit 0
        }
        "deactivate" {
            Deactivate-Pro
            exit 0
        }
        default {
            throw "unknown pro command '$($proArgs[0])'. Try: sentrux pro --help"
        }
    }
}

if ($RemainingArgs.Count -eq 0 -or $RemainingArgs[0] -in @("-h", "--help", "help")) {
    try {
        $core = Resolve-Core
        $help = & $core --help 2>&1 | Out-String
        Write-Output (Inject-ProHelp $help)
        exit 0
    }
    catch {
        $shimDir = Split-Path -Parent $PSCommandPath
        $liteCore = Join-Path $shimDir "sentrux-lite-core.ps1"
        if (Test-Path -LiteralPath $liteCore -PathType Leaf) {
            $help = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $liteCore --help 2>&1 | Out-String
            Write-Output (Inject-ProHelp $help)
        }
        else {
            Write-Output "Live codebase visualization and structural quality gate"
            Write-Output ""
            Write-Output "Commands:"
            Write-Output "  pro        Manage local open-source Pro activation"
        }
        exit 0
    }
}

Invoke-Core -CoreArgs $RemainingArgs
