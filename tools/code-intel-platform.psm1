#requires -Version 7.2

Set-StrictMode -Version Latest

function Get-CodeIntelPlatform {
    param(
        [ValidateSet("auto", "windows", "macos", "linux")]
        [string]$Platform = "auto"
    )

    if ($Platform -ne "auto") { return $Platform }
    if ($IsWindows) { return "windows" }
    if ($IsMacOS) { return "macos" }
    if ($IsLinux) { return "linux" }
    throw "Unsupported platform. Pass -Platform windows|macos|linux."
}

function Get-CodeIntelHome {
    param([string]$Root = "")

    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_HOME)) {
        return (Resolve-CodeIntelPath $env:CODE_INTEL_HOME)
    }
    if (-not [string]::IsNullOrWhiteSpace($Root)) {
        return (Resolve-CodeIntelPath $Root)
    }
    return (Resolve-CodeIntelPath (Get-Location).Path)
}

function Resolve-CodeIntelPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (Test-Path -LiteralPath $Path) {
        return (Get-Item -LiteralPath $Path).FullName
    }
    return [System.IO.Path]::GetFullPath($Path)
}

function Get-CodeIntelHomeDirectory {
    $homeDir = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
    if ([string]::IsNullOrWhiteSpace($homeDir)) { $homeDir = $HOME }
    return (Resolve-CodeIntelPath $homeDir)
}

function Get-CodeIntelDataRoot {
    param(
        [ValidateSet("auto", "windows", "macos", "linux")]
        [string]$Platform = "auto"
    )

    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_DATA_ROOT)) {
        return (Resolve-CodeIntelPath $env:CODE_INTEL_DATA_ROOT)
    }

    $os = Get-CodeIntelPlatform -Platform $Platform
    $homeDir = Get-CodeIntelHomeDirectory
    switch ($os) {
        "windows" {
            $base = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
            if ([string]::IsNullOrWhiteSpace($base)) { $base = Join-Path $homeDir ".code-intel" }
            return (Join-Path $base "code-intel")
        }
        "macos" {
            return (Join-Path (Join-Path $homeDir "Library") (Join-Path "Application Support" "code-intel"))
        }
        "linux" {
            $base = if (-not [string]::IsNullOrWhiteSpace($env:XDG_DATA_HOME)) {
                $env:XDG_DATA_HOME
            }
            else {
                Join-Path (Join-Path $homeDir ".local") "share"
            }
            return (Join-Path $base "code-intel")
        }
    }
}

function Get-CodeIntelBinDir {
    param(
        [ValidateSet("auto", "windows", "macos", "linux")]
        [string]$Platform = "auto"
    )

    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_BIN)) {
        return (Resolve-CodeIntelPath $env:CODE_INTEL_BIN)
    }
    return (Join-Path (Get-CodeIntelDataRoot -Platform $Platform) "bin")
}

function Get-CodeIntelPythonCommand {
    foreach ($name in @("python", "python3")) {
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($cmd) { return $cmd }
    }
    return $null
}

function Get-CodeIntelArtifactRoot {
    param(
        [ValidateSet("auto", "windows", "macos", "linux")]
        [string]$Platform = "auto"
    )

    $fromUser = [Environment]::GetEnvironmentVariable("CODE_INTEL_ARTIFACT_ROOT", "User")
    if (-not [string]::IsNullOrWhiteSpace($fromUser)) { return (Resolve-CodeIntelPath $fromUser) }
    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_ARTIFACT_ROOT)) {
        return (Resolve-CodeIntelPath $env:CODE_INTEL_ARTIFACT_ROOT)
    }
    return (Join-Path (Get-CodeIntelDataRoot -Platform $Platform) "artifacts")
}

function Get-CodeIntelShadowRoot {
    param(
        [ValidateSet("auto", "windows", "macos", "linux")]
        [string]$Platform = "auto"
    )

    $fromUser = [Environment]::GetEnvironmentVariable("CODE_INTEL_SHADOW_ROOT", "User")
    if (-not [string]::IsNullOrWhiteSpace($fromUser)) { return (Resolve-CodeIntelPath $fromUser) }
    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_SHADOW_ROOT)) {
        return (Resolve-CodeIntelPath $env:CODE_INTEL_SHADOW_ROOT)
    }
    return (Join-Path (Get-CodeIntelDataRoot -Platform $Platform) "repowise")
}

function Get-CodeIntelPaths {
    param(
        [ValidateSet("auto", "windows", "macos", "linux")]
        [string]$Platform = "auto",
        [string]$Root = ""
    )

    $os = Get-CodeIntelPlatform -Platform $Platform
    $dataRoot = Get-CodeIntelDataRoot -Platform $os
    return [pscustomobject][ordered]@{
        home = Get-CodeIntelHomeDirectory
        dataRoot = $dataRoot
        bin = Get-CodeIntelBinDir -Platform $os
        artifactRoot = Get-CodeIntelArtifactRoot -Platform $os
        shadowRoot = Get-CodeIntelShadowRoot -Platform $os
        codeIntelHome = Get-CodeIntelHome -Root $Root
    }
}

