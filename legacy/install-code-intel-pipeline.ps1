#requires -Version 7.2

param(
    [string]$Config = "",
    [string]$Repo = "",
    [string]$RepoPath = "",
    [string]$ArtifactRoot = "",
    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto",
    # Documentation language preference (issue #155). Explicit and always
    # wins; never triggers the interactive prompt below. Actual precedence
    # resolution and persistence live in Rust (`language_pref`/`language
    # set`) -- this script only collects the value and passes it through.
    [ValidateSet("", "zh", "en")]
    [string]$Language = "",
    [switch]$RepairSkillLinks,
    [switch]$CheckProvider,
    [switch]$InstallMissing,
    [switch]$AuditInstallPlan,
    [switch]$RequireRepowise,
    [switch]$RequireUnderstand,
    [switch]$SkipSentruxVlangOverlay,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$platformModule = Join-Path (Join-Path $PSScriptRoot "tools") "code-intel-platform.psm1"
Import-Module $platformModule -Force
$script:EffectivePlatform = Get-CodeIntelPlatform -Platform $Platform

function Add-Check {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Name,
        [string]$Category,
        [bool]$Required,
        [bool]$Ok,
        [string]$Detail = "",
        [string]$Fix = ""
    )

    $Checks.Add([pscustomobject][ordered]@{
        name = $Name
        category = $Category
        required = $Required
        ok = $Ok
        detail = $Detail
        fix = $Fix
    })
}

function Add-InstallAction {
    param(
        [System.Collections.Generic.List[object]]$Actions,
        [string]$Name,
        [string]$Status,
        [string]$Detail = "",
        [string]$Fix = "",
        [string]$PackageManager = "",
        [bool]$RequiresElevation = $false
    )

    $Actions.Add([pscustomobject][ordered]@{
        name = $Name
        status = $Status
        detail = $Detail
        fix = $Fix
        packageManager = $PackageManager
        requiresElevation = $RequiresElevation
    })
}

function Add-InstallPlan {
    param(
        [System.Collections.Generic.List[object]]$Plan,
        [string]$Name,
        [string]$Installer,
        [string]$Command,
        [string]$Purpose,
        [string]$Risk,
        [string]$Alternative = "",
        [string]$PackageManager = "",
        [bool]$RequiresElevation = $false
    )

    $Plan.Add([pscustomobject][ordered]@{
        name = $Name
        installer = $Installer
        command = $Command
        purpose = $Purpose
        risk = $Risk
        alternative = $Alternative
        packageManager = if ([string]::IsNullOrWhiteSpace($PackageManager)) { $Installer } else { $PackageManager }
        requiresElevation = $RequiresElevation
    })
}

function Invoke-WingetInstall {
    param(
        [string]$PackageId,
        [string]$PackageName
    )

    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        throw "winget is not available for installing $PackageName"
    }

    & winget install --id $PackageId -e --source winget --accept-package-agreements --accept-source-agreements
    if ($LASTEXITCODE -ne 0) {
        throw "winget install failed for $PackageName with exit code $LASTEXITCODE"
    }
}

function Invoke-ChocoInstall {
    param([string]$PackageName)

    if (-not (Get-Command choco -ErrorAction SilentlyContinue)) {
        throw "choco is not available for installing $PackageName"
    }
    & choco install $PackageName -y --no-progress
    if ($LASTEXITCODE -ne 0) {
        throw "choco install failed for $PackageName with exit code $LASTEXITCODE"
    }
}

function Invoke-ScoopInstall {
    param([string]$PackageName)

    if (-not (Get-Command scoop -ErrorAction SilentlyContinue)) {
        throw "scoop is not available for installing $PackageName"
    }
    & scoop install $PackageName
    if ($LASTEXITCODE -ne 0) {
        throw "scoop install failed for $PackageName with exit code $LASTEXITCODE"
    }
}

function Invoke-BrewInstall {
    param([string]$PackageName)

    if (-not (Get-Command brew -ErrorAction SilentlyContinue)) {
        throw "brew is not available for installing $PackageName"
    }
    & brew install $PackageName
    if ($LASTEXITCODE -ne 0) {
        throw "brew install failed for $PackageName with exit code $LASTEXITCODE"
    }
}

function Invoke-LinuxPackageInstall {
    param([string]$PackageName)

    if (Get-Command apt-get -ErrorAction SilentlyContinue) {
        $runner = if (Get-Command sudo -ErrorAction SilentlyContinue) { "sudo" } else { "apt-get" }
        if ($runner -eq "sudo") {
            & sudo apt-get update
            if ($LASTEXITCODE -ne 0) { throw "apt-get update failed with exit code $LASTEXITCODE" }
            & sudo apt-get install -y $PackageName
        }
        else {
            & apt-get update
            if ($LASTEXITCODE -ne 0) { throw "apt-get update failed with exit code $LASTEXITCODE" }
            & apt-get install -y $PackageName
        }
        if ($LASTEXITCODE -ne 0) { throw "apt-get install failed for $PackageName with exit code $LASTEXITCODE" }
        return
    }

    if (Get-Command dnf -ErrorAction SilentlyContinue) {
        if (Get-Command sudo -ErrorAction SilentlyContinue) { & sudo dnf install -y $PackageName } else { & dnf install -y $PackageName }
        if ($LASTEXITCODE -ne 0) { throw "dnf install failed for $PackageName with exit code $LASTEXITCODE" }
        return
    }

    if (Get-Command pacman -ErrorAction SilentlyContinue) {
        if (Get-Command sudo -ErrorAction SilentlyContinue) { & sudo pacman -Sy --noconfirm $PackageName } else { & pacman -Sy --noconfirm $PackageName }
        if ($LASTEXITCODE -ne 0) { throw "pacman install failed for $PackageName with exit code $LASTEXITCODE" }
        return
    }

    throw "no supported Linux package manager found for $PackageName; install apt, dnf, pacman, or install the tool manually"
}

function Get-ToolPackageName {
    param([string]$ToolName)

    switch ($ToolName) {
        "rg" {
            switch ($script:EffectivePlatform) {
                "windows" { return @{ winget = "BurntSushi.ripgrep.MSVC"; choco = "ripgrep"; scoop = "ripgrep" } }
                "macos" { return "ripgrep" }
                "linux" { return "ripgrep" }
            }
        }
        "git" {
            switch ($script:EffectivePlatform) {
                "windows" { return @{ winget = "Git.Git"; choco = "git"; scoop = "git" } }
                "macos" { return "git" }
                "linux" { return "git" }
            }
        }
        "python" {
            switch ($script:EffectivePlatform) {
                "windows" { return @{ winget = "Python.Python.3.11"; choco = "python"; scoop = "python" } }
                "macos" { return "python@3.11" }
                "linux" { return "python3" }
            }
        }
    }

    throw "no package mapping for $ToolName on $script:EffectivePlatform"
}

function Invoke-ToolPackageInstall {
    param([string]$ToolName)

    $package = Get-ToolPackageName $ToolName
    switch ($script:EffectivePlatform) {
        "windows" {
            if (Get-Command winget -ErrorAction SilentlyContinue) {
                Invoke-WingetInstall $package.winget $ToolName
                return
            }
            if (Get-Command choco -ErrorAction SilentlyContinue) {
                Invoke-ChocoInstall $package.choco
                return
            }
            if (Get-Command scoop -ErrorAction SilentlyContinue) {
                Invoke-ScoopInstall $package.scoop
                return
            }
            throw "no supported Windows installer found for $ToolName; install winget, choco, or scoop first"
        }
        "macos" {
            Invoke-BrewInstall $package
            return
        }
        "linux" {
            Invoke-LinuxPackageInstall $package
            return
        }
    }
}

function Invoke-RipgrepInstall {
    Invoke-ToolPackageInstall "rg"
}

function Invoke-PipInstall {
    param(
        [string]$PackageName,
        [string]$Version = ""
    )

    $python = Get-CodeIntelPythonCommand
    if (-not $python) {
        throw "python/python3 is not on PATH; install Python and rerun this script in a new shell"
    }
    $pythonCommand = if (-not [string]::IsNullOrWhiteSpace($python.Source)) { $python.Source } else { $python.Name }

    if ([string]::IsNullOrWhiteSpace($Version)) {
        & $pythonCommand -m pip install --user --upgrade $PackageName
    }
    else {
        & $pythonCommand -m pip install --user "$PackageName==$Version"
    }
    if ($LASTEXITCODE -ne 0) {
        throw "pip install failed for $PackageName with exit code $LASTEXITCODE"
    }
}

function Invoke-SentruxInstall {
    throw "no published sentrux package installer is configured; use the repo-owned shim/lite core or place a real sentrux.exe on PATH"
}

