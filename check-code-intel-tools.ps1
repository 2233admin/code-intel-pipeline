param(
    [string]$Config = "",
    [string]$Repo = "",
    [string]$RepoPath = "",
    [switch]$RequireRepowise,
    [switch]$RequireUnderstand,
    [switch]$Json
)

if (-not $PSBoundParameters.ContainsKey("RequireRepowise")) {
    $RequireRepowise = $true
}

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

function Test-CommandOutput {
    param(
        [string]$Name,
        [scriptblock]$Body,
        [string]$ExpectedPattern
    )

    try {
        $global:LASTEXITCODE = 0
        $output = & $Body 2>&1
        $text = ($output | ForEach-Object { $_.ToString() } | Out-String).Trim()
        [pscustomobject][ordered]@{
            name = $Name
            found = ($global:LASTEXITCODE -eq 0 -and $text -match $ExpectedPattern)
            output = $text
        }
    }
    catch {
        [pscustomobject][ordered]@{
            name = $Name
            found = $false
            output = $_.Exception.Message
        }
    }
}

$configData = $null
if ([string]::IsNullOrWhiteSpace($Config)) {
    $Config = Join-Path $PSScriptRoot "pipeline.config.json"
}
if (Test-Path -LiteralPath $Config -PathType Leaf) {
    $configData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
}

$pipelineRoot = Split-Path -Parent $PSCommandPath
$pipelineScript = Join-Path $pipelineRoot "run-code-intel.ps1"
$codeIntelCargo = Join-Path $pipelineRoot "crates\code-intel-cli\Cargo.toml"
$codeIntelGraphSource = Join-Path $pipelineRoot "crates\code-intel-cli\src\graph.rs"
$codeIntelGraphBinary = Join-Path $pipelineRoot "target\debug\code-intel.exe"
$repoConfig = Resolve-RepoConfig $Repo $configData
$repoInput = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) { $RepoPath } else { $Repo }
$repoPath = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
    if (Test-Path -LiteralPath $RepoPath -PathType Container) { (Get-Item -LiteralPath $RepoPath).FullName } else { $RepoPath }
}
else {
    Resolve-RepoPath $Repo $configData
}
$sentruxScope = Resolve-SentruxScope $repoPath $repoConfig

$userProfile = if ([string]::IsNullOrWhiteSpace($env:USERPROFILE)) { $HOME } else { $env:USERPROFILE }
$understandSkillCandidates = @(
    (Join-Path $userProfile ".claude\skills\understand\SKILL.md"),
    (Join-Path $userProfile ".agents\skills\understand\SKILL.md"),
    (Join-Path $userProfile ".codex\skills\understand\SKILL.md")
)
$repoParentForCandidates = if (-not [string]::IsNullOrWhiteSpace([string]$repoPath)) { Split-Path -Parent $repoPath } else { $pipelineRoot }
$understandPluginCandidates = @(
    (Join-Path $userProfile ".claude\plugins\cache\understand-anything"),
    (Join-Path $userProfile ".understand-anything-plugin"),
    (Join-Path $repoParentForCandidates "Understand-Anything")
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
    Test-Tool "repowise" ([bool]$RequireRepowise)
    Test-Tool "repomix" $false
    Test-Tool "sentrux" $true
)
$sentruxCore = Test-CommandOutput "sentrux-core" { sentrux check --help } "Enforce architectural rules"
$sentruxPro = Test-CommandOutput "sentrux-pro" { sentrux pro status } "Tier:\s+pro"

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
    sentrux = [ordered]@{
        core = $sentruxCore
        pro = $sentruxPro
    }
    understandAnything = [ordered]@{
        skillFound = [bool]$understandSkill
        skillPath = if ($understandSkill) { [string]$understandSkill } else { "" }
        pluginFound = [bool]$understandPlugin
        pluginPath = if ($understandPlugin) { [string]$understandPlugin } else { "" }
    }
    graphProvider = [ordered]@{
        sourceFound = (Test-Path -LiteralPath $codeIntelGraphSource -PathType Leaf)
        cargoFound = (Test-Path -LiteralPath $codeIntelCargo -PathType Leaf)
        binaryFound = (Test-Path -LiteralPath $codeIntelGraphBinary -PathType Leaf)
        command = "$codeIntelGraphBinary graph --repo <repo-path> --language zh --write --json"
    }
    repo = $repoState
}

$missing = New-Object System.Collections.Generic.List[string]
if (-not $checks.pipelineScript.found) { $missing.Add("pipeline script") }
if (-not $checks.config.found) { $missing.Add("pipeline config") }
foreach ($tool in $tools) {
    if ($tool.required -and -not $tool.found) { $missing.Add($tool.name) }
}
if (-not $sentruxCore.found) { $missing.Add("sentrux core") }
if (-not $sentruxPro.found) { $missing.Add("sentrux pro auto-activation") }
if ($RequireUnderstand -and -not $checks.graphProvider.sourceFound) { $missing.Add("internal graph provider source") }
if ($RequireUnderstand -and -not $checks.graphProvider.cargoFound) { $missing.Add("code-intel Rust runtime") }
if ($repoState -and -not $repoState.exists) { $missing.Add("repo path") }

$result = [ordered]@{
    ok = $missing.Count -eq 0
    missing = $missing
    checks = $checks
    strict = [ordered]@{
        requireRepowise = [bool]$RequireRepowise
        requireUnderstand = [bool]$RequireUnderstand
    }
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
    $coreMark = if ($sentruxCore.found) { "OK" } else { "MISSING" }
    $proMark = if ($sentruxPro.found) { "OK" } else { "MISSING" }
    Write-Host "$coreMark sentrux-core $($sentruxCore.output)"
    Write-Host "$proMark sentrux-pro $($sentruxPro.output)"
    $uaMark = if ($checks.understandAnything.skillFound -and $checks.understandAnything.pluginFound) { "OK" } else { "MISSING" }
    $graphMark = if ($checks.graphProvider.sourceFound -and $checks.graphProvider.cargoFound) { "OK" } else { "MISSING" }
    Write-Host "$graphMark internal graph provider source=$($checks.graphProvider.sourceFound) cargo=$($checks.graphProvider.cargoFound) binary=$($checks.graphProvider.binaryFound)"
    Write-Host "$uaMark external Understand fallback skill=$($checks.understandAnything.skillPath) plugin=$($checks.understandAnything.pluginPath)"
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

exit 0
