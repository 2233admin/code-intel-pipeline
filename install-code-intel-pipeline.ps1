#requires -Version 7.2

param(
    [string]$Config = "",
    [string]$Repo = "",
    [string]$RepoPath = "",
    [string]$ArtifactRoot = "",
    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto",
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
        [string]$PackageName
    )

    $python = Get-CodeIntelPythonCommand
    if (-not $python) {
        throw "python/python3 is not on PATH; install Python and rerun this script in a new shell"
    }
    $pythonCommand = if (-not [string]::IsNullOrWhiteSpace($python.Source)) { $python.Source } else { $python.Name }

    & $pythonCommand -m pip install --user --upgrade $PackageName
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
        "repowise" { return [ordered]@{ packageManager = "pip"; requiresElevation = $false } }
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
        Add-InstallAction $Actions $CommandName "already_present" $existing.Source "" $metadata.packageManager ([bool]$metadata.requiresElevation)
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

`$repoRoot = '$repoRootLiteral'
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

    $sourceDir = Join-Path (Join-Path $Root "tools") "sentrux-shim"
    $sourcePs1 = Join-Path $sourceDir "sentrux-shim.ps1"
    $sourceCmd = Join-Path $sourceDir "sentrux.cmd"
    $sourceShell = Join-Path $sourceDir "sentrux"
    $sourceLite = Join-Path $sourceDir "sentrux-lite-core.ps1"
    $sourceLauncher = if ($script:EffectivePlatform -eq "windows") { $sourceCmd } else { $sourceShell }
    if (-not (Test-Path -LiteralPath $sourcePs1 -PathType Leaf) -or -not (Test-Path -LiteralPath $sourceLauncher -PathType Leaf) -or -not (Test-Path -LiteralPath $sourceLite -PathType Leaf)) {
        Add-InstallAction $Actions "sentrux-shim" "install_failed" "missing shim source under $sourceDir" "Restore tools/sentrux-shim from the repository." "repo-local" $false
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
        $shimForwarder = New-ThinForwarderPs1 -RepoRoot $Root -RelativeTargetPath "tools/sentrux-shim/sentrux-shim.ps1" -CommandLabel "sentrux"
        Set-Content -LiteralPath (Join-Path $shimDir "sentrux-shim.ps1") -Value $shimForwarder -Encoding UTF8

        $liteForwarder = New-ThinForwarderPs1 -RepoRoot $Root -RelativeTargetPath "tools/sentrux-shim/sentrux-lite-core.ps1" -CommandLabel "sentrux-lite-core"
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

        $statusOutput = & $launcherPath pro status 2>&1
        $statusText = ($statusOutput | ForEach-Object { $_.ToString() } | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or $statusText -notmatch "Tier:\s+pro") {
            Add-InstallAction $Actions "sentrux-shim" "install_failed" $statusText "Run sentrux pro status and inspect the error." "repo-local" $false
            return
        }

        Add-InstallAction $Actions "sentrux-shim" "installed" "$shimDir (thin forwarder -> $Root) path=$($pathResult.detail)" "Open a new terminal if this shell cannot find sentrux from PATH." "repo-local" $false
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
        Add-InstallAction $Actions "code-intel" "installed" "$destination sha256=$digest path=$($pathResult.detail)" "Open a new terminal if this shell cannot resolve code-intel from PATH." "repo-local" $false
    }
    catch {
        Add-InstallAction $Actions "code-intel" "install_failed" $_.Exception.Message "Check write permission for the code-intel bin directory and close any process locking the old binary." "repo-local" $false
    }
}

function Repair-RepowiseThinkingBlockPatch {
    param(
        [System.Collections.Generic.List[object]]$Actions
    )

    # repowise's anthropic provider reads response.content[0].text; reasoning
    # models behind Anthropic-compatible endpoints (e.g. MiniMax-M2.x) return
    # a ThinkingBlock first, so docs generation fails on every page. Patch the
    # installed uv tool venv idempotently: uv tool upgrade wipes the patch and
    # rerunning this installer restores it. See overlays\repowise\README.md.
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

function Ensure-SkillLink {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Name,
        [string]$Path,
        [string]$Target,
        [bool]$Repair
    )

    $skillFile = Join-Path $Path "SKILL.md"
    $ok = Test-Path -LiteralPath $skillFile -PathType Leaf
    $detail = if ($ok) { $Path } else { "missing: $Path" }

    if (-not $ok -and $Repair) {
        if (-not (Test-Path -LiteralPath $Target -PathType Container)) {
            $detail = "source skill missing: $Target"
        }
        else {
            $link = New-CodeIntelLink -Path $Path -Target $Target -Platform $script:EffectivePlatform
            $ok = Test-Path -LiteralPath $skillFile -PathType Leaf
            $detail = if ($ok) { "repaired:$($link.mode): $Path" } else { "repair failed: $Path" }
        }
    }

    Add-Check $Checks "skill:$Name" "skill" $true $ok $detail "Run with -RepairSkillLinks, or link/copy $Target to $Path."
}

function Ensure-SkillSource {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Path,
        [string]$BundledPath,
        [bool]$Repair
    )

    $skillFile = Join-Path $Path "SKILL.md"
    $ok = Test-Path -LiteralPath $skillFile -PathType Leaf
    $detail = if ($ok) { $Path } else { "missing: $Path" }

    if (-not $ok -and $Repair) {
        $bundledSkillFile = Join-Path $BundledPath "SKILL.md"
        if (-not (Test-Path -LiteralPath $bundledSkillFile -PathType Leaf)) {
            $detail = "bundled skill missing: $BundledPath"
        }
        else {
            New-Item -ItemType Directory -Force -Path $Path | Out-Null
            Copy-Item -LiteralPath (Join-Path $BundledPath "SKILL.md") -Destination (Join-Path $Path "SKILL.md") -Force
            $bundledAgents = Join-Path $BundledPath "agents"
            if (Test-Path -LiteralPath $bundledAgents -PathType Container) {
                Copy-Item -LiteralPath $bundledAgents -Destination $Path -Recurse -Force
            }
            $ok = Test-Path -LiteralPath $skillFile -PathType Leaf
            $detail = if ($ok) { "installed from bundled skill: $BundledPath" } else { "install failed: $Path" }
        }
    }

    Add-Check $Checks "skill:source" "skill" $true $ok $detail "Run with -RepairSkillLinks to install the bundled skill into $Path."
}

$checks = New-Object System.Collections.Generic.List[object]
$installActions = New-Object System.Collections.Generic.List[object]
$installPlan = New-Object System.Collections.Generic.List[object]
$root = Split-Path -Parent $PSCommandPath
$paths = Get-CodeIntelPaths -Platform $script:EffectivePlatform -Root $root
$homeEnv = Set-CodeIntelUserEnv -Name "CODE_INTEL_HOME" -Value $paths.codeIntelHome -Platform $script:EffectivePlatform
Add-InstallAction $installActions "env:CODE_INTEL_HOME" "installed" $homeEnv.detail "" "env" $false
if ([string]::IsNullOrWhiteSpace($Config)) {
    $Config = Join-Path $root "pipeline.config.json"
}

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
Add-InstallPlan $installPlan "repowise" "pip" "python/python3 -m pip install --user --upgrade repowise" "Semantic index and wiki/docs memory." "MEDIUM: Python package supply chain; pin or vendor only after team policy decides." "Skip repowise with -SkipRepowise for exact-search-only runs." "pip" $false
Add-InstallPlan $installPlan "code-intel" "repo-local release binary" "copy bin/code-intel or target/release/code-intel into CODE_INTEL_BIN; build with cargo when no binary is present" "Manifest-bound DAG, evidence query, impact analysis, and atomic publication." "LOW: Pipeline-owned binary; installed digest is reported and --help is executed before success." "Use invoke-code-intel.ps1 from the source tree; it can build a debug binary on demand." "repo-local" $false
$sentruxBinaryName = if ($script:EffectivePlatform -eq "windows") { "sentrux.exe" } else { "sentrux" }
Add-InstallPlan $installPlan "sentrux" "repo-local shim or preinstalled binary" "install tools/sentrux-shim first; optionally place a real $sentruxBinaryName on PATH" "Structural quality and regression gate." "LOW for repo-owned shim; MEDIUM for any separately supplied $sentruxBinaryName." "The repo-owned sentrux-lite core keeps scan/check/gate/plugin usable until the real binary is installed." "repo-local" $false
Add-InstallPlan $installPlan "sentrux-shim" "repo-local" "copy tools/sentrux-shim launcher to CODE_INTEL_BIN and prepend PATH" "Open-source local Pro activation, stable forwarding to real sentrux, and deterministic lite-core fallback." "LOW: repo-owned PowerShell/CMD/sh shim; review tools/sentrux-shim before install." "Set SENTRUX_AUTO_PRO=0 to disable auto Pro activation." "repo-local" $false
Add-InstallPlan $installPlan "sentrux-vlang-overlay" "repo-local" "copy overlays/sentrux/vlang into the user Sentrux plugin directory when a platform grammar exists" "Fixes the broken upstream Windows vlang plugin package and enables V parsing in real sentrux." "LOW/MEDIUM: ships tree-sitter grammar artifacts; review overlays/sentrux/vlang/THIRD_PARTY.md." "Use -SkipSentruxVlangOverlay to skip this local plugin patch." "repo-local" $false

Install-MissingTool $installActions "rg" { Invoke-RipgrepInstall } "Install ripgrep with winget (`winget install --id BurntSushi.ripgrep.MSVC -e`) or ensure rg is on PATH."
Install-MissingTool $installActions "git" { Invoke-ToolPackageInstall "git" } "Install git with the platform package manager or ensure git is on PATH."
Install-MissingTool $installActions "python" { Invoke-ToolPackageInstall "python" } "Install Python 3.11+ with the platform package manager or ensure python is on PATH."
Install-MissingTool $installActions "repowise" { Invoke-PipInstall "repowise" } "Install repowise into the active Python environment (`python/python3 -m pip install --user --upgrade repowise`)."
Install-CodeIntelBinary $installActions $root
Install-SentruxShim $installActions $root
Install-MissingTool $installActions "sentrux" { Invoke-SentruxInstall } "Install the repo-owned shim or ensure sentrux.exe is on PATH."
Repair-RepowiseThinkingBlockPatch $installActions
Install-SentruxVlangPluginOverlay $installActions $root

$requiredFiles = @(
    "check-code-intel-tools.ps1",
    "invoke-code-intel.ps1",
    "Install-SentruxVlangOverlay.ps1",
    "Test-SentruxVlangOverlay.ps1",
    "run-code-intel.ps1",
    "Invoke-SentruxAgentTool.ps1",
    "Invoke-ScopedRepowise.ps1",
    "Run-ScopedRepowiseDocs.py",
    "Invoke-CodeNexusLite.ps1",
    "bootstrap-new-machine.ps1",
    "test-code-intel-pipeline.ps1",
    "test-code-intel-provider.ps1",
    "update-code-intel-index.ps1",
    "tools/code-intel-platform.psm1"
)

foreach ($file in $requiredFiles) {
    Test-File $checks "pipeline:$file" (Join-Path $root $file) $true
}
Test-File $checks "config" $Config $true
$shimSource = Join-Path (Join-Path $root "tools") "sentrux-shim"
$shimLauncherName = if ($script:EffectivePlatform -eq "windows") { "sentrux.cmd" } else { "sentrux" }
Test-File $checks "sentrux-shim:launcher" (Join-Path $shimSource $shimLauncherName) $true
Test-File $checks "sentrux-shim:ps1" (Join-Path $shimSource "sentrux-shim.ps1") $true
Test-File $checks "sentrux-shim:lite-core" (Join-Path $shimSource "sentrux-lite-core.ps1") $true
$overlayRoot = Join-Path (Join-Path (Join-Path $root "overlays") "sentrux") "vlang"
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
Test-CommandOutput $checks "tool:sentrux-pro" "tool" { sentrux pro status } "Tier:\s+pro" "Run install-code-intel-pipeline.ps1 again so the repo shim is installed and auto activation is enabled."

$userProfile = Get-CodeIntelHomeDirectory
$skillSource = Join-Path (Join-Path (Join-Path $userProfile ".agents") "skills") "code-intel-pipeline"
$codexSkill = Join-Path (Join-Path (Join-Path $userProfile ".codex") "skills") "code-intel-pipeline"
$claudeSkill = Join-Path (Join-Path (Join-Path $userProfile ".claude") "skills") "code-intel-pipeline"
$bundledSkill = Join-Path $root "skill"
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
}

if ($CheckProvider) {
    $providerScript = Join-Path $root "test-code-intel-provider.ps1"
    $providerName = [Environment]::GetEnvironmentVariable("CODE_INTEL_PROVIDER", "Process")
    if ([string]::IsNullOrWhiteSpace($providerName)) {
        $providerName = [Environment]::GetEnvironmentVariable("CODE_INTEL_PROVIDER", "User")
    }
    if ([string]::IsNullOrWhiteSpace($providerName)) { $providerName = "anthropic" }
    $providerModel = [Environment]::GetEnvironmentVariable("CODE_INTEL_MODEL", "Process")
    if ([string]::IsNullOrWhiteSpace($providerModel)) {
        $providerModel = [Environment]::GetEnvironmentVariable("CODE_INTEL_MODEL", "User")
    }
    if ([string]::IsNullOrWhiteSpace($providerModel) -and $providerName -eq "anthropic") { $providerModel = "MiniMax-M2.7" }
    $providerLabel = if ([string]::IsNullOrWhiteSpace($providerModel)) { "provider:$providerName" } else { "provider:$providerName/$providerModel" }
    try {
        $providerParams = @{ Json = $true; Provider = $providerName }
        if (-not [string]::IsNullOrWhiteSpace($providerModel)) { $providerParams.Model = $providerModel }
        $providerRaw = & $providerScript @providerParams
        $providerResult = $providerRaw | ConvertFrom-Json
        if ($null -eq $providerResult) {
            Add-Check $checks $providerLabel "provider" $true $false "provider script returned no output" "Run test-code-intel-provider.ps1 -Json manually."
        } else {
            $detail = if ($providerResult.ok) { $providerResult.message } else { "$($providerResult.category): $($providerResult.message)" }
            Add-Check $checks $providerLabel "provider" $true ([bool]$providerResult.ok) $detail "Check provider quota or CODE_INTEL_* provider env vars."
        }
    }
    catch {
        Add-Check $checks $providerLabel "provider" $true $false $_.Exception.Message "Run test-code-intel-provider.ps1 -Json manually."
    }
}

$missingRequired = @($checks | Where-Object { $_.required -and -not $_.ok })
$warnings = @($checks | Where-Object { -not $_.required -and -not $_.ok })
$result = [ordered]@{
    ok = $missingRequired.Count -eq 0
    root = $root
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
    Write-Host "Root: $root"
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