function Get-InstallMetadata {
    param([string]$CommandName)

    switch ($CommandName) {
        # pinnedVersion turns the supply-chain-003 pin from a declaration into a
        # gate. Without it Install-MissingTool returns on presence alone, so a
        # machine that installed repowise once keeps whatever version it landed
        # on and the pin never executes.
        "repowise" { return [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = $script:RepowisePinnedVersion } }
        "sentrux" { return [ordered]@{ packageManager = "manual"; requiresElevation = $false } }
    }

    switch ($script:EffectivePlatform) {
        "windows" {
            if (Get-Command winget -ErrorAction SilentlyContinue) { return [ordered]@{ packageManager = "winget"; requiresElevation = $false } }
            if (Get-Command choco -ErrorAction SilentlyContinue) { return [ordered]@{ packageManager = "choco"; requiresElevation = $true } }
            if (Get-Command scoop -ErrorAction SilentlyContinue) { return [ordered]@{ packageManager = "scoop"; requiresElevation = $false } }
            return [ordered]@{ packageManager = "manual"; requiresElevation = $false }
        }
        "macos" { return [ordered]@{ packageManager = "brew"; requiresElevation = $false } }
        "linux" {
            if (Get-Command apt-get -ErrorAction SilentlyContinue) { return [ordered]@{ packageManager = "apt"; requiresElevation = $true } }
            if (Get-Command dnf -ErrorAction SilentlyContinue) { return [ordered]@{ packageManager = "dnf"; requiresElevation = $true } }
            if (Get-Command pacman -ErrorAction SilentlyContinue) { return [ordered]@{ packageManager = "pacman"; requiresElevation = $true } }
            return [ordered]@{ packageManager = "manual"; requiresElevation = $false }
        }
    }
}

function Test-ToolVersionProbeAllowed {
    # Mirrors the resolution rule crates/code-intel-cli/src/tool_path.rs states
    # for every tool launch in this project: "only ever launches by absolute
    # path", and "relative PATH entries are skipped outright". This matters
    # here because the installer runs against a repository it does not trust,
    # and a `repowise.ps1` on PATH would be dot-run *inside this process* by
    # `& $Source` — arbitrary in-process code, not a sandboxed child.
    #
    # A refused probe is not a failed probe: the caller keeps the pre-existing
    # presence-only behaviour rather than reporting drift, so an unverifiable
    # source can never induce a reinstall.
    param([string]$Source)

    if ([string]::IsNullOrWhiteSpace($Source)) { return $false }
    if (-not [System.IO.Path]::IsPathRooted($Source)) { return $false }
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) { return $false }
    if ([System.IO.Path]::GetExtension($Source) -in @(".ps1", ".psm1", ".psd1")) { return $false }
    return $true
}

function Get-ToolVersion {
    # Reads a tool's own reported version.
    #
    # Returns $null when the probe was REFUSED (source is not a rooted, real,
    # non-script file) — the caller must fall back to presence-only reporting.
    # Returns "" when the probe RAN but produced no readable version, which
    # callers must treat as "unknown", never as "matches".
    param(
        [string]$Source,
        [string]$ExpectedName = ""
    )

    if (-not (Test-ToolVersionProbeAllowed $Source)) { return $null }

    # Launch the child explicitly so the exit code, stdout, and stderr all come
    # from the process we started. `$LASTEXITCODE` is only set by native
    # commands, so reading it after `& $Source` would either see a stale value
    # from an unrelated earlier call or, under Set-StrictMode, throw on an
    # unset automatic variable and abort the whole installer.
    $stdoutFile = [System.IO.Path]::GetTempFileName()
    $stderrFile = [System.IO.Path]::GetTempFileName()
    try {
        $process = Start-Process -FilePath $Source -ArgumentList "--version" `
            -RedirectStandardOutput $stdoutFile -RedirectStandardError $stderrFile `
            -NoNewWindow -PassThru -Wait -ErrorAction Stop
        if ($process.ExitCode -ne 0) { return "" }
        # stdout only: a deprecation banner on stderr must not win the match.
        $raw = Get-Content -Raw -LiteralPath $stdoutFile -ErrorAction SilentlyContinue
    }
    catch {
        return ""
    }
    finally {
        Remove-Item -LiteralPath $stdoutFile, $stderrFile -Force -ErrorAction SilentlyContinue
    }

    if ([string]::IsNullOrWhiteSpace($raw)) { return "" }

    # Anchor to the tool's own name when we know it, so an unrelated
    # version-shaped number in a warning line cannot forge a match either way.
    # Accepts "repowise, version 0.32.0" and "code-intel 0.7.0-beta.2".
    $versionPattern = '(\d+\.\d+\.\d+(?:[-.][0-9A-Za-z.]+)?)'
    if (-not [string]::IsNullOrWhiteSpace($ExpectedName)) {
        $named = [regex]::Match($raw, "(?im)^\s*$([regex]::Escape($ExpectedName))[,]?\s+(?:version\s+)?$versionPattern\s*$")
        if ($named.Success) { return $named.Groups[1].Value }
    }
    $match = [regex]::Match($raw, $versionPattern)
    if ($match.Success) { return $match.Groups[1].Value }
    return ""
}

function Add-VersionComplianceChecks {
    # A pin that only shows up in installActions is still a declaration nothing
    # enforces: `ok` is computed from $checks alone, and
    # bootstrap-new-machine.ps1 reads only installResult.ok. Without this a
    # drifted machine reports "Install OK: True" — the exact shape
    # doctor_provider_rows.rs was written to call out: "bootstrap reports ok
    # while a present external provider is broken".
    #
    # Only CONFIRMED drift is required: the tool answered and the answer did
    # not match the pin. A refused or unreadable probe is uncertainty, not a
    # known violation, and must not fail an install over something that could
    # not be measured.
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [System.Collections.Generic.List[object]]$Actions
    )

    foreach ($action in $Actions) {
        if ([string]$action.status -notin @("version_drift", "upgrade_failed")) { continue }
        # "unknown" is the single word both branches use for an unreadable
        # version (Install-MissingTool's $observed, and the post-reinstall
        # detail). Matching on it keeps this derivation in one place; the
        # alternative is a structured field on every Add-InstallAction call
        # site, which is a wider change than this fix warrants.
        $confirmed = -not ([string]$action.detail -like "*unknown*")
        Add-Check $Checks "version:$($action.name)" "version" $confirmed $false ([string]$action.detail) ([string]$action.fix)
    }
}

function Install-MissingTool {
    param(
        [System.Collections.Generic.List[object]]$Actions,
        [string]$CommandName,
        [scriptblock]$Installer,
        [string]$Fix
    )

    $metadata = Get-InstallMetadata $CommandName
    $existing = if ($CommandName -eq "python") { Get-CodeIntelPythonCommand } else { Get-Command $CommandName -ErrorAction SilentlyContinue }
    if ($existing) {
        $pinned = if ($metadata.Contains("pinnedVersion")) { [string]$metadata.pinnedVersion } else { "" }
        if ([string]::IsNullOrWhiteSpace($pinned)) {
            Add-InstallAction $Actions $CommandName "already_present" $existing.Source "" $metadata.packageManager ([bool]$metadata.requiresElevation)
            return
        }

        $actual = Get-ToolVersion $existing.Source -ExpectedName $CommandName
        if ($null -eq $actual) {
            # Probe refused (see Test-ToolVersionProbeAllowed). Fall back to the
            # pre-existing presence-only behaviour rather than reporting drift:
            # an unverifiable source must not be able to induce a reinstall.
            Add-InstallAction $Actions $CommandName "already_present" "$($existing.Source) (version not probed: source is not a rooted executable file)" "" $metadata.packageManager ([bool]$metadata.requiresElevation)
            return
        }
        if ($actual -eq $pinned) {
            Add-InstallAction $Actions $CommandName "already_present" "$($existing.Source) (version $actual)" "" $metadata.packageManager ([bool]$metadata.requiresElevation)
            return
        }

        # The pin is a floor, not an exact target: a tool the user upgraded
        # past the pin must pass. Only older-than-pin (or unparseable) counts
        # as drift — reinstalling at the pin would downgrade an intentional
        # upgrade on every rerun.
        $actualParsed = $null
        $pinnedParsed = $null
        if ([System.Version]::TryParse($actual, [ref]$actualParsed) -and
            [System.Version]::TryParse($pinned, [ref]$pinnedParsed) -and
            $actualParsed -gt $pinnedParsed) {
            Add-InstallAction $Actions $CommandName "already_present" "$($existing.Source) (version $actual, newer than pin $pinned)" "" $metadata.packageManager ([bool]$metadata.requiresElevation)
            return
        }

        $observed = if ([string]::IsNullOrWhiteSpace($actual)) { "unknown" } else { $actual }
        $driftDetail = "$($existing.Source) reports version $observed; pinned version is $pinned"
        $driftFix = "Rerun with -InstallMissing to reinstall $CommandName at the pinned version, or set the pin to the version you intend to run."

        # Drift is reported whether or not we are allowed to fix it. Staying
        # silent here is the failure this whole branch exists to prevent: a
        # present-but-wrong version currently reads as already_present, which is
        # indistinguishable from correct.
        if (-not $InstallMissing) {
            Add-InstallAction $Actions $CommandName "version_drift" $driftDetail $driftFix $metadata.packageManager ([bool]$metadata.requiresElevation)
            return
        }

        try {
            & $Installer
            $afterDrift = if ($CommandName -eq "python") { Get-CodeIntelPythonCommand } else { Get-Command $CommandName -ErrorAction SilentlyContinue }
            $afterVersion = if ($afterDrift) { Get-ToolVersion $afterDrift.Source } else { "" }
            if ($afterVersion -eq $pinned) {
                Add-InstallAction $Actions $CommandName "upgraded" "$($afterDrift.Source) (version $afterVersion, was $observed)" "" $metadata.packageManager ([bool]$metadata.requiresElevation)
            }
            else {
                $stillDetail = if ($afterDrift) { "$($afterDrift.Source) still reports $(if ([string]::IsNullOrWhiteSpace($afterVersion)) { 'unknown' } else { $afterVersion }) after reinstall; pinned version is $pinned" } else { "$CommandName is not visible in this shell after reinstall" }
                Add-InstallAction $Actions $CommandName "upgrade_failed" $stillDetail $driftFix $metadata.packageManager ([bool]$metadata.requiresElevation)
            }
        }
        catch {
            Add-InstallAction $Actions $CommandName "upgrade_failed" $_.Exception.Message $driftFix $metadata.packageManager ([bool]$metadata.requiresElevation)
        }
        return
    }

    if (-not $InstallMissing) {
        Add-InstallAction $Actions $CommandName "not_requested" "missing" $Fix $metadata.packageManager ([bool]$metadata.requiresElevation)
        return
    }

    try {
        & $Installer
        $after = if ($CommandName -eq "python") { Get-CodeIntelPythonCommand } else { Get-Command $CommandName -ErrorAction SilentlyContinue }
        if ($after) {
            Add-InstallAction $Actions $CommandName "installed" $after.Source "" $metadata.packageManager ([bool]$metadata.requiresElevation)
        }
        else {
            Add-InstallAction $Actions $CommandName "installed_restart_required" "installer completed but command is not visible in this shell" "Open a new terminal and rerun install-code-intel-pipeline.ps1." $metadata.packageManager ([bool]$metadata.requiresElevation)
        }
    }
    catch {
        Add-InstallAction $Actions $CommandName "install_failed" $_.Exception.Message $Fix $metadata.packageManager ([bool]$metadata.requiresElevation)
    }
}

