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

    if (-not [string]::IsNullOrWhiteSpace($Root)) {
        return (Resolve-CodeIntelPath $Root)
    }
    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_HOME)) {
        return (Resolve-CodeIntelPath $env:CODE_INTEL_HOME)
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

function Get-CodeIntelPosixProfileInstruction {
    # Pure: the single copy-paste line that makes new POSIX shells pick up
    # ~/.config/code-intel/env.sh. macOS defaults to zsh, Linux to bash.
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet("macos", "linux")]
        [string]$Platform
    )

    $shellProfile = if ($Platform -eq "macos") { "~/.zshrc" } else { "~/.bashrc" }
    return "echo 'source ~/.config/code-intel/env.sh' >> $shellProfile"
}

function ConvertTo-CodeIntelPosixEnvLine {
    # Pure: renders one POSIX sh export line with the value double-quoted and
    # sh metacharacters escaped. -AsPathPrefix renders the PATH prefix form
    # (export PATH="<dir>:$PATH") with the trailing $PATH left expandable.
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Value,
        [switch]$AsPathPrefix
    )

    $escaped = $Value.Replace('\', '\\').Replace('"', '\"').Replace('$', '\$').Replace('`', '\`')
    if ($AsPathPrefix) {
        return ('export {0}="{1}:$PATH"' -f $Name, $escaped)
    }
    return ('export {0}="{1}"' -f $Name, $escaped)
}

function Update-CodeIntelPosixEnvContent {
    # Pure: idempotent line-set update. Drops any exact duplicate of $Line and
    # any line matching $MatchPattern (a previous export of the same variable),
    # then appends the canonical $Line once.
    param(
        [string[]]$Lines = @(),
        [string]$MatchPattern = "",
        [Parameter(Mandatory = $true)][string]$Line
    )

    $kept = @($Lines | Where-Object {
        if ($_ -ceq $Line) { return $false }
        if (-not [string]::IsNullOrEmpty($MatchPattern) -and $_ -match $MatchPattern) { return $false }
        return $true
    })
    return @($kept + $Line)
}

function Update-CodeIntelPosixEnvFile {
    # Rewrites ~/.config/code-intel/env.sh idempotently with the given line and
    # returns the file path. Sourced from the user's shell profile so fresh
    # bash/zsh sessions keep PATH and CODE_INTEL_* across installs.
    param(
        [string]$MatchPattern = "",
        [Parameter(Mandatory = $true)][string]$Line
    )

    $configDir = Join-Path (Join-Path (Get-CodeIntelHomeDirectory) ".config") "code-intel"
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null
    $envFile = Join-Path $configDir "env.sh"
    $existing = @()
    if (Test-Path -LiteralPath $envFile -PathType Leaf) {
        $existing = @(Get-Content -LiteralPath $envFile)
    }
    $updated = Update-CodeIntelPosixEnvContent -Lines $existing -MatchPattern $MatchPattern -Line $Line
    Set-Content -LiteralPath $envFile -Value $updated -Encoding UTF8
    return $envFile
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
    $pwshLines = @()
    if (Test-Path -LiteralPath $envFile -PathType Leaf) {
        $pwshLines = @(Get-Content -LiteralPath $envFile)
    }
    $pwshPattern = '^\s*\$env:' + [regex]::Escape($Name) + '\s*='
    $pwshUpdated = Update-CodeIntelPosixEnvContent -Lines $pwshLines -MatchPattern $pwshPattern -Line "`$env:$Name = '$escaped'"
    Set-Content -LiteralPath $envFile -Value $pwshUpdated -Encoding UTF8
    $shPattern = '^\s*export\s+' + [regex]::Escape($Name) + '='
    $shFile = Update-CodeIntelPosixEnvFile -MatchPattern $shPattern -Line (ConvertTo-CodeIntelPosixEnvLine -Name $Name -Value $Value)
    return [pscustomobject][ordered]@{
        name = $Name
        persisted = $false
        detail = "process environment set; $shFile updated; run once: $(Get-CodeIntelPosixProfileInstruction -Platform $os)"
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

    $shFile = Update-CodeIntelPosixEnvFile -Line (ConvertTo-CodeIntelPosixEnvLine -Name "PATH" -Value $resolved -AsPathPrefix)
    return [pscustomobject][ordered]@{
        path = $resolved
        persisted = $false
        detail = "process PATH updated; $shFile updated; run once: $(Get-CodeIntelPosixProfileInstruction -Platform $os)"
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
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
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
    "Get-CodeIntelPosixProfileInstruction",
    "ConvertTo-CodeIntelPosixEnvLine",
    "Update-CodeIntelPosixEnvContent",
    "Update-CodeIntelPosixEnvFile",
    "Set-CodeIntelUserEnv",
    "Add-UserPathPrefix",
    "New-CodeIntelLink",
    "Invoke-CodeIntelNative"
)
