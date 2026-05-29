param(
    [string]$Config = "",
    [string]$Repo = "",
    [string]$RepoPath = "",
    [string]$ArtifactRoot = "",
    [switch]$RepairSkillLinks,
    [switch]$CheckProvider,
    [switch]$InstallMissing,
    [switch]$AuditInstallPlan,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

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
        [string]$Fix = ""
    )

    $Actions.Add([pscustomobject][ordered]@{
        name = $Name
        status = $Status
        detail = $Detail
        fix = $Fix
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
        [string]$Alternative = ""
    )

    $Plan.Add([pscustomobject][ordered]@{
        name = $Name
        installer = $Installer
        command = $Command
        purpose = $Purpose
        risk = $Risk
        alternative = $Alternative
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

function Invoke-RipgrepInstall {
    if (Get-Command winget -ErrorAction SilentlyContinue) {
        Invoke-WingetInstall "BurntSushi.ripgrep.MSVC" "ripgrep"
        return
    }

    if (Get-Command scoop -ErrorAction SilentlyContinue) {
        & scoop install ripgrep
        if ($LASTEXITCODE -ne 0) {
            throw "scoop install failed for ripgrep with exit code $LASTEXITCODE"
        }
        return
    }

    throw "no supported installer found for ripgrep; install winget or scoop first"
}

function Invoke-PipInstall {
    param(
        [string]$PackageName
    )

    $python = Get-Command python -ErrorAction SilentlyContinue
    if (-not $python) {
        throw "python is not on PATH; install Python and rerun this script in a new shell"
    }

    & python -m pip install --upgrade $PackageName
    if ($LASTEXITCODE -ne 0) {
        throw "pip install failed for $PackageName with exit code $LASTEXITCODE"
    }
}

function Invoke-SentruxInstall {
    if (Get-Command cargo -ErrorAction SilentlyContinue) {
        & cargo install sentrux --locked
        if ($LASTEXITCODE -ne 0) {
            throw "cargo install failed for sentrux with exit code $LASTEXITCODE"
        }
        return
    }

    throw "no automatic sentrux installer found; install sentrux.exe to PATH, for example under C:\Users\Administrator\bin"
}

function Install-MissingTool {
    param(
        [System.Collections.Generic.List[object]]$Actions,
        [string]$CommandName,
        [scriptblock]$Installer,
        [string]$Fix
    )

    $existing = Get-Command $CommandName -ErrorAction SilentlyContinue
    if ($existing) {
        Add-InstallAction $Actions $CommandName "already_present" $existing.Source ""
        return
    }

    if (-not $InstallMissing) {
        Add-InstallAction $Actions $CommandName "not_requested" "missing" $Fix
        return
    }

    try {
        & $Installer
        $after = Get-Command $CommandName -ErrorAction SilentlyContinue
        if ($after) {
            Add-InstallAction $Actions $CommandName "installed" $after.Source ""
        }
        else {
            Add-InstallAction $Actions $CommandName "installed_restart_required" "installer completed but command is not visible in this shell" "Open a new terminal and rerun install-code-intel-pipeline.ps1."
        }
    }
    catch {
        Add-InstallAction $Actions $CommandName "install_failed" $_.Exception.Message $Fix
    }
}

function Test-Tool {
    param(
        [System.Collections.Generic.List[object]]$Checks,
        [string]$Name,
        [bool]$Required = $true,
        [string]$Fix = ""
    )

    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
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
    $fromEnv = [Environment]::GetEnvironmentVariable("CODE_INTEL_ARTIFACT_ROOT", "User")
    if (-not [string]::IsNullOrWhiteSpace($fromEnv)) { return $fromEnv }
    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_ARTIFACT_ROOT)) { return $env:CODE_INTEL_ARTIFACT_ROOT }
    $base = if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { $env:LOCALAPPDATA } else { (Join-Path $HOME ".code-intel") }
    return (Join-Path $base "code-intel\artifacts")
}

function Get-CodeIntelBinDir {
    $fromEnv = [Environment]::GetEnvironmentVariable("CODE_INTEL_BIN", "User")
    if (-not [string]::IsNullOrWhiteSpace($fromEnv)) { return $fromEnv }
    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_BIN)) { return $env:CODE_INTEL_BIN }
    $base = if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { $env:LOCALAPPDATA } else { (Join-Path $HOME ".code-intel") }
    return (Join-Path $base "code-intel\bin")
}