function Test-Tool {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Name,
        [bool]$Required = $true,
        [string]$Fix = ""
    )

    $cmd = if ($Name -eq "python") { Get-CodeIntelPythonCommand } else { Get-Command $Name -ErrorAction SilentlyContinue }
    $detail = "missing"
    if ($cmd) {
        $detail = $cmd.Source
    }
    Add-Check $Checks "tool:$Name" "tool" $Required ([bool]$cmd) $detail $Fix
}

function Test-File {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Name,
        [string]$Path,
        [bool]$Required = $true
    )

    Add-Check $Checks $Name "file" $Required (Test-Path -LiteralPath $Path -PathType Leaf) $Path "Restore or reinstall the code-intel pipeline files."
}

function Test-Directory {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Name,
        [string]$Path,
        [bool]$Required = $true,
        [string]$Fix = ""
    )

    Add-Check $Checks $Name "directory" $Required (Test-Path -LiteralPath $Path -PathType Container) $Path $Fix
}

function Test-EnvVar {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Name,
        [bool]$Required = $false,
        [string]$ExpectedValue = ""
    )

    $value = [Environment]::GetEnvironmentVariable($Name, "User")
    $hasValue = -not [string]::IsNullOrWhiteSpace($value)
    $ok = $hasValue
    $detail = if ($hasValue) { "set" } else { "missing" }
    if ($hasValue -and -not [string]::IsNullOrWhiteSpace($ExpectedValue)) {
        $ok = $value -eq $ExpectedValue
        $detail = if ($ok) { "set" } else { "unexpected value" }
    }

    Add-Check $Checks "env:$Name" "env" $Required $ok $detail "Set user environment variable $Name. Do not commit secrets to repo files."
}

function Get-DefaultArtifactRoot {
    return (code-intel-platform\Get-CodeIntelArtifactRoot -Platform $script:EffectivePlatform)
}

function Get-CodeIntelBinDir {
    return (code-intel-platform\Get-CodeIntelBinDir -Platform $script:EffectivePlatform)
}

function Add-UserPathPrefix {
    param([string]$PathToAdd)

    return (code-intel-platform\Add-UserPathPrefix -PathToAdd $PathToAdd -Platform $script:EffectivePlatform)
}

function Get-PathRefreshFix {
    param([string]$CommandName)

    if ($script:EffectivePlatform -eq "windows") {
        return "Open a new terminal if this shell cannot find $CommandName from PATH."
    }
    # A new terminal does NOT pick up process-only PATH changes on macOS/Linux;
    # the one-time profile line is what makes fresh shells source env.sh.
    return "If a new shell cannot find $CommandName, run once: $(Get-CodeIntelPosixProfileInstruction -Platform $script:EffectivePlatform)"
}

function New-ThinForwarderPs1 {
    param(
        [string]$RepoRoot,
        [string]$RelativeTargetPath,
        [string]$CommandLabel
    )

    # This file is generated by install-code-intel-pipeline.ps1. Do not edit by hand -
    # it only forwards to the real script in the repo. Edit the repo source instead
    # and rerun install-code-intel-pipeline.ps1 only if $repoRoot below has moved.
    $repoRootLiteral = $RepoRoot.Replace("'", "''")
    $relativeLiteral = $RelativeTargetPath.Replace("'", "''")
    $labelLiteral = $CommandLabel.Replace("'", "''")

    return @"
# AUTO-GENERATED thin forwarder. Do not edit by hand.
# Forwards to the repo-owned script so that editing the repo takes effect
# immediately without rerunning install-code-intel-pipeline.ps1.
[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = `$true)]
    [string[]]`$RemainingArgs
)

# CODE_INTEL_REPO_ROOT is honoured here because the error below tells the
# operator to set it. The literal is the install-time location, which goes
# stale the moment the repository is moved or a directory is renamed — the
# override is how you recover without reinstalling.
`$repoRoot = if (-not [string]::IsNullOrWhiteSpace(`$env:CODE_INTEL_REPO_ROOT)) { `$env:CODE_INTEL_REPO_ROOT } else { '$repoRootLiteral' }
`$target = Join-Path `$repoRoot '$relativeLiteral'

if (-not (Test-Path -LiteralPath `$target -PathType Leaf)) {
    Write-Error "code-intel-pipeline: repo not found at `$repoRoot (missing '$relativeLiteral' - label: $labelLiteral). Re-run install-code-intel-pipeline.ps1 from the current repo location, or set CODE_INTEL_REPO_ROOT to override."
    exit 1
}

`$pwshExe = if (Get-Command pwsh -ErrorAction SilentlyContinue) { "pwsh" } else { "powershell" }
& `$pwshExe -NoProfile -ExecutionPolicy Bypass -File `$target @RemainingArgs
exit `$LASTEXITCODE
"@
}

function Install-SentruxShim {
    param(
        [System.Collections.Generic.List[object]]$Actions,
        [string]$Root
    )

    $sourceDir = Join-Path (Join-Path (Join-Path $Root "legacy") "tools") "sentrux-shim"
    $sourcePs1 = Join-Path $sourceDir "sentrux-shim.ps1"
    $sourceCmd = Join-Path $sourceDir "sentrux.cmd"
    $sourceShell = Join-Path $sourceDir "sentrux"
    $sourceLite = Join-Path $sourceDir "sentrux-lite-core.ps1"
    $sourceLauncher = if ($script:EffectivePlatform -eq "windows") { $sourceCmd } else { $sourceShell }
    if (-not (Test-Path -LiteralPath $sourcePs1 -PathType Leaf) -or -not (Test-Path -LiteralPath $sourceLauncher -PathType Leaf) -or -not (Test-Path -LiteralPath $sourceLite -PathType Leaf)) {
        Add-InstallAction $Actions "sentrux-shim" "install_failed" "missing shim source under $sourceDir" "Restore legacy/tools/sentrux-shim from the repository." "repo-local" $false
        return
    }

    try {
        $shimDir = Get-CodeIntelBinDir
        New-Item -ItemType Directory -Force -Path $shimDir | Out-Null
        foreach ($oldFile in @("sentrux.ps1")) {
            $oldPath = Join-Path $shimDir $oldFile
            if (Test-Path -LiteralPath $oldPath -PathType Leaf) {
                Remove-Item -LiteralPath $oldPath -Force
            }
        }

        # bin\ only ever holds thin forwarders now, never script bodies. The
        # forwarders hardcode $Root (the repo path resolved at install time) so
        # PATH invocations always run the live repo copy. Editing the repo takes
        # effect immediately; rerunning install is only needed if the repo moves.
        $shimForwarder = New-ThinForwarderPs1 -RepoRoot $Root -RelativeTargetPath "legacy/tools/sentrux-shim/sentrux-shim.ps1" -CommandLabel "sentrux"
        Set-Content -LiteralPath (Join-Path $shimDir "sentrux-shim.ps1") -Value $shimForwarder -Encoding UTF8

        $liteForwarder = New-ThinForwarderPs1 -RepoRoot $Root -RelativeTargetPath "legacy/tools/sentrux-shim/sentrux-lite-core.ps1" -CommandLabel "sentrux-lite-core"
        Set-Content -LiteralPath (Join-Path $shimDir "sentrux-lite-core.ps1") -Value $liteForwarder -Encoding UTF8

        $launcherName = if ($script:EffectivePlatform -eq "windows") { "sentrux.cmd" } else { "sentrux" }
        $launcherPath = Join-Path $shimDir $launcherName
        Copy-Item -LiteralPath $sourceLauncher -Destination $launcherPath -Force
        if ($script:EffectivePlatform -ne "windows" -and (Get-Command chmod -ErrorAction SilentlyContinue)) {
            & chmod +x $launcherPath
        }

        $repoConfig = [ordered]@{
            repoRoot = $Root
            generatedAt = (Get-Date).ToUniversalTime().ToString("o")
            note = "Generated by install-code-intel-pipeline.ps1. bin/ contains thin forwarders only; edit the repo source at repoRoot, not the files in this directory."
        }
        $repoConfig | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $shimDir "repo.json") -Encoding UTF8

        $pathResult = Add-UserPathPrefix $shimDir

        # Pro auto-activation is opt-in (SENTRUX_AUTO_PRO, see sentrux-shim.ps1):
        # without the opt-in, a healthy install reports Tier: free, so only
        # require Tier: pro when the operator actually opted in.
        $expectedTierPattern = if ($env:SENTRUX_AUTO_PRO -in @("1", "true", "True", "TRUE")) { "Tier:\s+pro" } else { "Tier:\s+(pro|free)" }
        $statusOutput = & $launcherPath pro status 2>&1
        $statusText = ($statusOutput | ForEach-Object { $_.ToString() } | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or $statusText -notmatch $expectedTierPattern) {
            Add-InstallAction $Actions "sentrux-shim" "install_failed" $statusText "Run sentrux pro status and inspect the error." "repo-local" $false
            return
        }

        Add-InstallAction $Actions "sentrux-shim" "installed" "$shimDir (thin forwarder -> $Root) path=$($pathResult.detail)" (Get-PathRefreshFix "sentrux") "repo-local" $false
    }
    catch {
        Add-InstallAction $Actions "sentrux-shim" "install_failed" $_.Exception.Message "Check write permission for the code-intel bin directory." "repo-local" $false
    }
}

