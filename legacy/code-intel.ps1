#requires -Version 7.2

[CmdletBinding(PositionalBinding = $false)]
param(
    [Parameter(Position = 0)]
    [string]$RepoPath = "",

    [ValidateSet("lite", "normal", "full")]
    [string]$Mode = "normal",

    [switch]$Update,
    [switch]$Json,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Remaining = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $false

function Get-CodeIntelDataRoot {
    if ($IsWindows -and -not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        return (Join-Path $env:LOCALAPPDATA "code-intel")
    }
    if ($IsMacOS) {
        return (Join-Path ([Environment]::GetFolderPath("UserProfile")) "Library/Application Support/code-intel")
    }
    $base = if ($env:XDG_DATA_HOME) {
        $env:XDG_DATA_HOME
    } else {
        Join-Path ([Environment]::GetFolderPath("UserProfile")) ".local/share"
    }
    return (Join-Path $base "code-intel")
}

function Get-CanonicalManifestJson {
    param([Parameter(Mandatory)]$Files)

    $names = [string[]]@($Files.PSObject.Properties.Name)
    [Array]::Sort($names, [StringComparer]::Ordinal)
    $parts = foreach ($name in $names) {
        $entry = $Files.PSObject.Properties[$name].Value
        $sha256 = [string]$entry.sha256
        $size = [long]$entry.size
        if ($sha256 -cnotmatch "^[0-9a-f]{64}$" -or $size -lt 0) {
            throw "invalid release manifest entry: $name"
        }
        $encodedName = [System.Text.Json.JsonEncodedText]::Encode([string]$name).ToString()
        $encodedName = "`"$encodedName`""
        "$encodedName`:{`"sha256`":`"$sha256`",`"size`":$size}"
    }
    return "{$($parts -join ',')}"
}

function Get-VerifiedReleaseBinary {
    param([Parameter(Mandatory)][string]$Root)

    try {
        $rootPath = [System.IO.Path]::GetFullPath($Root)
        $markerPath = Join-Path $rootPath ".code-intel-release.json"
        if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) { return $null }
        $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json -Depth 100
        if ([string]$marker.schema -cne "code-intel-skill-release.v2" -or
            [string]$marker.manifest_sha256 -cnotmatch "^[0-9a-f]{64}$" -or
            $null -eq $marker.files) {
            return $null
        }

        $canonical = Get-CanonicalManifestJson -Files $marker.files
        $digestBytes = [System.Security.Cryptography.SHA256]::HashData(
            [System.Text.Encoding]::UTF8.GetBytes($canonical)
        )
        $manifestDigest = [Convert]::ToHexString($digestBytes).ToLowerInvariant()
        if ($manifestDigest -cne [string]$marker.manifest_sha256) { return $null }

        $expected = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        $rootPrefix = $rootPath.TrimEnd(
            [System.IO.Path]::DirectorySeparatorChar,
            [System.IO.Path]::AltDirectorySeparatorChar
        ) + [System.IO.Path]::DirectorySeparatorChar
        foreach ($property in @($marker.files.PSObject.Properties)) {
            $relative = [string]$property.Name
            if ([string]::IsNullOrWhiteSpace($relative) -or
                $relative.Contains("\") -or
                [System.IO.Path]::IsPathRooted($relative) -or
                @($relative.Split("/")) -contains "..") {
                return $null
            }
            $target = [System.IO.Path]::GetFullPath(
                (Join-Path $rootPath $relative.Replace("/", [System.IO.Path]::DirectorySeparatorChar))
            )
            if (-not $target.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase) -or
                -not (Test-Path -LiteralPath $target -PathType Leaf)) {
                return $null
            }
            $file = Get-Item -LiteralPath $target
            if ($file.Length -ne [long]$property.Value.size) { return $null }
            $actualHash = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actualHash -cne [string]$property.Value.sha256) { return $null }
            $null = $expected.Add($relative)
        }

        $actual = @(Get-ChildItem -LiteralPath $rootPath -File -Recurse | ForEach-Object {
            [System.IO.Path]::GetRelativePath($rootPath, $_.FullName).Replace(
                [System.IO.Path]::DirectorySeparatorChar,
                "/"
            )
        } | Where-Object { $_ -cne ".code-intel-release.json" })
        if ($actual.Count -ne $expected.Count -or
            @($actual | Where-Object { -not $expected.Contains($_) }).Count -ne 0) {
            return $null
        }

        $binaryName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
        $binaryRelative = "bin/$binaryName"
        if (-not $expected.Contains($binaryRelative)) { return $null }
        return (Join-Path $rootPath $binaryRelative)
    }
    catch {
        if ($env:CODE_INTEL_DEBUG -eq "1") {
            [Console]::Error.WriteLine("Code Intel release validation failed for ${Root}: $($_.Exception.Message)")
        }
        return $null
    }
}

