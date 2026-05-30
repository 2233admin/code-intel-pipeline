param(
    [string]$Repo = "",
    [string]$RepoPath = "",

    [string]$Config = "",

    [ValidateSet("lite", "normal", "full")]
    [string]$Mode = "normal",

    [string]$SentruxPath = "",

    [switch]$SkipRepowise,
    [switch]$RepowiseDocs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Read-JsonFile {
    param([string]$Path)
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

$root = Split-Path -Parent $PSCommandPath
if ([string]::IsNullOrWhiteSpace($Config)) {
    $Config = Join-Path $root "pipeline.config.json"
}
$doctor = Join-Path $root "check-code-intel-tools.ps1"
$runner = Join-Path $root "run-code-intel.ps1"
$sentruxAgentTool = Join-Path $root "Invoke-SentruxAgentTool.ps1"

$label = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) { $RepoPath } else { $Repo }
if ([string]::IsNullOrWhiteSpace($label)) {
    throw "Specify -Repo <alias-or-path> or -RepoPath <path>."
}

$doctorJson = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
    & $doctor -Config $Config -RepoPath $RepoPath -Json | ConvertFrom-Json
}
else {
    & $doctor -Config $Config -Repo $Repo -Json | ConvertFrom-Json
}
if (-not $doctorJson.ok) {
    throw "Doctor failed: $($doctorJson.missing -join ', ')"
}

$runnerParams = @{
    Config = $Config
    Mode = $Mode
}
if (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
    $runnerParams.RepoPath = $RepoPath
}
else {
    $runnerParams.Repo = $Repo
}
if (-not [string]::IsNullOrWhiteSpace($SentruxPath)) {
    $runnerParams.SentruxPath = $SentruxPath
}
if ($SkipRepowise) {
    $runnerParams.SkipRepowise = $true
}
if ($RepowiseDocs) {
    $runnerParams.RepowiseDocs = $true
}
& $runner @runnerParams
if ($LASTEXITCODE -ne 0) {
    throw "Pipeline run failed for repo: $label"
}

$repoName = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) { Split-Path -Leaf (Get-Item -LiteralPath $RepoPath).FullName } else { $Repo }
$artifactRoot = if ($doctorJson.checks -and $doctorJson.checks.config -and (Test-Path -LiteralPath $Config -PathType Leaf)) {
    $configData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
    if ($configData.PSObject.Properties["artifactRoot"] -and -not [string]::IsNullOrWhiteSpace([string]$configData.artifactRoot)) { [string]$configData.artifactRoot } else { "" }
}
else { "" }
if ([string]::IsNullOrWhiteSpace($artifactRoot)) {
    $base = if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { $env:LOCALAPPDATA } else { (Join-Path $HOME ".code-intel") }
    $artifactRoot = Join-Path $base "code-intel\artifacts"
}

$artifactDir = Get-ChildItem -Path (Join-Path $artifactRoot $repoName) -Directory |
    Sort-Object Name -Descending |
    Select-Object -First 1

if ($null -eq $artifactDir) {
    throw "No artifact directory found for repo: $Repo"
}

$reportPath = Join-Path $artifactDir.FullName "report.json"
$summaryPath = Join-Path $artifactDir.FullName "summary.md"
$understandingPath = Join-Path $artifactDir.FullName "understanding.md"
if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
    throw "Missing report.json: $reportPath"
}
if (-not (Test-Path -LiteralPath $summaryPath -PathType Leaf)) {
    throw "Missing summary.md: $summaryPath"
}
if (-not (Test-Path -LiteralPath $understandingPath -PathType Leaf)) {
    throw "Missing understanding.md: $understandingPath"
}