function Install-CodeIntelBinary {
    param(
        [System.Collections.Generic.List[object]]$Actions,
        [string]$Root
    )

    $binaryName = if ($script:EffectivePlatform -eq "windows") { "code-intel.exe" } else { "code-intel" }
    $packaged = Join-Path $Root "bin/$binaryName"
    $source = if (Test-Path -LiteralPath $packaged -PathType Leaf) { $packaged } else { $null }
    $cargoManifest = Join-Path $Root "Cargo.toml"
    if ([string]::IsNullOrWhiteSpace([string]$source) -and
        (Test-Path -LiteralPath $cargoManifest -PathType Leaf) -and
        (Get-Command cargo -ErrorAction SilentlyContinue)) {
        try {
            Push-Location $Root
            & cargo build -p code-intel --release
            if ($LASTEXITCODE -ne 0) { throw "cargo build exited with $LASTEXITCODE" }
        }
        catch {
            Add-InstallAction $Actions "code-intel" "install_failed" $_.Exception.Message "Build with 'cargo build -p code-intel --release' or use a packaged release containing bin/$binaryName." "cargo" $false
            return
        }
        finally {
            Pop-Location
        }
        $source = Join-Path $Root "target/release/$binaryName"
    }
    if ([string]::IsNullOrWhiteSpace([string]$source)) {
        $source = @(
            (Join-Path $Root "target/release/$binaryName"),
            (Join-Path $Root "target/debug/$binaryName")
        ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    }
    if ([string]::IsNullOrWhiteSpace([string]$source) -or -not (Test-Path -LiteralPath $source -PathType Leaf)) {
        Add-InstallAction $Actions "code-intel" "install_failed" "No packaged or built $binaryName was found." "Install Rust and build the release binary, or use the release package." "repo-local" $false
        return
    }

    try {
        $binDir = Get-CodeIntelBinDir
        New-Item -ItemType Directory -Force -Path $binDir | Out-Null
        $destination = Join-Path $binDir $binaryName
        if ([System.IO.Path]::GetFullPath($source) -ne [System.IO.Path]::GetFullPath($destination)) {
            Copy-Item -LiteralPath $source -Destination $destination -Force
        }
        if ($script:EffectivePlatform -ne "windows" -and (Get-Command chmod -ErrorAction SilentlyContinue)) {
            & chmod +x $destination
        }
        $pathResult = Add-UserPathPrefix $binDir
        $help = @(& $destination --help 2>&1)
        if ($LASTEXITCODE -ne 0) {
            throw "installed binary failed --help: $($help -join [Environment]::NewLine)"
        }
        $digest = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
        Add-InstallAction $Actions "code-intel" "installed" "$destination sha256=$digest path=$($pathResult.detail)" (Get-PathRefreshFix "code-intel") "repo-local" $false
        Install-IntegrationsManifest $Actions $Root $binDir
    }
    catch {
        Add-InstallAction $Actions "code-intel" "install_failed" $_.Exception.Message "Check write permission for the code-intel bin directory and close any process locking the old binary." "repo-local" $false
    }
}

function Install-IntegrationsManifest {
    param(
        [System.Collections.Generic.List[object]]$Actions,
        [string]$Root,
        [string]$BinDir
    )

    # The installed binary resolves orchestration/integrations.json by walking
    # up from its own directory (discover_manifest in
    # crates/code-intel-cli/src/capability.rs probes each ancestor of the exe
    # dir for orchestration/integrations.json). Copy the repo manifest to the
    # first candidate of that walk, <bin>/orchestration/integrations.json, so
    # the installed binary works without a repo checkout. Overwrites on
    # reinstall to keep the copy current.
    $manifestSource = Join-Path (Join-Path $Root "orchestration") "integrations.json"
    if (-not (Test-Path -LiteralPath $manifestSource -PathType Leaf)) {
        Add-InstallAction $Actions "integrations-manifest" "install_failed" "missing $manifestSource" "Restore orchestration/integrations.json from the repository." "repo-local" $false
        return
    }

    try {
        $manifestDir = Join-Path $BinDir "orchestration"
        New-Item -ItemType Directory -Force -Path $manifestDir | Out-Null
        $destination = Join-Path $manifestDir "integrations.json"
        Copy-Item -LiteralPath $manifestSource -Destination $destination -Force
        Add-InstallAction $Actions "integrations-manifest" "installed" $destination "" "repo-local" $false
    }
    catch {
        Add-InstallAction $Actions "integrations-manifest" "install_failed" $_.Exception.Message "Check write permission for the code-intel bin directory." "repo-local" $false
    }
}

function Repair-RepowiseThinkingBlockPatch {
    param(
        [System.Collections.Generic.List[object]]$Actions
    )

    # repowise's anthropic provider used to read response.content[0].text;
    # reasoning models behind Anthropic-compatible endpoints (e.g. MiniMax-M2.x)
    # return a ThinkingBlock first, so docs generation failed on every page.
    # Patch the installed uv tool venv idempotently: uv tool upgrade wipes the
    # patch and rerunning this installer restores it.
    #
    # Upstream fixed this in repowise 0.32.0 by iterating response.content and
    # taking the first block that has .text. When we see that shape the overlay
    # is obsolete, not broken — report not_needed so a healthy install does not
    # look like a failed one. See overlays\repowise\README.md.
    if ([string]::IsNullOrWhiteSpace($env:APPDATA)) { return }
    $providerPath = Join-Path $env:APPDATA "uv\tools\repowise\Lib\site-packages\repowise\core\providers\llm\anthropic.py"
    if (-not (Test-Path -LiteralPath $providerPath -PathType Leaf)) {
        return
    }

    try {
        $content = Get-Content -LiteralPath $providerPath -Raw
        $patchedMarker = 'getattr(block, "type", "") == "text"'
        $vulnerable = "content=response.content[0].text,"
        if ($content.Contains($patchedMarker)) {
            Add-InstallAction $Actions "repowise-thinking-patch" "already_present" $providerPath ""
            return
        }
        if (-not $content.Contains($vulnerable)) {
            $upstreamFixed = $content.Contains("for block in response.content") -and $content.Contains('hasattr(block, "text")')
            if ($upstreamFixed) {
                Add-InstallAction $Actions "repowise-thinking-patch" "not_needed" "upstream repowise already skips non-text blocks in $providerPath" "None. Drop this overlay once every supported repowise install carries the upstream fix."
                return
            }
            Add-InstallAction $Actions "repowise-thinking-patch" "install_failed" "expected pattern not found in $providerPath; upstream layout changed" "Review overlays\repowise\README.md; patch manually or drop the overlay if upstream fixed it."
            return
        }
        $replacement = @'
content="".join(
                block.text
                for block in response.content
                if getattr(block, "type", "") == "text"
            ),
'@
        $content = $content.Replace($vulnerable, $replacement)
        Set-Content -LiteralPath $providerPath -Value $content -Encoding UTF8
        Add-InstallAction $Actions "repowise-thinking-patch" "installed" $providerPath "Re-run this installer after any 'uv tool upgrade repowise'."
    }
    catch {
        Add-InstallAction $Actions "repowise-thinking-patch" "install_failed" $_.Exception.Message "Patch manually per overlays\repowise\README.md."
    }
}

function Install-SentruxVlangPluginOverlay {
    param(
        [System.Collections.Generic.List[object]]$Actions,
        [string]$Root
    )

    if ($SkipSentruxVlangOverlay) {
        Add-InstallAction $Actions "sentrux-vlang-overlay" "not_requested" "skipped by -SkipSentruxVlangOverlay" ""
        return
    }

    $overlayScript = Join-Path $Root "Install-SentruxVlangOverlay.ps1"
    if (-not (Test-Path -LiteralPath $overlayScript -PathType Leaf)) {
        Add-InstallAction $Actions "sentrux-vlang-overlay" "install_failed" "missing $overlayScript" "Restore Install-SentruxVlangOverlay.ps1 from the repository."
        return
    }

    try {
        $output = & $overlayScript -Platform $script:EffectivePlatform 2>&1
        $text = ($output | ForEach-Object { $_.ToString() } | Out-String).Trim()
        if ($text -match "manual_required") {
            Add-InstallAction $Actions "sentrux-vlang-overlay" "manual_required" $text "Install or build a platform grammar artifact before enabling V parsing." "repo-local" $false
            return
        }
        if ($LASTEXITCODE -ne 0) {
            Add-InstallAction $Actions "sentrux-vlang-overlay" "install_failed" $text "Run Install-SentruxVlangOverlay.ps1 manually and inspect sentrux plugin validate output." "repo-local" $false
            return
        }
        Add-InstallAction $Actions "sentrux-vlang-overlay" "installed" $text "Run sentrux plugin list to confirm vlang is listed." "repo-local" $false
    }
    catch {
        Add-InstallAction $Actions "sentrux-vlang-overlay" "install_failed" $_.Exception.Message "Run Install-SentruxVlangOverlay.ps1 manually after sentrux is installed." "repo-local" $false
    }
}

function Test-CommandOutput {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Name,
        [string]$Category,
        [scriptblock]$Body,
        [string]$ExpectedPattern,
        [string]$Fix
    )

    try {
        $global:LASTEXITCODE = 0
        $output = & $Body 2>&1
        $text = ($output | ForEach-Object { $_.ToString() } | Out-String).Trim()
        $ok = $global:LASTEXITCODE -eq 0 -and $text -match $ExpectedPattern
        Add-Check $Checks $Name $Category $true $ok $text $Fix
    }
    catch {
        Add-Check $Checks $Name $Category $true $false $_.Exception.Message $Fix
    }
}

