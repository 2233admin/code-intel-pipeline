#requires -Version 7.2

param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,

    [ValidateSet("lite", "normal", "full")]
    [string]$Mode = "normal",

    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto",

    [switch]$CheckProvider,
    [switch]$SkipSmoke,
    [switch]$NoInstallMissing,
    [switch]$NoRepairSkillLinks,
    [switch]$RequireRepowise,
    [switch]$RequireUnderstand,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$platformModule = Join-Path (Join-Path $PSScriptRoot "tools") "code-intel-platform.psm1"
Import-Module $platformModule -Force
$effectivePlatform = Get-CodeIntelPlatform -Platform $Platform

function Get-BootstrapRoot {
    return (Join-Path (Get-CodeIntelDataRoot -Platform $effectivePlatform) "bootstrap")
}

function Invoke-JsonScript {
    param(
        [string]$Script,
        [hashtable]$Params
    )

    try {
        $raw = & $Script @Params 2>&1
    }
    catch {
        return [pscustomobject][ordered]@{
            ok = $false
            raw = $_.Exception.Message
            parseError = ""
        }
    }
    $text = ($raw | ForEach-Object { $_.ToString() } | Out-String).Trim()
    try {
        return $text | ConvertFrom-Json
    }
    catch {
        return [pscustomobject][ordered]@{
            ok = $false
            raw = $text
            parseError = $_.Exception.Message
        }
    }
}

$root = Split-Path -Parent $PSCommandPath
$repo = (Get-Item -LiteralPath $RepoPath -ErrorAction Stop).FullName
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$bootstrapRoot = Get-BootstrapRoot
New-Item -ItemType Directory -Force -Path $bootstrapRoot | Out-Null
$jsonPath = Join-Path $bootstrapRoot "bootstrap-$stamp.json"
$mdPath = Join-Path $bootstrapRoot "bootstrap-$stamp.md"

$installParams = @{
    RepoPath = $repo
    Platform = $effectivePlatform
    Json = $true
    RequireRepowise = [bool]$RequireRepowise
}
if (-not $NoInstallMissing) {
    $installParams.InstallMissing = $true
    $installParams.AuditInstallPlan = $true
}
if (-not $NoRepairSkillLinks) { $installParams.RepairSkillLinks = $true }
if ($CheckProvider) { $installParams.CheckProvider = $true }
if ($RequireUnderstand) { $installParams.RequireUnderstand = $true }

$doctorParams = @{
    RepoPath = $repo
    Platform = $effectivePlatform
    Json = $true
    RequireRepowise = [bool]$RequireRepowise
}
if ($RequireUnderstand) { $doctorParams.RequireUnderstand = $true }

$installResult = Invoke-JsonScript (Join-Path $root "install-code-intel-pipeline.ps1") $installParams
$doctorResult = Invoke-JsonScript (Join-Path $root "check-code-intel-tools.ps1") $doctorParams
$smokeResult = $null
if (-not $SkipSmoke) {
    $smokeParams = @{
        RepoPath = $repo
        Mode = $Mode
        Platform = $effectivePlatform
        SkipRepowise = $true
    }
    $smokeResult = Invoke-JsonScript (Join-Path $root "scripts/tests/test-code-intel-pipeline.ps1") $smokeParams
}

$ok = [bool]$installResult.ok -and [bool]$doctorResult.ok -and ($SkipSmoke -or [bool]$smokeResult.ok)
$result = [ordered]@{
    ok = $ok
    repo = $repo
    mode = $Mode
    generatedAt = (Get-Date).ToUniversalTime().ToString("o")
    root = $root
    install = $installResult
    doctor = $doctorResult
    smoke = $smokeResult
    reports = [ordered]@{
        json = $jsonPath
        markdown = $mdPath
    }
    nextAction = if ($ok) {
        "Run invoke-code-intel.ps1 -RepoPath $repo -Mode $Mode for normal use."
    }
    elseif (-not [bool]$installResult.ok) {
        "Fix install.missingRequired first."
    }
    elseif (-not [bool]$doctorResult.ok) {
        "Fix doctor.missing first."
    }
    else {
        "Open smoke.report or smoke.summary and fix the failed pipeline step."
    }
}

$result | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $jsonPath -Encoding UTF8

$md = @(
    "# Code Intel Bootstrap",
    "",
    "- Repo: $repo",
    "- Mode: $Mode",
    "- OK: $ok",
    "- Install OK: $([bool]$installResult.ok)",
    "- Doctor OK: $([bool]$doctorResult.ok)",
    "- Smoke OK: $(if ($SkipSmoke) { 'skipped' } else { [bool]$smokeResult.ok })",
    "- JSON: $jsonPath",
    "- Next: $($result.nextAction)",
    "",
    "## Missing",
    "- Install: $(if ($installResult.missingRequired) { (@($installResult.missingRequired) | ForEach-Object { $_.name }) -join ', ' } else { 'none' })",
    "- Doctor: $(if ($doctorResult.missing) { (@($doctorResult.missing) -join ', ') } else { 'none' })",
    "",
    "## Smoke",
    "- Artifact: $(if ($smokeResult -and $smokeResult.artifactDir) { $smokeResult.artifactDir } else { 'none' })",
    "- Summary: $(if ($smokeResult -and $smokeResult.summary) { $smokeResult.summary } else { 'none' })",
    "- CodeNexus: $(if ($smokeResult -and $smokeResult.codeNexusContext) { $smokeResult.codeNexusContext.path } else { 'none' })"
)
$md | Set-Content -LiteralPath $mdPath -Encoding UTF8

if ($Json) {
    $result | ConvertTo-Json -Depth 12
}
else {
    Write-Host "Code intel bootstrap: $(if ($ok) { 'OK' } else { 'FAILED' })"
    Write-Host "Repo: $repo"
    Write-Host "Report: $jsonPath"
    Write-Host "Summary: $mdPath"
    Write-Host "Next: $($result.nextAction)"
}

if (-not $ok) {
    exit 1
}
exit 0