function Set-CodeIntelUserEnv {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Value,
        [ValidateSet("auto", "windows", "macos", "linux")]
        [string]$Platform = "auto"
    )

    Set-Item -LiteralPath "env:$Name" -Value $Value
    $os = Get-CodeIntelPlatform -Platform $Platform
    if ($os -eq "windows") {
        [Environment]::SetEnvironmentVariable($Name, $Value, "User")
        return [pscustomobject][ordered]@{ name = $Name; persisted = $true; detail = "user environment" }
    }

    $configDir = Join-Path (Join-Path (Get-CodeIntelHomeDirectory) ".config") "code-intel"
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null
    $envFile = Join-Path $configDir "env.ps1"
    $escaped = $Value.Replace("'", "''")
    "`$env:$Name = '$escaped'" | Set-Content -LiteralPath $envFile -Encoding UTF8
    return [pscustomobject][ordered]@{
        name = $Name
        persisted = $false
        detail = "process environment set; dot-source $envFile from your pwsh profile to persist"
    }
}

function Add-UserPathPrefix {
    param(
        [Parameter(Mandatory = $true)][string]$PathToAdd,
        [ValidateSet("auto", "windows", "macos", "linux")]
        [string]$Platform = "auto"
    )

    $resolved = (New-Item -ItemType Directory -Force -Path $PathToAdd).FullName.TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
    $separator = [System.IO.Path]::PathSeparator

    $processParts = @($env:PATH -split [regex]::Escape([string]$separator) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $processParts = @($processParts | Where-Object {
        $entry = $_.TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
        -not [string]::Equals($entry, $resolved, [System.StringComparison]::OrdinalIgnoreCase)
    })
    $env:PATH = (($resolved) + $separator + ($processParts -join $separator)).TrimEnd($separator)

    $os = Get-CodeIntelPlatform -Platform $Platform
    if ($os -eq "windows") {
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        $userParts = @($userPath -split [regex]::Escape([string]$separator) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        $userParts = @($userParts | Where-Object {
            $entry = $_.TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
            -not [string]::Equals($entry, $resolved, [System.StringComparison]::OrdinalIgnoreCase)
        })
        [Environment]::SetEnvironmentVariable("Path", (($resolved) + $separator + ($userParts -join $separator)).TrimEnd($separator), "User")
        return [pscustomobject][ordered]@{ path = $resolved; persisted = $true; detail = "user PATH" }
    }

    return [pscustomobject][ordered]@{
        path = $resolved
        persisted = $false
        detail = "process PATH only; add this directory to your shell profile"
    }
}

function New-CodeIntelLink {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Target,
        [ValidateSet("auto", "windows", "macos", "linux")]
        [string]$Platform = "auto"
    )

    if (Test-Path -LiteralPath $Path) {
        return [pscustomobject][ordered]@{ ok = $true; mode = "existing"; path = $Path; target = $Target }
    }
    if (-not (Test-Path -LiteralPath $Target -PathType Container)) {
        return [pscustomobject][ordered]@{ ok = $false; mode = "missing_target"; path = $Path; target = $Target }
    }

    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $os = Get-CodeIntelPlatform -Platform $Platform

    try {
        if ($os -eq "windows") {
            New-Item -ItemType Junction -Path $Path -Target $Target | Out-Null
            return [pscustomobject][ordered]@{ ok = $true; mode = "junction"; path = $Path; target = $Target }
        }

        New-Item -ItemType SymbolicLink -Path $Path -Target $Target | Out-Null
        return [pscustomobject][ordered]@{ ok = $true; mode = "symlink"; path = $Path; target = $Target }
    }
    catch {
        Copy-Item -LiteralPath $Target -Destination $Path -Recurse -Force
        return [pscustomobject][ordered]@{ ok = $true; mode = "copy"; path = $Path; target = $Target; warning = $_.Exception.Message }
    }
}

function Invoke-CodeIntelNative {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [string[]]$Arguments = @()
    )

    $global:LASTEXITCODE = 0
    $started = Get-Date
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & $Command @Arguments 2>&1
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $finished = Get-Date

    return [pscustomobject][ordered]@{
        command = ($Command + " " + ($Arguments -join " ")).Trim()
        exitCode = $global:LASTEXITCODE
        output = ($output | ForEach-Object { $_.ToString() } | Out-String).Trim()
        durationMs = [int]($finished - $started).TotalMilliseconds
    }
}

Export-ModuleMember -Function @(
    "Get-CodeIntelPlatform",
    "Get-CodeIntelHome",
    "Resolve-CodeIntelPath",
    "Get-CodeIntelHomeDirectory",
    "Get-CodeIntelDataRoot",
    "Get-CodeIntelBinDir",
    "Get-CodeIntelPythonCommand",
    "Get-CodeIntelArtifactRoot",
    "Get-CodeIntelShadowRoot",
    "Get-CodeIntelPaths",
    "Set-CodeIntelUserEnv",
    "Add-UserPathPrefix",
    "New-CodeIntelLink",
    "Invoke-CodeIntelNative"
)