function Test-SkillPathServesTarget {
    param(
        [string]$Path,
        [string]$Target
    )

    # A SKILL.md existing at $Path proves nothing about whose skill it is. Agent
    # hosts share these directories with other skill managers, and an unrelated
    # manager's junction sitting at ~/.claude/skills/code-intel-pipeline will
    # satisfy a mere existence check while serving a stale skill from its own
    # store. Accept only two shapes: a link that resolves to $Target, or a plain
    # copy whose SKILL.md bytes match it (macOS/Linux installs fall back to a
    # copy when link creation is denied).
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return $false }
    if (-not (Test-Path -LiteralPath $Target -PathType Container)) { return $false }

    try {
        $item = Get-Item -LiteralPath $Path -Force
        if (-not [string]::IsNullOrWhiteSpace([string]$item.LinkTarget)) {
            $resolvedLink = [System.IO.Path]::GetFullPath([string]$item.LinkTarget).TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
            $resolvedTarget = [System.IO.Path]::GetFullPath($Target).TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
            return $resolvedLink -eq $resolvedTarget
        }
    }
    catch {
        return $false
    }

    $pathSkill = Join-Path $Path "SKILL.md"
    $targetSkill = Join-Path $Target "SKILL.md"
    if (-not (Test-Path -LiteralPath $pathSkill -PathType Leaf)) { return $false }
    if (-not (Test-Path -LiteralPath $targetSkill -PathType Leaf)) { return $false }
    return (Get-FileHash -LiteralPath $pathSkill -Algorithm SHA256).Hash -eq (Get-FileHash -LiteralPath $targetSkill -Algorithm SHA256).Hash
}

function Move-OccupiedSkillPathAside {
    param([string]$Path)

    # Never destroy whatever occupied the path. A reparse point can just be
    # unlinked — the store it pointed at keeps every byte. A real directory is
    # renamed, so a foreign skill remains recoverable next to its old location.
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes.HasFlag([System.IO.FileAttributes]::ReparsePoint)) {
        [System.IO.Directory]::Delete($item.FullName, $false)
        return "unlinked"
    }

    $backup = "$Path.replaced-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
    Move-Item -LiteralPath $Path -Destination $backup
    return "moved aside to $backup"
}

function Ensure-SkillLink {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Name,
        [string]$Path,
        [string]$Target,
        [bool]$Repair
    )

    $skillFile = Join-Path $Path "SKILL.md"
    $present = Test-Path -LiteralPath $skillFile -PathType Leaf
    $ok = $present -and (Test-SkillPathServesTarget -Path $Path -Target $Target)
    $detail = if ($ok) { $Path } elseif ($present) { "occupied by a different skill store, not $Target`: $Path" } else { "missing: $Path" }

    if (-not $ok -and $Repair) {
        if (-not (Test-Path -LiteralPath $Target -PathType Container)) {
            $detail = "source skill missing: $Target"
        }
        else {
            $displaced = ""
            if (Test-Path -LiteralPath $Path) {
                $displaced = " (previous occupant $(Move-OccupiedSkillPathAside $Path))"
            }
            $link = New-CodeIntelLink -Path $Path -Target $Target -Platform $script:EffectivePlatform
            $ok = Test-SkillPathServesTarget -Path $Path -Target $Target
            $detail = if ($ok) { "repaired:$($link.mode): $Path$displaced" } else { "repair failed: $Path$displaced" }
        }
    }

    # Required only when the operator asked for the repair: a default install
    # that was told NOT to touch skill links (no -RepairSkillLinks) must not
    # fail the whole install check over a link it was forbidden to create.
    # It degrades to a warning with the fix spelled out — the same contract
    # as the optional Understand Anything skill.
    Add-Check $Checks "skill:$Name" "skill" $Repair $ok $detail "Run with -RepairSkillLinks, or link/copy $Target to $Path."
}

function Ensure-SkillSource {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Path,
        [string]$BundledPath,
        [bool]$Repair
    )

    function Test-BundledSkillPathIncluded {
        param([string]$RelativePath)

        # Interpreter caches must never reach an agent host's skill directory.
        # They are gitignored build output, they differ per machine and Python
        # version, and one stray bootstrap.cpython-313.pyc left by a local
        # `bootstrap.py` run makes the byte-parity check below report the
        # bundled skill "outdated" forever.
        $segments = $RelativePath -split '[\\/]'
        if ($segments -contains "__pycache__") { return $false }
        return -not $RelativePath.EndsWith(".pyc")
    }

    function Get-BundledSkillRelativeFiles {
        param([string]$Root)

        return @(
            Get-ChildItem -LiteralPath $Root -File -Recurse -Force |
                ForEach-Object { [System.IO.Path]::GetRelativePath($Root, $_.FullName) } |
                Where-Object { Test-BundledSkillPathIncluded $_ } |
                Sort-Object
        )
    }

    function Test-BundledSkillCurrent {
        param(
            [string]$InstalledPath,
            [string]$SourcePath
        )

        $bundledSkillFile = Join-Path $SourcePath "SKILL.md"
        if (-not (Test-Path -LiteralPath $bundledSkillFile -PathType Leaf)) {
            return $false
        }
        if (-not (Test-Path -LiteralPath $InstalledPath -PathType Container)) {
            return $false
        }
        $sourceFiles = Get-BundledSkillRelativeFiles $SourcePath
        $installedFiles = Get-BundledSkillRelativeFiles $InstalledPath
        if ([string]::Join("`n", $sourceFiles) -cne [string]::Join("`n", $installedFiles)) {
            return $false
        }
        foreach ($relative in $sourceFiles) {
            $source = Join-Path $SourcePath $relative
            $destination = Join-Path $InstalledPath $relative
            if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) {
                return $false
            }
            $sourceDigest = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
            $destinationDigest = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
            if ($sourceDigest -ne $destinationDigest) {
                return $false
            }
        }
        return $true
    }

    function Install-BundledSkillAtomically {
        param(
            [string]$InstalledPath,
            [string]$SourcePath
        )

        $parent = Split-Path -Parent $InstalledPath
        $leaf = Split-Path -Leaf $InstalledPath
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
        $nonce = [guid]::NewGuid().ToString("N")
        $staging = Join-Path $parent ".$leaf.staging-$nonce"
        $backup = Join-Path $parent ".$leaf.backup-$nonce"
        $hadExisting = Test-Path -LiteralPath $InstalledPath
        try {
            New-Item -ItemType Directory -Path $staging | Out-Null
            foreach ($relative in (Get-BundledSkillRelativeFiles $SourcePath)) {
                $source = Join-Path $SourcePath $relative
                $destination = Join-Path $staging $relative
                $destinationParent = Split-Path -Parent $destination
                if (-not [string]::IsNullOrWhiteSpace($destinationParent)) {
                    New-Item -ItemType Directory -Force -Path $destinationParent | Out-Null
                }
                Copy-Item -LiteralPath $source -Destination $destination -Force
            }
            if ($hadExisting) {
                Move-Item -LiteralPath $InstalledPath -Destination $backup
            }
            Move-Item -LiteralPath $staging -Destination $InstalledPath
            if (Test-Path -LiteralPath $backup) {
                Remove-Item -LiteralPath $backup -Recurse -Force
            }
        }
        catch {
            if (-not (Test-Path -LiteralPath $InstalledPath) -and (Test-Path -LiteralPath $backup)) {
                Move-Item -LiteralPath $backup -Destination $InstalledPath
            }
            throw
        }
        finally {
            if (Test-Path -LiteralPath $staging) {
                Remove-Item -LiteralPath $staging -Recurse -Force
            }
        }
    }

    $bundledSkillFile = Join-Path $BundledPath "SKILL.md"
    $bundledAvailable = Test-Path -LiteralPath $bundledSkillFile -PathType Leaf
    $ok = $bundledAvailable -and (Test-BundledSkillCurrent $Path $BundledPath)
    $detail = if ($ok) { $Path } elseif (-not $bundledAvailable) { "bundled skill missing: $BundledPath" } else { "missing or outdated: $Path" }

    if (-not $ok -and $Repair -and $bundledAvailable) {
        Install-BundledSkillAtomically $Path $BundledPath
        $ok = Test-BundledSkillCurrent $Path $BundledPath
        $detail = if ($ok) { "installed current bundled skill atomically: $BundledPath" } else { "install failed: $Path" }
    }

    # Same contract as skill:$Name above: advisory unless -RepairSkillLinks
    # was requested and the install still could not deliver.
    Add-Check $Checks "skill:source" "skill" $Repair $ok $detail "Run with -RepairSkillLinks to install the bundled skill into $Path."
}