function Add-UserPathPrefix {
    param([string]$PathToAdd)

    $resolved = (New-Item -ItemType Directory -Force -Path $PathToAdd).FullName.TrimEnd('\')

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $userParts = @($userPath -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $userParts = @($userParts | Where-Object { -not [string]::Equals($_.TrimEnd('\'), $resolved, [System.StringComparison]::OrdinalIgnoreCase) })
    [Environment]::SetEnvironmentVariable("Path", (($resolved) + ";" + ($userParts -join ";")).TrimEnd(";"), "User")

    $processParts = @($env:Path -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $processParts = @($processParts | Where-Object { -not [string]::Equals($_.TrimEnd('\'), $resolved, [System.StringComparison]::OrdinalIgnoreCase) })
    $env:Path = (($resolved) + ";" + ($processParts -join ";")).TrimEnd(";")
}

function Install-SentruxShim {
    param(
        [System.Collections.Generic.List[object]]$Actions,
        [string]$Root
    )

    $sourceDir = Join-Path $Root "tools\sentrux-shim"
    $sourcePs1 = Join-Path $sourceDir "sentrux-shim.ps1"
    $sourceCmd = Join-Path $sourceDir "sentrux.cmd"
    if (-not (Test-Path -LiteralPath $sourcePs1 -PathType Leaf) -or -not (Test-Path -LiteralPath $sourceCmd -PathType Leaf)) {
        Add-InstallAction $Actions "sentrux-shim" "install_failed" "missing shim source under $sourceDir" "Restore tools\sentrux-shim from the repository."
        return
    }

    try {
        $shimDir = Get-CodeIntelBinDir
        New-Item -ItemType Directory -Force -Path $shimDir | Out-Null
        $oldPs1 = Join-Path $shimDir "sentrux.ps1"
        if (Test-Path -LiteralPath $oldPs1 -PathType Leaf) {
            Remove-Item -LiteralPath $oldPs1 -Force
        }
        Copy-Item -LiteralPath $sourcePs1 -Destination (Join-Path $shimDir "sentrux-shim.ps1") -Force
        Copy-Item -LiteralPath $sourceCmd -Destination (Join-Path $shimDir "sentrux.cmd") -Force
        Add-UserPathPrefix $shimDir

        $statusOutput = & (Join-Path $shimDir "sentrux.cmd") pro status 2>&1
        $statusText = ($statusOutput | ForEach-Object { $_.ToString() } | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or $statusText -notmatch "Tier:\s+pro") {
            Add-InstallAction $Actions "sentrux-shim" "install_failed" $statusText "Run sentrux pro status and inspect the error."
            return
        }

        Add-InstallAction $Actions "sentrux-shim" "installed" $shimDir "Open a new terminal if this shell cannot find sentrux from PATH."
    }
    catch {
        Add-InstallAction $Actions "sentrux-shim" "install_failed" $_.Exception.Message "Check write permission for the user CODE_INTEL_BIN or LOCALAPPDATA directory."
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
            $parent = Split-Path -Parent $Path
            New-Item -ItemType Directory -Force -Path $parent | Out-Null
            if (-not (Test-Path -LiteralPath $Path)) {
                New-Item -ItemType Junction -Path $Path -Target $Target | Out-Null
            }
            $ok = Test-Path -LiteralPath $skillFile -PathType Leaf
            $detail = if ($ok) { "repaired: $Path" } else { "repair failed: $Path" }
        }
    }

    Add-Check $Checks "skill:$Name" "skill" $true $ok $detail "Run with -RepairSkillLinks, or create a junction from $Path to $Target."
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
if ([string]::IsNullOrWhiteSpace($Config)) {
    $Config = Join-Path $root "pipeline.config.json"
}

Add-InstallPlan $installPlan "rg" "winget or scoop" "winget install --id BurntSushi.ripgrep.MSVC -e" "Exact file inventory and fast text search." "LOW: established CLI tool; install source should still be package-manager controlled." "Use the rg bundled with Codex if available."
Add-InstallPlan $installPlan "git" "winget" "winget install --id Git.Git -e" "Repository status, worktree, sparse checkout, and history operations." "LOW: foundational tool; ensure official Git for Windows package source." ""
Add-InstallPlan $installPlan "python" "winget" "winget install --id Python.Python.3.11 -e" "Runs provider preflight and scoped repowise docs helper." "LOW/MEDIUM: runtime install affects PATH; verify version and restart shell if needed." "Use an already managed Python 3.11+ runtime."
Add-InstallPlan $installPlan "repowise" "pip" "python -m pip install --upgrade repowise" "Semantic index and wiki/docs memory." "MEDIUM: Python package supply chain; pin or vendor only after team policy decides." "Skip repowise with -SkipRepowise for exact-search-only runs."
Add-InstallPlan $installPlan "sentrux" "cargo" "cargo install sentrux --locked" "Structural quality and regression gate." "MEDIUM: cargo source must be trusted; no automatic install if cargo is absent." "Install a reviewed sentrux.exe on PATH."
Add-InstallPlan $installPlan "sentrux-shim" "repo-local" "copy tools\\sentrux-shim to CODE_INTEL_BIN and prepend user PATH" "Open-source local Pro activation plus stable forwarding to the real sentrux binary." "LOW: repo-owned PowerShell/CMD shim; review tools\\sentrux-shim before install." "Set SENTRUX_AUTO_PRO=0 to disable auto Pro activation."

Install-MissingTool $installActions "rg" { Invoke-RipgrepInstall } "Install ripgrep with winget (`winget install --id BurntSushi.ripgrep.MSVC -e`) or ensure rg is on PATH."
Install-MissingTool $installActions "git" { Invoke-WingetInstall "Git.Git" "Git for Windows" } "Install Git for Windows (`winget install --id Git.Git -e`) or ensure git is on PATH."
Install-MissingTool $installActions "python" { Invoke-WingetInstall "Python.Python.3.11" "Python 3.11" } "Install Python 3.11+ (`winget install --id Python.Python.3.11 -e`) or ensure python is on PATH."
Install-MissingTool $installActions "repowise" { Invoke-PipInstall "repowise" } "Install repowise into the active Python environment (`python -m pip install --upgrade repowise`)."
Install-MissingTool $installActions "sentrux" { Invoke-SentruxInstall } "Install sentrux or ensure sentrux.exe is on PATH."
Install-SentruxShim $installActions $root

$requiredFiles = @(
    "check-code-intel-tools.ps1",
    "invoke-code-intel.ps1",
    "run-code-intel.ps1",
    "Invoke-SentruxAgentTool.ps1",
    "Invoke-ScopedRepowise.ps1",
    "Run-ScopedRepowiseDocs.py",
    "test-code-intel-pipeline.ps1",
    "test-code-intel-provider.ps1",
    "update-code-intel-index.ps1"
)

foreach ($file in $requiredFiles) {
    Test-File $checks "pipeline:$file" (Join-Path $root $file) $true
}
Test-File $checks "config" $Config $true
Test-File $checks "sentrux-shim:cmd" (Join-Path $root "tools\sentrux-shim\sentrux.cmd") $true
Test-File $checks "sentrux-shim:ps1" (Join-Path $root "tools\sentrux-shim\sentrux-shim.ps1") $true

Test-Tool $checks "rg" $true "Install ripgrep or ensure rg is on PATH."
Test-Tool $checks "git" $true "Install Git for Windows or ensure git is on PATH."
Test-Tool $checks "python" $true "Install Python 3.11+ or ensure python is on PATH."
Test-Tool $checks "repowise" $true "Install repowise into the active Python environment."
Test-Tool $checks "sentrux" $true "Install sentrux or ensure sentrux.exe is on PATH."
Test-CommandOutput $checks "tool:sentrux-core" "tool" { sentrux check --help } "Enforce architectural rules" "Install the real sentrux binary; the shim only handles local Pro activation and forwards other commands."
Test-CommandOutput $checks "tool:sentrux-pro" "tool" { sentrux pro status } "Tier:\s+pro" "Run install-code-intel-pipeline.ps1 again so the repo shim is installed and auto activation is enabled."

$userProfile = if ([string]::IsNullOrWhiteSpace($env:USERPROFILE)) { "C:\Users\Administrator" } else { $env:USERPROFILE }
$skillSource = Join-Path $userProfile ".agents\skills\code-intel-pipeline"
$codexSkill = Join-Path $userProfile ".codex\skills\code-intel-pipeline"
$claudeSkill = Join-Path $userProfile ".claude\skills\code-intel-pipeline"
$bundledSkill = Join-Path $root "skill"
Ensure-SkillSource $checks $skillSource $bundledSkill $RepairSkillLinks
Ensure-SkillLink $checks "codex" $codexSkill $skillSource $RepairSkillLinks
Ensure-SkillLink $checks "claude" $claudeSkill $skillSource $RepairSkillLinks

$understandCandidates = @(
    (Join-Path $userProfile ".claude\skills\understand\SKILL.md"),
    (Join-Path $userProfile ".agents\skills\understand\SKILL.md"),
    (Join-Path $userProfile ".codex\skills\understand\SKILL.md")
)
$understandFound = [bool]($understandCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1)
$understandDetail = "missing"
if ($understandFound) {
    $understandDetail = "found"
}
Add-Check $checks "skill:Understand Anything" "skill" $true $understandFound $understandDetail "Install or link the Understand Anything skill/plugin."

Test-EnvVar $checks "ANTHROPIC_BASE_URL" $false "https://api.minimaxi.com/anthropic"
Test-EnvVar $checks "REPOWISE_PROVIDER" $false "anthropic"
Test-EnvVar $checks "ANTHROPIC_API_KEY" $false
Test-EnvVar $checks "ANTHROPIC_AUTH_TOKEN" $false

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
        $doctorRaw = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
            & $doctor -Config $Config -RepoPath $RepoPath -Json
        }
        else {
            & $doctor -Config $Config -Repo $Repo -Json
        }
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
    try {
        $providerRaw = & $providerScript -Json
        $providerResult = $providerRaw | ConvertFrom-Json
        $detail = if ($providerResult.ok) { $providerResult.message } else { "$($providerResult.category): $($providerResult.message)" }
        Add-Check $checks "provider:MiniMax-M2.7" "provider" $true ([bool]$providerResult.ok) $detail "Check provider quota or user-scoped Anthropic-compatible env vars."
    }
    catch {
        Add-Check $checks "provider:MiniMax-M2.7" "provider" $true $false $_.Exception.Message "Run test-code-intel-provider.ps1 -Json manually."
    }
}

$missingRequired = @($checks | Where-Object { $_.required -and -not $_.ok })
$warnings = @($checks | Where-Object { -not $_.required -and -not $_.ok })
$result = [ordered]@{
    ok = $missingRequired.Count -eq 0
    root = $root
    config = $Config
    repo = $Repo
    repoPath = $RepoPath
    repairedSkillLinks = [bool]$RepairSkillLinks
    providerChecked = [bool]$CheckProvider
    installMissing = [bool]$InstallMissing
    auditInstallPlan = [bool]$AuditInstallPlan
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