function Test-CodeIntelBinary {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    try {
        & $Path --help *> $null
        return $LASTEXITCODE -eq 0
    }
    catch {
        return $false
    }
}

function Get-VerifiedReleaseRoots {
    $roots = [System.Collections.Generic.List[string]]::new()
    $roots.Add($PSScriptRoot)
    $releases = Join-Path (Get-CodeIntelDataRoot) "releases"
    if (Test-Path -LiteralPath $releases -PathType Container) {
        $versioned = foreach ($directory in @(Get-ChildItem -LiteralPath $releases -Directory)) {
            try {
                [pscustomobject]@{
                    Path = $directory.FullName
                    Version = [version]$directory.Name.TrimStart("v")
                }
            }
            catch {
                continue
            }
        }
        foreach ($release in @($versioned | Sort-Object Version -Descending)) {
            $roots.Add($release.Path)
        }
    }
    return $roots | Select-Object -Unique
}

function Get-DevelopmentCandidates {
    # Explicit source-tree escape hatch; these are never recovery-trusted candidates.
    if ($env:CODE_INTEL_ALLOW_UNVERIFIED_DEV -ne "1") { return @() }
    $binaryName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
    $candidates = [System.Collections.Generic.List[string]]::new()
    foreach ($directory in @(
        $env:CODE_INTEL_BIN,
        (Join-Path (Split-Path -Parent $PSScriptRoot) "bin"),
        (Join-Path (Split-Path -Parent $PSScriptRoot) "target/release"),
        (Join-Path (Split-Path -Parent $PSScriptRoot) "target/debug")
    )) {
        if (-not [string]::IsNullOrWhiteSpace($directory)) {
            $candidates.Add((Join-Path $directory $binaryName))
        }
    }
    $command = Get-Command code-intel -CommandType Application -ErrorAction SilentlyContinue
    if ($null -ne $command) { $candidates.Add($command.Source) }
    return $candidates | Select-Object -Unique
}

function Repair-CodeIntel {
    param([Parameter(Mandatory)][string]$Repository)

    $python = Get-Command python, python3 -ErrorAction SilentlyContinue | Select-Object -First 1
    $bootstrap = Join-Path (Split-Path -Parent $PSScriptRoot) "skills/code-intel-pipeline/scripts/bootstrap.py"
    if ($null -eq $python -or -not (Test-Path -LiteralPath $bootstrap -PathType Leaf)) {
        [Console]::Error.WriteLine("Code Intel recovery requires Python and the packaged Skill bootstrap.")
        return $null
    }
    $output = @(& $python.Source $bootstrap --repo-path $Repository --json 2>&1)
    if ($LASTEXITCODE -ne 0) {
        [Console]::Error.WriteLine(($output -join [Environment]::NewLine))
        return $null
    }
    try {
        $result = ($output -join [Environment]::NewLine) | ConvertFrom-Json
        return [string]$result.release_root
    }
    catch {
        [Console]::Error.WriteLine("Code Intel recovery returned invalid JSON.")
        return $null
    }
}

$repo = if ([string]::IsNullOrWhiteSpace($RepoPath)) { (Get-Location).Path } else { $RepoPath }
$binary = $null
if (-not $Update) {
    foreach ($root in @(Get-VerifiedReleaseRoots)) {
        $candidate = Get-VerifiedReleaseBinary -Root $root
        if (-not [string]::IsNullOrWhiteSpace($candidate) -and (Test-CodeIntelBinary $candidate)) {
            $binary = $candidate
            break
        }
    }
    if ($null -eq $binary) {
        $binary = Get-DevelopmentCandidates |
            Where-Object { Test-CodeIntelBinary $_ } |
            Select-Object -First 1
    }
}

if ($Update -or $null -eq $binary) {
    $repairedRoot = Repair-CodeIntel -Repository $repo
    if ([string]::IsNullOrWhiteSpace($repairedRoot)) { exit 69 }
    $binary = Get-VerifiedReleaseBinary -Root $repairedRoot
    if ([string]::IsNullOrWhiteSpace($binary) -or -not (Test-CodeIntelBinary $binary)) { exit 70 }
}

$arguments = [System.Collections.Generic.List[string]]::new()
if (-not [string]::IsNullOrWhiteSpace($RepoPath)) { $arguments.Add($RepoPath) }
if ($PSBoundParameters.ContainsKey("Mode")) {
    $arguments.Add("--mode")
    $arguments.Add($Mode)
}
if ($Json) { $arguments.Add("--json") }
foreach ($argument in $Remaining) { $arguments.Add($argument) }

& $binary @arguments
exit $LASTEXITCODE
