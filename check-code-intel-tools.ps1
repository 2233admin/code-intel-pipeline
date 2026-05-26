param(
    [string]$Config = "D:\projects\_tools\code-intel-pipeline\pipeline.config.json",
    [string]$Repo = "",
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-JsonProperty {
    param(
        [object]$Object,
        [string]$Name
    )

    if ($null -eq $Object) { return $null }
    $prop = $Object.PSObject.Properties[$Name]
    if ($null -eq $prop) { return $null }
    return $prop.Value
}

function Resolve-RepoPath {
    param(
        [string]$RepoInput,
        [object]$ConfigData
    )

    if ([string]::IsNullOrWhiteSpace($RepoInput)) { return $null }

    $repoConfig = Resolve-RepoConfig $RepoInput $ConfigData

    $path = $RepoInput
    if ($null -ne $repoConfig) {
        $configuredPath = Get-JsonProperty $repoConfig "path"
        if (-not [string]::IsNullOrWhiteSpace([string]$configuredPath)) {
            $path = [string]$configuredPath
        }
    }

    if (Test-Path -LiteralPath $path -PathType Container) {
        return (Get-Item -LiteralPath $path).FullName
    }

    return $path
}

function Resolve-RepoConfig {
    param(
        [string]$RepoInput,
        [object]$ConfigData
    )

    if ([string]::IsNullOrWhiteSpace($RepoInput)) { return $null }
    $reposConfig = Get-JsonProperty $ConfigData "repos"
    if ($null -eq $reposConfig) { return $null }
    return Get-JsonProperty $reposConfig $RepoInput
}

function Resolve-SentruxScope {
    param(
        [string]$RepoPath,
        [object]$RepoConfig
    )

    if ([string]::IsNullOrWhiteSpace($RepoPath)) { return $RepoPath }
    $configuredScope = Get-JsonProperty $RepoConfig "sentruxPath"
    if ([string]::IsNullOrWhiteSpace([string]$configuredScope)) { return $RepoPath }

    if ([System.IO.Path]::IsPathRooted([string]$configuredScope)) {
        $scope = [string]$configuredScope
    }
    else {
        $scope = Join-Path $RepoPath ([string]$configuredScope)
    }

    if (Test-Path -LiteralPath $scope -PathType Container) {
        return (Get-Item -LiteralPath $scope).FullName
    }

    return $scope
}

function Test-Tool {
    param(
        [string]$Name,
        [bool]$Required = $true
    )

    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    [pscustomobject][ordered]@{
        name = $Name
        required = $Required
        found = [bool]$cmd
        source = if ($cmd) { $cmd.Source } else { "" }
    }
}

$configData = $null
if (Test-Path -LiteralPath $Config -PathType Leaf) {
    $configData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
}

$pipelineRoot = Split-Path -Parent $PSCommandPath
$pipelineScript = Join-Path $pipelineRoot "run-code-intel.ps1"
$repoConfig = Resolve-RepoConfig $Repo $configData
$repoPath = Resolve-RepoPath $Repo $configData
$sentruxScope = Resolve-SentruxScope $repoPath $repoConfig

$understandSkillCandidates = @(
    "C:\Users\Administrator\.claude\skills\understand\SKILL.md",
    "C:\Users\Administrator\.agents\skills\understand\SKILL.md",
    "C:\Users\Administrator\.codex\skills\understand\SKILL.md"
)
$understandPluginCandidates = @(
    "C:\Users\Administrator\.claude\plugins\cache\understand-anything",
    "C:\Users\Administrator\.understand-anything-plugin",
    "D:\projects\Understand-Anything"
)

$understandSkill = $understandSkillCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
$understandPlugin = $understandPluginCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Container } | Select-Object -First 1

