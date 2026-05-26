param(
    [string]$Config = "D:\projects\_tools\code-intel-pipeline\pipeline.config.json",
    [string]$Repo = "",
    [switch]$RepairSkillLinks,
    [switch]$CheckProvider,
    [switch]$InstallMissing,
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

$checks = New-Object System.Collections.Generic.List[object]
$installActions = New-Object System.Collections.Generic.List[object]
$root = Split-Path -Parent $PSCommandPath
$artifactRoot = "D:\projects\_artifacts\code-intel"

Install-MissingTool $installActions "rg" { Invoke-RipgrepInstall } "Install ripgrep with winget (`winget install --id BurntSushi.ripgrep.MSVC -e`) or ensure rg is on PATH."
Install-MissingTool $installActions "git" { Invoke-WingetInstall "Git.Git" "Git for Windows" } "Install Git for Windows (`winget install --id Git.Git -e`) or ensure git is on PATH."
Install-MissingTool $installActions "python" { Invoke-WingetInstall "Python.Python.3.11" "Python 3.11" } "Install Python 3.11+ (`winget install --id Python.Python.3.11 -e`) or ensure python is on PATH."
Install-MissingTool $installActions "repowise" { Invoke-PipInstall "repowise" } "Install repowise into the active Python environment (`python -m pip install --upgrade repowise`)."
Install-MissingTool $installActions "sentrux" { Invoke-SentruxInstall } "Install sentrux or ensure sentrux.exe is on PATH."

$requiredFiles = @(
    "check-code-intel-tools.ps1",
    "invoke-code-intel.ps1",
    "run-code-intel.ps1",
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
Test-Directory $checks "artifactRoot" $artifactRoot $false "The pipeline will create this directory on first run."

Test-Tool $checks "rg" $true "Install ripgrep or ensure rg is on PATH."
Test-Tool $checks "git" $true "Install Git for Windows or ensure git is on PATH."
Test-Tool $checks "python" $true "Install Python 3.11+ or ensure python is on PATH."
Test-Tool $checks "repowise" $true "Install repowise into the active Python environment."
Test-Tool $checks "sentrux" $true "Install sentrux or ensure sentrux.exe is on PATH."

$skillSource = "C:\Users\Administrator\.agents\skills\code-intel-pipeline"
Ensure-SkillLink $checks "source" $skillSource $skillSource $false
Ensure-SkillLink $checks "codex" "C:\Users\Administrator\.codex\skills\code-intel-pipeline" $skillSource $RepairSkillLinks
Ensure-SkillLink $checks "claude" "C:\Users\Administrator\.claude\skills\code-intel-pipeline" $skillSource $RepairSkillLinks

$understandCandidates = @(
    "C:\Users\Administrator\.claude\skills\understand\SKILL.md",
    "C:\Users\Administrator\.agents\skills\understand\SKILL.md",
    "C:\Users\Administrator\.codex\skills\understand\SKILL.md"
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
if (Test-Path -LiteralPath $Config -PathType Leaf) {
    try {
        $configData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
        $configOk = $true
        $repos = $configData.repos.PSObject.Properties.Name
        Add-Check $checks "config:repos" "config" $true ($repos.Count -gt 0) ("repos=" + ($repos -join ",")) "Add at least one repo alias under repos."
    }
    catch {
        Add-Check $checks "config:parse" "config" $true $false $_.Exception.Message "Fix invalid JSON in pipeline config."
    }
}

if (-not [string]::IsNullOrWhiteSpace($Repo)) {
    $doctor = Join-Path $root "check-code-intel-tools.ps1"
    try {
        $doctorRaw = & $doctor -Config $Config -Repo $Repo -Json
        $doctorResult = $doctorRaw | ConvertFrom-Json
        Add-Check $checks "doctor:$Repo" "doctor" $true ([bool]$doctorResult.ok) (($doctorResult.missing -join ",")) "Fix missing doctor checks before running the pipeline."
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
    repairedSkillLinks = [bool]$RepairSkillLinks
    providerChecked = [bool]$CheckProvider
    installMissing = [bool]$InstallMissing
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
