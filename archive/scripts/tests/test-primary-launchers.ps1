#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$launcher = Join-Path $root "code-intel.ps1"
$legacy = Join-Path $root "invoke-code-intel.ps1"
$missingRepo = Join-Path ([System.IO.Path]::GetTempPath()) "code-intel-launcher-missing-repo"
$missingConfig = Join-Path ([System.IO.Path]::GetTempPath()) "code-intel-launcher-missing-config.json"
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-launcher-{0}-{1}" -f $PID, [guid]::NewGuid().ToString("N"))
$previousDevelopmentOverride = $env:CODE_INTEL_ALLOW_UNVERIFIED_DEV
$previousLocalAppData = $env:LOCALAPPDATA
$previousPath = $env:PATH
$env:CODE_INTEL_ALLOW_UNVERIFIED_DEV = "1"

try {
    $help = @(& pwsh -NoLogo -NoProfile -File $launcher --help 2>&1)
    if ($LASTEXITCODE -ne 0 -or ($help -join "`n") -notmatch "code-intel \.") {
        throw "recovery launcher did not delegate to the compiled CLI"
    }

    $legacyOutput = @(& pwsh -NoLogo -NoProfile -File $legacy -RepoPath $missingRepo -Mode lite 2>&1)
    if ($LASTEXITCODE -ne 64 -or
        ($legacyOutput -join "`n") -notmatch "repository path is not a directory:" -or
        ($legacyOutput -join "`n") -match "validate integration orchestration") {
        throw "legacy entry did not quietly forward to the compiled CLI"
    }

    $retiredOutput = @(& pwsh -NoLogo -NoProfile -File $legacy -RepoPath $missingRepo -SkipRepowise 2>&1)
    if ($LASTEXITCODE -ne 64 -or ($retiredOutput -join "`n") -notmatch "unsupported compatibility option: -SkipRepowise") {
        throw "legacy entry silently ignored an unsupported option"
    }

    $configOutput = @(& pwsh -NoLogo -NoProfile -File $legacy -RepoPath $missingRepo -Config $missingConfig 2>&1)
    if ($LASTEXITCODE -ne 64 -or ($configOutput -join "`n") -notmatch "config file does not exist:") {
        throw "legacy entry silently ignored a missing explicit config"
    }

    function New-VerifiedReleaseFixture {
        param([string]$Version)

        $releaseRoot = Join-Path $testRoot "code-intel/releases/$Version"
        $binDir = Join-Path $releaseRoot "bin"
        New-Item -ItemType Directory -Path $binDir -Force | Out-Null
        $binaryName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
        $binaryPath = Join-Path $binDir $binaryName
        Copy-Item -LiteralPath (Join-Path $root "target/debug/$binaryName") -Destination $binaryPath
        $file = Get-Item -LiteralPath $binaryPath
        $sha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $relative = "bin/$binaryName"
        $canonical = "{`"$relative`":{`"sha256`":`"$sha256`",`"size`":$($file.Length)}}"
        $manifestDigest = [Convert]::ToHexString(
            [System.Security.Cryptography.SHA256]::HashData([System.Text.Encoding]::UTF8.GetBytes($canonical))
        ).ToLowerInvariant()
        [ordered]@{
            schema = "code-intel-skill-release.v2"
            tag = $Version
            asset = "fixture.zip"
            url = "https://github.com/2233admin/code-intel-pipeline/releases/download/$Version/fixture.zip"
            sha256 = ("0" * 64)
            manifest_sha256 = $manifestDigest
            files = [ordered]@{
                ($relative) = [ordered]@{
                    sha256 = $sha256
                    size = $file.Length
                }
            }
        } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $releaseRoot ".code-intel-release.json") -Encoding utf8NoBOM
        return $binaryPath
    }

    $env:LOCALAPPDATA = $testRoot
    $env:CODE_INTEL_ALLOW_UNVERIFIED_DEV = $null
    $fallbackBinary = New-VerifiedReleaseFixture -Version "v0.9.0"
    $newerBinary = New-VerifiedReleaseFixture -Version "v0.10.0"
    [System.IO.File]::AppendAllText($newerBinary, "tampered")
    $verifiedHelp = @(& pwsh -NoLogo -NoProfile -File $launcher --help 2>&1)
    if ($LASTEXITCODE -ne 0 -or ($verifiedHelp -join "`n") -notmatch "code-intel \.") {
        throw "recovery launcher did not reject the tampered newer release and use the verified fallback: $($verifiedHelp -join [Environment]::NewLine)"
    }

    $pwshPath = (Get-Process -Id $PID).Path
    $env:PATH = ""
    $updateOutput = @(& $pwshPath -NoLogo -NoProfile -File $launcher -Update --help 2>&1)
    if ($LASTEXITCODE -ne 69 -or ($updateOutput -join "`n") -notmatch "recovery requires Python") {
        throw "explicit update failure silently fell back to an installed binary"
    }

    Write-Host "Primary launchers: OK"
}
finally {
    $env:CODE_INTEL_ALLOW_UNVERIFIED_DEV = $previousDevelopmentOverride
    $env:LOCALAPPDATA = $previousLocalAppData
    $env:PATH = $previousPath
    if (Test-Path -LiteralPath $testRoot -PathType Container) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
