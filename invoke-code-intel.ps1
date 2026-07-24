#requires -Version 7.2

[CmdletBinding()]
param(
    [string]$Repo = "",
    [string]$RepoPath = "",
    [string[]]$Repos = @(),
    [switch]$All,
    [string]$Config = "",
    [ValidateSet("auto", "windows", "macos", "linux")][string]$Platform = "auto",
    [ValidateSet("lite", "normal", "full")][string]$Mode = "normal",
    [switch]$RepowiseDocs,
    [string]$RepowiseProvider = "",
    [string]$RepowiseModel = "",
    [string]$RepowiseReasoning = "",
    [switch]$SaveSentruxBaseline,
    [switch]$AutoSaveMissingSentruxBaseline,
    [switch]$RequireUnderstandGraph,
    [switch]$SkipGitHubResearch,
    [switch]$SkipRepowise,
    [switch]$NoIndexUpdate,
    [switch]$ValidateInstallation,
    [switch]$LegacyCompatibility,
    [ValidateSet("auto", "enabled", "disabled")][string]$ProactiveSkillSuggestions = "auto",
    [ValidateSet("auto", "ask", "enabled", "disabled")][string]$AutomaticPullRequests = "auto",
    [string]$BugSkill = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$launcher = Join-Path $PSScriptRoot "code-intel.ps1"

$supported = @("Repo", "RepoPath", "Config", "Mode", "ValidateInstallation")
$unsupported = @($PSBoundParameters.Keys | Where-Object { $_ -notin $supported })
if ($unsupported.Count -gt 0) {
    [Console]::Error.WriteLine("Code Intel error: unsupported compatibility option: -$($unsupported[0])")
    exit 64
}
if ($ValidateInstallation) {
    & $launcher --help
    exit $LASTEXITCODE
}

$configData = $null
$configPath = $null
if ($PSBoundParameters.ContainsKey("Config") -or
    ([string]::IsNullOrWhiteSpace($RepoPath) -and -not [string]::IsNullOrWhiteSpace($Repo))) {
    $configPath = if ([string]::IsNullOrWhiteSpace($Config)) {
        Join-Path $PSScriptRoot "pipeline.config.json"
    } else {
        $Config
    }
    if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
        [Console]::Error.WriteLine("Code Intel error: config file does not exist: $configPath")
        exit 64
    }
    try {
        $configData = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
    }
    catch {
        [Console]::Error.WriteLine("Code Intel error: config file is not valid JSON: $configPath")
        exit 64
    }
    if ($null -eq $configData -or
        $null -eq $configData.PSObject.Properties["artifactRoot"]) {
        [Console]::Error.WriteLine("Code Intel error: config is missing artifactRoot: $configPath")
        exit 64
    }
}

if ([string]::IsNullOrWhiteSpace($RepoPath) -and -not [string]::IsNullOrWhiteSpace($Repo)) {
    if ($null -eq $configData.PSObject.Properties["repos"]) {
        [Console]::Error.WriteLine("Code Intel error: config is missing repos: $configPath")
        exit 64
    }
    $entry = $configData.repos.PSObject.Properties[$Repo]
    if ($null -eq $entry -or [string]::IsNullOrWhiteSpace([string]$entry.Value.path)) {
        [Console]::Error.WriteLine("Code Intel error: repository alias has no configured path: $Repo")
        exit 64
    }
    $RepoPath = [string]$entry.Value.path
}

$arguments = @{
    RepoPath = $RepoPath
    Mode = $Mode
}
if ($null -ne $configData) {
    $configuredRoot = [string]$configData.artifactRoot
    if (-not [string]::IsNullOrWhiteSpace($configuredRoot)) {
        $arguments.Remaining = @("--artifact-root", $configuredRoot)
    }
}
& $launcher @arguments
exit $LASTEXITCODE