$checks = New-Object System.Collections.Generic.List[object]
$installActions = New-Object System.Collections.Generic.List[object]
$installPlan = New-Object System.Collections.Generic.List[object]
$root = Split-Path -Parent $PSCommandPath
$repoRoot = Split-Path -Parent $root
$paths = Get-CodeIntelPaths -Platform $script:EffectivePlatform -Root $repoRoot
$homeEnv = Set-CodeIntelUserEnv -Name "CODE_INTEL_HOME" -Value $paths.codeIntelHome -Platform $script:EffectivePlatform
Add-InstallAction $installActions "env:CODE_INTEL_HOME" "installed" $homeEnv.detail "" "env" $false
if ([string]::IsNullOrWhiteSpace($Config)) {
    $Config = Join-Path $repoRoot "pipeline.config.json"
}

# repowise comes from PyPI; pin the exact version so `--upgrade` cannot pull a
# newer, unreviewed release onto the machine (supply-chain-003).
$script:RepowisePinnedVersion = "0.38.0"

function Add-ToolInstallPlan {
    param(
        [string]$Name,
        [string]$Command,
        [string]$Purpose,
        [string]$Risk,
        [string]$Alternative = ""
    )

    $metadata = Get-InstallMetadata $Name
    Add-InstallPlan $installPlan $Name $metadata.packageManager $Command $Purpose $Risk $Alternative $metadata.packageManager ([bool]$metadata.requiresElevation)
}

switch ($script:EffectivePlatform) {
    "windows" {
        Add-ToolInstallPlan "rg" "winget/choco/scoop install ripgrep" "Exact file inventory and fast text search." "LOW: established CLI tool; install source should still be package-manager controlled." "Use the rg bundled with Codex if available."
        Add-ToolInstallPlan "git" "winget/choco/scoop install git" "Repository status, worktree, sparse checkout, and history operations." "LOW: foundational tool; ensure official Git for Windows package source." ""
        Add-ToolInstallPlan "python" "winget/choco/scoop install Python 3.11+" "Runs provider preflight and scoped repowise docs helper." "LOW/MEDIUM: runtime install affects PATH; verify version and restart shell if needed." "Use an already managed Python 3.11+ runtime."
    }
    "macos" {
        Add-ToolInstallPlan "rg" "brew install ripgrep" "Exact file inventory and fast text search." "LOW: established CLI tool; install source should still be package-manager controlled." "Use the rg bundled with Codex if available."
        Add-ToolInstallPlan "git" "brew install git" "Repository status, worktree, sparse checkout, and history operations." "LOW: foundational tool; ensure official Git package source." ""
        Add-ToolInstallPlan "python" "brew install python@3.11" "Runs provider preflight and scoped repowise docs helper." "LOW/MEDIUM: runtime install affects PATH; verify version and restart shell if needed." "Use an already managed Python 3.11+ runtime."
    }
    "linux" {
        Add-ToolInstallPlan "rg" "apt/dnf/pacman install ripgrep" "Exact file inventory and fast text search." "LOW: established CLI tool; install source should still be package-manager controlled." "Use the rg bundled with Codex if available."
        Add-ToolInstallPlan "git" "apt/dnf/pacman install git" "Repository status, worktree, sparse checkout, and history operations." "LOW: foundational tool; ensure distro package source." ""
        Add-ToolInstallPlan "python" "apt/dnf/pacman install python3" "Runs provider preflight and scoped repowise docs helper." "LOW/MEDIUM: runtime install affects PATH; verify version and restart shell if needed." "Use an already managed Python 3.11+ runtime."
    }
}
Add-InstallPlan $installPlan "repowise" "pip" "python/python3 -m pip install --user repowise==$script:RepowisePinnedVersion" "Semantic index and wiki/docs memory." "MEDIUM: Python package supply chain; installed version is pinned to repowise==$script:RepowisePinnedVersion." "Skip repowise with -SkipRepowise for exact-search-only runs." "pip" $false
Add-InstallPlan $installPlan "code-intel" "repo-local release binary" "copy bin/code-intel or target/release/code-intel into CODE_INTEL_BIN; build with cargo when no binary is present" "Manifest-bound DAG, evidence query, impact analysis, and atomic publication." "LOW: Pipeline-owned binary; installed digest is reported and --help is executed before success." "Use code-intel.ps1 only when the compiled command needs recovery." "repo-local" $false
Add-InstallPlan $installPlan "integrations-manifest" "repo-local" "copy orchestration/integrations.json into CODE_INTEL_BIN/orchestration so the installed binary resolves capabilities outside a repo checkout" "Capability registry for the installed code-intel binary; overwritten on every reinstall." "LOW: repo-owned JSON manifest copied verbatim." "Set CODE_INTEL_INTEGRATIONS_MANIFEST to point at a custom manifest instead." "repo-local" $false
$sentruxBinaryName = if ($script:EffectivePlatform -eq "windows") { "sentrux.exe" } else { "sentrux" }
Add-InstallPlan $installPlan "sentrux" "repo-local shim or preinstalled binary" "install legacy/tools/sentrux-shim first; optionally place a real $sentruxBinaryName on PATH" "Structural quality and regression gate." "LOW for repo-owned shim; MEDIUM for any separately supplied $sentruxBinaryName." "The repo-owned sentrux-lite core keeps scan/check/gate/plugin usable until the real binary is installed." "repo-local" $false
Add-InstallPlan $installPlan "sentrux-shim" "repo-local" "copy legacy/tools/sentrux-shim launcher to CODE_INTEL_BIN and prepend PATH" "Opt-in local Pro activation, stable forwarding to real sentrux, and deterministic lite-core fallback." "LOW: repo-owned PowerShell/CMD/sh shim; review legacy/tools/sentrux-shim before install." "Pro auto-activation is off by default; set SENTRUX_AUTO_PRO=1 to opt in." "repo-local" $false
Add-InstallPlan $installPlan "sentrux-vlang-overlay" "repo-local" "copy overlays/sentrux/vlang into the user Sentrux plugin directory when a platform grammar exists" "Fixes the broken upstream Windows vlang plugin package and enables V parsing in real sentrux." "LOW/MEDIUM: ships tree-sitter grammar artifacts; review overlays/sentrux/vlang/THIRD_PARTY.md." "Use -SkipSentruxVlangOverlay to skip this local plugin patch." "repo-local" $false

Install-MissingTool $installActions "rg" { Invoke-RipgrepInstall } "Install ripgrep with winget (`winget install --id BurntSushi.ripgrep.MSVC -e`) or ensure rg is on PATH."
Install-MissingTool $installActions "git" { Invoke-ToolPackageInstall "git" } "Install git with the platform package manager or ensure git is on PATH."
Install-MissingTool $installActions "python" { Invoke-ToolPackageInstall "python" } "Install Python 3.11+ with the platform package manager or ensure python is on PATH."
Install-MissingTool $installActions "repowise" { Invoke-PipInstall "repowise" -Version $script:RepowisePinnedVersion } "Install repowise into the active Python environment (`python/python3 -m pip install --user repowise==$script:RepowisePinnedVersion`)."
Install-CodeIntelBinary $installActions $repoRoot
Install-SentruxShim $installActions $repoRoot
Install-MissingTool $installActions "sentrux" { Invoke-SentruxInstall } "Install the repo-owned shim or ensure sentrux.exe is on PATH."
Repair-RepowiseThinkingBlockPatch $installActions
Install-SentruxVlangPluginOverlay $installActions $root