$repoState = $null
if (-not [string]::IsNullOrWhiteSpace([string]$repoPath) -and (Test-Path -LiteralPath $repoPath -PathType Container)) {
    $knowledgeGraph = Join-Path $repoPath ".understand-anything\knowledge-graph.json"
    $repowiseDir = Join-Path $repoPath ".repowise"
    $sentruxDir = Join-Path $sentruxScope ".sentrux"
    $repoState = [ordered]@{
        path = $repoPath
        exists = $true
        isGitRepo = Test-Path -LiteralPath (Join-Path $repoPath ".git")
        understandGraph = Test-Path -LiteralPath $knowledgeGraph -PathType Leaf
        repowiseState = Test-Path -LiteralPath $repowiseDir -PathType Container
        sentruxScope = $sentruxScope
        sentruxRules = Test-Path -LiteralPath (Join-Path $sentruxDir "rules.toml") -PathType Leaf
        sentruxBaseline = Test-Path -LiteralPath (Join-Path $sentruxDir "baseline.json") -PathType Leaf
    }
}
elseif (-not [string]::IsNullOrWhiteSpace([string]$repoPath)) {
    $repoState = [ordered]@{
        path = $repoPath
        exists = $false
    }
}

$tools = @(
    Test-Tool "rg" $true
    Test-Tool "git" $true
    Test-Tool "repowise" $true
    Test-Tool "sentrux" $true
)

$checks = [ordered]@{
    pipelineScript = [ordered]@{
        path = $pipelineScript
        found = Test-Path -LiteralPath $pipelineScript -PathType Leaf
    }
    config = [ordered]@{
        path = $Config
        found = Test-Path -LiteralPath $Config -PathType Leaf
    }
    tools = $tools
    understandAnything = [ordered]@{
        skillFound = [bool]$understandSkill
        skillPath = if ($understandSkill) { [string]$understandSkill } else { "" }
        pluginFound = [bool]$understandPlugin
        pluginPath = if ($understandPlugin) { [string]$understandPlugin } else { "" }
    }
    repo = $repoState
}

$missing = New-Object System.Collections.Generic.List[string]
if (-not $checks.pipelineScript.found) { $missing.Add("pipeline script") }
if (-not $checks.config.found) { $missing.Add("pipeline config") }
foreach ($tool in $tools) {
    if ($tool.required -and -not $tool.found) { $missing.Add($tool.name) }
}
if (-not $checks.understandAnything.skillFound) { $missing.Add("Understand Anything skill") }
if (-not $checks.understandAnything.pluginFound) { $missing.Add("Understand Anything plugin") }
if ($repoState -and -not $repoState.exists) { $missing.Add("repo path") }

$result = [ordered]@{
    ok = $missing.Count -eq 0
    missing = $missing
    checks = $checks
}

if ($Json) {
    $result | ConvertTo-Json -Depth 8
}
else {
    if ($result.ok) {
        Write-Host "Code intel doctor: OK"
    }
    else {
        Write-Host "Code intel doctor: missing $($missing -join ', ')"
    }

    Write-Host "Pipeline: $pipelineScript"
    Write-Host "Config: $Config"
    foreach ($tool in $tools) {
        $mark = if ($tool.found) { "OK" } else { "MISSING" }
        Write-Host "$mark $($tool.name) $($tool.source)"
    }
    $uaMark = if ($checks.understandAnything.skillFound -and $checks.understandAnything.pluginFound) { "OK" } else { "MISSING" }
    Write-Host "$uaMark Understand Anything skill=$($checks.understandAnything.skillPath) plugin=$($checks.understandAnything.pluginPath)"
    if ($repoState) {
        Write-Host "Repo: $($repoState.path)"
        Write-Host "Repo exists: $($repoState.exists)"
        if ($repoState.exists) {
            Write-Host "Understand graph: $($repoState.understandGraph)"
            Write-Host "Repowise state: $($repoState.repowiseState)"
            Write-Host "Sentrux scope: $($repoState.sentruxScope)"
            Write-Host "Sentrux rules: $($repoState.sentruxRules)"
            Write-Host "Sentrux baseline: $($repoState.sentruxBaseline)"
        }
    }
}

if (-not $result.ok) {
    exit 1
}