$report = Read-JsonFile $reportPath
$requiredCategories = @("providerQuota", "localToolError", "graphMissing", "sentruxFail")
$missingCategories = @()
foreach ($key in $requiredCategories) {
    if ($null -eq $report.summary.failureCategories.$key) {
        $missingCategories += $key
    }
}
if ($missingCategories.Count -gt 0) {
    throw "Missing failure category counters: $($missingCategories -join ', ')"
}
if ($null -eq $report.sentruxInsight) {
    throw "Missing sentruxInsight in report.json"
}
if ($null -eq $report.sentruxDsm) {
    throw "Missing sentruxDsm in report.json"
}
if ([string]::IsNullOrWhiteSpace([string]$report.sentruxDsm.path) -or -not (Test-Path -LiteralPath ([string]$report.sentruxDsm.path) -PathType Leaf)) {
    throw "Missing sentrux-dsm.json artifact."
}
if ([int]$report.sentruxDsm.colorModes -ne 9) {
    throw "sentrux-dsm.json artifact did not report 9 color modes."
}
if ($null -eq $report.sentruxFileDetails) {
    throw "Missing sentruxFileDetails in report.json"
}
if ([string]::IsNullOrWhiteSpace([string]$report.sentruxFileDetails.path) -or -not (Test-Path -LiteralPath ([string]$report.sentruxFileDetails.path) -PathType Leaf)) {
    throw "Missing sentrux-file-details.json artifact."
}
if ([int]$report.sentruxFileDetails.functions -lt 1) {
    throw "sentrux-file-details.json artifact did not report any functions."
}
if ($null -eq $report.sentruxHotspots) {
    throw "Missing sentruxHotspots in report.json"
}
if ([string]::IsNullOrWhiteSpace([string]$report.sentruxHotspots.path) -or -not (Test-Path -LiteralPath ([string]$report.sentruxHotspots.path) -PathType Leaf)) {
    throw "Missing sentrux-hotspots.json artifact."
}
if ([int]$report.sentruxHotspots.functions -lt 1) {
    throw "sentrux-hotspots.json artifact did not report any function hotspots."
}
if ($null -eq $report.sentruxEvolution) {
    throw "Missing sentruxEvolution in report.json"
}
if ([string]::IsNullOrWhiteSpace([string]$report.sentruxEvolution.path) -or -not (Test-Path -LiteralPath ([string]$report.sentruxEvolution.path) -PathType Leaf)) {
    throw "Missing sentrux-evolution.json artifact."
}
$evolutionArtifact = Read-JsonFile ([string]$report.sentruxEvolution.path)
if ($null -eq $evolutionArtifact.hotspots -or $null -eq $evolutionArtifact.coupling -or $null -eq $evolutionArtifact.bus_factor) {
    throw "sentrux-evolution.json artifact is missing hotspots, coupling, or bus_factor details."
}
if ($null -eq $report.sentruxWhatIf) {
    throw "Missing sentruxWhatIf in report.json"
}
if ([string]::IsNullOrWhiteSpace([string]$report.sentruxWhatIf.path) -or -not (Test-Path -LiteralPath ([string]$report.sentruxWhatIf.path) -PathType Leaf)) {
    throw "Missing sentrux-what-if.json artifact."
}
$whatIfArtifact = Read-JsonFile ([string]$report.sentruxWhatIf.path)
if ($null -eq $whatIfArtifact.scenarios -or $whatIfArtifact.scenarios.Count -lt 1) {
    throw "sentrux-what-if.json artifact did not report any scenarios."
}
if ($null -eq $whatIfArtifact.summary -or $null -eq $whatIfArtifact.recommendations) {
    throw "sentrux-what-if.json artifact is missing summary or recommendations."
}
if ($null -eq $report.codeNexusContext) {
    throw "Missing codeNexusContext in report.json"
}
if ([string]::IsNullOrWhiteSpace([string]$report.codeNexusContext.path) -or -not (Test-Path -LiteralPath ([string]$report.codeNexusContext.path) -PathType Leaf)) {
    throw "Missing codenexus-context.json artifact."
}
$codeNexusArtifact = Read-JsonFile ([string]$report.codeNexusContext.path)
if ($null -eq $codeNexusArtifact.summary -or $null -eq $codeNexusArtifact.files) {
    throw "codenexus-context.json artifact is missing summary or files."
}

$sentruxAgentHealth = $null
$sentruxAgentDsm = $null
$sentruxAgentGitStats = $null
$sentruxTarget = $null
$sentruxPathProperty = $report.PSObject.Properties["sentruxPath"]
if ($null -ne $sentruxPathProperty) {
    $sentruxTarget = [string]$sentruxPathProperty.Value
}
if (-not [string]::IsNullOrWhiteSpace($sentruxTarget) -and (Test-Path -LiteralPath $sentruxAgentTool -PathType Leaf)) {
    $sentruxAgentHealth = & $sentruxAgentTool health $sentruxTarget | ConvertFrom-Json
    if ($null -eq $sentruxAgentHealth -or [string]::IsNullOrWhiteSpace([string]$sentruxAgentHealth.status)) {
        throw "Sentrux Agent health wrapper did not return a status."
    }
    $dsm = & $sentruxAgentTool dsm $sentruxTarget | ConvertFrom-Json
    if ($null -eq $dsm -or $dsm.color_modes.Count -ne 9) {
        throw "Sentrux Agent DSM wrapper did not return 9 color modes."
    }
    $gitStats = & $sentruxAgentTool sentrux_git_stats $sentruxTarget | ConvertFrom-Json
    if ($null -eq $gitStats -or $null -eq $gitStats.summary -or $null -eq $gitStats.hotspots) {
        throw "Sentrux Agent git_stats wrapper did not return summary and hotspots."
    }
    $sentruxAgentDsm = [ordered]@{
        defaultColorMode = $dsm.default_color_mode
        colorModes = $dsm.color_modes.Count
        modules = $dsm.modules.Count
        files = $dsm.file_details.Count
        functions = [int](($dsm.file_details | Measure-Object -Property function_count -Sum).Sum)
    }
    $sentruxAgentGitStats = [ordered]@{
        files = [int]$gitStats.summary.files
        dirtyFiles = [int]$gitStats.summary.dirty_files
        untrackedFiles = [int]$gitStats.summary.untracked_files
        totalChurn = [int]$gitStats.summary.total_churn
        authors = [int]$gitStats.summary.authors
    }
}

$result = [ordered]@{
    ok = $true
    repo = $label
    mode = $Mode
    artifactDir = $artifactDir.FullName
    report = $reportPath
    summary = $summaryPath
    understanding = $understandingPath
    steps = $report.steps.Count
    failed = $report.summary.failed
    manualRequired = $report.summary.manualRequired
    failureCategories = $report.summary.failureCategories
    sentruxAgentHealth = $sentruxAgentHealth
    sentruxAgentDsm = $sentruxAgentDsm
    sentruxAgentGitStats = $sentruxAgentGitStats
    codeNexusContext = [ordered]@{
        path = [string]$report.codeNexusContext.path
        files = [int]$report.codeNexusContext.files
        references = [int]$report.codeNexusContext.references
        recentCommits = [int]$report.codeNexusContext.recentCommits
    }
}

$result | ConvertTo-Json -Depth 6