Add-VersionComplianceChecks $checks $installActions

$requiredFiles = @(
    "check-code-intel-tools.ps1",
    "code-intel.ps1",
    "invoke-code-intel.ps1",
    "Install-SentruxVlangOverlay.ps1",
    "scripts/tests/Test-SentruxVlangOverlay.ps1",
    "run-code-intel.ps1",
    "Invoke-SentruxAgentTool.ps1",
    "Invoke-ScopedRepowise.ps1",
    "Invoke-CodeNexusLite.ps1",
    "bootstrap-new-machine.ps1",
    "scripts/tests/test-code-intel-pipeline.ps1",
    "scripts/tests/test-code-intel-provider.ps1",
    "update-code-intel-index.ps1",
    "tools/code-intel-platform.psm1"
)

foreach ($file in $requiredFiles) {
    Test-File $checks "pipeline:$file" (Join-Path $root $file) $true
}
# stayed at the repository root when the PowerShell moved under legacy/
Test-File $checks "pipeline:Run-ScopedRepowiseDocs.py" (Join-Path $repoRoot "Run-ScopedRepowiseDocs.py") $true
Test-File $checks "config" $Config $true
$shimSource = Join-Path (Join-Path (Join-Path $repoRoot "legacy") "tools") "sentrux-shim"
$shimLauncherName = if ($script:EffectivePlatform -eq "windows") { "sentrux.cmd" } else { "sentrux" }
Test-File $checks "sentrux-shim:launcher" (Join-Path $shimSource $shimLauncherName) $true
Test-File $checks "sentrux-shim:ps1" (Join-Path $shimSource "sentrux-shim.ps1") $true
Test-File $checks "sentrux-shim:lite-core" (Join-Path $shimSource "sentrux-lite-core.ps1") $true
$overlayRoot = Join-Path (Join-Path (Join-Path $repoRoot "overlays") "sentrux") "vlang"
Test-File $checks "sentrux-vlang-overlay:plugin" (Join-Path $overlayRoot "plugin.toml") $true
Test-File $checks "sentrux-vlang-overlay:query" (Join-Path (Join-Path $overlayRoot "queries") "tags.scm") $true
$grammarName = switch ($script:EffectivePlatform) {
    "windows" { "windows-x86_64.dll" }
    "macos" { "darwin-arm64.dylib" }
    "linux" { "linux-x86_64.so" }
}
Test-File $checks "sentrux-vlang-overlay:grammar" (Join-Path (Join-Path $overlayRoot "grammars") $grammarName) $false

Test-Tool $checks "rg" $true "Install ripgrep or ensure rg is on PATH."
Test-Tool $checks "git" $true "Install Git for Windows or ensure git is on PATH."
Test-Tool $checks "python" $true "Install Python 3.11+ or ensure python/python3 is on PATH."
Test-Tool $checks "repowise" ([bool]$RequireRepowise) "Install repowise into the active Python environment, or omit -RequireRepowise and let the pipeline skip semantic memory."
Test-Tool $checks "code-intel" $true "Run install-code-intel-pipeline.ps1 so the Pipeline-owned binary is copied into CODE_INTEL_BIN."
Test-Tool $checks "sentrux" $true "Install sentrux or ensure it is on PATH."
Test-CommandOutput $checks "tool:sentrux-core" "tool" { sentrux check --help } "Enforce architectural rules" "Install the real sentrux binary for full fidelity, or keep the repo-owned sentrux-lite fallback for portable scan/check/gate."
# Tier: free is healthy without the SENTRUX_AUTO_PRO opt-in (Pro auto-activation
# is opt-in; see legacy/tools/sentrux-shim/sentrux-shim.ps1).
$sentruxTierPattern = if ($env:SENTRUX_AUTO_PRO -in @("1", "true", "True", "TRUE")) { "Tier:\s+pro" } else { "Tier:\s+(pro|free)" }
Test-CommandOutput $checks "tool:sentrux-pro" "tool" { sentrux pro status } $sentruxTierPattern "Run install-code-intel-pipeline.ps1 again so the repo shim is installed; set SENTRUX_AUTO_PRO=1 first if you expect Pro auto activation."

$userProfile = Get-CodeIntelHomeDirectory
$skillSource = Join-Path (Join-Path (Join-Path $userProfile ".agents") "skills") "code-intel-pipeline"
$codexSkill = Join-Path (Join-Path (Join-Path $userProfile ".codex") "skills") "code-intel-pipeline"
$claudeSkill = Join-Path (Join-Path (Join-Path $userProfile ".claude") "skills") "code-intel-pipeline"
$bundledSkill = Join-Path (Join-Path $repoRoot "skills") "code-intel-pipeline"
Ensure-SkillSource $checks $skillSource $bundledSkill $RepairSkillLinks
Ensure-SkillLink $checks "codex" $codexSkill $skillSource $RepairSkillLinks
Ensure-SkillLink $checks "claude" $claudeSkill $skillSource $RepairSkillLinks

$understandCandidates = @(
    (Join-Path (Join-Path (Join-Path (Join-Path $userProfile ".claude") "skills") "understand") "SKILL.md"),
    (Join-Path (Join-Path (Join-Path (Join-Path $userProfile ".agents") "skills") "understand") "SKILL.md"),
    (Join-Path (Join-Path (Join-Path (Join-Path $userProfile ".codex") "skills") "understand") "SKILL.md")
)
$understandFound = [bool]($understandCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1)
$understandDetail = "missing"
if ($understandFound) {
    $understandDetail = "found"
}
Add-Check $checks "skill:Understand Anything" "skill" ([bool]$RequireUnderstand) $understandFound $understandDetail "Install or link the Understand Anything skill/plugin, or omit -RequireUnderstand and let the pipeline emit the /understand command as a manual step."

# Provider credentials live in dedicated CODE_INTEL_ANTHROPIC_* vars. Global
# ANTHROPIC_* is deliberately NOT checked: on dev machines it belongs to the
# Claude Code proxy chain and must not be repointed at the docs provider.
# CODE_INTEL_PROVIDER is optional: absent means the anthropic default.
Test-EnvVar $checks "CODE_INTEL_PROVIDER" $false
Test-EnvVar $checks "CODE_INTEL_ANTHROPIC_BASE_URL" $false "https://api.minimaxi.com/anthropic"
Test-EnvVar $checks "REPOWISE_PROVIDER" $false "anthropic"
Test-EnvVar $checks "CODE_INTEL_ANTHROPIC_API_KEY" $false

$configOk = $false
$configData = $null
if (Test-Path -LiteralPath $Config -PathType Leaf) {
    try {
        $configData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
        $configOk = $true
        $repos = New-Object System.Collections.Generic.List[string]
        $reposProp = $configData.PSObject.Properties["repos"]
        if ($null -ne $reposProp -and $null -ne $reposProp.Value) {
            foreach ($repoProperty in @($reposProp.Value.PSObject.Properties)) {
                if (-not [string]::IsNullOrWhiteSpace([string]$repoProperty.Name)) {
                    $repos.Add([string]$repoProperty.Name)
                }
            }
        }
        $requiresRepoAlias = -not [string]::IsNullOrWhiteSpace($Repo) -and [string]::IsNullOrWhiteSpace($RepoPath)
        Add-Check $checks "config:repos" "config" $requiresRepoAlias ($repos.Count -gt 0 -or -not $requiresRepoAlias) ("repos=" + ($repos -join ",")) "Add repo aliases under repos, or use -RepoPath for arbitrary project paths."
    }
    catch {
        Add-Check $checks "config:parse" "config" $true $false $_.Exception.Message "Fix invalid JSON in pipeline config."
    }
}

if ([string]::IsNullOrWhiteSpace($ArtifactRoot)) {
    $configuredArtifactRoot = if ($null -ne $configData -and $null -ne $configData.PSObject.Properties["artifactRoot"]) { [string]$configData.artifactRoot } else { "" }
    $ArtifactRoot = if ([string]::IsNullOrWhiteSpace($configuredArtifactRoot)) { Get-DefaultArtifactRoot } else { $configuredArtifactRoot }
}
Test-Directory $checks "artifactRoot" $ArtifactRoot $false "The pipeline will create this directory on first run."

if (-not [string]::IsNullOrWhiteSpace($Repo) -or -not [string]::IsNullOrWhiteSpace($RepoPath)) {
    $doctor = Join-Path $root "check-code-intel-tools.ps1"
    try {
        $doctorParams = @{
            Config = $Config
            Json = $true
            RequireRepowise = [bool]$RequireRepowise
            RequireUnderstand = [bool]$RequireUnderstand
        }
        if (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
            $doctorParams.RepoPath = $RepoPath
        }
        else {
            $doctorParams.Repo = $Repo
        }
        if ($RequireRepowise) { $doctorParams.RequireRepowise = $true }
        if ($RequireUnderstand) { $doctorParams.RequireUnderstand = $true }
        $doctorRaw = & $doctor @doctorParams
        $doctorResult = $doctorRaw | ConvertFrom-Json
        $doctorName = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) { $RepoPath } else { $Repo }
        Add-Check $checks "doctor:$doctorName" "doctor" $true ([bool]$doctorResult.ok) (($doctorResult.missing -join ",")) "Fix missing doctor checks before running the pipeline."
    }
    catch {
        Add-Check $checks "doctor:$Repo" "doctor" $true $false $_.Exception.Message "Run check-code-intel-tools.ps1 manually for details."
    }

    # Issue #155: persist a documentation language preference for this one
    # target repo. All resolution/precedence/persistence logic lives in the
    # Rust `language_pref` module and its `language set` command; this
    # script only decides *whether to ask* and *what value to pass through*.
    try {
        $languageRepoPath = [string]$doctorResult.checks.repo.path
        $resolvedLanguage = $Language
        # Skill/agent installs (`$code-intel-pipeline 为 <path> 安装并运行稳定版`)
        # run unattended with nobody at a keyboard, so a prompt here must
        # never be reached. [Console]::IsInputRedirected is true whenever
        # stdin is not a live console -- piped, redirected, or a
        # non-interactive agent host -- which is exactly the deterministic
        # signal to gate on: no guessing, no other file or history consulted.
        if ([string]::IsNullOrWhiteSpace($resolvedLanguage) -and -not [Console]::IsInputRedirected) {
            $response = Read-Host "Preferred documentation language? [zh/en, Enter to skip]"
            if ($response -match '^\s*(zh|en)\s*$') {
                $resolvedLanguage = $Matches[1]
            }
        }
        if (-not [string]::IsNullOrWhiteSpace($resolvedLanguage) -and -not [string]::IsNullOrWhiteSpace($languageRepoPath)) {
            $languageBinaryName = if ($script:EffectivePlatform -eq "windows") { "code-intel.exe" } else { "code-intel" }
            $languageBinary = Join-Path $paths.bin $languageBinaryName
            if (Test-Path -LiteralPath $languageBinary -PathType Leaf) {
                $languageResult = Invoke-CodeIntelNative -Command $languageBinary -Arguments @("language", "set", "--language", $resolvedLanguage, "--repo", $languageRepoPath, "--json")
                $languageStatus = if ($languageResult.exitCode -eq 0) { "installed" } else { "install_failed" }
                Add-InstallAction $installActions "language:$languageRepoPath" $languageStatus $languageResult.output "Run '$languageBinary language set --language $resolvedLanguage --repo $languageRepoPath' manually." "repo-local" $false
            }
        }
    }
    catch {
        # Never let a language-preference hiccup fail or block the install;
        # it is a convenience, not a requirement (--language on any later
        # command, project config, or the resolver's own fallbacks all still
        # work without this step having run).
        Add-InstallAction $installActions "language" "install_failed" $_.Exception.Message "Run 'code-intel language set --language <zh|en> --repo <path>' manually." "repo-local" $false
    }
}

if ($CheckProvider) {
    $providerScript = Join-Path $root "scripts/tests/test-code-intel-provider.ps1"
    $providerName = [Environment]::GetEnvironmentVariable("CODE_INTEL_PROVIDER", "Process")
    if ([string]::IsNullOrWhiteSpace($providerName)) {
        $providerName = [Environment]::GetEnvironmentVariable("CODE_INTEL_PROVIDER", "User")
    }
    if ([string]::IsNullOrWhiteSpace($providerName)) { $providerName = "anthropic" }
    $providerModel = [Environment]::GetEnvironmentVariable("CODE_INTEL_MODEL", "Process")
    if ([string]::IsNullOrWhiteSpace($providerModel)) {
        $providerModel = [Environment]::GetEnvironmentVariable("CODE_INTEL_MODEL", "User")
    }
    if ([string]::IsNullOrWhiteSpace($providerModel)) {
        $providerModel = [Environment]::GetEnvironmentVariable("CODE_INTEL_REPOWISE_DEFAULT_MODEL", "Process")
        if ([string]::IsNullOrWhiteSpace($providerModel)) {
            $providerModel = [Environment]::GetEnvironmentVariable("CODE_INTEL_REPOWISE_DEFAULT_MODEL", "User")
        }
    }
    if ([string]::IsNullOrWhiteSpace($providerModel)) {
        $providerModel = [Environment]::GetEnvironmentVariable("CODE_INTEL_DEFAULT_MODEL", "Process")
        if ([string]::IsNullOrWhiteSpace($providerModel)) {
            $providerModel = [Environment]::GetEnvironmentVariable("CODE_INTEL_DEFAULT_MODEL", "User")
        }
    }
    if ([string]::IsNullOrWhiteSpace($providerModel) -and $providerName -eq "anthropic") { $providerModel = "MiniMax-M2.7" }
    $providerLabel = if ([string]::IsNullOrWhiteSpace($providerModel)) { "provider:$providerName" } else { "provider:$providerName/$providerModel" }
    try {
        $providerParams = @{ Json = $true; Provider = $providerName }
        if (-not [string]::IsNullOrWhiteSpace($providerModel)) { $providerParams.Model = $providerModel }
        $providerRaw = & $providerScript @providerParams
        $providerResult = $providerRaw | ConvertFrom-Json
        if ($null -eq $providerResult) {
            Add-Check $checks $providerLabel "provider" $true $false "provider script returned no output" "Run scripts/tests/test-code-intel-provider.ps1 -Json manually."
        } else {
            $detail = if ($providerResult.ok) { $providerResult.message } else { "$($providerResult.category): $($providerResult.message)" }
            Add-Check $checks $providerLabel "provider" $true ([bool]$providerResult.ok) $detail "Check provider quota or CODE_INTEL_* provider env vars."
        }
    }
    catch {
        Add-Check $checks $providerLabel "provider" $true $false $_.Exception.Message "Run scripts/tests/test-code-intel-provider.ps1 -Json manually."
    }
}

$missingRequired = @($checks | Where-Object { $_.required -and -not $_.ok })
$warnings = @($checks | Where-Object { -not $_.required -and -not $_.ok })
$result = [ordered]@{
    ok = $missingRequired.Count -eq 0
    root = $repoRoot
    config = $Config
    platform = [ordered]@{
        os = $script:EffectivePlatform
        shell = $PSVersionTable.PSEdition
        psVersion = $PSVersionTable.PSVersion.ToString()
    }
    paths = [ordered]@{
        home = $paths.home
        dataRoot = $paths.dataRoot
        bin = $paths.bin
        codeIntelHome = $paths.codeIntelHome
        artifactRoot = if ([string]::IsNullOrWhiteSpace($ArtifactRoot)) { $paths.artifactRoot } else { $ArtifactRoot }
    }
    repo = $Repo
    repoPath = $RepoPath
    repairedSkillLinks = [bool]$RepairSkillLinks
    providerChecked = [bool]$CheckProvider
    installMissing = [bool]$InstallMissing
    auditInstallPlan = [bool]$AuditInstallPlan
    requireRepowise = [bool]$RequireRepowise
    requireUnderstand = [bool]$RequireUnderstand
    sentruxVlangOverlaySkipped = [bool]$SkipSentruxVlangOverlay
    installPlan = $installPlan
    installActions = $installActions
    missingRequired = $missingRequired
    warnings = $warnings
    checks = $checks
}

if ($Json) {
    $result | ConvertTo-Json -Depth 8
}
else {
    if ($result.ok) {
        Write-Host "Code intel install check: OK"
    }
    else {
        Write-Host "Code intel install check: FAILED"
    }
    Write-Host "Root: $repoRoot"
    Write-Host "Config: $Config"
    if ($AuditInstallPlan) {
        foreach ($planItem in $installPlan) {
            Write-Host "PLAN $($planItem.name) via $($planItem.installer): $($planItem.command)"
            Write-Host "  purpose: $($planItem.purpose)"
            Write-Host "  risk: $($planItem.risk)"
            if (-not [string]::IsNullOrWhiteSpace($planItem.alternative)) {
                Write-Host "  alternative: $($planItem.alternative)"
            }
        }
    }
    foreach ($action in $installActions) {
        Write-Host "INSTALL $($action.status) $($action.name) $($action.detail)"
        if ($action.status -eq "install_failed" -and -not [string]::IsNullOrWhiteSpace($action.fix)) {
            Write-Host "  fix: $($action.fix)"
        }
    }
    foreach ($check in $checks) {
        $mark = if ($check.ok) { "OK" } elseif ($check.required) { "MISSING" } else { "WARN" }
        Write-Host "$mark $($check.name) [$($check.category)] $($check.detail)"
        if (-not $check.ok -and -not [string]::IsNullOrWhiteSpace($check.fix)) {
            Write-Host "  fix: $($check.fix)"
        }
    }
}

if (-not $result.ok) {
    exit 1
}
exit 0
