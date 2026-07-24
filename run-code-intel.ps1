#requires -Version 7.2

param(
    [string]$Repo = "",
    [string]$RepoPath = "",

    [string]$Config = "",

    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto",

    [ValidateSet("lite", "normal", "full")]
    [string]$Mode = "normal",

    [string]$Language = "",

    [string]$ArtifactRoot = "",
    [string]$SentruxPath = "",
    [string]$RepowiseWorkspaceRoot = "",
    [string]$RepowiseShadowRoot = "",
    [string[]]$RepowiseScopePaths = @(),
    [string[]]$RepowiseRootFiles = @(),
    [int]$RepowiseTimeoutSeconds = 600,
    [string]$RepowiseProvider = "",
    [string]$RepowiseModel = "",
    [string]$RepowiseReasoning = "",
    [string]$ModelRoutingResult = "",
    [string]$ModelInventoryResult = "",
    [string]$ModelExecutableHandle = "",
    [string]$ModelPromptFile = "",
    [string]$ModelEndpoint = "",
    [ValidateSet("", "openai", "anthropic", "ollama")]
    [string]$ModelProtocol = "",
    [string]$ModelCredentialEnvName = "",
    [ValidateRange(1, 3600)]
    [int]$ModelTimeoutSeconds = 300,
    [ValidateSet("json", "jsonl")]
    [string]$ModelResponseFormat = "json",
    [string]$ModelAdapterRequest = "",
    [string]$ModelAdapterArtifactRoot = "",
    [string]$RuntimeCiEvidenceRequest = "",
    [string]$RuntimeCiEvidenceArtifactRoot = "",
    [string]$RepowiseAdapterRequest = "",
    [string]$RepowiseAdapterArtifactRoot = "",
    [long]$RepowiseAdapterEvaluatedAt = 0,
    [long]$RepowiseAdapterMaxAgeSeconds = 0,
    [string]$GraphAdapterRequest = "",
    [string]$GraphAdapterArtifactRoot = "",
    [long]$GraphAdapterEvaluatedAt = 0,
    [long]$GraphAdapterMaxAgeSeconds = 0,
    [string]$SentruxAdapterRequest = "",
    [string]$SentruxAdapterArtifactRoot = "",
    [long]$SentruxAdapterEvaluatedAt = 0,
    [long]$SentruxAdapterMaxAgeSeconds = 0,
    [string]$CodeNexusAdapterRequest = "",
    [string]$CodeNexusAdapterArtifactRoot = "",
    [long]$CodeNexusAdapterEvaluatedAt = 0,
    [long]$CodeNexusAdapterMaxAgeSeconds = 0,
    [string]$SurvivalScanRequest = "",
    [string]$SurvivalScanArtifactRoot = "",
    [string]$RunCommitSourceRoot = "",
    [string]$RunCommitAuthorityRoot = "",
    [string]$RunCommitManifestRef = "",
    [string]$RunCommitFinalName = "",
    [string[]]$InventoryExclude = @(),

    [switch]$DagCoordinate,

    [switch]$SaveSentruxBaseline,
    [switch]$AutoSaveMissingSentruxBaseline,
    [switch]$SkipRepowise,
    [switch]$RepowiseDocs,
    [switch]$AllowRepowiseShadowMutation,
    [switch]$SkipRepomix,
    [ValidateSet("xml", "markdown", "json", "plain")]
    [string]$RepomixStyle = "markdown",
    [switch]$RepomixCompress,
    [switch]$SkipSentrux,
[switch]$SkipSentruxCheck,
[switch]$SkipSentruxGate,
[switch]$RequireUnderstandGraph,
[switch]$SkipGitHubResearch,
[switch]$WorkspaceAdd,
[switch]$SkipOpenSpec,
[switch]$AutoOpenSpec,
[ValidateSet("auto", "enabled", "disabled")]
[string]$ProactiveSkillSuggestions = "auto",
[ValidateSet("auto", "ask", "enabled", "disabled")]
[string]$AutomaticPullRequests = "auto",
[string]$BugSkill = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$platformModule = Join-Path (Join-Path $PSScriptRoot "tools") "code-intel-platform.psm1"
Import-Module $platformModule -Force
$followUpAutomationModule = Join-Path (Join-Path $PSScriptRoot "tools") "code-intel-follow-up-automation.psm1"
Import-Module $followUpAutomationModule -Force
$effectivePlatform = Get-CodeIntelPlatform -Platform $Platform
$codeIntelPaths = Get-CodeIntelPaths -Platform $effectivePlatform -Root $PSScriptRoot
$rustExecutableName = if ($effectivePlatform -eq "windows") { "code-intel.exe" } else { "code-intel" }
$defaultRustCli = Join-Path $PSScriptRoot (Join-Path "target/debug" $rustExecutableName)

[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$OutputEncoding = [System.Text.UTF8Encoding]::new()
$env:PYTHONIOENCODING = "utf-8"
$env:PYTHONUTF8 = "1"
$env:TERM = "xterm"
$env:NO_COLOR = "1"
$env:RICH_FORCE_TERMINAL = "0"

if (-not [string]::IsNullOrWhiteSpace($ModelInventoryResult)) {
    if ([string]::IsNullOrWhiteSpace($ModelRoutingResult) -or
        [string]::IsNullOrWhiteSpace($ModelPromptFile) -or
        [string]::IsNullOrWhiteSpace($ModelAdapterArtifactRoot)) {
        throw "Model request synthesis requires inventory, routing, prompt, and adapter artifact root"
    }
    $synthesisScript = Join-Path $PSScriptRoot "New-ModelAdapterRequest.ps1"
    $delegateScript = Join-Path $PSScriptRoot "Invoke-ModelChannelDelegate.ps1"
    if (-not (Test-Path -LiteralPath $synthesisScript -PathType Leaf) -or -not (Test-Path -LiteralPath $delegateScript -PathType Leaf)) {
        throw "Model request synthesis or delegate implementation is missing"
    }
    New-Item -ItemType Directory -Force -Path $ModelAdapterArtifactRoot | Out-Null
    $synthesizedRequest = Join-Path ([IO.Path]::GetFullPath($ModelAdapterArtifactRoot)) "model-adapter-request.v2.json"
    $synthesisParameters = @{
        Inventory = $ModelInventoryResult
        Routing = $ModelRoutingResult
        PromptFile = $ModelPromptFile
        OutputPath = $synthesizedRequest
        TimeoutSeconds = $ModelTimeoutSeconds
        ResponseFormat = $ModelResponseFormat
    }
    if (-not [string]::IsNullOrWhiteSpace($ModelExecutableHandle)) { $synthesisParameters.ExecutableHandle = $ModelExecutableHandle }
    if (-not [string]::IsNullOrWhiteSpace($ModelEndpoint)) { $synthesisParameters.Endpoint = $ModelEndpoint }
    if (-not [string]::IsNullOrWhiteSpace($ModelProtocol)) { $synthesisParameters.Protocol = $ModelProtocol }
    if (-not [string]::IsNullOrWhiteSpace($ModelCredentialEnvName)) { $synthesisParameters.CredentialEnvName = $ModelCredentialEnvName }
    & $synthesisScript @synthesisParameters | Out-Null
    & $delegateScript -Request $synthesizedRequest -ArtifactRoot $ModelAdapterArtifactRoot
    exit $LASTEXITCODE
}

if (-not [string]::IsNullOrWhiteSpace($ModelAdapterRequest)) {
    if ([string]::IsNullOrWhiteSpace($ModelAdapterArtifactRoot)) { throw "Model adapter facade requires an artifact root" }
    $delegateScript = Join-Path $PSScriptRoot "Invoke-ModelChannelDelegate.ps1"
    if (-not (Test-Path -LiteralPath $delegateScript -PathType Leaf)) { throw "Model channel delegate is missing: $delegateScript" }
    & $delegateScript -Request $ModelAdapterRequest -ArtifactRoot $ModelAdapterArtifactRoot
    exit $LASTEXITCODE
}

if (-not [string]::IsNullOrWhiteSpace($RepowiseAdapterRequest)) {
    if ([string]::IsNullOrWhiteSpace($RepowiseAdapterArtifactRoot) -or
        $RepowiseAdapterEvaluatedAt -lt 0 -or
        $RepowiseAdapterMaxAgeSeconds -le 0) {
        throw "Repowise adapter facade requires artifact root, non-negative evaluated-at, and positive max-age"
    }
    $rustCli = $defaultRustCli
    if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
        throw "Repowise adapter binary is missing: $rustCli"
    }
    & $rustCli provider repowise-adapt `
        --request $RepowiseAdapterRequest `
        --artifact-root $RepowiseAdapterArtifactRoot `
        --evaluated-at $RepowiseAdapterEvaluatedAt `
        --max-age-seconds $RepowiseAdapterMaxAgeSeconds
    exit $LASTEXITCODE
}

if (-not [string]::IsNullOrWhiteSpace($GraphAdapterRequest)) {
    if ([string]::IsNullOrWhiteSpace($GraphAdapterArtifactRoot) -or
        $GraphAdapterEvaluatedAt -lt 0 -or
        $GraphAdapterMaxAgeSeconds -le 0) {
        throw "Graph adapter facade requires artifact root, non-negative evaluated-at, and positive max-age"
    }
    $rustCli = $defaultRustCli
    if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
        throw "Graph adapter binary is missing: $rustCli"
    }
    & $rustCli provider graph-adapt `
        --request $GraphAdapterRequest `
        --artifact-root $GraphAdapterArtifactRoot `
        --evaluated-at $GraphAdapterEvaluatedAt `
        --max-age-seconds $GraphAdapterMaxAgeSeconds
    exit $LASTEXITCODE
}

if (-not [string]::IsNullOrWhiteSpace($SentruxAdapterRequest)) {
    if ([string]::IsNullOrWhiteSpace($SentruxAdapterArtifactRoot) -or
        $SentruxAdapterEvaluatedAt -lt 0 -or
        $SentruxAdapterMaxAgeSeconds -le 0) {
        throw "Sentrux adapter facade requires artifact root, non-negative evaluated-at, and positive max-age"
    }
    $rustCli = $defaultRustCli
    if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) { throw "Sentrux adapter binary is missing: $rustCli" }
    & $rustCli provider sentrux-adapt --request $SentruxAdapterRequest --artifact-root $SentruxAdapterArtifactRoot --evaluated-at $SentruxAdapterEvaluatedAt --max-age-seconds $SentruxAdapterMaxAgeSeconds
    exit $LASTEXITCODE
}

if (-not [string]::IsNullOrWhiteSpace($CodeNexusAdapterRequest)) {
    if ([string]::IsNullOrWhiteSpace($CodeNexusAdapterArtifactRoot) -or
        $CodeNexusAdapterEvaluatedAt -lt 0 -or
        $CodeNexusAdapterMaxAgeSeconds -le 0) {
        throw "CodeNexus adapter facade requires artifact root, non-negative evaluated-at, and positive max-age"
    }
    $rustCli = $defaultRustCli
    if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
        throw "CodeNexus adapter binary is missing: $rustCli"
    }
    & $rustCli provider codenexus-adapt `
        --request $CodeNexusAdapterRequest `
        --artifact-root $CodeNexusAdapterArtifactRoot `
        --evaluated-at $CodeNexusAdapterEvaluatedAt `
        --max-age-seconds $CodeNexusAdapterMaxAgeSeconds
    exit $LASTEXITCODE
}

if (-not [string]::IsNullOrWhiteSpace($SurvivalScanRequest)) {
    if ([string]::IsNullOrWhiteSpace($SurvivalScanArtifactRoot)) {
        throw "Repository survival scan facade requires an artifact root"
    }
    $rustCli = $defaultRustCli
    if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
        throw "Repository survival scan binary is missing: $rustCli"
    }
    & $rustCli repository survival-scan `
        --request $SurvivalScanRequest `
        --artifact-root $SurvivalScanArtifactRoot
    exit $LASTEXITCODE
}

if (-not [string]::IsNullOrWhiteSpace($RunCommitManifestRef)) {
    if ([string]::IsNullOrWhiteSpace($RunCommitSourceRoot) -or
        [string]::IsNullOrWhiteSpace($RunCommitAuthorityRoot) -or
        [string]::IsNullOrWhiteSpace($RunCommitFinalName)) {
        throw "Run commit facade requires source root, authority root, manifest Artifact Ref, and final name"
    }
    $rustCli = $defaultRustCli
    if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
        throw "Run commit binary is missing: $rustCli"
    }
    & $rustCli run commit `
        --source-root $RunCommitSourceRoot `
        --authority-root $RunCommitAuthorityRoot `
        --manifest-ref $RunCommitManifestRef `
        --final-name $RunCommitFinalName
    exit $LASTEXITCODE
}

function Resolve-Repo {
    param([string]$Path)

    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        throw "Repo path is not a directory: $Path"
    }
    return $item.FullName
}

function Find-RepoConfigByPath {
    param([object]$ReposConfig, [string]$ResolvedRepoPath)

    if ($null -eq $ReposConfig -or [string]::IsNullOrWhiteSpace($ResolvedRepoPath)) { return $null }
    $normalizedRepoPath = [System.IO.Path]::TrimEndingDirectorySeparator($ResolvedRepoPath)
    foreach ($entry in $ReposConfig.PSObject.Properties) {
        $configuredPath = Get-JsonProperty $entry.Value "path"
        if ([string]::IsNullOrWhiteSpace([string]$configuredPath)) { continue }
        try {
            $resolvedConfiguredPath = Resolve-Repo ([string]$configuredPath)
        }
        catch {
            continue
        }
        $normalizedConfiguredPath = [System.IO.Path]::TrimEndingDirectorySeparator($resolvedConfiguredPath)
        if ([string]::Equals($normalizedConfiguredPath, $normalizedRepoPath, [System.StringComparison]::OrdinalIgnoreCase)) {
            return $entry.Value
        }
    }
    return $null
}

function Test-CommandAvailable {
    param([string]$Name)
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function Test-GitRepository {
param([string]$Path)

if (-not (Test-CommandAvailable "git")) { return $false }
$output = & git -C $Path rev-parse --is-inside-work-tree 2>$null
return ($LASTEXITCODE -eq 0 -and [string]$output -eq "true")
}

# Workflow recommendations are owned by the standalone advisory atom in OpenSpec-Detector.ps1.

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

function Resolve-ConfigString {
    param(
        [string]$Value,
        [object]$RepoConfig,
        [object]$ConfigData,
        [string]$Name,
        [string[]]$EnvNames = @(),
        [string]$Default = ""
    )

    if (-not [string]::IsNullOrWhiteSpace($Value)) { return $Value }

    $repoValue = Get-JsonProperty $RepoConfig $Name
    if (-not [string]::IsNullOrWhiteSpace([string]$repoValue)) { return [string]$repoValue }

    $globalValue = Get-JsonProperty $ConfigData $Name
    if (-not [string]::IsNullOrWhiteSpace([string]$globalValue)) { return [string]$globalValue }

    foreach ($envName in $EnvNames) {
        $envValue = [Environment]::GetEnvironmentVariable($envName, "Process")
        if ([string]::IsNullOrWhiteSpace($envValue)) {
            $envValue = [Environment]::GetEnvironmentVariable($envName, "User")
        }
        if (-not [string]::IsNullOrWhiteSpace($envValue)) { return $envValue }
    }

    return $Default
}

function Normalize-RepowiseProvider {
    param([string]$Provider)
    if ([string]::IsNullOrWhiteSpace($Provider)) { return "mock" }
    $normalized = $Provider.Trim()
    if ($normalized -ieq "ccw") { return "codex_cli" }
    return $normalized
}

function Get-RepowiseProviderArgs {
    param(
        [string]$Provider,
        [string]$Model,
        [string]$Reasoning
    )

    $args = @("--provider", $Provider)
    if (-not [string]::IsNullOrWhiteSpace($Model)) { $args += @("--model", $Model) }
    if (-not [string]::IsNullOrWhiteSpace($Reasoning)) { $args += @("--reasoning", $Reasoning) }
    return $args
}

function Get-DefaultArtifactRoot {
    return (Get-CodeIntelArtifactRoot -Platform $effectivePlatform)
}

function Get-DefaultShadowRoot {
    return (Get-CodeIntelShadowRoot -Platform $effectivePlatform)
}

function Resolve-ChildPath {
    param(
        [string]$Base,
        [string]$Path
    )

    if ([string]::IsNullOrWhiteSpace($Path)) { return $Base }
    if ([System.IO.Path]::IsPathRooted($Path)) { return (Resolve-Repo $Path) }
    return Resolve-Repo (Join-Path $Base $Path)
}

function Invoke-LoggedStep {
    param(
        [string]$Name,
        [scriptblock]$Body
    )

    $started = Get-Date
    $entry = [ordered]@{
        name = $Name
        startedAt = $started.ToString("o")
        status = "running"
        exitCode = $null
        output = ""
        error = ""
        finishedAt = $null
        durationMs = $null
    }

    try {
        $global:LASTEXITCODE = 0
        $previousErrorActionPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = "Continue"
            $output = & $Body 2>&1
        }
        finally {
            $ErrorActionPreference = $previousErrorActionPreference
        }
        $entry.output = ($output | ForEach-Object { $_.ToString() } | Out-String).Trim()
        if ($global:LASTEXITCODE -ne 0) {
            throw "Command exited with code $global:LASTEXITCODE"
        }
        $entry.status = "passed"
        $entry.exitCode = 0
    }
    catch {
        $entry.status = "failed"
        if ($global:LASTEXITCODE -ne 0) {
            $entry.exitCode = $global:LASTEXITCODE
        }
        else {
            $entry.exitCode = 1
        }
        $entry.error = $_.Exception.Message
        if ([string]::IsNullOrWhiteSpace([string]$entry.output)) {
            $entry.output = ($_ | Out-String).Trim()
        }
    }
    finally {
        $finished = Get-Date
        $entry.finishedAt = $finished.ToString("o")
        $entry.durationMs = [int]($finished - $started).TotalMilliseconds
    }

    return [pscustomobject]$entry
}

function Convert-OptionalRepowiseTimeout {
    param([object]$Step)

    if ($null -eq $Step) { return $Step }
    $blob = (([string]$Step.error) + "`n" + ([string]$Step.output)).ToLowerInvariant()
    if ([string]$Step.status -eq "failed" -and [string]$Step.name -like "repowise*" -and $blob -match "timed out after") {
        $Step.status = "skipped"
        $Step.exitCode = $null
        $Step.output = "Optional Repowise step skipped after timeout. $($Step.error)"
        $Step.error = ""
    }
    return $Step
}

function Get-RelativePathSafe {
    param(
        [string]$Base,
        [string]$Path
    )

    try {
        return [System.IO.Path]::GetRelativePath($Base, $Path)
    }
    catch {
        try {
            $baseFull = [System.IO.Path]::GetFullPath($Base)
            $pathFull = [System.IO.Path]::GetFullPath($Path)
            if (-not $baseFull.EndsWith([System.IO.Path]::DirectorySeparatorChar)) {
                $baseFull = $baseFull + [System.IO.Path]::DirectorySeparatorChar
            }
            if ((Test-Path -LiteralPath $pathFull -PathType Container) -and -not $pathFull.EndsWith([System.IO.Path]::DirectorySeparatorChar)) {
                $pathFull = $pathFull + [System.IO.Path]::DirectorySeparatorChar
            }
            $relative = ([uri]$baseFull).MakeRelativeUri([uri]$pathFull).ToString()
            $relative = [uri]::UnescapeDataString($relative).Replace("/", [System.IO.Path]::DirectorySeparatorChar)
            if ([string]::IsNullOrWhiteSpace($relative)) { return "." }
            return $relative
        }
        catch {
            return $Path
        }
    }
}

function Get-StepFailureCategory {
    param([object]$Step)

    $name = [string]$Step.name
    $status = [string]$Step.status
    $blob = (([string]$Step.error) + "`n" + ([string]$Step.output)).ToLowerInvariant()

    if ($name -eq "understand graph" -and ($status -eq "failed" -or $status -eq "manual_required")) {
        return "graph_missing"
    }
    if ($name -like "sentrux*" -and ($status -eq "failed" -or $status -eq "manual_required")) {
        return "sentrux_fail"
    }
    if (($name -like "repowise*" -or $name -eq "provider preflight") -and $blob -match "rate_limit|quota|usage limit exceeded|error code: 429|too many requests|provider_quota") {
        return "provider_quota"
    }
    if (($name -like "repowise*" -or $name -eq "provider preflight") -and $blob -match "provider_unavailable|model_not_found|not_found_error|error code: 404|status code: 404") {
        return "provider_unavailable"
    }
    if (($name -like "repowise*" -or $name -eq "provider preflight") -and $blob -match "config_error|authentication_error|invalid api key|not authorized|token not match") {
        return "config_error"
    }
    if ($status -eq "failed") {
        return "local_tool_error"
    }
    return $null
}

function Get-CodeIntelEffectiveFailedSteps {
    param(
        [object[]]$FailedSteps,
        [int]$BlockingSentruxDebt
    )

    return @($FailedSteps | Where-Object {
        $category = [string](Get-StepFailureCategory $_)
        $category -ne "sentrux_fail" -or $BlockingSentruxDebt -gt 0
    })
}

function Test-GitHubSolutionResearchRequired {
param([object]$FailureCounts)

    if ($null -eq $FailureCounts) { return $false }
    if ($FailureCounts.localToolError -gt 0) { return $true }
    if ($FailureCounts.sentruxFail -gt 0) { return $true }
    if ($FailureCounts.providerQuota -gt 0) { return $true }

    return $false
}

function Complete-NodeLintHygieneStep {
    param(
        [System.Collections.Specialized.OrderedDictionary]$Step,
        [datetime]$Started
    )

    $finished = Get-Date
    $Step["finishedAt"] = $finished.ToString("o")
    $Step["durationMs"] = [int]($finished - $Started).TotalMilliseconds
    return [pscustomobject]$Step
}

function Get-NodeLintHygieneStep {
    param(
        [string]$RepoPath,
        [bool]$RgAvailable
    )

    $started = Get-Date
    $step = [ordered]@{
        name = "node lint hygiene"
        startedAt = $started.ToString("o")
        status = "skipped"
        exitCode = $null
        output = ""
        error = ""
        finishedAt = ""
        durationMs = 0
    }

    try {
        $packageJson = Join-Path $RepoPath "package.json"
        if (-not (Test-Path -LiteralPath $packageJson -PathType Leaf)) {
            $step["output"] = "No package.json found."
            return (Complete-NodeLintHygieneStep -Step $step -Started $started)
        }

        $package = Get-Content -LiteralPath $packageJson -Raw | ConvertFrom-Json
        $scripts = Get-JsonProperty $package "scripts"
        $lintScript = [string](Get-JsonProperty $scripts "lint")
        if ([string]::IsNullOrWhiteSpace($lintScript) -or $lintScript -notmatch "\beslint\b") {
            $step["output"] = "No root ESLint lint script detected."
            return (Complete-NodeLintHygieneStep -Step $step -Started $started)
        }

        if (-not $RgAvailable) {
            $step["output"] = "rg unavailable; skip static ESLint asset-boundary check."
            return (Complete-NodeLintHygieneStep -Step $step -Started $started)
        }

        $rgArgs = @(
            "--files",
            "--hidden",
            "--no-ignore",
            "-g", "!**/.git/**",
            "-g", "!**/node_modules/**",
            "-g", "!**/dist/**",
            "-g", "!**/build/**",
            $RepoPath
        )
        $repoFiles = @(& rg @rgArgs 2>$null)
        $global:LASTEXITCODE = 0
        $normalizedFiles = @($repoFiles | ForEach-Object { ([string]$_).Replace("\", "/") })

        $assetPatterns = New-Object System.Collections.Generic.List[string]
        if (@($normalizedFiles | Where-Object { $_ -match "(^|/)apps/[^/]+/public/charting_library/" } | Select-Object -First 1).Count -gt 0) {
            $assetPatterns.Add("apps/*/public/charting_library/**")
        }
        if (@($normalizedFiles | Where-Object { $_ -match "(^|/)apps/[^/]+/public/datafeeds/" } | Select-Object -First 1).Count -gt 0) {
            $assetPatterns.Add("apps/*/public/datafeeds/**")
        }
        if (@($normalizedFiles | Where-Object { $_ -match "(^|/)packages/[^/]+/vendor/" } | Select-Object -First 1).Count -gt 0) {
            $assetPatterns.Add("packages/*/vendor/**")
        }
        if (@($normalizedFiles | Where-Object { $_ -match "(^|/)vendor/" } | Select-Object -First 1).Count -gt 0) {
            $assetPatterns.Add("vendor/**")
        }

        if ($assetPatterns.Count -eq 0) {
            $step["status"] = "passed"
            $step["exitCode"] = 0
            $step["output"] = "Root ESLint lint script detected; no known generated/vendor static asset directories found."
            return (Complete-NodeLintHygieneStep -Step $step -Started $started)
        }

        $configNames = @("eslint.config.js", "eslint.config.mjs", "eslint.config.cjs", ".eslintignore", ".eslintrc", ".eslintrc.json", ".eslintrc.js", ".eslintrc.cjs")
        $configFiles = @($configNames | ForEach-Object {
                $candidate = Join-Path $RepoPath $_
                if (Test-Path -LiteralPath $candidate -PathType Leaf) { $candidate }
            })
        if ($configFiles.Count -eq 0) {
            $step["status"] = "manual_required"
            $step["exitCode"] = 0
            $step["output"] = "Root lint script uses ESLint and known generated/vendor static asset dirs exist, but no root ESLint config or ignore file was found. Add ignores for: $($assetPatterns -join ', '), then run root lint before push."
            return (Complete-NodeLintHygieneStep -Step $step -Started $started)
        }

        $configText = (($configFiles | ForEach-Object { Get-Content -LiteralPath $_ -Raw }) -join [Environment]::NewLine).Replace("\", "/")
        $missing = New-Object System.Collections.Generic.List[string]
        foreach ($pattern in $assetPatterns) {
            $covered = $false
            if ($pattern -eq "apps/*/public/charting_library/**") {
                $covered = ($configText -match "charting_library|apps/\*/public|\*\*/public|public/\*\*")
            }
            elseif ($pattern -eq "apps/*/public/datafeeds/**") {
                $covered = ($configText -match "datafeeds|apps/\*/public|\*\*/public|public/\*\*")
            }
            elseif ($pattern -eq "packages/*/vendor/**" -or $pattern -eq "vendor/**") {
                $covered = ($configText -match "vendor")
            }

            if (-not $covered) {
                $missing.Add($pattern)
            }
        }

        if ($missing.Count -gt 0) {
            $step["status"] = "manual_required"
            $step["exitCode"] = 0
            $step["output"] = "Root lint script uses ESLint and known generated/vendor static asset dirs exist, but ignore coverage appears incomplete for: $($missing -join ', '). Add explicit ESLint ignores or run root lint before push."
        }
        else {
            $step["status"] = "passed"
            $step["exitCode"] = 0
            $step["output"] = "Root ESLint lint script has ignore coverage for known generated/vendor static asset dirs: $($assetPatterns -join ', ')."
        }
    }
    catch {
        $step["status"] = "manual_required"
        $step["exitCode"] = 0
        $step["output"] = "Node lint hygiene check could not complete. Run root lint before push and inspect generated/vendor asset ignores."
        $step["error"] = $_.Exception.Message
    }
    finally {
        $finished = Get-Date
        $step["finishedAt"] = $finished.ToString("o")
        $step["durationMs"] = [int]($finished - $started).TotalMilliseconds
    }

    return (Complete-NodeLintHygieneStep -Step $step -Started $started)
}

function New-GitHubSolutionResearchNotApplicable {
    return [ordered]@{
        status = "not_applicable"
        required = $false
        path = ""
        markdown = ""
        reason = "No blocker category requires GitHub solution research."
        candidates = 0
        queries = 0
        evidenceLinks = @()
        exitCriteria = @("GitHub research is not required for clean, graph-missing, governance-only, or surgery-plan-only scans.")
    }
}

function Join-StatusNames {
    param(
        [object[]]$Items,
        [string]$Empty = "none"
    )

    if ($Items.Count -eq 0) { return $Empty }
    return (($Items | ForEach-Object { "$($_.name)=$($_.status)" }) -join "; ")
}

function Read-JsonFileSafe {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path) -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $null
    }
    try {
        return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    }
    catch {
        return $null
    }
}

function Get-CodeEvidenceLanguage {
    param([string]$Extension)

    switch ($Extension.ToLowerInvariant()) {
        ".ps1" { return "powershell" }
        ".psm1" { return "powershell" }
        ".py" { return "python" }
        ".js" { return "javascript" }
        ".jsx" { return "javascript" }
        ".mjs" { return "javascript" }
        ".cjs" { return "javascript" }
        ".ts" { return "typescript" }
        ".tsx" { return "typescript" }
        ".rs" { return "rust" }
        ".go" { return "go" }
        ".java" { return "java" }
        ".cs" { return "csharp" }
        default { return "text" }
    }
}

function New-CodeEvidenceNativeSymbol {
    param(
        [string]$RelativePath,
        [string]$Language,
        [int]$LineNumber,
        [string]$Kind,
        [string]$Name
    )

    return [ordered]@{
        id = "$RelativePath#$Kind`:$Name"
        kind = $Kind
        name = $Name
        file = $RelativePath
        startLine = $LineNumber
        endLine = $LineNumber
        language = $Language
        confidence = 0.55
        source = "native-minimal"
    }
}

function Get-CodeEvidencePowerShellSymbol {
    param([string]$Line)

    if ($Line -match '^\s*function\s+([A-Za-z0-9_\-:]+)') {
        return [ordered]@{ kind = "function"; name = $Matches[1] }
    }
    return $null
}

function Get-CodeEvidencePythonSymbol {
    param([string]$Line)

    if ($Line -match '^\s*(def|class)\s+([A-Za-z_][A-Za-z0-9_]*)') {
        $kind = if ($Matches[1] -eq "class") { "class" } else { "function" }
        return [ordered]@{ kind = $kind; name = $Matches[2] }
    }
    return $null
}

function Get-CodeEvidenceJavaScriptSymbol {
    param([string]$Line)

    if ($Line -match '^\s*(export\s+)?(async\s+)?function\s+([A-Za-z_$][A-Za-z0-9_$]*)') {
        return [ordered]@{ kind = "function"; name = $Matches[3] }
    }
    if ($Line -match '^\s*(export\s+)?(const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(async\s*)?(\([^)]*\)|[A-Za-z_$][A-Za-z0-9_$]*)\s*=>') {
        return [ordered]@{ kind = "function"; name = $Matches[3] }
    }
    if ($Line -match '^\s*(export\s+)?(class|interface)\s+([A-Za-z_$][A-Za-z0-9_$]*)') {
        return [ordered]@{ kind = $Matches[2]; name = $Matches[3] }
    }
    return $null
}

function Get-CodeEvidenceRustSymbol {
    param([string]$Line)

    if ($Line -match '^\s*(pub\s+)?(async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)') {
        return [ordered]@{ kind = "function"; name = $Matches[3] }
    }
    return $null
}

function Get-CodeEvidenceGoSymbol {
    param([string]$Line)

    if ($Line -match '^\s*func\s+(\([^)]+\)\s*)?([A-Za-z_][A-Za-z0-9_]*)') {
        return [ordered]@{ kind = "function"; name = $Matches[2] }
    }
    return $null
}

function Get-CodeEvidenceJavaSymbol {
    param([string]$Line)

    if ($Line -match '^\s*(public|private|protected)?\s*(class|interface|enum)\s+([A-Za-z_][A-Za-z0-9_]*)') {
        return [ordered]@{ kind = $Matches[2]; name = $Matches[3] }
    }
    return $null
}

function Get-CodeEvidenceSymbolCandidate {
    param(
        [string]$Language,
        [string]$Line
    )

    switch ($Language) {
        "powershell" { return Get-CodeEvidencePowerShellSymbol $Line }
        "python" { return Get-CodeEvidencePythonSymbol $Line }
        "javascript" { return Get-CodeEvidenceJavaScriptSymbol $Line }
        "typescript" { return Get-CodeEvidenceJavaScriptSymbol $Line }
        "rust" { return Get-CodeEvidenceRustSymbol $Line }
        "go" { return Get-CodeEvidenceGoSymbol $Line }
        "java" { return Get-CodeEvidenceJavaSymbol $Line }
        default { return $null }
    }
}

function Get-CodeEvidenceSymbols {
    param(
        [string]$RelativePath,
        [string]$Language,
        [string[]]$Lines
    )

    $symbols = New-Object System.Collections.Generic.List[object]
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $candidate = Get-CodeEvidenceSymbolCandidate -Language $Language -Line ([string]$Lines[$i])
        if ($null -eq $candidate -or [string]::IsNullOrWhiteSpace([string]$candidate["name"])) {
            continue
        }

        $symbols.Add((New-CodeEvidenceNativeSymbol `
            -RelativePath $RelativePath `
            -Language $Language `
            -LineNumber ($i + 1) `
            -Kind ([string]$candidate["kind"]) `
            -Name ([string]$candidate["name"])))
    }
    return $symbols.ToArray()
}

function Get-CodeEvidenceImports {
param(
[string]$RelativePath,
[string]$Language,
[string[]]$Lines
    )

    $imports = New-Object System.Collections.Generic.List[object]
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $line = [string]$Lines[$i]
        $target = ""
        if ($Language -in @("javascript", "typescript") -and $line -match 'from\s+["'']([^"'']+)["'']') {
            $target = $Matches[1]
        } elseif ($Language -in @("javascript", "typescript") -and $line -match 'require\(["'']([^"'']+)["'']\)') {
            $target = $Matches[1]
        } elseif ($Language -eq "python" -and $line -match '^\s*(from|import)\s+([A-Za-z0-9_\.]+)') {
            $target = $Matches[2]
        } elseif ($Language -eq "rust" -and $line -match '^\s*use\s+([^;]+);') {
            $target = $Matches[1].Trim()
        } elseif ($Language -eq "go" -and $line -match '^\s*import\s+["'']([^"'']+)["'']') {
            $target = $Matches[1]
        } elseif ($line -match '^\s*#include\s+[<"]([^>"]+)[>"]') {
            $target = $Matches[1]
        }

        if (-not [string]::IsNullOrWhiteSpace($target)) {
            $imports.Add([ordered]@{
                file = $RelativePath
                line = $i + 1
                target = $target
                language = $Language
                confidence = 0.6
                source = "native-minimal"
            })
        }
    }
return $imports.ToArray()
}

function New-AgentCodeSliceRanking {
param(
[object[]]$Files,
[object[]]$Symbols,
[object[]]$Imports
)

$symbolsByFile = @{}
foreach ($symbol in @($Symbols)) {
$file = [string]$symbol.file
if ([string]::IsNullOrWhiteSpace($file)) { continue }
if (-not $symbolsByFile.ContainsKey($file)) {
$symbolsByFile[$file] = New-Object System.Collections.Generic.List[object]
}
$symbolsByFile[$file].Add($symbol)
}

$importsByFile = @{}
foreach ($import in @($Imports)) {
$file = [string]$import.file
if ([string]::IsNullOrWhiteSpace($file)) { continue }
if (-not $importsByFile.ContainsKey($file)) {
$importsByFile[$file] = New-Object System.Collections.Generic.List[object]
}
$importsByFile[$file].Add($import)
}

$rankedFiles = New-Object System.Collections.Generic.List[object]
foreach ($file in @($Files)) {
$path = [string]$file.path
if ([string]::IsNullOrWhiteSpace($path)) { continue }

$reasons = New-Object System.Collections.Generic.List[string]
$score = 0
if ($path -match '(^|/)(index|main|app|server|cli)\.') {
$reasons.Add("entrypoint")
$score += 40
}
if ($path -match '(test|spec)\.' -or $path -match '(^|/)(tests?|spec)/') {
$reasons.Add("test")
$score += 35
}
if ($symbolsByFile.ContainsKey($path) -and $symbolsByFile[$path].Count -gt 0) {
$reasons.Add("symbols")
$score += [Math]::Min(20, 5 * $symbolsByFile[$path].Count)
}
if ($importsByFile.ContainsKey($path) -and $importsByFile[$path].Count -gt 0) {
$reasons.Add("imports")
$score += [Math]::Min(15, 5 * $importsByFile[$path].Count)
}
if ($score -eq 0) {
$reasons.Add("inventory")
$score = 1
}

$rankedFiles.Add([ordered]@{
path = $path
language = [string]$file.language
score = $score
reasons = @($reasons.ToArray())
symbols = if ($symbolsByFile.ContainsKey($path)) { @($symbolsByFile[$path] | ForEach-Object { $_.name }) } else { @() }
imports = if ($importsByFile.ContainsKey($path)) { @($importsByFile[$path] | ForEach-Object { $_.target }) } else { @() }
})
}

$ordered = @($rankedFiles.ToArray() | Sort-Object -Property @{ Expression = "score"; Descending = $true }, @{ Expression = "path"; Descending = $false })
return [ordered]@{
schema = "agent-code-slice-ranking.v1"
strategy = "native-evidence-default"
files = $ordered
}
}

function Write-CodeEvidenceAgentSlices {
param(
[string]$AgentDir,
[string]$SliceDir,
[object[]]$Files,
[object[]]$Symbols,
[object[]]$Imports,
[object]$CocoOutcome
)

$ranking = New-AgentCodeSliceRanking -Files $Files -Symbols $Symbols -Imports $Imports
$rankingPath = Join-Path $AgentDir "ranking.json"
$ranking | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $rankingPath -Encoding UTF8

$agentIndexPath = Join-Path $AgentDir "index.md"
@(
"# Agent Code Map",
"",
"## Status",
"- Code Evidence Layer: ok",
"- Native minimal layer: enabled",
"- Ranking: [ranking.json](ranking.json)",
"- Native retrieval slice: [native-retrieval](slices/native-retrieval.md)",
"- cocoindex-code adapter: $($CocoOutcome.status) ($($CocoOutcome.reasonCode))",
"",
"## Full Dumps",
"- [files](../full/files.json)",
"- [symbols](../full/symbols.json)",
"- [chunks](../full/chunks.json)",
"- [symbol chunks](../full/symbol-chunks.json)",
"- [imports](../full/imports.json)",
"",
"## Slices",
"- [native retrieval](slices/native-retrieval.md)",
"- [entrypoints](slices/entrypoints.md)",
"- [tests](slices/tests.md)",
"- [risk hotspots](slices/risk-hotspots.md)"
) | Set-Content -LiteralPath $agentIndexPath -Encoding UTF8

$topRanked = @($ranking.files | Select-Object -First 20)
@(
"# Native Retrieval Slice",
"",
"- Strategy: native-evidence-default",
"- Source: Code Evidence files/symbols/imports only",
"",
"## Ranked Files"
) + @($topRanked | ForEach-Object {
"- $($_.path) score=$($_.score) reasons=$(@($_.reasons) -join ',')"
}) | Set-Content -LiteralPath (Join-Path $SliceDir "native-retrieval.md") -Encoding UTF8

$entrypoints = @($Files | Where-Object { $_.path -match '(^|/)(index|main|app|server|cli)\.' } | Select-Object -First 20)
@("# Entrypoints", "") + @($entrypoints | ForEach-Object { "- $($_.path) ($($_.language))" }) | Set-Content -LiteralPath (Join-Path $SliceDir "entrypoints.md") -Encoding UTF8

$tests = @($Files | Where-Object { $_.path -match '(test|spec)\.' -or $_.path -match '(^|/)(tests?|spec)/' } | Select-Object -First 30)
@("# Tests", "") + @($tests | ForEach-Object { "- $($_.path) ($($_.language))" }) | Set-Content -LiteralPath (Join-Path $SliceDir "tests.md") -Encoding UTF8

@(
"# Risk Hotspots",
"",
"- Native minimal layer does not calculate complexity.",
"- Treat file-sized chunks as fallback evidence until structural chunking is enabled.",
"- cocoindex-code adapter outcome: $($CocoOutcome.status) ($($CocoOutcome.reasonCode))."
) | Set-Content -LiteralPath (Join-Path $SliceDir "risk-hotspots.md") -Encoding UTF8

return [ordered]@{
agentIndex = $agentIndexPath
ranking = $rankingPath
nativeRetrieval = Join-Path $SliceDir "native-retrieval.md"
}
}

function New-CodeEvidenceLayer {
param(
[string]$RepoPath,
[string]$RunDir,
[object[]]$Files,
[object]$CodeEvidenceConfig = $null
)

$root = Join-Path $RunDir "code-evidence"
$fullDir = Join-Path $root "merged\full"
$agentDir = Join-Path $root "merged\agent"
$sliceDir = Join-Path $agentDir "slices"
$adapterDir = Join-Path $root "adapters\cocoindex-code"
foreach ($dir in @($fullDir, $agentDir, $sliceDir, $adapterDir)) {
New-Item -ItemType Directory -Force -Path $dir | Out-Null
}

$fileRows = New-Object System.Collections.Generic.List[object]
$symbols = New-Object System.Collections.Generic.List[object]
$chunks = New-Object System.Collections.Generic.List[object]
$symbolChunks = New-Object System.Collections.Generic.List[object]
$imports = New-Object System.Collections.Generic.List[object]

foreach ($file in @($Files)) {
$fileText = [string]$file
if ([string]::IsNullOrWhiteSpace($fileText)) { continue }
$fullPath = if ([System.IO.Path]::IsPathRooted($fileText)) { $fileText } else { Join-Path $RepoPath $fileText }
if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) { continue }

$relativePath = (Get-RelativePathSafe $RepoPath $fullPath).Replace("\", "/")
$extension = [System.IO.Path]::GetExtension($fullPath)
$language = Get-CodeEvidenceLanguage -Extension $extension
$content = Get-Content -LiteralPath $fullPath -Raw -ErrorAction SilentlyContinue
        if ($null -eq $content) { $content = "" }
        $lines = if ([string]::IsNullOrEmpty($content)) { @() } else { @($content -split "`r?`n") }
        $lines = @($lines)
        $contentBytes = [System.Text.Encoding]::UTF8.GetBytes($content)
        $hashBytes = [System.Security.Cryptography.SHA256]::HashData($contentBytes)
$hash = [System.BitConverter]::ToString($hashBytes).Replace("-", "").ToLowerInvariant()

$fileRows.Add([ordered]@{
path = $relativePath
language = $language
bytes = $contentBytes.Length
lines = $lines.Count
textHash = $hash
source = "native-minimal"
})

$fileSymbols = @(Get-CodeEvidenceSymbols -RelativePath $relativePath -Language $language -Lines $lines)
foreach ($symbol in $fileSymbols) { $symbols.Add($symbol) }

$chunkId = "$relativePath#file"
$chunks.Add([ordered]@{
id = $chunkId
file = $relativePath
startLine = 1
endLine = [Math]::Max(1, $lines.Count)
kind = "file"
containsSymbols = @($fileSymbols | ForEach-Object { $_.id })
textHash = $hash
source = "native-minimal"
})

foreach ($symbol in $fileSymbols) {
$symbolChunks.Add([ordered]@{
symbolId = $symbol.id
chunkId = $chunkId
relation = "contained_by"
confidence = 0.55
})
}

foreach ($import in @(Get-CodeEvidenceImports -RelativePath $relativePath -Language $language -Lines $lines)) {
$imports.Add($import)
}
}

$fileRowsArray = @($fileRows.ToArray())
$symbolsArray = @($symbols.ToArray())
$chunksArray = @($chunks.ToArray())
$symbolChunksArray = @($symbolChunks.ToArray())
$importsArray = @($imports.ToArray())

([ordered]@{ schema = "code-evidence-files.v1"; files = $fileRowsArray }) | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $fullDir "files.json") -Encoding UTF8
([ordered]@{ schema = "code-evidence-symbols.v1"; symbols = $symbolsArray }) | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $fullDir "symbols.json") -Encoding UTF8
([ordered]@{ schema = "code-evidence-chunks.v1"; chunks = $chunksArray }) | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $fullDir "chunks.json") -Encoding UTF8
([ordered]@{ schema = "code-evidence-symbol-chunks.v1"; mappings = $symbolChunksArray }) | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $fullDir "symbol-chunks.json") -Encoding UTF8
([ordered]@{ schema = "code-evidence-imports.v1"; imports = $importsArray }) | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $fullDir "imports.json") -Encoding UTF8

# R07 reviewed retirement: this compatibility artifact is a static tombstone.
# There is intentionally no configuration lookup, executable discovery, or provider invocation.
$cocoOutcome = [ordered]@{
schema = "code-evidence-adapter-outcome.v1"
adapter = "cocoindex-code"
enabled = $false
required = $false
status = "skipped"
fatal = $false
reasonCode = "reviewed_deletion"
reason = "cocoindex-code is a reviewed retirement tombstone; legacy configuration cannot restore discovery or invocation."
command = ""
}

$cocoOutcomePath = Join-Path $adapterDir "outcome.json"
$cocoOutcome | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $cocoOutcomePath -Encoding UTF8

$scorecard = [ordered]@{
schema = "code-evidence-scorecard.v1"
status = "ok"
nativeMinimal = $true
adapters = @($cocoOutcome)
metrics = [ordered]@{
files = $fileRowsArray.Count
symbols = $symbolsArray.Count
chunks = $chunksArray.Count
imports = $importsArray.Count
symbolContainmentRate = if ($symbolsArray.Count -gt 0) { 1.0 } else { $null }
fallbackChunkRate = 1.0
}
}
$scorecardPath = Join-Path $root "merged\scorecard.json"
$scorecard | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $scorecardPath -Encoding UTF8
$scorecardMarkdownPath = Join-Path $root "merged\scorecard.md"
@(
"# Code Evidence Scorecard",
"",
"- Status: ok",
"- Native minimal: true",
"- Files: $($fileRowsArray.Count)",
"- Symbols: $($symbolsArray.Count)",
"- Chunks: $($chunksArray.Count)",
"- Imports: $($importsArray.Count)",
"- cocoindex-code: $($cocoOutcome.status) ($($cocoOutcome.reasonCode))"
) | Set-Content -LiteralPath $scorecardMarkdownPath -Encoding UTF8

$agentSlices = Write-CodeEvidenceAgentSlices `
-AgentDir $agentDir `
-SliceDir $sliceDir `
-Files $fileRowsArray `
-Symbols $symbolsArray `
-Imports $importsArray `
-CocoOutcome $cocoOutcome

return [ordered]@{
schema = "code-evidence-summary.v1"
status = "ok"
fatal = $false
root = $root
agentIndex = $agentSlices.agentIndex
scorecard = $scorecardPath
scorecardMarkdown = $scorecardMarkdownPath
files = $fileRowsArray.Count
symbols = $symbolsArray.Count
chunks = $chunksArray.Count
imports = $importsArray.Count
adapters = @($cocoOutcome)
}
}

function ConvertTo-NullableDouble {
    param([object]$Value)

    if ($null -eq $Value) { return $null }
    try {
        return [double]$Value
    }
    catch {
        return $null
    }
}

function Get-SentruxMetricPair {
    param(
        [string]$Output,
        [string]$Label
    )

    if ([string]::IsNullOrWhiteSpace($Output)) { return $null }
    $pattern = [regex]::Escape($Label) + ":\s+([0-9.]+)\s+[^\r\n0-9.]+\s+([0-9.]+)"
    $match = [regex]::Match($Output, $pattern)
    if (-not $match.Success) { return $null }

    return [ordered]@{
        before = ConvertTo-NullableDouble $match.Groups[1].Value
        after = ConvertTo-NullableDouble $match.Groups[2].Value
    }
}

function New-SentruxMetricDelta {
    param(
        [string]$Name,
        [object]$Before,
        [object]$After,
        [ValidateSet("higher_is_better", "lower_is_better")]
        [string]$Polarity = "lower_is_better"
    )

    $beforeValue = ConvertTo-NullableDouble $Before
    $afterValue = ConvertTo-NullableDouble $After
    $delta = $null
    $direction = "unknown"
    $regressed = $false

    if ($null -ne $beforeValue -and $null -ne $afterValue) {
        $delta = $afterValue - $beforeValue
        if ([math]::Abs($delta) -lt 0.000001) {
            $direction = "stable"
        }
        elseif ($delta -gt 0) {
            $direction = "up"
        }
        else {
            $direction = "down"
        }

        if ($Polarity -eq "higher_is_better") {
            $regressed = $delta -lt 0
        }
        else {
            $regressed = $delta -gt 0
        }
    }

    return [ordered]@{
        name = $Name
        before = $beforeValue
        after = $afterValue
        delta = $delta
        direction = $direction
        polarity = $Polarity
        regressed = $regressed
    }
}

function Test-SentruxGateNoDegradation {
    param([string]$GateOutput)

    return (-not [string]::IsNullOrWhiteSpace($GateOutput) -and $GateOutput -match "No degradation detected")
}

function Resolve-SentruxMetricRegressions {
    param(
        [object[]]$Metrics,
        [bool]$NoDegradation
    )

    foreach ($metric in @($Metrics)) {
        if ($null -eq $metric) {
            continue
        }

        $rawRegressed = [bool]$metric.regressed
        $gateAccepted = $NoDegradation -and $rawRegressed
        $metric | Add-Member -NotePropertyName rawRegressed -NotePropertyValue $rawRegressed -Force
        $metric | Add-Member -NotePropertyName gateAccepted -NotePropertyValue $gateAccepted -Force
        if ($gateAccepted) {
            $metric.regressed = $false
        }
        $metric
    }
}

function New-SentruxInsight {
    param(
        [string]$RepoName,
        [string]$TargetPath,
        [string]$BaselinePath,
        [object[]]$Steps
    )

    $gateStep = @($Steps | Where-Object { $_.name -like "sentrux gate*" } | Select-Object -Last 1)
    $checkStep = @($Steps | Where-Object { $_.name -eq "sentrux check" } | Select-Object -First 1)
    $rulesPath = if ([string]::IsNullOrWhiteSpace($TargetPath)) { "" } else { Join-Path (Join-Path $TargetPath ".sentrux") "rules.toml" }
    $baseline = Read-JsonFileSafe $BaselinePath
    $gateOutput = if ($gateStep.Count -gt 0) { [string]$gateStep[0].output } else { "" }
    $noDegradation = Test-SentruxGateNoDegradation $gateOutput

    $qualityPair = Get-SentruxMetricPair $gateOutput "Quality"
    $couplingPair = Get-SentruxMetricPair $gateOutput "Coupling"
    $cyclesPair = Get-SentruxMetricPair $gateOutput "Cycles"
    $godFilesPair = Get-SentruxMetricPair $gateOutput "God files"
    $distance = $null
    $distanceMatch = [regex]::Match($gateOutput, "Distance from Main Sequence:\s+([0-9.]+)")
    if ($distanceMatch.Success) {
        $distance = ConvertTo-NullableDouble $distanceMatch.Groups[1].Value
    }

    $scan = [ordered]@{}
    $resolveMatch = [regex]::Match($gateOutput, "\[resolve\]\s+([0-9]+)\s+resolved,\s+([0-9]+)\s+unresolved")
    if ($resolveMatch.Success) {
        $scan["resolvedImports"] = [int]$resolveMatch.Groups[1].Value
        $scan["unresolvedImports"] = [int]$resolveMatch.Groups[2].Value
    }
    $graphMatch = [regex]::Match($gateOutput, "\[build_graphs\]\s+([0-9]+)\s+files.*\|\s+([0-9]+)\s+import,\s+([0-9]+)\s+call,\s+([0-9]+)\s+inherit edges")
    if ($graphMatch.Success) {
        $scan["files"] = [int]$graphMatch.Groups[1].Value
        $scan["importEdges"] = [int]$graphMatch.Groups[2].Value
        $scan["callEdges"] = [int]$graphMatch.Groups[3].Value
        $scan["inheritEdges"] = [int]$graphMatch.Groups[4].Value
    }

    $metrics = @()
    if ($null -ne $qualityPair) {
        $metrics += [pscustomobject](New-SentruxMetricDelta "quality" $qualityPair["before"] $qualityPair["after"] "higher_is_better")
    }
    if ($null -ne $couplingPair) {
        $metrics += [pscustomobject](New-SentruxMetricDelta "coupling" $couplingPair["before"] $couplingPair["after"] "lower_is_better")
    }
    if ($null -ne $cyclesPair) {
        $metrics += [pscustomobject](New-SentruxMetricDelta "cycles" $cyclesPair["before"] $cyclesPair["after"] "lower_is_better")
    }
    if ($null -ne $godFilesPair) {
        $metrics += [pscustomobject](New-SentruxMetricDelta "god_files" $godFilesPair["before"] $godFilesPair["after"] "lower_is_better")
    }
    $metrics = @(Resolve-SentruxMetricRegressions -Metrics $metrics -NoDegradation $noDegradation)

    $regressions = @($metrics | Where-Object { $_.regressed })
    $nextActions = @()
    $codeNexusHints = @()

    if ([string]::IsNullOrWhiteSpace($TargetPath)) {
        $nextActions += "Sentrux target was not resolved; inspect pipeline configuration."
    }
    elseif (-not (Test-Path -LiteralPath $BaselinePath -PathType Leaf)) {
        $nextActions += "Create an intentional Sentrux baseline for this scope before using it as a gate."
    }
    elseif ($gateStep.Count -gt 0 -and $gateStep[0].status -eq "failed") {
        $nextActions += "Inspect the Sentrux gate output before saving any new baseline."
    }
    elseif ($regressions.Count -gt 0) {
        $nextActions += "Investigate regressed structural metrics before accepting this change."
    }
    else {
        $nextActions += "No structural regression detected for this scope."
    }

    if (-not [string]::IsNullOrWhiteSpace($rulesPath) -and -not (Test-Path -LiteralPath $rulesPath -PathType Leaf)) {
        $nextActions += "Add .sentrux/rules.toml when this scope needs explicit architecture boundary rules."
    }

    if (@($regressions | Where-Object { $_.name -in @("coupling", "cycles") }).Count -gt 0) {
        $codeNexusHints += "Use CodeNexus impact/context on symbols in newly coupled modules."
        $codeNexusHints += "Suggested query: gitnexus query `"cross module import dependency cycle`" --repo $RepoName"
    }
    elseif (@($regressions | Where-Object { $_.name -eq "quality" }).Count -gt 0) {
        $codeNexusHints += "Use CodeNexus query to locate the flow behind the quality drop."
        $codeNexusHints += "Suggested query: gitnexus query `"complex hotspot structural regression`" --repo $RepoName"
    }
    else {
        $codeNexusHints += "If a future gate regresses, start with CodeNexus context/impact on the changed files."
    }

    return [ordered]@{
        targetPath = $TargetPath
        baselinePath = $BaselinePath
        baselineExists = (-not [string]::IsNullOrWhiteSpace($BaselinePath) -and (Test-Path -LiteralPath $BaselinePath -PathType Leaf))
        rulesPath = $rulesPath
        rulesExists = (-not [string]::IsNullOrWhiteSpace($rulesPath) -and (Test-Path -LiteralPath $rulesPath -PathType Leaf))
        checkStatus = if ($checkStep.Count -gt 0) { $checkStep[0].status } else { "not_run" }
        gateStatus = if ($gateStep.Count -gt 0) { $gateStep[0].status } else { "not_run" }
        noDegradation = $noDegradation
        metrics = $metrics
        baseline = [ordered]@{
            qualitySignal = ConvertTo-NullableDouble (Get-JsonProperty $baseline "quality_signal")
            couplingScore = ConvertTo-NullableDouble (Get-JsonProperty $baseline "coupling_score")
            cycleCount = ConvertTo-NullableDouble (Get-JsonProperty $baseline "cycle_count")
            complexFnCount = ConvertTo-NullableDouble (Get-JsonProperty $baseline "complex_fn_count")
            crossModuleEdges = ConvertTo-NullableDouble (Get-JsonProperty $baseline "cross_module_edges")
            totalImportEdges = ConvertTo-NullableDouble (Get-JsonProperty $baseline "total_import_edges")
        }
        distanceFromMainSequence = $distance
        scan = $scan
        regressions = $regressions
        nextActions = $nextActions
        codeNexusHints = $codeNexusHints
    }
}

function Get-StepMatch {
    param(
        [object[]]$Steps,
        [string]$Pattern,
        [switch]$Last
    )

    $matches = @($Steps | Where-Object { [string]$_.name -like $Pattern })
    if ($matches.Count -eq 0) { return $null }
    if ($Last) { return $matches[-1] }
    return $matches[0]
}

function Get-StepScore {
    param([object]$Step)

    if ($null -eq $Step) { return 0 }
    switch ([string]$Step.status) {
        "passed" { return 100 }
        default { return 0 }
    }
}

function Get-FailureCount {
    param(
        [object]$FailureCounts,
        [string]$Name
    )

    if ($FailureCounts -is [System.Collections.IDictionary] -and $FailureCounts.Contains($Name)) {
        return [int]$FailureCounts[$Name]
    }
    if ($null -ne $FailureCounts -and $null -ne $FailureCounts.PSObject.Properties[$Name]) {
        return [int]$FailureCounts.$Name
    }

    return 0
}

function Get-FirstLine {
    param([string]$Text)

    if ([string]::IsNullOrWhiteSpace($Text)) { return "" }
    return (($Text -split "\r?\n") | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 1)
}

function New-QualityDimension {
    param(
        [string]$Name,
        [int]$Score,
        [string]$Status,
        [string]$Evidence
    )

    return [ordered]@{
        name = $Name
        score = [math]::Max(0, [math]::Min(100, $Score))
        status = $Status
        evidence = $Evidence
    }
}

function New-Modality {
    param(
        [string]$Name,
        [string]$Role,
        [object]$Step,
        [int]$Confidence,
        [string]$Artifact,
        [string]$Finding,
        [string]$Limit
    )

    $status = if ($Finding -eq "not generated" -and [string]::IsNullOrWhiteSpace($Artifact)) {
        "missing"
    }
    elseif ($null -ne $Step) {
        [string]$Step.status
    }
    elseif (-not [string]::IsNullOrWhiteSpace($Artifact)) {
        "generated"
    }
    else {
        "not_run"
    }
    return [ordered]@{
        name = $Name
        role = $Role
        status = $status
        confidence = [math]::Max(0, [math]::Min(100, $Confidence))
        artifact = $Artifact
        finding = $Finding
        limit = $Limit
        durationMs = if ($null -eq $Step -or $null -eq $Step.durationMs) { $null } else { [int]$Step.durationMs }
    }
}

function New-HospitalProtocol {
    param(
        [string]$Name,
        [string]$Status,
        [string]$Command,
        [string]$ExitCriteria
    )

    return [ordered]@{
        name = $Name
        status = $Status
        command = $Command
        exit_criteria = $ExitCriteria
    }
}

function New-StateTransition {
    param(
        [string]$From,
        [string]$To,
        [string]$Guard,
        [bool]$Pass
    )

    return [ordered]@{
        from = $From
        to = $To
        guard = $Guard
        pass = $Pass
    }
}

function New-HospitalStateMachine {
    param(
        [object]$FailureCounts,
        [bool]$RulesExists,
        [string]$GateStatus,
        [string]$CheckStatus,
        [int]$FailingWhatIfCount,
        [string]$Disposition,
        [string]$NextProtocol,
        [bool]$StructuralEvidenceComplete = $true,
        [string]$SurgeryTarget = "",
        [string]$CurrentTopHotspot = ""
    )

    # Keep this guard self-contained because the state-machine seam is also
    # extracted independently by the regression harness.
    $providerQuotaCount = 0
    $providerUnavailableCount = 0
    $configErrorCount = 0
    if ($FailureCounts -is [System.Collections.IDictionary] -and $FailureCounts.Contains("providerQuota")) {
        $providerQuotaCount = [int]$FailureCounts["providerQuota"]
    }
    elseif ($null -ne $FailureCounts -and $null -ne $FailureCounts.PSObject.Properties["providerQuota"]) {
        $providerQuotaCount = [int]$FailureCounts.providerQuota
    }
    if ($FailureCounts -is [System.Collections.IDictionary] -and $FailureCounts.Contains("providerUnavailable")) {
        $providerUnavailableCount = [int]$FailureCounts["providerUnavailable"]
    }
    elseif ($null -ne $FailureCounts -and $null -ne $FailureCounts.PSObject.Properties["providerUnavailable"]) {
        $providerUnavailableCount = [int]$FailureCounts.providerUnavailable
    }
    if ($FailureCounts -is [System.Collections.IDictionary] -and $FailureCounts.Contains("configError")) {
        $configErrorCount = [int]$FailureCounts["configError"]
    }
    elseif ($null -ne $FailureCounts -and $null -ne $FailureCounts.PSObject.Properties["configError"]) {
        $configErrorCount = [int]$FailureCounts.configError
    }

    $toolsOk = ([int]$FailureCounts.localToolError -eq 0)
    $providerAvailable = ($providerQuotaCount -eq 0 -and $providerUnavailableCount -eq 0 -and $configErrorCount -eq 0)
    $graphOk = ([int]$FailureCounts.graphMissing -eq 0)
    $sentruxOk = ([int]$FailureCounts.sentruxFail -eq 0 -and $RulesExists -and $GateStatus -eq "passed" -and $CheckStatus -eq "passed")
    $surgeryDebtCleared = ($StructuralEvidenceComplete -and $FailingWhatIfCount -eq 0)

    # surgery_plan -> post_op: the surgery target has actually been treated
    # (it no longer shows up as the current top hotspot) and sentrux confirms
    # the governed scope is clean, so it is safe to move on to post-op review.
    $surgeryTargetResolved = (-not [string]::IsNullOrWhiteSpace($SurgeryTarget) -and
        -not [string]::IsNullOrWhiteSpace($CurrentTopHotspot) -and
        ($SurgeryTarget -ne $CurrentTopHotspot))
    $surgeryToPostOpOk = ($sentruxOk -and $StructuralEvidenceComplete -and $surgeryTargetResolved)
    $postOpOk = ($toolsOk -and $providerAvailable -and $graphOk -and $sentruxOk -and $surgeryDebtCleared -and $surgeryTargetResolved)

    $currentState = switch ($NextProtocol) {
        "triage" { "triage" }
        "diagnose" { "diagnose" }
        "govern" { "govern" }
        "surgery_plan" { "surgery_plan" }
        "post_op" { if ($Disposition -eq "discharge_ready") { "discharge_ready" } else { "post_op" } }
        default { "triage" }
    }

    return [ordered]@{
        schema = "code-intel-hospital-state-machine.v1"
        current_state = $currentState
        disposition = $Disposition
        next_protocol = $NextProtocol
        states = @("triage", "diagnose", "govern", "surgery_plan", "post_op", "discharge_ready")
        transitions = @(
            (New-StateTransition "triage" "diagnose" "local toolchain is available" $toolsOk)
            (New-StateTransition "diagnose" "govern" "architecture graph exists or graph absence is accepted" $graphOk)
            (New-StateTransition "govern" "surgery_plan" "rules and gate pass, but what-if still has planned debt" ($sentruxOk -and -not $surgeryDebtCleared))
            (New-StateTransition "govern" "post_op" "rules and gate pass, no planned surgery debt remains" ($sentruxOk -and $surgeryDebtCleared))
            (New-StateTransition "surgery_plan" "post_op" "sentrux gate/check pass and the surgery target no longer appears as the current top hotspot" $surgeryToPostOpOk)
            (New-StateTransition "post_op" "discharge_ready" "post-op verification passes with no regressions" $postOpOk)
        )
        guards = [ordered]@{
            tools_ok = $toolsOk
            provider_available = $providerAvailable
            graph_ok = $graphOk
            rules_exists = $RulesExists
            sentrux_check = $CheckStatus
            sentrux_gate = $GateStatus
            sentrux_ok = $sentruxOk
            failing_what_if = $FailingWhatIfCount
            structural_evidence_complete = $StructuralEvidenceComplete
            surgery_debt_cleared = $surgeryDebtCleared
            surgery_target = $SurgeryTarget
            current_top_hotspot = $CurrentTopHotspot
            surgery_target_resolved = $surgeryTargetResolved
            surgery_to_post_op_ok = $surgeryToPostOpOk
            post_op_ok = $postOpOk
        }
    }
}

function Read-JsonPathIfExists {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) { return $null }
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }

    return Read-JsonFileSafe $Path
}

function Get-SourceAnchorText {
    param([object]$SourceAnchor)

    if ($null -eq $SourceAnchor) { return "" }
    if ($SourceAnchor -is [string]) { return [string]$SourceAnchor }
    if ($null -ne $SourceAnchor.label) { return [string]$SourceAnchor.label }
    if ($null -ne $SourceAnchor.path) { return [string]$SourceAnchor.path }

    return [string]$SourceAnchor
}

function New-CodeIntelSurgeryPlan {
    param(
        [object]$Hospital,
        [string]$RepoPath,
        [string]$SentruxTargetPath,
        [string]$HotspotsPath,
        [string]$WhatIfPath,
        [string]$CodeNexusPath
    )

    $hotspots = Read-JsonPathIfExists $HotspotsPath
    $whatIf = Read-JsonPathIfExists $WhatIfPath
    $codeNexus = Read-JsonPathIfExists $CodeNexusPath

    $primaryFunction = $null
    if ($null -ne $hotspots -and $null -ne $hotspots.functions -and @($hotspots.functions).Count -gt 0) {
        $primaryFunction = $hotspots.functions[0]
    }
    $primaryFile = $null
    if ($null -ne $hotspots -and $null -ne $hotspots.files -and @($hotspots.files).Count -gt 0) {
        $primaryFile = $hotspots.files[0]
    }
    $primaryScenario = $null
    $failingScenarios = @()
    if ($null -ne $whatIf -and $null -ne $whatIf.scenarios) {
        $failingScenarios = @($whatIf.scenarios | Where-Object { -not $_.pass })
        if ($failingScenarios.Count -gt 0) { $primaryScenario = $failingScenarios[0] }
    }
    $contextFile = $null
    if ($null -ne $codeNexus -and $null -ne $codeNexus.files -and @($codeNexus.files).Count -gt 0) {
        $contextFile = $codeNexus.files[0]
    }

    $targetFile = if ($null -ne $primaryFunction) { [string]$primaryFunction.file } elseif ($null -ne $primaryFile) { [string]$primaryFile.path } else { "" }
    $targetName = if ($null -ne $primaryFunction) { [string]$primaryFunction.name } elseif ($null -ne $primaryFile) { [string]$primaryFile.path } else { "" }
    $targetAnchor = if ($null -ne $primaryFunction) { Get-SourceAnchorText $primaryFunction.sourceAnchor } elseif ($null -ne $primaryFile) { Get-SourceAnchorText $primaryFile.sourceAnchor } else { "" }
    $targetComplexity = if ($null -ne $primaryFunction) { [int]$primaryFunction.complexity } elseif ($null -ne $primaryFile) { [int]$primaryFile.maxComplexity } else { $null }
    $scenarioName = if ($null -ne $primaryScenario) { [string]$primaryScenario.name } else { "" }
    $scenarioAction = if ($null -ne $primaryScenario) { [string]$primaryScenario.action } else { "" }
    $status = if ([string]$Hospital.triage.next_protocol -eq "surgery_plan" -or
        ([string]$Hospital.triage.disposition -eq "admit" -and -not [string]::IsNullOrWhiteSpace($targetFile))) {
        "planned"
    }
    else {
        "not_required"
    }

    return [ordered]@{
        schema = "code-intel-surgery-plan.v1"
        status = $status
        repo = $RepoPath
        scope = $SentruxTargetPath
        admission = [ordered]@{
            disposition = $Hospital.triage.disposition
            diagnosis = $Hospital.triage.primary_diagnosis
            reason = $Hospital.triage.admission_reason
        }
        primary_target = [ordered]@{
            file = $targetFile
            name = $targetName
            source_anchor = $targetAnchor
            complexity = $targetComplexity
            scenario = $scenarioName
            scenario_action = $scenarioAction
            codenexus_file = if ($null -ne $contextFile) { [string]$contextFile.path } else { "" }
        }
        operating_plan = @(
            "Open the primary target and its CodeNexus context before editing.",
            "Reduce the selected hotspot by extraction, boundary clarification, or testable decomposition.",
            "Do not raise Sentrux thresholds to make the surgery pass.",
            "Add or update the smallest test that proves the behavior stayed intact."
        )
        verification = @(
            "Invoke-SentruxAgentTool.ps1 check_rules `"$SentruxTargetPath`"",
            "Invoke-SentruxAgentTool.ps1 session_end `"$SentruxTargetPath`"",
            "scripts/tests/test-code-intel-pipeline.ps1 -RepoPath `"$RepoPath`" -SentruxPath `"$((Get-RelativePathSafe $RepoPath $SentruxTargetPath) -replace '\\', '/')`" -SkipRepowise -Mode normal"
        )
        discharge_criteria = $Hospital.triage.discharge_criteria
        evidence = [ordered]@{
            hotspots = $HotspotsPath
            what_if = $WhatIfPath
            codenexus = $CodeNexusPath
            failing_scenarios = @($failingScenarios | Select-Object -First 5)
        }
    }
}

function Convert-SurgeryPlanToMarkdown {
    param([object]$Plan)

    $lines = @(
        "# Code Intel Surgery Plan",
        "",
        "- Status: $($Plan.status)",
        "- Repo: $($Plan.repo)",
        "- Scope: $($Plan.scope)",
        "- Diagnosis: $($Plan.admission.diagnosis)",
        "- Admission reason: $($Plan.admission.reason)",
        "",
        "## Primary Target",
        "- File: $($Plan.primary_target.file)",
        "- Symbol: $($Plan.primary_target.name)",
        "- Anchor: $($Plan.primary_target.source_anchor)",
        "- Complexity: $($Plan.primary_target.complexity)",
        "- Scenario: $($Plan.primary_target.scenario)",
        "- Action: $($Plan.primary_target.scenario_action)",
        "- CodeNexus file: $($Plan.primary_target.codenexus_file)",
        "",
        "## Operating Plan"
    )
    foreach ($item in @($Plan.operating_plan)) {
        $lines += "- $item"
    }
    $lines += ""
    $lines += "## Verification"
    foreach ($item in @($Plan.verification)) {
        $lines += "- ``$item``"
    }
    $lines += ""
    $lines += "## Discharge Criteria"
    foreach ($item in @($Plan.discharge_criteria)) {
        $lines += "- $item"
    }
    return $lines
}

function Get-HospitalDiagnosis {
    param(
        [object]$FailureCounts,
        [bool]$RulesExists,
        [int]$FailingWhatIfCount
    )

    $providerQuotaCount = Get-FailureCount $FailureCounts "providerQuota"
    $providerUnavailableCount = Get-FailureCount $FailureCounts "providerUnavailable"
    $configErrorCount = Get-FailureCount $FailureCounts "configError"

    if ($FailureCounts.localToolError -gt 0) {
        return [ordered]@{ severity = "red"; primaryDiagnosis = "local tool failure" }
    }
    if ($providerQuotaCount -gt 0) {
        return [ordered]@{ severity = "amber"; primaryDiagnosis = "provider quota exhausted" }
    }
    if ($providerUnavailableCount -gt 0) {
        return [ordered]@{ severity = "amber"; primaryDiagnosis = "provider unavailable" }
    }
    if ($configErrorCount -gt 0) {
        return [ordered]@{ severity = "amber"; primaryDiagnosis = "provider configuration error" }
    }
    if ($FailureCounts.sentruxFail -gt 0) {
        return [ordered]@{ severity = "red"; primaryDiagnosis = "architecture gate failure" }
    }
    if ($FailureCounts.graphMissing -gt 0) {
        return [ordered]@{ severity = "amber"; primaryDiagnosis = "architecture graph missing" }
    }
    if (-not $RulesExists) {
        return [ordered]@{ severity = "amber"; primaryDiagnosis = "ungoverned structural scope" }
    }
    if ($FailingWhatIfCount -gt 0) {
        return [ordered]@{ severity = "amber"; primaryDiagnosis = "known modernization debt" }
    }

    return [ordered]@{ severity = "green"; primaryDiagnosis = "clean snapshot" }
}

function Get-HospitalNextProtocol {
    param(
        [object]$FailureCounts,
        [bool]$RulesExists,
        [int]$FailingWhatIfCount,
        [object]$GitHubResearch
    )

    $providerQuotaCount = Get-FailureCount $FailureCounts "providerQuota"
    $providerUnavailableCount = Get-FailureCount $FailureCounts "providerUnavailable"
    $configErrorCount = Get-FailureCount $FailureCounts "configError"

    if ($FailureCounts.localToolError -gt 0) { return "triage" }
    if ($providerQuotaCount -gt 0) { return "triage" }
    if ($providerUnavailableCount -gt 0) { return "triage" }
    if ($configErrorCount -gt 0) { return "triage" }
    if ($null -ne $GitHubResearch -and [bool]$GitHubResearch.required) { return "github_solution_research" }
    if ($FailureCounts.graphMissing -gt 0) { return "diagnose" }
    if (-not $RulesExists) { return "govern" }
    if ($FailingWhatIfCount -gt 0) { return "surgery_plan" }

    return "post_op"
}

function Get-HospitalAdmissionReason {
    param([string]$PrimaryDiagnosis)

    switch ($PrimaryDiagnosis) {
        "clean snapshot" { return "No active inpatient issue; ready for discharge after post-op verification." }
        "architecture graph missing" { return "Admit for diagnostic imaging: Understand graph is missing or stale." }
        "ungoverned structural scope" { return "Admit for governance: rules are missing for the selected scope." }
        "known modernization debt" { return "Admit for planned surgery: what-if scenarios show debt that should be scheduled, not ignored." }
        "architecture gate failure" { return "Admit for structural treatment: Sentrux gate or rules failed." }
        "provider quota exhausted" { return "Admit for triage: provider quota prevented complete evidence collection." }
        "provider unavailable" { return "Admit for triage: the configured upstream provider route or model was unavailable." }
        "provider configuration error" { return "Admit for triage: provider credentials, endpoint, or model configuration must be corrected." }
        "structural evidence incomplete" { return "Admit for diagnosis: required structural summaries are incomplete." }
        "local tool failure" { return "Admit for triage: local toolchain failed before diagnosis can be trusted." }
        default { return "Admit until the next protocol clears the diagnosis." }
    }
}

function Get-HospitalTreatmentPlan {
    param(
        [object]$FailureCounts,
        [bool]$RulesExists,
        [int]$FailingWhatIfCount,
        [string]$UnderstandCommand,
        [string]$TopContextFile
    )

    $providerQuotaCount = Get-FailureCount $FailureCounts "providerQuota"
    $providerUnavailableCount = Get-FailureCount $FailureCounts "providerUnavailable"
    $configErrorCount = Get-FailureCount $FailureCounts "configError"

    $treatment = @()
    if ($FailureCounts.localToolError -gt 0) { $treatment += "Fix local tool errors before interpreting architecture signals." }
    if ($providerQuotaCount -gt 0) { $treatment += "Restore provider quota or use a complete local evidence path before interpreting the result." }
    if ($providerUnavailableCount -gt 0) { $treatment += "Verify the provider model catalog and route availability; keep local index-only evidence available." }
    if ($configErrorCount -gt 0) { $treatment += "Correct provider endpoint, model, or credential configuration before retrying provider-backed docs." }
    if ($FailureCounts.graphMissing -gt 0) { $treatment += "Refresh Understand graph with: $UnderstandCommand" }
    if (-not $RulesExists) { $treatment += "Add .sentrux/rules.toml for the chosen scope." }
    if ($FailingWhatIfCount -gt 0) { $treatment += "Use what-if failures as the tightening roadmap; start with the first failing scenario." }
    if (-not [string]::IsNullOrWhiteSpace($TopContextFile)) { $treatment += "Start CodeNexus review at $TopContextFile." }
    if ($treatment.Count -eq 0) { $treatment += "Keep this artifact as the current clean snapshot and compare the next session against it." }

    return $treatment
}

function New-HospitalDecisionBlock {
    param(
        [object]$FailureCounts,
        [bool]$RulesExists,
        [string]$GateStatus,
        [string]$CheckStatus,
        [int]$FailingWhatIfCount,
        [string]$UnderstandCommand,
        [string]$TopContextFile,
        [bool]$StructuralEvidenceComplete = $false,
        [string]$SurgeryTarget = "",
        [string]$CurrentTopHotspot = "",
        [object]$GitHubResearch
    )

    $diagnosis = Get-HospitalDiagnosis $FailureCounts $RulesExists $FailingWhatIfCount
    $nextProtocol = Get-HospitalNextProtocol $FailureCounts $RulesExists $FailingWhatIfCount $GitHubResearch
    $sentruxVerified = ($RulesExists -and $GateStatus -eq "passed" -and $CheckStatus -eq "passed")
    if ($diagnosis.severity -eq "green" -and -not $sentruxVerified) {
        $hasExplicitFailure = ($GateStatus -eq "failed" -or $CheckStatus -eq "failed")
        $diagnosis = [ordered]@{
            severity = if ($hasExplicitFailure) { "red" } else { "amber" }
            primaryDiagnosis = if ($hasExplicitFailure) { "architecture gate failure" } else { "architecture verification incomplete" }
        }
        $nextProtocol = "govern"
    }
    elseif ($diagnosis.severity -eq "green" -and -not $StructuralEvidenceComplete) {
        $diagnosis = [ordered]@{ severity = "amber"; primaryDiagnosis = "structural evidence incomplete" }
        $nextProtocol = "diagnose"
    }
    $postOpResolved = (-not [string]::IsNullOrWhiteSpace($SurgeryTarget) -and
        -not [string]::IsNullOrWhiteSpace($CurrentTopHotspot) -and
        $SurgeryTarget -ne $CurrentTopHotspot)
    $disposition = if ($diagnosis.severity -ne "green") {
        "admit"
    }
    elseif ($sentruxVerified -and $StructuralEvidenceComplete -and $postOpResolved) {
        "discharge_ready"
    }
    else {
        "observe"
    }
    $admissionReason = Get-HospitalAdmissionReason $diagnosis.primaryDiagnosis
    $dischargeCriteria = @(
        "failure category counters are zero",
        "Sentrux check and gate pass for the governed scope",
        "hospital triage status is green or explicitly accepted for observation",
        "session_end reports no quality regression after Agent edits"
    )
    if ($null -ne $GitHubResearch -and [bool]$GitHubResearch.required) {
        $dischargeCriteria += "GitHub evidence linked or GitHub evidence insufficiency recorded in github-solution-research artifacts"
    }
    $treatment = Get-HospitalTreatmentPlan $FailureCounts $RulesExists $FailingWhatIfCount $UnderstandCommand $TopContextFile
    if (-not $sentruxVerified) {
        $treatment = @($treatment) + "Obtain passing Sentrux check and gate evidence before discharge."
    }

    $stateMachine = New-HospitalStateMachine `
        -FailureCounts $FailureCounts `
        -RulesExists $RulesExists `
        -GateStatus $GateStatus `
        -CheckStatus $CheckStatus `
        -FailingWhatIfCount $FailingWhatIfCount `
        -Disposition $disposition `
        -NextProtocol $nextProtocol `
        -StructuralEvidenceComplete $StructuralEvidenceComplete `
        -SurgeryTarget $SurgeryTarget `
        -CurrentTopHotspot $CurrentTopHotspot

    return [ordered]@{
        severity = $diagnosis.severity
        primaryDiagnosis = $diagnosis.primaryDiagnosis
        nextProtocol = $nextProtocol
        disposition = $disposition
        admissionReason = $admissionReason
        dischargeCriteria = $dischargeCriteria
        treatment = $treatment
        stateMachine = $stateMachine
    }
}

function New-HospitalFindings {
    param(
        [int]$InventoryFiles,
        [object]$SentruxFileDetailsSummary,
        [string]$TopFunction,
        [string]$TopModule,
        [object]$ResolvedRatio,
        [int]$ResolvedImports,
        [int]$UnresolvedImports,
        [int]$ExcludedFiles
    )

    $findings = @()
    if ($InventoryFiles -gt 0) { $findings += "X-ray inventory found $InventoryFiles files." }
    if ($null -ne $SentruxFileDetailsSummary) { $findings += "CT structural scan found $($SentruxFileDetailsSummary.files) files and $($SentruxFileDetailsSummary.functions) functions." }
    if (-not [string]::IsNullOrWhiteSpace($TopFunction)) { $findings += "Top surgical hotspot: $TopFunction." }
    if (-not [string]::IsNullOrWhiteSpace($TopModule)) { $findings += "Top module hotspot: $TopModule." }
    if ($ResolvedRatio -ne $null) { $findings += "Import resolution ratio is $ResolvedRatio% ($ResolvedImports resolved, $UnresolvedImports unresolved)." }
    if ($ExcludedFiles -gt 0) { $findings += "$ExcludedFiles files were quarantined from governed source metrics." }
    return $findings
}

function New-HospitalModalities {
    param(
        [object]$InventoryStep,
        [object]$UnderstandStep,
        [object]$RepowiseStep,
        [object]$SentruxCheckStep,
        [object]$SentruxGateStep,
        [int]$GraphScore,
        [int]$MemoryScore,
        [int]$MriScore,
        [string]$MriStatus,
        [int]$CtScore,
        [string]$CtStatus,
        [int]$PetScore,
        [string]$PetStatus,
        [int]$GovernanceScore,
        [string]$RunDir,
        [string]$RepoPath,
        [int]$InventoryFiles,
        [object]$SentruxDsmSummary,
        [object]$SentruxFileDetailsSummary,
        [object]$CodeNexusContextSummary,
        [object]$SentruxWhatIfSummary,
        [object]$RuntimeCiSummary,
        [string]$GovernanceArtifact,
        [string]$GovernanceFinding
    )

    $xrayFinding = if ($InventoryFiles -gt 0) { "$InventoryFiles files inventoried" } else { "no inventory" }
    $ctArtifact = if ($CtStatus -eq "available") { [string]$SentruxDsmSummary.path } else { "" }
    $ctFinding = if ($CtStatus -eq "available") { "$($SentruxDsmSummary.modules) modules, $($SentruxFileDetailsSummary.functions) functions" } else { "not generated" }
    $mriArtifact = if ($MriStatus -eq "available") { [string]$CodeNexusContextSummary.path } else { "" }
    $mriFinding = if ($MriStatus -eq "available") { "$($CodeNexusContextSummary.files) files, $($CodeNexusContextSummary.references) references" } else { "not generated" }
    $petArtifact = if ($null -ne $RuntimeCiSummary) { [string]$RuntimeCiSummary.path } elseif ($PetStatus -eq "available") { [string]$SentruxWhatIfSummary.path } else { "" }
    $petFinding = if ($null -ne $RuntimeCiSummary) { "runtime/CI health=$($RuntimeCiSummary.health); freshness=$($RuntimeCiSummary.freshness); completeness=$($RuntimeCiSummary.completeness)" } elseif ($PetStatus -eq "available") { "$($SentruxWhatIfSummary.failing) failing what-if scenarios" } else { "not generated" }
    $petLimitation = if ($null -ne $RuntimeCiSummary) { "Provider-neutral runtime/CI evidence is cited; provider logs remain outside this report." } else { "No live runtime trace is captured yet." }
    $chartFinding = if ($null -ne $RepowiseStep) { [string]$RepowiseStep.status } else { "not run" }

    return @(
        (New-Modality "xray" "fast file inventory and repo surface" $InventoryStep (Get-StepScore $InventoryStep) (Join-Path $RunDir "files.txt") $xrayFinding "Sees files, not semantic impact.")
        (New-Modality "anatomy" "Understand Anything architecture graph" $UnderstandStep $GraphScore (Join-Path (Join-Path $RepoPath ".understand-anything") "knowledge-graph.json") (Get-FirstLine ([string]$UnderstandStep.output)) "Requires a prebuilt graph from the Understand tool.")
        (New-Modality "ct" "Sentrux DSM, hotspots, and structural slices" $SentruxGateStep $CtScore $ctArtifact $ctFinding "Static structure is not runtime truth.")
        (New-Modality "mri" "CodeNexus context and impact localization" $null $MriScore $mriArtifact $mriFinding "Lite mode is local evidence, not a full semantic backend.")
        (New-Modality "pet" "runtime/CI evidence with test gaps, evolution, and what-if fallback" $null $PetScore $petArtifact $petFinding $petLimitation)
        (New-Modality "chart" "Repowise long-term project memory" $RepowiseStep $MemoryScore "" $chartFinding "Provider quota and index freshness can limit semantic memory.")
        (New-Modality "governance" "rules, gate, and session safety rails" $SentruxCheckStep $GovernanceScore $GovernanceArtifact $GovernanceFinding "Rules only protect boundaries that have been encoded.")
    )
}

function New-HospitalQualityDimensions {
    param(
        [int]$SourceCoverageScore,
        [string]$SourceScopeStatus,
        [int]$InventoryFiles,
        [int]$ScanFiles,
        [int]$GraphScore,
        [object]$UnderstandStep,
        [int]$ResolutionScore,
        [string]$ImportResolutionStatus,
        [int]$ResolvedImports,
        [int]$UnresolvedImports,
        [int]$PollutionScore,
        [string]$PollutionStatus,
        [int]$ExcludedFiles,
        [int]$GovernanceScore,
        [string]$GovernanceStatus,
        [string]$GovernanceEvidence,
        [int]$MriScore,
        [string]$LocalizationStatus,
        [string]$TopContextFile,
        [int]$MemoryScore,
        [string]$MemoryStatus,
        [string]$MemoryEvidence
    )

    return @(
        (New-QualityDimension "source_coverage" $SourceCoverageScore $SourceScopeStatus "inventory=$InventoryFiles; sentrux_scan=$ScanFiles")
        (New-QualityDimension "graph_freshness" $GraphScore ([string]$UnderstandStep.status) (Get-FirstLine ([string]$UnderstandStep.output)))
        (New-QualityDimension "import_resolution" $ResolutionScore $ImportResolutionStatus "resolved=$ResolvedImports; unresolved=$UnresolvedImports")
        (New-QualityDimension "pollution_control" $PollutionScore $PollutionStatus "excluded=$ExcludedFiles")
        (New-QualityDimension "governance" $GovernanceScore $GovernanceStatus $GovernanceEvidence)
        (New-QualityDimension "localization" $MriScore $LocalizationStatus "top_file=$TopContextFile")
        (New-QualityDimension "memory" $MemoryScore $MemoryStatus $MemoryEvidence)
    )
}

function Read-HospitalArtifactFile {
    param([object]$Summary)

    if ($null -eq $Summary) { return $null }

    $path = [string]$Summary.path
    if ([string]::IsNullOrWhiteSpace($path)) { return $null }
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $null }

    return Read-JsonFileSafe $path
}

function Read-HospitalArtifacts {
    param(
        [object]$SentruxDsmSummary,
        [object]$SentruxFileDetailsSummary,
        [object]$SentruxHotspotsSummary,
        [object]$SentruxEvolutionSummary,
        [object]$SentruxWhatIfSummary,
        [object]$CodeNexusContextSummary
    )

    return [ordered]@{
        dsm = Read-HospitalArtifactFile $SentruxDsmSummary
        file_details = Read-HospitalArtifactFile $SentruxFileDetailsSummary
        hotspots = Read-HospitalArtifactFile $SentruxHotspotsSummary
        evolution = Read-HospitalArtifactFile $SentruxEvolutionSummary
        what_if = Read-HospitalArtifactFile $SentruxWhatIfSummary
        codenexus = Read-HospitalArtifactFile $CodeNexusContextSummary
    }
}

function New-HospitalMeasurements {
    param(
        [object]$InventoryStep,
        [object]$SentruxInsight,
        [object]$DsmObject
    )

    $inventoryFiles = 0
    $inventoryMatch = [regex]::Match([string]$InventoryStep.output, "files=([0-9]+)")
    if ($inventoryMatch.Success) { $inventoryFiles = [int]$inventoryMatch.Groups[1].Value }

    $scan = if ($null -ne $SentruxInsight -and $null -ne $SentruxInsight["scan"]) { $SentruxInsight["scan"] } else { @{} }
    $scanFiles = if ($scan.Contains("files")) { [int]$scan["files"] } else { 0 }
    $unresolvedImports = if ($scan.Contains("unresolvedImports")) { [int]$scan["unresolvedImports"] } else { 0 }
    $resolvedImports = if ($scan.Contains("resolvedImports")) { [int]$scan["resolvedImports"] } else { 0 }
    $totalImports = $resolvedImports + $unresolvedImports
    $resolvedRatio = if ($totalImports -gt 0) { [math]::Round(($resolvedImports * 100.0) / $totalImports, 1) } else { $null }
    $dsmScope = $null
    if ($DsmObject -is [System.Collections.IDictionary] -and $DsmObject.Contains("scope")) {
        $dsmScope = $DsmObject["scope"]
    }
    elseif ($null -ne $DsmObject -and $null -ne $DsmObject.PSObject.Properties["scope"]) {
        $dsmScope = $DsmObject.scope
    }

    $excludedFilesValue = $null
    $hasPollutionEvidence = $false
    if ($dsmScope -is [System.Collections.IDictionary] -and $dsmScope.Contains("excluded_files")) {
        $excludedFilesValue = $dsmScope["excluded_files"]
        $hasPollutionEvidence = ($null -ne $excludedFilesValue)
    }
    elseif ($null -ne $dsmScope -and $null -ne $dsmScope.PSObject.Properties["excluded_files"]) {
        $excludedFilesValue = $dsmScope.excluded_files
        $hasPollutionEvidence = ($null -ne $excludedFilesValue)
    }

    $excludedFiles = if ($hasPollutionEvidence) { [int]$excludedFilesValue } else { 0 }
    $sourceScopeStatus = if ($inventoryFiles -gt 0 -and $scanFiles -gt 0) { "measured" } else { "unknown" }
    $pollutionStatus = if (-not $hasPollutionEvidence) { "unknown" } elseif ($excludedFiles -gt 0) { "quarantined" } else { "clean" }

    return [ordered]@{
        inventory_files = $inventoryFiles
        scan_files = $scanFiles
        unresolved_imports = $unresolvedImports
        resolved_imports = $resolvedImports
        resolved_ratio = $resolvedRatio
        excluded_files = $excludedFiles
        source_scope_status = $sourceScopeStatus
        pollution_status = $pollutionStatus
    }
}

function Get-ImportResolutionScore {
    param([object]$ResolvedRatio)

    if ($null -eq $ResolvedRatio) { return 0 }
    if ($ResolvedRatio -ge 75) { return 100 }
    if ($ResolvedRatio -ge 50) { return 75 }
    if ($ResolvedRatio -ge 25) { return 50 }

    return 30
}

function Get-SourceCoverageScore {
    param(
        [int]$ScanFiles,
        [int]$InventoryFiles
    )

    if ($ScanFiles -le 0 -or $InventoryFiles -le 0) { return 0 }

    return [int][math]::Round([math]::Min(100.0, ($ScanFiles * 100.0) / $InventoryFiles))
}

function New-HospitalScoreBlock {
    param(
        [object]$SentruxInsight,
        [object]$Measurements,
        [object]$UnderstandStep,
        [object]$RepowiseStep,
        [object]$SentruxCheckStep,
        [object]$SentruxGateStep,
        [object]$SentruxDsmObject,
        [object]$SentruxFileDetailsObject,
        [object]$SentruxEvolutionObject,
        [object]$SentruxWhatIfObject,
        [object]$CodeNexusContextObject,
        [object]$RuntimeCiSummary
    )

    $rulesExists = [bool]$SentruxInsight["rulesExists"]
    $rulesScore = if ($rulesExists) { 100 } else { 45 }
    $gateScore = Get-StepScore $SentruxGateStep
    $checkScore = Get-StepScore $SentruxCheckStep
    $graphScore = Get-StepScore $UnderstandStep
    $memoryScore = Get-StepScore $RepowiseStep
    $mriStatus = if ($null -ne $CodeNexusContextObject) { "available" } else { "missing" }
    $ctStatus = if ($null -ne $SentruxDsmObject -and $null -ne $SentruxFileDetailsObject) { "available" } else { "missing" }
    $petStatus = if ($null -ne $RuntimeCiSummary) { if ([string]$RuntimeCiSummary.health -eq "unknown") { "unknown" } else { "available" } } elseif ($null -ne $SentruxWhatIfObject -and $null -ne $SentruxEvolutionObject) { "available" } else { "missing" }
    $mriScore = if ($mriStatus -eq "available") { 100 } else { 0 }
    $ctScore = if ($ctStatus -eq "available") { 100 } else { 0 }
    $petScore = if ($null -ne $RuntimeCiSummary) { switch ([string]$RuntimeCiSummary.health) { "green" { 100 } "red" { 0 } default { 30 } } } elseif ($petStatus -eq "available") { 70 } else { 0 }
    $resolutionScore = Get-ImportResolutionScore $Measurements.resolved_ratio
    $pollutionStatus = [string]$Measurements.pollution_status
    $pollutionScore = if ($pollutionStatus -eq "unknown") { 0 } elseif ($Measurements.excluded_files -gt 0) { 100 } else { 80 }
    $governanceScore = [int][math]::Round(($rulesScore + $gateScore + $checkScore) / 3.0)
    $diagnosticScore = [int][math]::Round(($ctScore + $mriScore + $graphScore + $memoryScore) / 4.0)
    $overallScore = [int][math]::Round(($diagnosticScore + $governanceScore + $resolutionScore + $pollutionScore) / 4.0)
    $governanceArtifact = if ($rulesExists) { [string]$SentruxInsight["rulesPath"] } else { "" }
    $resolvedRatio = $Measurements.resolved_ratio

    return [ordered]@{
        rules_exists = $rulesExists
        gate_status = [string]$SentruxInsight["gateStatus"]
        check_status = [string]$SentruxInsight["checkStatus"]
        graph_score = $graphScore
        memory_score = $memoryScore
        mri_score = $mriScore
        mri_status = $mriStatus
        ct_score = $ctScore
        ct_status = $ctStatus
        pet_score = $petScore
        pet_status = $petStatus
        resolution_score = $resolutionScore
        pollution_score = $pollutionScore
        governance_score = $governanceScore
        diagnostic_score = $diagnosticScore
        overall_score = $overallScore
        source_coverage_score = Get-SourceCoverageScore $Measurements.scan_files $Measurements.inventory_files
        import_resolution_status = if ($null -eq $resolvedRatio) { "unknown" } else { "$resolvedRatio%" }
        pollution_status = $pollutionStatus
        governance_status = if ($rulesExists) { "rules_present" } else { "rules_missing" }
        governance_artifact = $governanceArtifact
        governance_finding = "rules=$($SentruxInsight['rulesExists']); gate=$($SentruxInsight['gateStatus']); check=$($SentruxInsight['checkStatus'])"
        governance_evidence = "gate=$($SentruxInsight['gateStatus']); check=$($SentruxInsight['checkStatus'])"
        localization_status = $mriStatus
        memory_status = if ($null -ne $RepowiseStep) { [string]$RepowiseStep.status } else { "not_run" }
        memory_evidence = if ($null -ne $RepowiseStep) { Get-FirstLine ([string]$RepowiseStep.output) } else { "" }
    }
}

function New-HospitalEvidenceBlock {
    param(
        [object]$HotspotsObject,
        [object]$WhatIfObject,
        [object]$CodeNexusContextSummary
    )

    $failingWhatIf = @()
    if ($null -ne $WhatIfObject -and $null -ne $WhatIfObject.scenarios) {
        $failingWhatIf = @($WhatIfObject.scenarios | Where-Object { -not $_.pass })
    }

    $topFunction = ""
    if ($null -ne $HotspotsObject -and $null -ne $HotspotsObject.functions -and @($HotspotsObject.functions).Count -gt 0) {
        $topFunction = "{0} in {1} (cc={2})" -f $HotspotsObject.functions[0].name, $HotspotsObject.functions[0].file, $HotspotsObject.functions[0].complexity
    }

    $topModule = ""
    if ($null -ne $HotspotsObject -and $null -ne $HotspotsObject.modules -and @($HotspotsObject.modules).Count -gt 0) {
        $topModule = "{0} (risk={1})" -f $HotspotsObject.modules[0].name, $HotspotsObject.modules[0].risk
    }

    return [ordered]@{
        failing_what_if = $failingWhatIf
        top_function = $topFunction
        top_module = $topModule
        top_context_file = if ($null -ne $CodeNexusContextSummary) { [string]$CodeNexusContextSummary.topFile } else { "" }
    }
}

function New-HospitalProtocolBlock {
    param(
        [bool]$RulesExists,
        [int]$FailingWhatIfCount
    )

    $governProtocolStatus = if ($RulesExists) { "active" } else { "needs_rules" }
    $surgeryProtocolStatus = if ($FailingWhatIfCount -gt 0) { "available" } else { "low_risk" }

    return @(
        (New-HospitalProtocol "triage" "available" "run-code-intel.ps1 -RepoPath <repo> -Mode lite" "Classify provider/tool/graph/Sentrux failure bucket and choose next protocol.")
        (New-HospitalProtocol "diagnose" "available" "run-code-intel.ps1 -RepoPath <repo> -Mode normal" "Produce summary.md, hospital.md, sentrux artifacts, and codenexus context.")
        (New-HospitalProtocol "govern" $governProtocolStatus "sentrux check <scope>; sentrux gate <scope>" "Rules pass and gate reports no degradation.")
        (New-HospitalProtocol "surgery_plan" $surgeryProtocolStatus "read sentrux-what-if.json and codenexus-context.json" "Choose one hotspot, one boundary, and one verification command before editing.")
        (New-HospitalProtocol "post_op" "available" "Invoke-SentruxAgentTool.ps1 session_end <scope>" "Signal does not drop, rules pass, and touched hotspot is lower risk.")
    )
}

function Get-PreviousSurgeryTarget {
    param([string]$RunDir)

    if ([string]::IsNullOrWhiteSpace($RunDir)) { return "" }
    $repoArtifactRoot = Split-Path -Parent $RunDir
    if ([string]::IsNullOrWhiteSpace($repoArtifactRoot) -or -not (Test-Path -LiteralPath $repoArtifactRoot -PathType Container)) { return "" }

    $currentName = Split-Path -Leaf $RunDir
    $previousRun = Get-ChildItem -LiteralPath $repoArtifactRoot -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -ne $currentName } |
        Sort-Object Name -Descending |
        Select-Object -First 1
    if ($null -eq $previousRun) { return "" }

    $previousPlanPath = Join-Path $previousRun.FullName "surgery-plan.json"
    if (-not (Test-Path -LiteralPath $previousPlanPath -PathType Leaf)) { return "" }

    $previousPlan = Read-JsonFileSafe $previousPlanPath
    if ($null -eq $previousPlan -or $null -eq $previousPlan.primary_target) { return "" }
    if ([string]::IsNullOrWhiteSpace([string]$previousPlan.primary_target.name)) { return "" }

    return "$($previousPlan.primary_target.name) in $($previousPlan.primary_target.file)"
}

function New-CodeIntelHospitalReport {
    param(
        [string]$RepoPath,
        [string]$Mode,
        [string]$RunDir,
        [string]$ReportPath,
        [string]$SummaryPath,
        [string]$UnderstandingPath,
        [object[]]$Steps,
        [object]$FailureCounts,
        [object]$SentruxInsight,
        [object]$SentruxDsmSummary,
        [object]$SentruxFileDetailsSummary,
        [object]$SentruxHotspotsSummary,
[object]$SentruxEvolutionSummary,
[object]$SentruxWhatIfSummary,
[object]$CodeNexusContextSummary,
[object]$RuntimeCiSummary,
[string]$UnderstandCommand,
[object]$ToolState,
[object]$GitHubResearch
)

    $gitStep = Get-StepMatch $Steps "git status"
    $inventoryStep = Get-StepMatch $Steps "rg file inventory"
    $understandStep = Get-StepMatch $Steps "understand graph"
    $repowiseStep = Get-StepMatch $Steps "repowise*" -Last
    $sentruxCheckStep = Get-StepMatch $Steps "sentrux check"
    $sentruxGateStep = Get-StepMatch $Steps "sentrux gate*" -Last

    $artifacts = Read-HospitalArtifacts $SentruxDsmSummary $SentruxFileDetailsSummary $SentruxHotspotsSummary $SentruxEvolutionSummary $SentruxWhatIfSummary $CodeNexusContextSummary
    $structuralEvidenceComplete = ($null -ne $artifacts.dsm -and
        $null -ne $artifacts.file_details -and
        $null -ne $artifacts.hotspots -and
        $null -ne $artifacts.evolution -and
        $null -ne $artifacts.what_if)
    $measurements = New-HospitalMeasurements $inventoryStep $SentruxInsight $artifacts.dsm
    $scores = New-HospitalScoreBlock `
        -SentruxInsight $SentruxInsight `
        -Measurements $measurements `
        -UnderstandStep $understandStep `
        -RepowiseStep $repowiseStep `
        -SentruxCheckStep $sentruxCheckStep `
        -SentruxGateStep $sentruxGateStep `
        -SentruxDsmObject $artifacts.dsm `
        -SentruxFileDetailsObject $artifacts.file_details `
        -SentruxEvolutionObject $artifacts.evolution `
        -SentruxWhatIfObject $artifacts.what_if `
        -CodeNexusContextObject $artifacts.codenexus `
        -RuntimeCiSummary $RuntimeCiSummary
    $evidence = New-HospitalEvidenceBlock $artifacts.hotspots $artifacts.what_if $CodeNexusContextSummary

    $currentTopHotspot = ""
    if ($null -ne $artifacts.hotspots -and $null -ne $artifacts.hotspots.functions -and @($artifacts.hotspots.functions).Count -gt 0) {
        $topFn = $artifacts.hotspots.functions[0]
        $currentTopHotspot = "$($topFn.name) in $($topFn.file)"
    }
    $surgeryTarget = Get-PreviousSurgeryTarget $RunDir

    $decision = New-HospitalDecisionBlock `
        -FailureCounts $FailureCounts `
        -RulesExists $scores.rules_exists `
        -GateStatus $scores.gate_status `
        -CheckStatus $scores.check_status `
        -FailingWhatIfCount @($evidence.failing_what_if).Count `
        -UnderstandCommand $UnderstandCommand `
        -TopContextFile $evidence.top_context_file `
        -StructuralEvidenceComplete $structuralEvidenceComplete `
        -SurgeryTarget $surgeryTarget `
        -CurrentTopHotspot $currentTopHotspot `
        -GitHubResearch $GitHubResearch

    $findings = New-HospitalFindings `
        -InventoryFiles $measurements.inventory_files `
        -SentruxFileDetailsSummary $SentruxFileDetailsSummary `
        -TopFunction $evidence.top_function `
        -TopModule $evidence.top_module `
        -ResolvedRatio $measurements.resolved_ratio `
        -ResolvedImports $measurements.resolved_imports `
        -UnresolvedImports $measurements.unresolved_imports `
        -ExcludedFiles $measurements.excluded_files

    $modalities = New-HospitalModalities `
        -InventoryStep $inventoryStep `
        -UnderstandStep $understandStep `
        -RepowiseStep $repowiseStep `
        -SentruxCheckStep $sentruxCheckStep `
        -SentruxGateStep $sentruxGateStep `
        -GraphScore $scores.graph_score `
        -MemoryScore $scores.memory_score `
        -MriScore $scores.mri_score `
        -MriStatus $scores.mri_status `
        -CtScore $scores.ct_score `
        -CtStatus $scores.ct_status `
        -PetScore $scores.pet_score `
        -PetStatus $scores.pet_status `
        -GovernanceScore $scores.governance_score `
        -RunDir $RunDir `
        -RepoPath $RepoPath `
        -InventoryFiles $measurements.inventory_files `
        -SentruxDsmSummary $SentruxDsmSummary `
        -SentruxFileDetailsSummary $SentruxFileDetailsSummary `
        -CodeNexusContextSummary $CodeNexusContextSummary `
        -SentruxWhatIfSummary $SentruxWhatIfSummary `
        -RuntimeCiSummary $RuntimeCiSummary `
        -GovernanceArtifact $scores.governance_artifact `
        -GovernanceFinding $scores.governance_finding

    $quality = New-HospitalQualityDimensions `
        -SourceCoverageScore $scores.source_coverage_score `
        -SourceScopeStatus $measurements.source_scope_status `
        -InventoryFiles $measurements.inventory_files `
        -ScanFiles $measurements.scan_files `
        -GraphScore $scores.graph_score `
        -UnderstandStep $understandStep `
        -ResolutionScore $scores.resolution_score `
        -ImportResolutionStatus $scores.import_resolution_status `
        -ResolvedImports $measurements.resolved_imports `
        -UnresolvedImports $measurements.unresolved_imports `
        -PollutionScore $scores.pollution_score `
        -PollutionStatus $scores.pollution_status `
        -ExcludedFiles $measurements.excluded_files `
        -GovernanceScore $scores.governance_score `
        -GovernanceStatus $scores.governance_status `
        -GovernanceEvidence $scores.governance_evidence `
        -MriScore $scores.mri_score `
        -LocalizationStatus $scores.localization_status `
        -TopContextFile $evidence.top_context_file `
        -MemoryScore $scores.memory_score `
        -MemoryStatus $scores.memory_status `
        -MemoryEvidence $scores.memory_evidence

    $protocols = New-HospitalProtocolBlock $scores.rules_exists @($evidence.failing_what_if).Count

    return [ordered]@{
        schema = "code-intel-hospital.v1"
        generatedAt = (Get-Date).ToString("o")
        repo = $RepoPath
        mode = $Mode
        artifacts = [ordered]@{
            runDir = $RunDir
            report = $ReportPath
        summary = $SummaryPath
        understanding = $UnderstandingPath
        runtime_ci = if ($null -ne $RuntimeCiSummary) { [string]$RuntimeCiSummary.path } else { "" }
        github_solution_research = if ($null -ne $GitHubResearch) { [string]$GitHubResearch.path } else { "" }
        github_solution_research_markdown = if ($null -ne $GitHubResearch) { [string]$GitHubResearch.markdown } else { "" }
    }
    triage = [ordered]@{
        status = $decision.severity
        disposition = $decision.disposition
        primary_diagnosis = $decision.primaryDiagnosis
        overall_score = $scores.overall_score
        next_protocol = $decision.nextProtocol
        research_status = if ($null -ne $GitHubResearch) { [string]$GitHubResearch.status } else { "not_applicable" }
        research_required = if ($null -ne $GitHubResearch) { [bool]$GitHubResearch.required } else { $false }
        exit_criteria = if ($null -ne $GitHubResearch) { @($GitHubResearch.exitCriteria) } else { @() }
        admission_reason = $decision.admissionReason
        discharge_criteria = $decision.dischargeCriteria
    }
        state_machine = $decision.stateMachine
        modalities = $modalities
        policies = [ordered]@{
            admission = [ordered]@{
                admit_when = @(
                    "local toolchain fails",
                    "architecture graph is missing",
                    "Sentrux rules are missing",
                    "Sentrux check or gate fails",
                    "what-if reports planned modernization debt"
                )
                current_reason = $decision.admissionReason
            }
            discharge = [ordered]@{
                criteria = $decision.dischargeCriteria
                current_state = $decision.stateMachine.current_state
            }
        }
        report_quality = [ordered]@{
            overall_score = $scores.overall_score
            diagnostic_score = $scores.diagnostic_score
            governance_score = $scores.governance_score
            dimensions = $quality
        }
        diagnosis = [ordered]@{
            findings = $findings
            impression = $decision.primaryDiagnosis
            risk = $decision.severity
            evidence = [ordered]@{
                top_function = $evidence.top_function
                top_module = $evidence.top_module
                top_context_file = $evidence.top_context_file
                failing_what_if = @($evidence.failing_what_if | Select-Object -First 5)
            }
        }
        treatment = [ordered]@{
            plan = $decision.treatment
            follow_up = @(
                "Rerun normal mode after code changes.",
                "Compare hospital-report.json overall_score and Sentrux quality signal.",
                "Use session_start/session_end around Agent edits."
            )
        }
        protocols = $protocols
        tools = $ToolState
    }
}

function Convert-HospitalReportToMarkdown {
    param([object]$Hospital)

    $lines = @(
        "# Code Intel Hospital Report",
        "",
        "- Repo: $($Hospital.repo)",
        "- Mode: $($Hospital.mode)",
        "- Status: $($Hospital.triage.status)",
        "- Disposition: $($Hospital.triage.disposition)",
        "- Primary diagnosis: $($Hospital.triage.primary_diagnosis)",
        "- Admission reason: $($Hospital.triage.admission_reason)",
"- Overall score: $($Hospital.triage.overall_score)",
"- Next protocol: $($Hospital.triage.next_protocol)",
"- Research status: $($Hospital.triage.research_status)",
"- Research required: $($Hospital.triage.research_required)",
"- Current state: $($Hospital.state_machine.current_state)",
"",
"## Imaging Modalities"
)
if ($null -ne $Hospital.triage.exit_criteria -and @($Hospital.triage.exit_criteria).Count -gt 0) {
    $lines += ""
    $lines += "## Exit Criteria"
    foreach ($criterion in @($Hospital.triage.exit_criteria)) {
        $lines += "- $criterion"
    }
}
foreach ($item in @($Hospital.modalities)) {
        $lines += "- $($item.name): $($item.status), confidence=$($item.confidence), finding=$($item.finding)"
    }
    $lines += ""
    $lines += "## Report Quality"
    foreach ($dimension in @($Hospital.report_quality.dimensions)) {
        $lines += "- $($dimension.name): $($dimension.score) ($($dimension.status)) - $($dimension.evidence)"
    }
    $lines += ""
    $lines += "## Diagnosis"
    foreach ($finding in @($Hospital.diagnosis.findings)) {
        $lines += "- $finding"
    }
    $lines += ""
    $lines += "## Treatment"
    foreach ($item in @($Hospital.treatment.plan)) {
        $lines += "- $item"
    }
    if ($null -ne $Hospital.surgery_plan) {
        $lines += ""
        $lines += "## Surgery Plan"
        $lines += "- Status: $($Hospital.surgery_plan.status)"
        $lines += "- Report: $($Hospital.surgery_plan.path)"
        $lines += "- Markdown: $($Hospital.surgery_plan.markdown)"
        $lines += "- Primary target: $($Hospital.surgery_plan.primary_target)"
    }
    $lines += ""
    $lines += "## Discharge Criteria"
    foreach ($item in @($Hospital.triage.discharge_criteria)) {
        $lines += "- $item"
    }
    $lines += ""
    $lines += "## State Machine"
    foreach ($transition in @($Hospital.state_machine.transitions)) {
        $lines += "- $($transition.from) -> $($transition.to): pass=$($transition.pass), guard=$($transition.guard)"
    }
    $lines += ""
    $lines += "## Protocols"
    foreach ($protocol in @($Hospital.protocols)) {
        $lines += "- $($protocol.name): $($protocol.status) - $($protocol.exit_criteria)"
    }
    return $lines
}

function Get-CodeIntelSentruxStep {
    param(
        [object[]]$Steps,
        [string]$NamePattern,
        [switch]$Last
    )

    $matches = @($Steps | Where-Object { [string]$_.name -like $NamePattern })
    if ($matches.Count -eq 0) { return $null }
    if ($Last) { return $matches[-1] }
    return $matches[0]
}

function Get-CodeIntelBoundedExcerpt {
    param(
        [string]$Text,
        [int]$MaxLength = 500
    )

    if ([string]::IsNullOrWhiteSpace($Text)) { return "" }
    $singleLine = (($Text -split "`r?`n") | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 8) -join " | "
    if ($singleLine.Length -le $MaxLength) { return $singleLine }
    return $singleLine.Substring(0, $MaxLength)
}

function New-CodeIntelSentruxTarget {
    param(
        [ValidateSet("resolved", "unresolved", "aggregate", "not_applicable")]
        [string]$Status,
        [string]$File = "",
        [string]$Symbol = ""
    )

    $target = [ordered]@{ status = $Status }
    if (-not [string]::IsNullOrWhiteSpace($File)) { $target["file"] = $File }
    if (-not [string]::IsNullOrWhiteSpace($Symbol)) { $target["symbol"] = $Symbol }
    return $target
}

function New-CodeIntelSentruxRecord {
    param(
        [string]$Id,
        [string]$Kind,
        [string]$Source,
        [string]$SourceStep,
        [string]$RawOutputPath,
        [string]$Stdout,
        [object]$Target,
        [string]$Metric = "",
        [Nullable[int]]$Value = $null,
        [Nullable[int]]$Threshold = $null,
        [Nullable[int]]$Before = $null,
        [Nullable[int]]$After = $null
    )

    $record = [ordered]@{
        id = $Id
        kind = $Kind
        source = $Source
        source_step = $SourceStep
        provenance = "stdout"
        raw_output_path = $RawOutputPath
        stdout_excerpt = Get-CodeIntelBoundedExcerpt $Stdout
        parsed_at = (Get-Date).ToString("o")
        target = $Target
    }
    if (-not [string]::IsNullOrWhiteSpace($Metric)) { $record["metric"] = $Metric }
    if ($null -ne $Value) { $record["value"] = [int]$Value }
    if ($null -ne $Threshold) { $record["threshold"] = [int]$Threshold }
    if ($null -ne $Before) { $record["before"] = [int]$Before }
    if ($null -ne $After) { $record["after"] = [int]$After }
    return $record
}

function Get-CodeIntelObjectValue {
    param(
        [object]$Object,
        [string]$Name
    )

    if ($null -eq $Object) { return $null }
    if ($Object -is [System.Collections.IDictionary] -and $Object.Contains($Name)) {
        return $Object[$Name]
    }
    return Get-JsonProperty $Object $Name
}

function New-CodeIntelSentruxConflict {
    param(
        [object]$Authoritative,
        [object]$Conflicting,
        [string]$ConflictingSource,
        [string]$RawPointer
    )

    if ($null -eq $Authoritative -or $null -eq $Conflicting) { return $null }
    $authoritativeValue = ConvertTo-NullableDouble (Get-CodeIntelObjectValue $Authoritative "value")
    $conflictingValue = ConvertTo-NullableDouble (Get-CodeIntelObjectValue $Conflicting "complexity")
    if ($null -eq $authoritativeValue -or $null -eq $conflictingValue) { return $null }
    if ([int]$authoritativeValue -eq [int]$conflictingValue) { return $null }

    $conflictingId = "{0}:max_cc:{1}:{2}" -f $ConflictingSource, [string](Get-CodeIntelObjectValue $Conflicting "file"), [string](Get-CodeIntelObjectValue $Conflicting "name")
    return [ordered]@{
        kind = "metric_conflict"
        authoritative_record_id = [string](Get-CodeIntelObjectValue $Authoritative "id")
        conflicting_record_id = $conflictingId
        metric = "cyclomatic_complexity"
        authoritative_value = [int]$authoritativeValue
        conflicting_value = [int]$conflictingValue
        authoritative_source = [string](Get-CodeIntelObjectValue $Authoritative "source")
        conflicting_source = $ConflictingSource
        raw_output_path = $RawPointer
        stdout_excerpt = Get-CodeIntelBoundedExcerpt ("{0} {1} (cc={2})" -f [string](Get-CodeIntelObjectValue $Conflicting "name"), [string](Get-CodeIntelObjectValue $Conflicting "file"), [string](Get-CodeIntelObjectValue $Conflicting "complexity"))
        parsed_at = (Get-Date).ToString("o")
        resolution = "authoritative_stdout_wins"
    }
}

function New-CodeIntelSentruxFailures {
    param(
        [object[]]$Steps,
        [string]$OutputPath = "",
        [string]$HotspotsPath = "",
        [string]$FileDetailsPath = ""
    )

    $checkStep = Get-CodeIntelSentruxStep -Steps $Steps -NamePattern "sentrux check"
    $gateStep = Get-CodeIntelSentruxStep -Steps $Steps -NamePattern "sentrux gate*" -Last
    $records = [System.Collections.Generic.List[object]]::new()
    $parserNotes = [System.Collections.Generic.List[string]]::new()
    $parserErrors = [System.Collections.Generic.List[string]]::new()

    if ($null -ne $checkStep) {
        $checkStatus = [string]$checkStep.status
        $checkText = (([string]$checkStep.output) + "`n" + ([string]$checkStep.error)).Trim()
        if ($checkStatus -eq "failed" -or $checkStatus -eq "manual_required") {
            $namedMatches = @([regex]::Matches($checkText, "(?im)(?<file>[^\s:()]+(?:\.ps1|\.psm1|\.ts|\.tsx|\.js|\.jsx|\.py|\.rs|\.go|\.cs|\.java|\.kt|\.v)):(?<symbol>[A-Za-z_][A-Za-z0-9_.:-]*)\s*\(cc=(?<cc>\d+)\)"))
            if ($namedMatches.Count -gt 0) {
                foreach ($match in $namedMatches) {
                    $file = [string]$match.Groups["file"].Value
                    $symbol = [string]$match.Groups["symbol"].Value
                    $value = [int]$match.Groups["cc"].Value
                    $records.Add((New-CodeIntelSentruxRecord `
                        -Id ("check:max_cc:{0}:{1}" -f $file, $symbol) `
                        -Kind "max_cc" `
                        -Source "sentrux check" `
                        -SourceStep "sentrux check" `
                        -RawOutputPath "report.json#/steps/sentrux check/output" `
                        -Stdout $checkText `
                        -Metric "cyclomatic_complexity" `
                        -Value $value `
                        -Threshold 70 `
                        -Target (New-CodeIntelSentruxTarget -Status "resolved" -File $file -Symbol $symbol)))
                }
            }
            elseif ($checkText -match "(?i)max[_ -]?cc|cyclomatic|complex") {
                $value = $null
                $valueMatch = [regex]::Match($checkText, "(?i)(?:max[_ -]?cc|cc|cyclomatic[^0-9]*)(?:\D+)(?<cc>\d+)")
                if ($valueMatch.Success) { $value = [int]$valueMatch.Groups["cc"].Value }
                $records.Add((New-CodeIntelSentruxRecord `
                    -Id "check:max_cc:unresolved" `
                    -Kind "max_cc" `
                    -Source "sentrux check" `
                    -SourceStep "sentrux check" `
                    -RawOutputPath "report.json#/steps/sentrux check/output" `
                    -Stdout $checkText `
                    -Metric "cyclomatic_complexity" `
                    -Value $value `
                    -Threshold 70 `
                    -Target (New-CodeIntelSentruxTarget -Status "unresolved")))
            }
            else {
                $parserErrors.Add("sentrux check failed but stdout did not match known max_cc formats.")
            }
        }
    }

    if ($null -ne $gateStep) {
        $gateStatus = [string]$gateStep.status
        $gateText = (([string]$gateStep.output) + "`n" + ([string]$gateStep.error)).Trim()
        if ($gateStatus -eq "failed" -or $gateStatus -eq "manual_required") {
            $gateMatches = @([regex]::Matches($gateText, "(?im)(?<label>Complex functions|God files|Cycles|Coupling|Quality)[^\r\n:]*:\s*(?<before>\d+)\s*(?:->|→)\s*(?<after>\d+)"))
            if ($gateMatches.Count -gt 0) {
                foreach ($match in $gateMatches) {
                    $label = ([string]$match.Groups["label"].Value).ToLowerInvariant().Replace(" ", "_")
                    $records.Add((New-CodeIntelSentruxRecord `
                        -Id ("gate:{0}" -f $label) `
                        -Kind $label `
                        -Source "sentrux gate" `
                        -SourceStep "sentrux gate" `
                        -RawOutputPath "report.json#/steps/sentrux gate/output" `
                        -Stdout $gateText `
                        -Before ([int]$match.Groups["before"].Value) `
                        -After ([int]$match.Groups["after"].Value) `
                        -Target (New-CodeIntelSentruxTarget -Status "aggregate")))
                }
            }
            elseif ($gateStatus -eq "manual_required") {
                $records.Add((New-CodeIntelSentruxRecord `
                    -Id "gate:manual_required" `
                    -Kind "manual_required" `
                    -Source "sentrux gate" `
                    -SourceStep "sentrux gate" `
                    -RawOutputPath "report.json#/steps/sentrux gate/output" `
                    -Stdout $gateText `
                    -Target (New-CodeIntelSentruxTarget -Status "not_applicable")))
            }
            elseif (-not [string]::IsNullOrWhiteSpace($gateText)) {
                $parserErrors.Add("sentrux gate failed but stdout did not match known gate regression formats.")
            }
        }
    }

    $conflicts = [System.Collections.Generic.List[object]]::new()
    $primary = @($records | Where-Object { [string]$_.source -eq "sentrux check" } | Select-Object -First 1)
    if ($primary.Count -gt 0 -and -not [string]::IsNullOrWhiteSpace($HotspotsPath) -and (Test-Path -LiteralPath $HotspotsPath -PathType Leaf)) {
        $hotspots = Read-JsonFileSafe $HotspotsPath
        $topFunction = $null
        if ($null -ne $hotspots -and $null -ne $hotspots.functions -and @($hotspots.functions).Count -gt 0) {
            $topFunction = @($hotspots.functions)[0]
        }
        $conflict = New-CodeIntelSentruxConflict -Authoritative $primary[0] -Conflicting $topFunction -ConflictingSource "sentrux-hotspots" -RawPointer "sentrux-hotspots.json#/functions/0"
        if ($null -ne $conflict) { $conflicts.Add($conflict) }
    }

    $artifactStatus = "ok"
    if ($null -eq $checkStep -and $null -eq $gateStep) {
        $artifactStatus = "not_run"
    }
    elseif (@($Steps | Where-Object { [string]$_.name -like "sentrux*" -and [string]$_.status -eq "skipped" }).Count -gt 0 -and $records.Count -eq 0) {
        $artifactStatus = "skipped"
    }
    elseif (@($Steps | Where-Object { [string]$_.name -like "sentrux*" -and [string]$_.status -eq "manual_required" }).Count -gt 0) {
        $artifactStatus = "manual_required"
    }
    elseif ($records.Count -gt 0) {
        $artifactStatus = if ($parserErrors.Count -gt 0) { "partial" } else { "failed" }
    }
    elseif ($parserErrors.Count -gt 0) {
        $artifactStatus = "unparsed"
    }

    $gate = @($records | Where-Object { [string]$_.source -eq "sentrux gate" } | Select-Object -First 1)
    $artifact = [ordered]@{
        schema = "code-intel-sentrux-failures.v1"
        status = $artifactStatus
        generatedAt = (Get-Date).ToString("o")
        primary = if ($primary.Count -gt 0) { $primary[0] } else { $null }
        gate = if ($gate.Count -gt 0) { $gate[0] } else { $null }
        records = @($records)
        conflicts = @($conflicts)
        parser = [ordered]@{
            status = if ($parserErrors.Count -gt 0) { "partial" } else { "ok" }
            notes = @($parserNotes)
            errors = @($parserErrors)
            enrichment = [ordered]@{
                hotspots = $HotspotsPath
                fileDetails = $FileDetailsPath
            }
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
        $artifact | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
    }
    return $artifact
}

function Get-CodeIntelSentruxFailureSummary {
    param([object]$Failures, [string]$Path = "")

    if ($null -eq $Failures) { return $null }
    return [ordered]@{
        path = $Path
        schema = [string]$Failures.schema
        status = [string]$Failures.status
        primaryId = if ($null -ne $Failures.primary) { [string]$Failures.primary.id } else { "" }
        primaryTargetStatus = if ($null -ne $Failures.primary -and $null -ne $Failures.primary.target) { [string]$Failures.primary.target.status } else { "" }
        gateId = if ($null -ne $Failures.gate) { [string]$Failures.gate.id } else { "" }
        records = @($Failures.records).Count
        conflicts = @($Failures.conflicts).Count
    }
}

function Get-CodeIntelSentruxPrimaryTargetText {
    param([object]$Failures)

    if ($null -eq $Failures -or $null -eq $Failures.primary -or $null -eq $Failures.primary.target) { return "" }
    $target = $Failures.primary.target
    if ([string]$target.status -eq "resolved") {
        return "{0} in {1} (cc={2})" -f [string]$target.symbol, [string]$target.file, [string]$Failures.primary.value
    }
    if ([string]$target.status -eq "unresolved") {
        return "Sentrux check reported max_cc failure without authoritative symbol target"
    }
    return ""
}

function New-CodeIntelSentruxDebtEntry {
    param(
        [object]$Record,
        [string]$Classification,
        [string]$Reason,
        [string]$RunTimestamp
    )

    $target = if ($null -ne $Record -and $null -ne $Record.target) { $Record.target } else { $null }
    return [ordered]@{
        id = if ($null -ne $Record) { [string]$Record.id } else { "" }
        classification = $Classification
        blocking = ($Classification -in @("new_debt", "worsened_debt"))
        reason = $Reason
        firstSeen = $RunTimestamp
        source = if ($null -ne $Record) { [string]$Record.source } else { "" }
        kind = if ($null -ne $Record) { [string]$Record.kind } else { "" }
        value = if ($null -ne (Get-CodeIntelObjectValue $Record "value")) { [int](Get-CodeIntelObjectValue $Record "value") } else { $null }
        threshold = if ($null -ne (Get-CodeIntelObjectValue $Record "threshold")) { [int](Get-CodeIntelObjectValue $Record "threshold") } else { $null }
        before = if ($null -ne (Get-CodeIntelObjectValue $Record "before")) { [int](Get-CodeIntelObjectValue $Record "before") } else { $null }
        after = if ($null -ne (Get-CodeIntelObjectValue $Record "after")) { [int](Get-CodeIntelObjectValue $Record "after") } else { $null }
        target = [ordered]@{
            status = if ($null -ne $target) { [string](Get-CodeIntelObjectValue $target "status") } else { "not_applicable" }
            file = if ($null -ne $target) { [string](Get-CodeIntelObjectValue $target "file") } else { "" }
            symbol = if ($null -ne $target) { [string](Get-CodeIntelObjectValue $target "symbol") } else { "" }
        }
    }
}

function Get-CodeIntelSentruxDebtClassification {
    param([object]$Record)

    if ($null -eq $Record) {
        return [ordered]@{ classification = "informational"; reason = "No Sentrux failure record." }
    }

    $source = [string]$Record.source
    $kind = [string]$Record.kind
    $target = $Record.target
    $targetStatus = if ($null -ne $target) { [string](Get-CodeIntelObjectValue $target "status") } else { "" }
    $targetFile = if ($null -ne $target) { [string](Get-CodeIntelObjectValue $target "file") } else { "" }
    $targetSymbol = if ($null -ne $target) { [string](Get-CodeIntelObjectValue $target "symbol") } else { "" }

    if ($kind -in @("manual_required", "skipped", "unparsed") -or $targetStatus -eq "not_applicable") {
        return [ordered]@{
            classification = "informational"
            reason = "Sentrux record is not an actionable structural debt target."
        }
    }

    if ($source -eq "sentrux check" -and $kind -eq "max_cc" -and
        $targetStatus -eq "resolved" -and
        $targetFile -eq "run-code-intel.ps1" -and
        $targetSymbol -eq "Get-CodeEvidenceSymbols") {
        return [ordered]@{
            classification = "known_debt"
            reason = "Current pipeline historical max_cc debt; tracked but not blocking understanding artifacts."
        }
    }

    if ($source -eq "sentrux check" -and $kind -eq "max_cc" -and $targetStatus -eq "unresolved") {
        return [ordered]@{
            classification = "informational"
            reason = "Aggregate max_cc output has no authoritative symbol target; do not invent a debt owner."
        }
    }

    $before = Get-CodeIntelObjectValue $Record "before"
    $after = Get-CodeIntelObjectValue $Record "after"
    if ($source -eq "sentrux gate" -and $null -ne $before -and $null -ne $after) {
        $beforeNumber = [double]$before
        $afterNumber = [double]$after
        $worsened = if ($kind -eq "quality") {
            $afterNumber -lt $beforeNumber
        }
        else {
            $afterNumber -gt $beforeNumber
        }

        if ($worsened) {
            return [ordered]@{
                classification = "worsened_debt"
                reason = "Sentrux gate reports that this structural metric moved in its regressing direction."
            }
        }

        return [ordered]@{
            classification = "informational"
            reason = "Sentrux gate metric was stable or moved in its improving direction."
        }
    }

    if ($source -eq "sentrux gate" -or $source -eq "sentrux check") {
        return [ordered]@{
            classification = "new_debt"
            reason = "Sentrux reported a structural failure not matched by known historical debt policy."
        }
    }

    return [ordered]@{
        classification = "informational"
        reason = "Sentrux status is informational for blocking policy."
    }
}

function New-CodeIntelSentruxDebtRegister {
    param(
        [object]$Failures,
        [string]$RepoPath = "",
        [string]$RunTimestamp = "",
        [string]$OutputPath = ""
    )

    if ([string]::IsNullOrWhiteSpace($RunTimestamp)) {
        $RunTimestamp = (Get-Date).ToString("o")
    }

    $entries = [System.Collections.Generic.List[object]]::new()
    foreach ($record in @($Failures.records)) {
        $classification = Get-CodeIntelSentruxDebtClassification -Record $record
        $entries.Add((New-CodeIntelSentruxDebtEntry `
            -Record $record `
            -Classification ([string]$classification.classification) `
            -Reason ([string]$classification.reason) `
            -RunTimestamp $RunTimestamp))
    }

    if ($entries.Count -eq 0) {
        $status = if ($null -ne $Failures) { [string]$Failures.status } else { "not_run" }
        if ($status -in @("manual_required", "skipped", "unparsed", "not_run")) {
            $entries.Add((New-CodeIntelSentruxDebtEntry `
                -Record $null `
                -Classification "informational" `
                -Reason "Sentrux status '$status' does not represent actionable structural debt." `
                -RunTimestamp $RunTimestamp))
        }
    }

    $known = @($entries | Where-Object { [string]$_.classification -eq "known_debt" })
    $new = @($entries | Where-Object { [string]$_.classification -eq "new_debt" })
    $worsened = @($entries | Where-Object { [string]$_.classification -eq "worsened_debt" })
    $informational = @($entries | Where-Object { [string]$_.classification -eq "informational" })
    $blocking = @($entries | Where-Object { [bool]$_.blocking })

    $artifact = [ordered]@{
        schema = "code-intel-sentrux-debt-register.v1"
        generatedAt = $RunTimestamp
        repoPath = $RepoPath
        source = "sentrux-failures.json"
        policy = [ordered]@{
            knownDebtBlocks = $false
            blockingClassifications = @("new_debt", "worsened_debt")
            informationalClassifications = @("informational")
        }
        summary = [ordered]@{
            knownDebt = $known.Count
            newDebt = $new.Count
            worsenedDebt = $worsened.Count
            informational = $informational.Count
            blocking = $blocking.Count
        }
        entries = @($entries)
    }

    if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
        $artifact | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
    }
    return $artifact
}

function Get-CodeIntelSentruxDebtSummary {
    param([object]$DebtRegister, [string]$Path = "")

    if ($null -eq $DebtRegister) { return $null }
    return [ordered]@{
        path = $Path
        schema = [string]$DebtRegister.schema
        knownDebt = [int]$DebtRegister.summary.knownDebt
        newDebt = [int]$DebtRegister.summary.newDebt
        worsenedDebt = [int]$DebtRegister.summary.worsenedDebt
        informational = [int]$DebtRegister.summary.informational
        blocking = [int]$DebtRegister.summary.blocking
    }
}

$configData = $null
if ([string]::IsNullOrWhiteSpace($Config)) {
    $Config = Join-Path $PSScriptRoot "pipeline.config.json"
}
if (-not [string]::IsNullOrWhiteSpace($Config)) {
    $configPath = Resolve-Path -LiteralPath $Config -ErrorAction Stop
    $configData = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
}

$repoConfig = $null
$reposConfig = Get-JsonProperty $configData "repos"
if ($null -ne $reposConfig -and [string]::IsNullOrWhiteSpace($RepoPath) -and -not [string]::IsNullOrWhiteSpace($Repo)) {
    $repoConfig = Get-JsonProperty $reposConfig $Repo
}

$repoInput = if (-not [string]::IsNullOrWhiteSpace($RepoPath)) { $RepoPath } else { $Repo }
if ([string]::IsNullOrWhiteSpace($repoInput)) {
    throw "Specify -Repo <alias-or-path> or -RepoPath <path>."
}
if ([string]::IsNullOrWhiteSpace($RepoPath) -and $null -ne $repoConfig) {
    $configuredPath = Get-JsonProperty $repoConfig "path"
    if (-not [string]::IsNullOrWhiteSpace([string]$configuredPath)) {
        $repoInput = [string]$configuredPath
    }
}

$repoPath = Resolve-Repo $repoInput
if ($null -ne $reposConfig -and -not [string]::IsNullOrWhiteSpace($RepoPath)) {
    $repoConfig = Find-RepoConfigByPath -ReposConfig $reposConfig -ResolvedRepoPath $repoPath
}
$repoName = Split-Path -Leaf $repoPath
$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"

if ([string]::IsNullOrWhiteSpace($ArtifactRoot)) {
    $configuredArtifactRoot = Get-JsonProperty $configData "artifactRoot"
    $ArtifactRoot = if ([string]::IsNullOrWhiteSpace([string]$configuredArtifactRoot)) {
        Get-DefaultArtifactRoot
    }
    else {
        [string]$configuredArtifactRoot
    }
}

if ([string]::IsNullOrWhiteSpace($RepowiseWorkspaceRoot)) {
    $configuredWorkspaceRoot = Get-JsonProperty $configData "repowiseWorkspaceRoot"
    $RepowiseWorkspaceRoot = if ([string]::IsNullOrWhiteSpace([string]$configuredWorkspaceRoot)) {
        Split-Path -Parent $repoPath
    }
    else {
        [string]$configuredWorkspaceRoot
    }
}

if ([string]::IsNullOrWhiteSpace($Language)) {
    $configuredLanguage = Get-JsonProperty $repoConfig "language"
    $Language = if ([string]::IsNullOrWhiteSpace([string]$configuredLanguage)) { "zh" } else { [string]$configuredLanguage }
}

if ([string]::IsNullOrWhiteSpace($RepowiseShadowRoot)) {
    $configuredShadowRoot = Get-JsonProperty $repoConfig "repowiseShadowRoot"
    $RepowiseShadowRoot = if ([string]::IsNullOrWhiteSpace([string]$configuredShadowRoot)) {
        Get-DefaultShadowRoot
    }
    else {
        [string]$configuredShadowRoot
    }
}

$defaultRepowiseProvider = if ($RepowiseDocs) { "anthropic" } else { "mock" }
$RepowiseProvider = Normalize-RepowiseProvider (Resolve-ConfigString `
    -Value $RepowiseProvider `
    -RepoConfig $repoConfig `
    -ConfigData $configData `
    -Name "repowiseProvider" `
    -EnvNames @("CODE_INTEL_REPOWISE_PROVIDER", "REPOWISE_PROVIDER") `
    -Default $defaultRepowiseProvider)
$defaultRepowiseModel = if ($RepowiseProvider -ieq "anthropic") { "MiniMax-M2.7" } else { "" }
$RepowiseModel = Resolve-ConfigString `
    -Value $RepowiseModel `
    -RepoConfig $repoConfig `
    -ConfigData $configData `
    -Name "repowiseModel" `
    -EnvNames @("CODE_INTEL_REPOWISE_MODEL", "REPOWISE_MODEL") `
    -Default $defaultRepowiseModel
$RepowiseReasoning = Resolve-ConfigString `
    -Value $RepowiseReasoning `
    -RepoConfig $repoConfig `
    -ConfigData $configData `
    -Name "repowiseReasoning" `
    -EnvNames @("CODE_INTEL_REPOWISE_REASONING", "REPOWISE_REASONING") `
    -Default "auto"
$repowiseProviderArgs = Get-RepowiseProviderArgs -Provider $RepowiseProvider -Model $RepowiseModel -Reasoning $RepowiseReasoning

if ([string]::IsNullOrWhiteSpace($SentruxPath)) {
    $configuredSentruxPath = Get-JsonProperty $repoConfig "sentruxPath"
    $SentruxPath = if ($null -eq $configuredSentruxPath) { "" } else { [string]$configuredSentruxPath }
}

if ($RepowiseScopePaths.Count -eq 0) {
    $configuredScopePaths = Get-JsonProperty $repoConfig "repowiseScopePaths"
    if ($null -ne $configuredScopePaths) {
        $RepowiseScopePaths = @($configuredScopePaths | ForEach-Object { [string]$_ })
    }
}

if ($RepowiseRootFiles.Count -eq 0) {
    $configuredRootFiles = Get-JsonProperty $repoConfig "repowiseRootFiles"
    if ($null -ne $configuredRootFiles) {
        $RepowiseRootFiles = @($configuredRootFiles | ForEach-Object { [string]$_ })
    }
}

if ($RepowiseScopePaths.Count -eq 0 -and $RepowiseRootFiles.Count -eq 0 -and -not [string]::IsNullOrWhiteSpace($SentruxPath)) {
    $RepowiseScopePaths = @($SentruxPath)
}

$configuredExcludes = Get-JsonProperty $configData "inventoryExclude"
if ($InventoryExclude.Count -eq 0 -and $null -ne $configuredExcludes) {
    $InventoryExclude = @($configuredExcludes | ForEach-Object { [string]$_ })
}

$defaultInventoryExclude = @(
    "!**/.git/**",
    "!**/node_modules/**",
    "!**/.repowise/**",
    "!**/.understand-anything/**",
    "!**/.sentrux/**",
    "!**/target/**",
    "!**/dist/**",
    "!**/build/**",
    "!**/.venv/**",
    "!**/__pycache__/**"
)
$allInventoryExclude = @($defaultInventoryExclude + $InventoryExclude | Select-Object -Unique)

$artifactRoot = Join-Path $ArtifactRoot $repoName
$finalRunDir = Join-Path $artifactRoot $timestamp
if (Test-Path -LiteralPath $finalRunDir) {
    $suffix = 1
    do {
        $candidate = Join-Path $artifactRoot ("{0}-{1:D2}" -f $timestamp, $suffix)
        $suffix++
    } while (Test-Path -LiteralPath $candidate)
    $finalRunDir = $candidate
}

$followUpConfig = Get-JsonProperty $configData "followUpAutomation"
$followUpSettings = Resolve-CodeIntelFollowUpSettings -Config $followUpConfig -ProactiveSkillSuggestions $ProactiveSkillSuggestions -AutomaticPullRequests $AutomaticPullRequests -BugSkill $BugSkill
$ProactiveSkillSuggestions = [string]$followUpSettings.proactiveSkillSuggestions
$AutomaticPullRequests = [string]$followUpSettings.automaticPullRequests
$BugSkill = [string]$followUpSettings.bugSkill

if ($DagCoordinate) {
    $dagRoot = $artifactRoot
    New-Item -ItemType Directory -Force -Path $dagRoot | Out-Null
    $dagOut = Join-Path $dagRoot ((Get-Date -Format "yyyyMMdd-HHmmss") + ".dag-staging-" + [guid]::NewGuid().ToString("N").Substring(0, 12))
    $rustCli = $defaultRustCli
    if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
        throw "DAG coordinator binary is missing: $rustCli"
    }
    & $rustCli run dag-coordinate --repo $repoPath --out $dagOut
    if ($LASTEXITCODE -ne 0) {
        throw "DAG coordinator failed with exit code $LASTEXITCODE"
    }
    return
}
$stagingNonce = [guid]::NewGuid().ToString("N").Substring(0, 12)
$runDir = "$finalRunDir.staging-$stagingNonce"
New-Item -ItemType Directory -Force -Path $runDir | Out-Null

$steps = New-Object System.Collections.Generic.List[object]
$notes = New-Object System.Collections.Generic.List[string]
$modelChannel = [ordered]@{
    status = "not_requested"
    category = $null
    routeArtifact = $null
    assistanceDossier = $null
    selectedAdapter = $null
    selectedModel = $null
}
$modelConsumptionConsent = "unanswered"
$modelExternalDataConsent = "unanswered"
$modelPaidSpendConsent = "unanswered"
$modelCostScope = "metered_api"

function New-ModelAssistanceDossier {
    param(
        [string]$Destination,
        [string]$Status,
        [string]$Category,
        [string[]]$Reasons
    )
    $dossier = [ordered]@{
        schema = "code-intel-model-assistance-dossier.v1"
        status = "manual_required"
        category = $Category
        reason = @($Reasons | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Unique)
        artifactRefs = [ordered]@{
            run = $runDir
            fileInventory = (Join-Path $runDir "files.txt")
            report = (Join-Path $finalRunDir "report.json")
            understanding = (Join-Path $finalRunDir "understanding.md")
        }
        minimalContext = [ordered]@{
            repo = $repoPath
            mode = $Mode
            requestedTask = "Generate the optional provider-backed repository documentation from the emitted deterministic code-intel artifacts."
        }
        copyablePrompt = "Using only the explicitly approved repository artifacts, produce the optional documentation analysis. Cite Artifact Refs, distinguish observed facts from inference, and return a machine-readable summary plus Markdown. Do not request or expose credentials."
        resume = [ordered]@{
            command = "pwsh -File run-code-intel.ps1 -RepoPath <repo> -ModelRoutingResult <routing-result.json> -RepowiseDocs"
            requiredState = @("route_ready", "consumption_authorized", "external_data_authorized_if_required", "paid_spend_authorized_if_metered")
            currentStatus = $Status
        }
    }
    [IO.File]::WriteAllText($Destination, ($dossier | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
    return $dossier
}

function Assert-ExactModelProperties {
    param([object]$Object, [string[]]$Names, [string]$Label)
    if ($null -eq $Object) { throw "$Label must be an object" }
    $actual = @($Object.PSObject.Properties.Name | Sort-Object)
    $expected = @($Names | Sort-Object)
    if (($actual -join "`n") -ne ($expected -join "`n")) { throw "$Label must contain exactly: $($Names -join ', ')" }
}

function Assert-ModelEnum {
    param([string]$Value, [string[]]$Allowed, [string]$Label)
    if ($Value -notin $Allowed) { throw "$Label is outside the closed vocabulary" }
}

if (-not [string]::IsNullOrWhiteSpace($ModelRoutingResult)) {
    $routePath = (Resolve-Path -LiteralPath $ModelRoutingResult -ErrorAction Stop).Path
    $route = Get-Content -LiteralPath $routePath -Raw | ConvertFrom-Json -ErrorAction Stop
    Assert-ExactModelProperties $route @("schema", "status", "selected", "authorization", "attempts", "manualAction") "model routing result"
    if ([string]$route.schema -ne "code-intel-model-routing-result.v1") { throw "model routing result schema is unsupported" }
    $routeStatus = [string]$route.status
    Assert-ModelEnum $routeStatus @("ready", "consent_required", "deterministic_degraded") "model routing status"
    Assert-ExactModelProperties $route.authorization @("consumptionAuthorization", "externalData", "paidSpend") "model routing authorization"
    Assert-ExactModelProperties $route.authorization.consumptionAuthorization @("status", "scopes") "consumption authorization"
    Assert-ExactModelProperties $route.authorization.externalData @("status") "external-data authorization"
    Assert-ExactModelProperties $route.authorization.paidSpend @("status") "paid-spend authorization"
    $modelConsumptionConsent = [string]$route.authorization.consumptionAuthorization.status
    $modelExternalDataConsent = [string]$route.authorization.externalData.status
    $modelPaidSpendConsent = [string]$route.authorization.paidSpend.status
    foreach ($statusValue in @($modelConsumptionConsent, $modelExternalDataConsent, $modelPaidSpendConsent)) {
        Assert-ModelEnum $statusValue @("unanswered", "granted", "denied") "model authorization status"
    }
    $allowedScopes = @("local_compute", "subscription_cli", "free_or_internal_quota", "metered_api")
    $authorizationScopes = @($route.authorization.consumptionAuthorization.scopes | ForEach-Object { [string]$_ })
    foreach ($scope in $authorizationScopes) { Assert-ModelEnum $scope $allowedScopes "authorized consumption scope" }
    if (@($authorizationScopes | Sort-Object -Unique).Count -ne $authorizationScopes.Count) { throw "authorized consumption scopes must be unique" }
    $selected = $route.selected
    $routeCategory = $null
    $attemptReasons = [Collections.Generic.List[string]]::new()
    foreach ($attempt in @($route.attempts)) {
        Assert-ExactModelProperties $attempt @("candidateId", "readinessState", "eligible", "failureCategory", "reason") "model routing attempt"
        if ([string]::IsNullOrWhiteSpace([string]$attempt.candidateId) -or [string]::IsNullOrWhiteSpace([string]$attempt.reason)) { throw "model routing attempt ids and reasons must be non-empty" }
        Assert-ModelEnum ([string]$attempt.readinessState) @("discovered", "executable_verified", "auth_present", "model_available", "egress_allowed", "spend_allowed", "ready") "model readiness state"
        if ($attempt.eligible -isnot [bool]) { throw "model routing attempt eligible must be boolean" }
        if ($null -ne $attempt.failureCategory) {
            Assert-ModelEnum ([string]$attempt.failureCategory) @("consent_required", "model_unavailable", "provider_unavailable", "provider_quota", "config_error", "local_tool_error", "adapter_protocol_error", "external_data_forbidden", "paid_usage_forbidden") "model failure category"
            if ($null -eq $routeCategory) { $routeCategory = [string]$attempt.failureCategory }
        }
        $attemptReasons.Add("$([string]$attempt.candidateId):$([string]$attempt.reason)")
    }
    if ($routeStatus -ne "ready" -and $null -eq $routeCategory) { $routeCategory = "model_unavailable" }
    $modelChannel.status = $routeStatus
    $modelChannel.category = $routeCategory
    $modelChannel.routeArtifact = $routePath
    if ($null -ne $selected) {
        Assert-ExactModelProperties $selected @("candidateId", "channelKind", "provider", "model", "costScope", "readinessState") "selected model route"
        if ([string]::IsNullOrWhiteSpace([string]$selected.candidateId)) { throw "selected model candidateId must be non-empty" }
        if ($null -ne $selected.provider -and [string]::IsNullOrWhiteSpace([string]$selected.provider)) { throw "selected provider must be null or non-empty" }
        if ($null -ne $selected.model -and [string]::IsNullOrWhiteSpace([string]$selected.model)) { throw "selected model must be null or non-empty" }
        Assert-ModelEnum ([string]$selected.channelKind) @("local_compatible", "ollama", "claude_cli", "opencode_cli", "codex_cli") "selected channel kind"
        if ([string]$selected.readinessState -ne "ready") { throw "selected model route must have readinessState=ready" }
        $modelChannel.selectedAdapter = [string]$selected.channelKind
        $modelChannel.selectedModel = if ($null -eq $selected.model) { $null } else { [string]$selected.model }
        $modelCostScope = [string]$selected.costScope
        Assert-ModelEnum $modelCostScope $allowedScopes "selected cost scope"
        if ($null -ne $selected.provider -and [string]::IsNullOrWhiteSpace($RepowiseProvider)) { $RepowiseProvider = [string]$selected.provider }
        if ($null -ne $selected.model -and [string]::IsNullOrWhiteSpace($RepowiseModel)) { $RepowiseModel = [string]$selected.model }
    }
    if ($routeStatus -eq "ready" -and $null -eq $selected) { throw "ready model route requires selected" }
    if ($routeStatus -ne "ready" -and $null -ne $selected) { throw "non-ready model route must not select a candidate" }
    if ($routeStatus -eq "ready" -and $null -ne $route.manualAction) { throw "ready model route must not require manual action" }
    if ($routeStatus -eq "consent_required" -and [string]$route.manualAction -ne "obtain_explicit_authorization") { throw "consent-required route has an invalid manual action" }
    if ($routeStatus -eq "deterministic_degraded" -and [string]$route.manualAction -ne "provide_or_enable_model_channel") { throw "degraded route has an invalid manual action" }
    if ($routeStatus -eq "ready" -and $RepowiseDocs -and ([string]$selected.channelKind -eq "claude_cli" -or $null -eq $selected.provider)) {
        $routeStatus = "deterministic_degraded"
        $routeCategory = "config_error"
        $modelChannel.status = $routeStatus
        $modelChannel.category = $routeCategory
        $attemptReasons.Add("$([string]$selected.candidateId):selected route is delegate-only for this workload; invoke -ModelAdapterRequest or select a Repowise-compatible provider route")
    }
    if ($routeStatus -ne "ready") {
        $RepowiseDocs = $false
        $reasons = if ($attemptReasons.Count -gt 0) { @($attemptReasons) } else { @([string]$route.manualAction) }
        $dossierPath = Join-Path $runDir "model-assistance-dossier.json"
        [void](New-ModelAssistanceDossier -Destination $dossierPath -Status $routeStatus -Category $routeCategory -Reasons $reasons)
        $modelChannel.assistanceDossier = $dossierPath
        $steps.Add([pscustomobject][ordered]@{
            name = "provider-backed documentation"
            startedAt = (Get-Date).ToString("o")
            status = "manual_required"
            exitCode = 0
            output = "deterministic_degraded; category=$routeCategory; dossier=$dossierPath"
            error = ""
            finishedAt = (Get-Date).ToString("o")
            durationMs = 0
        })
        $notes.Add("Optional model work is deterministic_degraded; deterministic inventory, graph, Sentrux, hospital, and registry work continues.")
    }
}
elseif ($RepowiseDocs) {
    $RepowiseDocs = $false
    $modelChannel.status = "consent_required"
    $modelChannel.category = "consent_required"
    $dossierPath = Join-Path $runDir "model-assistance-dossier.json"
    [void](New-ModelAssistanceDossier -Destination $dossierPath -Status "consent_required" -Category "consent_required" -Reasons @("No model routing result with explicit consumption authorization was supplied."))
    $modelChannel.assistanceDossier = $dossierPath
    $steps.Add([pscustomobject][ordered]@{
        name = "provider-backed documentation"; startedAt = (Get-Date).ToString("o"); status = "manual_required"; exitCode = 0
        output = "deterministic_degraded; category=consent_required; dossier=$dossierPath"; error = ""
        finishedAt = (Get-Date).ToString("o"); durationMs = 0
    })
    $notes.Add("Provider-backed docs were not invoked because consumption authorization is unanswered; deterministic work continues.")
}

$toolState = [ordered]@{
    rg = Test-CommandAvailable "rg"
    repowise = Test-CommandAvailable "repowise"
    repomix = Test-CommandAvailable "repomix"
    sentrux = Test-CommandAvailable "sentrux"
    git = Test-CommandAvailable "git"
}

# Historical options now map to the standalone advisory atom: Skip disables it and
# Auto is recorded as a compatibility option. The atom never prompts or initializes tools.
$workflowStackResult = $null
$openSpecResult = $null
if (-not $SkipOpenSpec) {
    $rustCli = $defaultRustCli
    if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
        throw "Workflow recommendation capability binary not found: $rustCli"
    }
    $snapshotText = (& $rustCli snapshot identity --repo $repoPath --working-tree-policy explicit_overlay --scope . | Out-String)
    if ($LASTEXITCODE -ne 0) { throw "Workflow recommendation snapshot failed with exit code $LASTEXITCODE" }
    $workflowSnapshot = $snapshotText | ConvertFrom-Json
    $workflowRequest = [ordered]@{
        schema = "code-intel-capability-request.v1"
        capability = "advisory.workflow-recommend"
        contractVersion = 1
        implementation = [ordered]@{
            id = "advisory.workflow-recommend.compat"
            version = "1.0.0"
            toolchainDigests = @(
                "03d9cbed70d83c59f7d9540fccc606ce0b2723135efd2c5e32943d367008a199",
                "748c8b087c9d1a68f9aa5711cda200204ac0d05845058a1ee50058b161582de9"
            )
        }
        snapshot = $workflowSnapshot.snapshot
        options = [ordered]@{ repoPath = $repoPath; auto = [bool]$AutoOpenSpec }
        inputs = @()
        effectPolicy = [ordered]@{ allowedEffects = @() }
    }
    $workflowRequestPath = Join-Path $runDir "workflow-recommendation.request.json"
    $workflowOut = Join-Path $runDir "workflow-recommendation"
    [IO.File]::WriteAllText($workflowRequestPath, ($workflowRequest | ConvertTo-Json -Depth 20 -Compress), [Text.UTF8Encoding]::new($false))
    $workflowEnvelopeText = (& $rustCli capability exec advisory.workflow-recommend --request $workflowRequestPath --out $workflowOut | Out-String)
    if ($LASTEXITCODE -ne 0) { throw "Workflow recommendation capability failed with exit code $LASTEXITCODE" }
    $workflowEnvelope = $workflowEnvelopeText | ConvertFrom-Json
    if (@($workflowEnvelope.declaredEffects).Count -ne 0 -or @($workflowEnvelope.observedEffects).Count -ne 0) {
        throw "Workflow recommendation capability violated its zero-effect envelope"
    }
    $workflowStackResult = Get-Content -LiteralPath (Join-Path $workflowOut "workflow-recommendation.json") -Raw | ConvertFrom-Json -AsHashtable
    $openSpecResult = $workflowStackResult.recommendation
    $notes.Add("Spec-driven score: $($openSpecResult.score)/100 ($($openSpecResult.verdict), tool=$($openSpecResult.candidate))")
}
else {
    $openSpecResult = @{
        stack = "spec-driven"
        tool = $null
        verdict = "skipped"
        score = 0
        reasons = @("Skipped via -SkipOpenSpec")
        entrySkills = @()
    }
}

if (-not $toolState.rg) {
    throw "Missing required tool: rg"
}

if ($RepowiseDocs -and -not $SkipRepowise) {
    $providerProbeScript = Join-Path $PSScriptRoot "Invoke-RepowiseProviderProbe.ps1"
    $preflightStep = Invoke-LoggedStep "repowise provider health" {
        if (-not (Test-Path -LiteralPath $providerProbeScript -PathType Leaf)) {
            throw "Repowise provider health probe not found: $providerProbeScript"
        }
        & $providerProbeScript -Json -Provider $RepowiseProvider -Model $RepowiseModel
    }
    $steps.Add($preflightStep)
    if ($preflightStep.status -ne "passed") {
        $RepowiseDocs = $false
        $notes.Add("Repowise docs disabled because provider health failed. Index-only repowise will still run.")
    }
}

if (-not $toolState.git) {
    $steps.Add([pscustomobject][ordered]@{
        name = "git status"
        startedAt = (Get-Date).ToString("o")
        status = "skipped"
        exitCode = $null
        output = "git not found"
        error = ""
        finishedAt = (Get-Date).ToString("o")
        durationMs = 0
    })
}
elseif (-not (Test-GitRepository $repoPath)) {
    $steps.Add([pscustomobject][ordered]@{
        name = "git status"
        startedAt = (Get-Date).ToString("o")
        status = "skipped"
        exitCode = $null
        output = "Not a git repository: $repoPath"
        error = ""
        finishedAt = (Get-Date).ToString("o")
        durationMs = 0
    })
}
else {
    $steps.Add((Invoke-LoggedStep "git status" {
        git -C $repoPath status --short --branch
    }))
}

$steps.Add((Invoke-LoggedStep "rg file inventory" {
    $rgArgs = @("--files", "--hidden")
    foreach ($pattern in $allInventoryExclude) {
        $rgArgs += @("-g", $pattern)
    }
    Push-Location -LiteralPath $repoPath
    try {
        # Run from the repository authority so ripgrep emits relocation-safe relative paths.
        $files = @(& rg @rgArgs)
    }
    finally {
        Pop-Location
    }

    $fileListPath = Join-Path $runDir "files.txt"
    $files | Set-Content -LiteralPath $fileListPath -Encoding UTF8
"files=$($files.Count)"
}))

$inventoryFileListPath = Join-Path $runDir "files.txt"
$inventoryFiles = if (Test-Path -LiteralPath $inventoryFileListPath -PathType Leaf) {
    @(Get-Content -LiteralPath $inventoryFileListPath)
} else {
    @()
}
$codeEvidenceConfig = Get-JsonProperty $configData "codeEvidence"
$codeEvidence = New-CodeEvidenceLayer -RepoPath $repoPath -RunDir $runDir -Files $inventoryFiles -CodeEvidenceConfig $codeEvidenceConfig
$repomixConfig = Get-JsonProperty $configData "repomix"
if ($null -ne $repomixConfig) {
    $configuredRepomixStyle = Get-JsonProperty $repomixConfig "style"
    if (-not [string]::IsNullOrWhiteSpace([string]$configuredRepomixStyle) -and [string]$configuredRepomixStyle -in @("xml", "markdown", "json", "plain")) {
        $RepomixStyle = [string]$configuredRepomixStyle
    }
    $configuredRepomixCompress = Get-JsonProperty $repomixConfig "compress"
    if ($null -ne $configuredRepomixCompress) {
        $RepomixCompress = [bool]$configuredRepomixCompress
    }
    $configuredRepomixEnabled = Get-JsonProperty $repomixConfig "enabled"
    if ($null -ne $configuredRepomixEnabled -and -not [bool]$configuredRepomixEnabled) {
        $SkipRepomix = $true
    }
}
$repomixPack = [ordered]@{
    schema = "code-intel-repomix-pack.v1"
    status = "skipped"
    reason = "repomix disabled or unavailable"
    style = $RepomixStyle
    path = ""
    summaryPath = ""
}
$repomixPack["reason"] = if ($SkipRepomix) {
    "Skipped by -SkipRepomix."
} else {
    "Repomix production participation was reviewed and removed; no pinned executable or production conformance evidence is present."
}

$nodeLintHygieneStep = Get-NodeLintHygieneStep -RepoPath $repoPath -RgAvailable $toolState.rg
$steps.Add($nodeLintHygieneStep)
if ($nodeLintHygieneStep.status -eq "manual_required") {
    $notes.Add([string]$nodeLintHygieneStep.output)
}

$understandDir = Join-Path $repoPath ".understand-anything"
$knowledgeGraph = Join-Path $understandDir "knowledge-graph.json"
$understandCommand = "/understand $repoPath --language $Language"
if ($Mode -eq "full") {
    $understandCommand = "$understandCommand --full"
}

if (Test-Path -LiteralPath $knowledgeGraph) {
    $graphItem = Get-Item -LiteralPath $knowledgeGraph
    $notes.Add("Understand graph found: $knowledgeGraph")
    $steps.Add([pscustomobject][ordered]@{
        name = "understand graph"
        startedAt = (Get-Date).ToString("o")
        status = "passed"
        exitCode = 0
        output = "path=$knowledgeGraph; bytes=$($graphItem.Length); updated=$($graphItem.LastWriteTime.ToString("o"))"
        error = ""
        finishedAt = (Get-Date).ToString("o")
        durationMs = 0
    })
}
else {
    $message = "Understand graph missing. Run in Claude: $understandCommand"
    $notes.Add($message)
    $status = if ($RequireUnderstandGraph) { "failed" } else { "manual_required" }
    $steps.Add([pscustomobject][ordered]@{
        name = "understand graph"
        startedAt = (Get-Date).ToString("o")
        status = $status
        exitCode = if ($RequireUnderstandGraph) { 1 } else { 0 }
        output = $message
        error = if ($RequireUnderstandGraph) { "knowledge-graph.json is required but missing" } else { "" }
        finishedAt = (Get-Date).ToString("o")
        durationMs = 0
    })
}

if (-not $SkipRepowise) {
    if (-not $toolState.repowise) {
        $steps.Add([pscustomobject][ordered]@{
            name = "repowise"
            startedAt = (Get-Date).ToString("o")
            status = "skipped"
            exitCode = $null
            output = ""
            error = "repowise not found"
            finishedAt = (Get-Date).ToString("o")
            durationMs = 0
        })
    }
    else {
        if (($RepowiseScopePaths.Count -gt 0 -or $RepowiseRootFiles.Count -gt 0) -and -not $AllowRepowiseShadowMutation) {
            $steps.Add([pscustomobject][ordered]@{
                name = "repowise scoped authority"
                startedAt = (Get-Date).ToString("o")
                status = "skipped"
                exitCode = $null
                output = ""
                error = "Scoped Repowise requires explicit -AllowRepowiseShadowMutation authority."
                finishedAt = (Get-Date).ToString("o")
                durationMs = 0
            })
        }
        elseif ($RepowiseScopePaths.Count -gt 0 -or $RepowiseRootFiles.Count -gt 0) {
            $scopedRepowiseScript = Join-Path $PSScriptRoot "Invoke-ScopedRepowise.ps1"
            if ($RepowiseDocs -and $Mode -ne "lite") {
                $repowiseStep = Invoke-LoggedStep "repowise scoped docs" {
                    & $scopedRepowiseScript `
                        -RepoPath $repoPath `
                        -Platform $effectivePlatform `
                        -ShadowRoot $RepowiseShadowRoot `
                        -ScopePaths $RepowiseScopePaths `
                        -RootFiles $RepowiseRootFiles `
                        -AllowShadowWorktreeMutation `
                        -TimeoutSeconds $RepowiseTimeoutSeconds `
                        -Provider $RepowiseProvider `
                        -Model $RepowiseModel `
                        -Reasoning $RepowiseReasoning `
                        -ConsumptionConsent $modelConsumptionConsent `
                        -ExternalDataConsent $modelExternalDataConsent `
                        -PaidSpendConsent $modelPaidSpendConsent `
                        -CostScope $modelCostScope `
                        -Docs
                }
                $steps.Add((Convert-OptionalRepowiseTimeout $repowiseStep))
            }
            else {
                $repowiseStep = Invoke-LoggedStep "repowise scoped index" {
                    & $scopedRepowiseScript `
                        -RepoPath $repoPath `
                        -Platform $effectivePlatform `
                        -ShadowRoot $RepowiseShadowRoot `
                        -ScopePaths $RepowiseScopePaths `
                        -RootFiles $RepowiseRootFiles `
                        -AllowShadowWorktreeMutation `
                        -TimeoutSeconds $RepowiseTimeoutSeconds `
                        -Provider $RepowiseProvider `
                        -Model $RepowiseModel `
                        -Reasoning $RepowiseReasoning
                }
                $steps.Add((Convert-OptionalRepowiseTimeout $repowiseStep))
            }
        }
        else {
            Push-Location $repoPath
            try {
                $steps.Add((Invoke-LoggedStep "repowise status" {
                    repowise status --no-workspace
                }))

                if ($Mode -ne "lite") {
                    $repowiseDir = Join-Path $repoPath ".repowise"
                    $repowiseWorkspacePath = Join-Path $repoPath ".repowise-workspace.yaml"
                    $repowiseStatePath = Join-Path $repowiseDir "state.json"
                    $repowiseDbPath = Join-Path $repowiseDir "wiki.db"
                    $hasRepowiseState = (Test-Path -LiteralPath $repowiseStatePath -PathType Leaf) -or (Test-Path -LiteralPath $repowiseDbPath -PathType Leaf)
                    $hasRepowiseWorkspace = Test-Path -LiteralPath $repowiseWorkspacePath -PathType Leaf

                    if ($hasRepowiseState -and $hasRepowiseWorkspace) {
                        $steps.Add((Invoke-LoggedStep "repowise update" {
                            repowise update --workspace --index-only @repowiseProviderArgs
                        }))
                    }
                    elseif ($hasRepowiseState) {
                        $steps.Add((Invoke-LoggedStep "repowise update" {
                            repowise update --no-workspace --index-only @repowiseProviderArgs
                        }))
                    }
                    else {
                        $steps.Add((Invoke-LoggedStep "repowise init" {
                            repowise init . --index-only -y --no-claude-md --no-onboarding --embedder mock @repowiseProviderArgs
                        }))
                    }

                    if ($RepowiseDocs) {
                        $steps.Add((Invoke-LoggedStep "repowise docs" {
                            repowise update --docs --no-workspace @repowiseProviderArgs
                        }))
                    }
                }
            }
            finally {
                Pop-Location
            }
        }

        if ($WorkspaceAdd) {
            Push-Location $RepowiseWorkspaceRoot
            try {
                $steps.Add((Invoke-LoggedStep "repowise workspace add" {
                    repowise workspace add $repoPath
                }))
            }
            finally {
                Pop-Location
            }
        }
    }
}

$sentruxTargetPath = ""
$sentruxDir = ""
$baselinePath = ""

if ($Mode -eq "lite") {
    $steps.Add([pscustomobject][ordered]@{
        name = "sentrux"
        startedAt = (Get-Date).ToString("o")
        status = "skipped"
        exitCode = $null
        output = "Skipped in lite mode"
        error = ""
        finishedAt = (Get-Date).ToString("o")
        durationMs = 0
    })
}
elseif (-not $SkipSentrux) {
    if (-not $toolState.sentrux) {
        $steps.Add([pscustomobject][ordered]@{
            name = "sentrux"
            startedAt = (Get-Date).ToString("o")
            status = "skipped"
            exitCode = $null
            output = ""
            error = "sentrux not found"
            finishedAt = (Get-Date).ToString("o")
            durationMs = 0
        })
    }
    else {
        $sentruxTargetPath = Resolve-ChildPath $repoPath $SentruxPath
        $sentruxDir = Join-Path $sentruxTargetPath ".sentrux"
        $hasSentruxConfig = Test-Path -LiteralPath (Join-Path $sentruxDir "rules.toml")
        $baselinePath = Join-Path $sentruxDir "baseline.json"

        if ($hasSentruxConfig -and -not $SkipSentruxCheck) {
            $steps.Add((Invoke-LoggedStep "sentrux check" {
                sentrux check $sentruxTargetPath
            }))
        }
        else {
            $reason = if ($SkipSentruxCheck) { "Skipped by -SkipSentruxCheck" } else { "No .sentrux/rules.toml found" }
            $notes.Add("$reason. sentrux check skipped for $sentruxTargetPath.")
            $steps.Add([pscustomobject][ordered]@{
                name = "sentrux check"
                startedAt = (Get-Date).ToString("o")
                status = "skipped"
                exitCode = $null
                output = $reason
                error = ""
                finishedAt = (Get-Date).ToString("o")
                durationMs = 0
            })
        }

        if ($SkipSentruxGate) {
            $steps.Add([pscustomobject][ordered]@{
                name = "sentrux gate"
                startedAt = (Get-Date).ToString("o")
                status = "skipped"
                exitCode = $null
                output = "Skipped by -SkipSentruxGate"
                error = ""
                finishedAt = (Get-Date).ToString("o")
                durationMs = 0
            })
        }
        elseif ($SaveSentruxBaseline -or ($AutoSaveMissingSentruxBaseline -and -not (Test-Path -LiteralPath $baselinePath))) {
            $previousBaseline = $null
            $baselinePrevPath = Join-Path $sentruxDir "baseline.prev.json"
            if (Test-Path -LiteralPath $baselinePath -PathType Leaf) {
                $previousBaseline = Read-JsonFileSafe $baselinePath
                Copy-Item -LiteralPath $baselinePath -Destination $baselinePrevPath -Force
            }

            $steps.Add((Invoke-LoggedStep "sentrux gate save" {
                sentrux gate --save $sentruxTargetPath
            }))

            $newBaseline = Read-JsonFileSafe $baselinePath
            $oldQuality = if ($null -ne $previousBaseline -and $null -ne $previousBaseline.PSObject.Properties["quality_signal"]) { $previousBaseline.quality_signal } else { "n/a" }
            $newQuality = if ($null -ne $newBaseline -and $null -ne $newBaseline.PSObject.Properties["quality_signal"]) { $newBaseline.quality_signal } else { "n/a" }
            Write-Host "Sentrux baseline saved: quality_signal $oldQuality -> $newQuality"
        }
        elseif (-not (Test-Path -LiteralPath $baselinePath)) {
            $message = "Sentrux baseline missing at $baselinePath. Re-run with -SaveSentruxBaseline or -AutoSaveMissingSentruxBaseline."
            $notes.Add($message)
            $steps.Add([pscustomObject][ordered]@{
                name = "sentrux gate"
                startedAt = (Get-Date).ToString("o")
                status = "manual_required"
                exitCode = 0
                output = $message
                error = ""
                finishedAt = (Get-Date).ToString("o")
                durationMs = 0
            })
        }
        else {
            $steps.Add((Invoke-LoggedStep "sentrux gate" {
                sentrux gate $sentruxTargetPath
            }))
        }
    }
}

$failed = @($steps | Where-Object { $_.status -eq "failed" })
$manual = @($steps | Where-Object { $_.status -eq "manual_required" })
$failureClassifications = @(
    $steps |
    ForEach-Object {
        $category = Get-StepFailureCategory $_
        if ($null -ne $category) {
            [pscustomobject]@{
                category = $category
                step = $_.name
                status = $_.status
                detail = if (-not [string]::IsNullOrWhiteSpace([string]$_.error)) { [string]$_.error } else { [string]$_.output }
            }
        }
    } |
    Where-Object { $null -ne $_ }
)
$failureCounts = [ordered]@{
    providerQuota = @($failureClassifications | Where-Object { $_.category -eq "provider_quota" }).Count
    providerUnavailable = @($failureClassifications | Where-Object { $_.category -eq "provider_unavailable" }).Count
    configError = @($failureClassifications | Where-Object { $_.category -eq "config_error" }).Count
    localToolError = @($failureClassifications | Where-Object { $_.category -eq "local_tool_error" }).Count
    graphMissing = @($failureClassifications | Where-Object { $_.category -eq "graph_missing" }).Count
    sentruxFail = @($failureClassifications | Where-Object { $_.category -eq "sentrux_fail" }).Count
}

$preliminarySentruxFailures = New-CodeIntelSentruxFailures -Steps $steps
$preliminarySentruxDebtRegister = New-CodeIntelSentruxDebtRegister `
    -Failures $preliminarySentruxFailures `
    -RepoPath $repoPath `
    -RunTimestamp $timestamp
$effectiveFailureCounts = [ordered]@{
    providerQuota = [int]$failureCounts.providerQuota
    providerUnavailable = [int]$failureCounts.providerUnavailable
    configError = [int]$failureCounts.configError
    localToolError = [int]$failureCounts.localToolError
    graphMissing = [int]$failureCounts.graphMissing
    sentruxFail = [int]$preliminarySentruxDebtRegister.summary.blocking
}
$effectiveFailed = @(Get-CodeIntelEffectiveFailedSteps `
    -FailedSteps $failed `
    -BlockingSentruxDebt ([int]$preliminarySentruxDebtRegister.summary.blocking))
# R08 reviewed retirement: the representative authenticated blocker query was not
# reproducible. Production runs remain local-only and expose no GitHub call site.
$githubResearch = New-GitHubSolutionResearchNotApplicable

$sentruxInsight = New-SentruxInsight -RepoName $repoName -TargetPath $sentruxTargetPath -BaselinePath $baselinePath -Steps $steps
$sentruxDsmPath = Join-Path $runDir "sentrux-dsm.json"
$sentruxFileDetailsPath = Join-Path $runDir "sentrux-file-details.json"
$sentruxHotspotsPath = Join-Path $runDir "sentrux-hotspots.json"
$sentruxEvolutionPath = Join-Path $runDir "sentrux-evolution.json"
$sentruxWhatIfPath = Join-Path $runDir "sentrux-what-if.json"
$codeNexusContextPath = Join-Path $runDir "codenexus-context.json"
$sentruxDsmSummary = $null
$sentruxFileDetailsSummary = $null
$sentruxHotspotsSummary = $null
$sentruxEvolutionSummary = $null
$sentruxWhatIfSummary = $null
$codeNexusContextSummary = $null
$sentruxAgentTool = Join-Path $PSScriptRoot "Invoke-SentruxAgentTool.ps1"
if (-not [string]::IsNullOrWhiteSpace($sentruxTargetPath) -and (Test-Path -LiteralPath $sentruxAgentTool -PathType Leaf)) {
    try {
        $sentruxDsmPreference = if ([string]::IsNullOrWhiteSpace($env:CODE_INTEL_SENTRUX_DSM_PROVIDER)) {
            "rust"
        } else {
            $env:CODE_INTEL_SENTRUX_DSM_PROVIDER.Trim().ToLowerInvariant()
        }
        if ($sentruxDsmPreference -notin @("rust", "powershell")) {
            throw "CODE_INTEL_SENTRUX_DSM_PROVIDER must be 'rust' or 'powershell'"
        }
        $sentruxDsmRustCli = if ([string]::IsNullOrWhiteSpace($env:CODE_INTEL_RUST_CLI)) {
            $defaultRustCli
        } else {
            [IO.Path]::GetFullPath($env:CODE_INTEL_RUST_CLI)
        }
        $dsmObject = $null
        $sentruxDsmProvider = ""
        if ($sentruxDsmPreference -eq "rust" -and (Test-Path -LiteralPath $sentruxDsmRustCli -PathType Leaf)) {
            $previousErrorActionPreference = $ErrorActionPreference
            $dsmRaw = @()
            $dsmExitCode = $null
            $dsmLaunchError = $null
            try {
                $ErrorActionPreference = "Continue"
                $global:LASTEXITCODE = 0
                try {
                    $dsmRaw = & $sentruxDsmRustCli sentrux dsm $sentruxTargetPath 2>&1
                    $dsmExitCode = $global:LASTEXITCODE
                }
                catch {
                    $dsmLaunchError = $_.Exception.Message
                }
            }
            finally {
                $ErrorActionPreference = $previousErrorActionPreference
            }
            if (-not [string]::IsNullOrWhiteSpace($dsmLaunchError)) {
                $notes.Add("Rust Sentrux DSM could not be launched ($dsmLaunchError); using the explicit PowerShell compatibility fallback.")
            }
            elseif ($dsmExitCode -eq 0) {
                $dsmText = ($dsmRaw | ForEach-Object { $_.ToString() } | Out-String).Trim()
                try {
                    $dsmObject = $dsmText | ConvertFrom-Json
                    $sentruxDsmProvider = "rust"
                }
                catch {
                    $notes.Add("Rust Sentrux DSM returned invalid JSON; using the explicit PowerShell compatibility fallback.")
                }
            }
            else {
                $notes.Add("Rust Sentrux DSM exited with code $dsmExitCode; using the explicit PowerShell compatibility fallback.")
            }
        }
        elseif ($sentruxDsmPreference -eq "rust") {
            $notes.Add("Rust Sentrux DSM binary was unavailable at $sentruxDsmRustCli; using the explicit PowerShell compatibility fallback.")
        }
        if ($null -eq $dsmObject) {
            $previousErrorActionPreference = $ErrorActionPreference
            try {
                $ErrorActionPreference = "Continue"
                $dsmRaw = & $sentruxAgentTool dsm $sentruxTargetPath 2>&1
            }
            finally {
                $ErrorActionPreference = $previousErrorActionPreference
            }
            $dsmText = ($dsmRaw | ForEach-Object { $_.ToString() } | Out-String).Trim()
            $dsmObject = $dsmText | ConvertFrom-Json
            $sentruxDsmProvider = "powershell_compatibility"
        }
        $notes.Add("Sentrux DSM provider: $sentruxDsmProvider")
        $fileDetails = @($dsmObject.file_details)
        $dsmObject.PSObject.Properties.Remove("file_details")
        $dsmObject | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $sentruxDsmPath -Encoding UTF8
        $functionCount = 0
        $maxFunctionComplexity = 0
        $hotspotFile = $null
        foreach ($file in $fileDetails) {
            $functionCount += [int]$file.function_count
            if ([int]$file.max_complexity -gt $maxFunctionComplexity) {
                $maxFunctionComplexity = [int]$file.max_complexity
                $hotspotFile = [string]$file.path
            }
        }
        $fileDetailsPayload = [ordered]@{
            tool = "file_details"
            path = $sentruxTargetPath
            generated_from = $sentruxDsmPath
            files = $fileDetails
        }
        $fileDetailsPayload | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $sentruxFileDetailsPath -Encoding UTF8
        $moduleHotspots = @($dsmObject.modules |
            Sort-Object { $_.colors.Risk.score } -Descending |
            Select-Object -First 20 |
            ForEach-Object {
                [ordered]@{
                    id = $_.id
                    name = $_.name
                    risk = $_.metrics.risk
                    riskScore = $_.colors.Risk.score
                    color = $_.colors.Risk.color
                    files = $_.files
                    blastRadius = $_.metrics.blast_radius
                    gitFiles = $_.metrics.git_files
                }
            })
        $fileHotspots = @($fileDetails |
            Sort-Object { $_.max_complexity } -Descending |
            Select-Object -First 30 |
            ForEach-Object {
                [ordered]@{
                    id = $_.id
                    path = $_.path
                    sourceAnchor = $_.source_anchor
                    functionCount = $_.function_count
                    maxComplexity = $_.max_complexity
                    avgComplexity = $_.avg_complexity
                    loc = $_.loc
                    git = $_.git
                }
            })
        $functionHotspots = @()
        foreach ($file in $fileDetails) {
            foreach ($fn in @($file.functions)) {
                $functionHotspots += [ordered]@{
                    id = $fn.id
                    fileId = $file.id
                    file = $file.path
                    name = $fn.name
                    sourceAnchor = $fn.source_anchor
                    startLine = $fn.start_line
                    endLine = $fn.end_line
                    complexity = $fn.complexity
                    loc = $fn.loc
                    params = $fn.params
                    async = $fn.async
                    public = $fn.public
                }
            }
        }
        $functionHotspots = @($functionHotspots | Sort-Object { $_["complexity"] } -Descending | Select-Object -First 50)
        $hotspotsPayload = [ordered]@{
            tool = "hotspots"
            path = $sentruxTargetPath
            generated_from = [ordered]@{
                dsm = $sentruxDsmPath
                fileDetails = $sentruxFileDetailsPath
            }
            modules = $moduleHotspots
            files = $fileHotspots
            functions = $functionHotspots
        }
        $hotspotsPayload | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $sentruxHotspotsPath -Encoding UTF8
        $sentruxDsmSummary = [ordered]@{
            path = $sentruxDsmPath
            defaultColorMode = $dsmObject.default_color_mode
            colorModes = $dsmObject.color_modes.Count
            modules = $dsmObject.modules.Count
        }
        $sentruxFileDetailsSummary = [ordered]@{
            path = $sentruxFileDetailsPath
            files = $fileDetails.Count
            functions = $functionCount
            maxFunctionComplexity = $maxFunctionComplexity
            hotspotFile = $hotspotFile
        }
        $topFunction = if ($functionHotspots.Count -gt 0) { "{0}:{1}" -f $functionHotspots[0]["name"], $functionHotspots[0]["complexity"] } else { "" }
        $sentruxHotspotsSummary = [ordered]@{
            path = $sentruxHotspotsPath
            modules = $moduleHotspots.Count
            files = $fileHotspots.Count
            functions = $functionHotspots.Count
            topFunction = $topFunction
        }

        $evolutionRaw = & $sentruxAgentTool evolution $sentruxTargetPath 2>&1
        $evolutionText = ($evolutionRaw | ForEach-Object { $_.ToString() } | Out-String).Trim()
        $evolutionObject = $evolutionText | ConvertFrom-Json
        $evolutionText | Set-Content -LiteralPath $sentruxEvolutionPath -Encoding UTF8
        $evolutionFunctions = @($evolutionObject.hotspots.functions)
        $evolutionCouplingModules = @($evolutionObject.coupling.modules)
        $evolutionBusModules = @($evolutionObject.bus_factor.modules)
        $topEvolutionHotspot = if ($evolutionFunctions.Count -gt 0) { "{0}:{1}" -f $evolutionFunctions[0].name, $evolutionFunctions[0].complexity } else { "" }
        $topEvolutionCoupling = if ($evolutionCouplingModules.Count -gt 0) { "{0}:{1}" -f $evolutionCouplingModules[0].name, $evolutionCouplingModules[0].coupling } else { "" }
        $topEvolutionBusFactor = if ($evolutionBusModules.Count -gt 0) { "{0}:{1}" -f $evolutionBusModules[0].name, $evolutionBusModules[0].bus_factor_risk } else { "" }
        $sentruxEvolutionSummary = [ordered]@{
            path = $sentruxEvolutionPath
            sessions = $evolutionObject.count
            trend = $evolutionObject.trend.direction
            topHotspot = $topEvolutionHotspot
            topCoupling = $topEvolutionCoupling
            topBusFactorRisk = $topEvolutionBusFactor
        }

        $whatIfRaw = & $sentruxAgentTool what_if $sentruxTargetPath 2>&1
        $whatIfText = ($whatIfRaw | ForEach-Object { $_.ToString() } | Out-String).Trim()
        $whatIfObject = $whatIfText | ConvertFrom-Json
        $whatIfText | Set-Content -LiteralPath $sentruxWhatIfPath -Encoding UTF8
        $failingScenarios = @($whatIfObject.scenarios | Where-Object { -not $_.pass })
        $topWhatIf = if ($failingScenarios.Count -gt 0) { "{0}:{1}" -f $failingScenarios[0].name, $failingScenarios[0].impact_count } else { "" }
        $sentruxWhatIfSummary = [ordered]@{
            path = $sentruxWhatIfPath
            scenarios = $whatIfObject.summary.scenarios
            failing = $whatIfObject.summary.failing
            primaryRisk = $whatIfObject.summary.primary_risk
            topScenario = $topWhatIf
        }
    }
    catch {
        $notes.Add("Sentrux structural artifacts were not generated: $($_.Exception.Message)")
    }
}

$codeNexusLiteTool = Join-Path $PSScriptRoot "Invoke-CodeNexusLite.ps1"
if (-not [string]::IsNullOrWhiteSpace($sentruxTargetPath) -and (Test-Path -LiteralPath $codeNexusLiteTool -PathType Leaf)) {
    try {
        $global:LASTEXITCODE = 0
        & $codeNexusLiteTool `
            -RepoPath $repoPath `
            -TargetPath $sentruxTargetPath `
            -RunDir $runDir `
            -OutputPath $codeNexusContextPath `
            -MaxCommitsPerFile 0 `
            -Quiet
        if ($global:LASTEXITCODE -ne 0) {
            throw "CodeNexus-lite exited with code $global:LASTEXITCODE"
        }
        if (-not (Test-Path -LiteralPath $codeNexusContextPath -PathType Leaf)) {
            throw "CodeNexus-lite did not write $codeNexusContextPath"
        }
        $codeNexusObject = Get-Content -LiteralPath $codeNexusContextPath -Raw | ConvertFrom-Json
        $codeNexusContextSummary = [ordered]@{
            path = $codeNexusContextPath
            files = $codeNexusObject.summary.files
            references = $codeNexusObject.summary.references
            recentCommits = $codeNexusObject.summary.recentCommits
            topFile = if (@($codeNexusObject.files).Count -gt 0) { [string]$codeNexusObject.files[0].path } else { "" }
        }
    }
    catch {
        $notes.Add("CodeNexus-lite context was not generated: $($_.Exception.Message)")
    }
}

$reportPath = Join-Path $runDir "report.json"
$understandingPath = Join-Path $runDir "understanding.md"
$summaryPath = Join-Path $runDir "summary.md"
$hospitalReportPath = Join-Path $runDir "hospital-report.json"
$hospitalMarkdownPath = Join-Path $runDir "hospital.md"
$surgeryPlanPath = Join-Path $runDir "surgery-plan.json"
$surgeryMarkdownPath = Join-Path $runDir "surgery-plan.md"
$sentruxFailuresPath = Join-Path $runDir "sentrux-failures.json"
$sentruxDebtRegisterPath = Join-Path $runDir "sentrux-debt-register.json"
$sentruxFailures = New-CodeIntelSentruxFailures `
    -Steps $steps `
    -OutputPath $sentruxFailuresPath `
    -HotspotsPath $(if ($null -ne $sentruxHotspotsSummary) { [string]$sentruxHotspotsSummary.path } else { "" }) `
    -FileDetailsPath $(if ($null -ne $sentruxFileDetailsSummary) { [string]$sentruxFileDetailsSummary.path } else { "" })
$sentruxDebtRegister = New-CodeIntelSentruxDebtRegister `
    -Failures $sentruxFailures `
    -RepoPath $repoPath `
    -RunTimestamp $timestamp `
    -OutputPath $sentruxDebtRegisterPath
$followUpAutomation = New-CodeIntelFollowUpAutomation `
    -FailureClassifications $failureClassifications `
    -BlockingSentruxDebt ([int]$sentruxDebtRegister.summary.blocking) `
    -EvidencePath $sentruxDebtRegisterPath `
    -OutputDirectory $runDir `
    -ProactiveSkillSuggestions $ProactiveSkillSuggestions `
    -AutomaticPullRequests $AutomaticPullRequests `
    -BugSkill $BugSkill
$followUpAutomationPath = Join-Path $runDir "follow-up-automation.json"
$effectiveFailureCounts["sentruxFail"] = [int]$sentruxDebtRegister.summary.blocking
$effectiveFailed = @(Get-CodeIntelEffectiveFailedSteps `
    -FailedSteps $failed `
    -BlockingSentruxDebt ([int]$sentruxDebtRegister.summary.blocking))
$sentruxInsight["failures"] = Get-CodeIntelSentruxFailureSummary -Failures $sentruxFailures -Path $sentruxFailuresPath
$sentruxInsight["debtRegister"] = Get-CodeIntelSentruxDebtSummary -DebtRegister $sentruxDebtRegister -Path $sentruxDebtRegisterPath
$sentruxInsight["authoritativePrimaryTarget"] = Get-CodeIntelSentruxPrimaryTargetText -Failures $sentruxFailures
$runtimeCiSummary = $null
if (-not [string]::IsNullOrWhiteSpace($RuntimeCiEvidenceRequest)) {
    if ([string]::IsNullOrWhiteSpace($RuntimeCiEvidenceArtifactRoot)) { throw "Runtime/CI evidence requires an artifact root" }
    $runtimeCiCli = $defaultRustCli
    if (-not (Test-Path -LiteralPath $runtimeCiCli -PathType Leaf)) { throw "Runtime/CI evidence provider binary is missing: $runtimeCiCli" }
    $runtimeCiSummaryPath = Join-Path $runDir "runtime-ci-summary.json"
    & $runtimeCiCli provider runtime-ci-evidence --artifact-root $RuntimeCiEvidenceArtifactRoot --request $RuntimeCiEvidenceRequest --out $runtimeCiSummaryPath
    if ($LASTEXITCODE -ne 0) { throw "Runtime/CI evidence provider rejected the request" }
    $runtimeCiSummary = Read-JsonFileSafe $runtimeCiSummaryPath
    if ($null -eq $runtimeCiSummary -or [string]$runtimeCiSummary.schema -ne "code-intel-runtime-ci-summary.v1") { throw "Runtime/CI summary is missing or invalid" }
    $runtimeCiSummary | Add-Member -NotePropertyName path -NotePropertyValue $runtimeCiSummaryPath
}

$hospitalReport = New-CodeIntelHospitalReport `
    -RepoPath $repoPath `
    -Mode $Mode `
    -RunDir $runDir `
    -ReportPath $reportPath `
    -SummaryPath $summaryPath `
    -UnderstandingPath $understandingPath `
    -Steps $steps `
    -FailureCounts $effectiveFailureCounts `
    -SentruxInsight $sentruxInsight `
    -SentruxDsmSummary $sentruxDsmSummary `
    -SentruxFileDetailsSummary $sentruxFileDetailsSummary `
    -SentruxHotspotsSummary $sentruxHotspotsSummary `
    -SentruxEvolutionSummary $sentruxEvolutionSummary `
-SentruxWhatIfSummary $sentruxWhatIfSummary `
-CodeNexusContextSummary $codeNexusContextSummary `
-RuntimeCiSummary $runtimeCiSummary `
-UnderstandCommand $understandCommand `
-ToolState $toolState `
-GitHubResearch $githubResearch
$hotspotsForSurgery = if ($null -ne $sentruxHotspotsSummary) { [string]$sentruxHotspotsSummary.path } else { "" }
$whatIfForSurgery = if ($null -ne $sentruxWhatIfSummary) { [string]$sentruxWhatIfSummary.path } else { "" }
$codeNexusForSurgery = if ($null -ne $codeNexusContextSummary) { [string]$codeNexusContextSummary.path } else { "" }
$surgeryPlan = New-CodeIntelSurgeryPlan `
    -Hospital $hospitalReport `
    -RepoPath $repoPath `
    -SentruxTargetPath $sentruxTargetPath `
    -HotspotsPath $hotspotsForSurgery `
    -WhatIfPath $whatIfForSurgery `
    -CodeNexusPath $codeNexusForSurgery
if ($null -ne $sentruxFailures -and $null -ne $sentruxFailures.primary -and $null -ne $sentruxFailures.primary.target) {
    $normalizedTarget = $sentruxFailures.primary.target
    if ($null -ne $surgeryPlan.primary_target) {
        if ([string]$normalizedTarget.status -eq "resolved") {
            $surgeryPlan.primary_target["file"] = [string]$normalizedTarget.file
            $surgeryPlan.primary_target["name"] = [string]$normalizedTarget.symbol
            $surgeryPlan.primary_target["complexity"] = $sentruxFailures.primary.value
        }
        elseif ([string]$normalizedTarget.status -eq "unresolved") {
            $surgeryPlan.primary_target["file"] = ""
            $surgeryPlan.primary_target["name"] = "unresolved sentrux max_cc"
            $surgeryPlan.primary_target["complexity"] = $sentruxFailures.primary.value
        }
        $surgeryPlan.primary_target["authority"] = "sentrux-failures.json"
        $surgeryPlan.primary_target["target_status"] = [string]$normalizedTarget.status
    }
    $hospitalReport.diagnosis.evidence["top_function"] = Get-CodeIntelSentruxPrimaryTargetText -Failures $sentruxFailures
}
$surgeryPlan | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $surgeryPlanPath -Encoding UTF8
Convert-SurgeryPlanToMarkdown $surgeryPlan | Set-Content -LiteralPath $surgeryMarkdownPath -Encoding UTF8
$hospitalReport["artifacts"]["surgeryPlan"] = $surgeryPlanPath
$hospitalReport["artifacts"]["surgeryPlanMarkdown"] = $surgeryMarkdownPath
$hospitalReport["artifacts"]["sentruxFailures"] = $sentruxFailuresPath
$hospitalReport["artifacts"]["sentruxDebtRegister"] = $sentruxDebtRegisterPath
$hospitalReport["diagnosis"]["sentrux_failures"] = Get-CodeIntelSentruxFailureSummary -Failures $sentruxFailures -Path $sentruxFailuresPath
$hospitalReport["diagnosis"]["sentrux_debt"] = Get-CodeIntelSentruxDebtSummary -DebtRegister $sentruxDebtRegister -Path $sentruxDebtRegisterPath
$hospitalReport["surgery_plan"] = [ordered]@{
    path = $surgeryPlanPath
    markdown = $surgeryMarkdownPath
    status = $surgeryPlan.status
    primary_target = if (-not [string]::IsNullOrWhiteSpace([string]$surgeryPlan.primary_target.name)) {
        "$($surgeryPlan.primary_target.name) in $($surgeryPlan.primary_target.file)"
    }
    else {
        [string]$surgeryPlan.primary_target.file
    }
}
$hospitalReport | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $hospitalReportPath -Encoding UTF8
Convert-HospitalReportToMarkdown $hospitalReport | Set-Content -LiteralPath $hospitalMarkdownPath -Encoding UTF8

$report = [ordered]@{
    repo = $repoPath
    repoInput = $Repo
    repoName = $repoName
    mode = $Mode
    language = $Language
    platform = [ordered]@{
        os = $effectivePlatform
        shell = $PSVersionTable.PSEdition
        psVersion = $PSVersionTable.PSVersion.ToString()
    }
    paths = [ordered]@{
        home = $codeIntelPaths.home
        dataRoot = $codeIntelPaths.dataRoot
        bin = $codeIntelPaths.bin
        codeIntelHome = $codeIntelPaths.codeIntelHome
    }
    artifactDir = $runDir
    sentruxPath = if ([string]::IsNullOrWhiteSpace($SentruxPath)) { $repoPath } else { (Resolve-ChildPath $repoPath $SentruxPath) }
    tools = $toolState
    understandCommand = $understandCommand
    modelChannel = $modelChannel
    steps = $steps
    sentruxInsight = $sentruxInsight
    sentruxFailures = Get-CodeIntelSentruxFailureSummary -Failures $sentruxFailures -Path $sentruxFailuresPath
    sentruxDebtRegister = Get-CodeIntelSentruxDebtSummary -DebtRegister $sentruxDebtRegister -Path $sentruxDebtRegisterPath
        sentruxDsm = $sentruxDsmSummary
    sentruxFileDetails = $sentruxFileDetailsSummary
    sentruxHotspots = $sentruxHotspotsSummary
    sentruxEvolution = $sentruxEvolutionSummary
        sentruxWhatIf = $sentruxWhatIfSummary
    codeNexusContext = $codeNexusContextSummary
    runtimeCi = $runtimeCiSummary
        codeEvidence = $codeEvidence
        repomixPack = $repomixPack
        githubResearch = $githubResearch
        openSpec = [ordered]@{
            recommendation = $openSpecResult.verdict
            score = $openSpecResult.score
            tool = if ($openSpecResult.Contains("candidate")) { $openSpecResult.candidate } else { $openSpecResult.tool }
            reasons = $openSpecResult.reasons
            recommendationBrief = if ($openSpecResult.Contains("brief")) { $openSpecResult.brief } elseif ($openSpecResult.Contains("recommendationBrief")) { $openSpecResult.recommendationBrief } else { $null }
        }
        workflowRecommendation = $workflowStackResult
        workflows = if ($workflowStackResult) {
            @($workflowStackResult.alternatives | ForEach-Object {
                $wf = $_
                [ordered]@{
                stack = $wf.stack
                tool = if ($wf.Contains("candidate")) { $wf.candidate } elseif ($wf.Contains("tool")) { $wf.tool } else { $null }
                verdict = $wf.verdict
                    score = if ($wf.Contains("score")) { $wf.score } else { $null }
                    reasons = $wf.reasons
                    entrySkills = $wf.entrySkills
                    recommendationBrief = if ($wf.Contains("brief")) { $wf.brief } elseif ($wf.Contains("recommendationBrief")) { $wf.recommendationBrief } else { $null }
                }
            })
        } else { @() }
hospital = [ordered]@{
        path = $hospitalReportPath
        markdown = $hospitalMarkdownPath
        surgeryPlan = $surgeryPlanPath
        surgeryPlanMarkdown = $surgeryMarkdownPath
        schema = $hospitalReport.schema
        status = $hospitalReport.triage.status
        disposition = $hospitalReport.triage.disposition
        primaryDiagnosis = $hospitalReport.triage.primary_diagnosis
        overallScore = $hospitalReport.triage.overall_score
        nextProtocol = $hospitalReport.triage.next_protocol
currentState = $hospitalReport.state_machine.current_state
modalities = @($hospitalReport.modalities).Count
researchStatus = $hospitalReport.triage.research_status
researchRequired = $hospitalReport.triage.research_required
}
    notes = $notes
    failureClassifications = $failureClassifications
    followUpAutomation = [ordered]@{
        path = $followUpAutomationPath
        proactiveSkillSuggestions = $followUpAutomation.proactiveSkillSuggestions
        automaticPullRequests = $followUpAutomation.automaticPullRequests
    }
    summary = [ordered]@{
        failed = $failed.Count
        effectiveFailed = $effectiveFailed.Count
        manualRequired = $manual.Count
        passed = @($steps | Where-Object { $_.status -eq "passed" }).Count
        skipped = @($steps | Where-Object { $_.status -eq "skipped" }).Count
        failureCategories = $failureCounts
        effectiveFailureCategories = $effectiveFailureCounts
        blockingSentruxDebt = [int]$sentruxDebtRegister.summary.blocking
        knownSentruxDebt = [int]$sentruxDebtRegister.summary.knownDebt
    }
}

$report["understanding"] = $understandingPath
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8

$sentruxMetrics = @($sentruxInsight['metrics'])
$sentruxNextActions = @($sentruxInsight['nextActions'])
$sentruxCodeNexusHints = @($sentruxInsight['codeNexusHints'])
$sentruxScan = $sentruxInsight['scan']
$githubResearchSummary = [string]$githubResearch.status
if ([bool]$githubResearch.required) {
    $githubResearchSummary = "$githubResearchSummary ($($githubResearch.markdown))"
}
$summaryLines = @(
    "# Code Intel Pipeline",
    "",
    "- Repo: $repoPath",
    "- Mode: $Mode",
    "- Report: $reportPath",
    "- Understanding: $understandingPath",
    "- Hospital: $hospitalMarkdownPath",
    "- Understand command: ``$understandCommand``",
    "",
    "## Workflow Stack Recommendations",
    "",
    $(if ($workflowStackResult) {
        @($workflowStackResult.alternatives | ForEach-Object {
            $wf = $_
            $workflowTool = if ($wf.Contains("candidate")) { $wf.candidate } elseif ($wf.Contains("tool")) { $wf.tool } else { $null }
            $toolText = if ($workflowTool) { " (tool=$workflowTool)" } else { "" }
            $skillsText = if ($wf.entrySkills.Count -gt 0) { $wf.entrySkills -join " " } else { "(none)" }
            "- $($wf.stack)${toolText}: $($wf.verdict -replace '_', ' ') -> $skillsText"
        }) -join "`n"
        } else {
            "- spec-driven: skipped -> (none)"
        }),
        $(if ($workflowStackResult -and $workflowStackResult.recommendation -and $workflowStackResult.recommendation.Contains("brief")) {
            $brief = $workflowStackResult.recommendation.brief
            $doFirstText = @($brief.doFirst) -join "; "
            $guardrailText = @($brief.doNotDoYet) -join "; "
            $acceptanceText = @($brief.acceptance) -join "; "
            "- spec-driven brief: $($brief.recommended); confidence=$($brief.confidence); do first: $doFirstText; guardrails: $guardrailText; acceptance: $acceptanceText"
        } else {
            ""
        }),
        "",
        "## Summary",
    "",
    "- Passed: $(@($steps | Where-Object { $_.status -eq 'passed' }).Count)",
    "- Failed: $($failed.Count)",
    "- Effective failed: $($effectiveFailed.Count)",
    "- Manual required: $($manual.Count)",
    "- Model channel: $($modelChannel.status)",
    $(if ($modelChannel.assistanceDossier) { "- Model assistance dossier: $($modelChannel.assistanceDossier)" } else { "- Model assistance dossier: (none)" }),
    "- Skipped: $(@($steps | Where-Object { $_.status -eq 'skipped' }).Count)",
    "- Provider quota: $($failureCounts.providerQuota)",
    "- Provider unavailable: $($failureCounts.providerUnavailable)",
    "- Provider config error: $($failureCounts.configError)",
    "- Local tool error: $($failureCounts.localToolError)",
    "",
    (Get-CodeIntelFollowUpSummaryLines -Automation $followUpAutomation),
    "- Graph missing: $($failureCounts.graphMissing)",
    "- Sentrux fail: $($failureCounts.sentruxFail)",
    "- Blocking Sentrux debt: $($sentruxDebtRegister.summary.blocking)",
    "- Known Sentrux debt: $($sentruxDebtRegister.summary.knownDebt)",
    "- Hospital status: $($hospitalReport.triage.status)",
    "- Hospital disposition: $($hospitalReport.triage.disposition)",
"- Hospital state: $($hospitalReport.state_machine.current_state)",
"- Hospital score: $($hospitalReport.triage.overall_score)",
"- Next protocol: $($hospitalReport.triage.next_protocol)",
"- GitHub research: $githubResearchSummary",
"- Follow-up automation: ``$followUpAutomationPath`` (skillSuggestions=$($followUpAutomation.proactiveSkillSuggestions.status), automaticPrConsent=$($followUpAutomation.automaticPullRequests.consentStatus), execution=$($followUpAutomation.automaticPullRequests.executionStatus))",
"",
"## Steps"
)
foreach ($step in $steps) {
    $summaryLines += "- $($step.status): $($step.name)"
}
$summaryLines += ""
$summaryLines += "## Hospital"
$summaryLines += "- Report: $hospitalReportPath"
$summaryLines += "- Markdown: $hospitalMarkdownPath"
$summaryLines += "- Status: $($hospitalReport.triage.status)"
$summaryLines += "- Disposition: $($hospitalReport.triage.disposition)"
$summaryLines += "- Current state: $($hospitalReport.state_machine.current_state)"
$summaryLines += "- Primary diagnosis: $($hospitalReport.triage.primary_diagnosis)"
$summaryLines += "- Admission reason: $($hospitalReport.triage.admission_reason)"
$summaryLines += "- Overall score: $($hospitalReport.triage.overall_score)"
$summaryLines += "- Next protocol: $($hospitalReport.triage.next_protocol)"
$summaryLines += "- Surgery plan: $surgeryMarkdownPath (status=$($surgeryPlan.status))"
foreach ($modality in @($hospitalReport.modalities)) {
    $summaryLines += "- Modality $($modality.name): $($modality.status), confidence=$($modality.confidence)"
}
$summaryLines += ""
$summaryLines += "## Code Evidence"
$summaryLines += "- Status: $($codeEvidence.status)"
$summaryLines += "- Agent Code Slice: $($codeEvidence.agentIndex)"
$summaryLines += "- Scorecard: $($codeEvidence.scorecard)"
$summaryLines += "- Files: $($codeEvidence.files), symbols: $($codeEvidence.symbols), chunks: $($codeEvidence.chunks), imports: $($codeEvidence.imports)"
$summaryLines += ""
$summaryLines += "## Repomix Pack"
$summaryLines += "- Status: $($repomixPack.status)"
$summaryLines += "- Style: $($repomixPack.style)"
$summaryLines += "- Output: $($repomixPack.path)"
$summaryLines += "- Summary: $($repomixPack.summaryPath)"
if ([string]$repomixPack.status -eq "ok" -and -not [string]::IsNullOrWhiteSpace([string]$repomixPack.path)) {
    $summaryLines += "- Agent read order: read this pack first for whole-repo orientation, then use Code Evidence slices for ranked navigation."
}
$summaryLines += ""
$summaryLines += "## Sentrux Insight"
$summaryLines += "- Target: $($sentruxInsight['targetPath'])"
$summaryLines += "- Baseline: $($sentruxInsight['baselinePath'])"
$summaryLines += "- Rules: $($sentruxInsight['rulesPath'])"
$summaryLines += "- Gate: $($sentruxInsight['gateStatus'])"
$summaryLines += "- Check: $($sentruxInsight['checkStatus'])"
$summaryLines += "- No degradation: $($sentruxInsight['noDegradation'])"
$summaryLines += "- Failures artifact: $sentruxFailuresPath (status=$($sentruxFailures.status), records=$(@($sentruxFailures.records).Count), conflicts=$(@($sentruxFailures.conflicts).Count))"
$summaryLines += "- Debt register: $sentruxDebtRegisterPath (known=$($sentruxDebtRegister.summary.knownDebt), new=$($sentruxDebtRegister.summary.newDebt), worsened=$($sentruxDebtRegister.summary.worsenedDebt), blocking=$($sentruxDebtRegister.summary.blocking), informational=$($sentruxDebtRegister.summary.informational))"
if ($null -ne $sentruxFailures.primary) {
    $summaryLines += "- Authoritative primary: $($sentruxFailures.primary.id) target=$($sentruxFailures.primary.target.status)"
}
if (@($sentruxFailures.conflicts).Count -gt 0) {
    foreach ($conflict in @($sentruxFailures.conflicts)) {
        $summaryLines += "- Conflict: $($conflict.kind) $($conflict.authoritative_record_id) vs $($conflict.conflicting_record_id)"
    }
}
if ($null -ne $sentruxDsmSummary) {
    $summaryLines += "- DSM: $($sentruxDsmSummary.path) (modes=$($sentruxDsmSummary.colorModes), modules=$($sentruxDsmSummary.modules), default=$($sentruxDsmSummary.defaultColorMode))"
}
if ($null -ne $sentruxFileDetailsSummary) {
    $summaryLines += "- File details: $($sentruxFileDetailsSummary.path) (files=$($sentruxFileDetailsSummary.files), functions=$($sentruxFileDetailsSummary.functions), maxComplexity=$($sentruxFileDetailsSummary.maxFunctionComplexity), hotspot=$($sentruxFileDetailsSummary.hotspotFile))"
}
if ($null -ne $sentruxHotspotsSummary) {
    $summaryLines += "- Hotspots: $($sentruxHotspotsSummary.path) (modules=$($sentruxHotspotsSummary.modules), files=$($sentruxHotspotsSummary.files), functions=$($sentruxHotspotsSummary.functions), topFunction=$($sentruxHotspotsSummary.topFunction))"
}
if ($null -ne $sentruxEvolutionSummary) {
    $summaryLines += "- Evolution: $($sentruxEvolutionSummary.path) (sessions=$($sentruxEvolutionSummary.sessions), trend=$($sentruxEvolutionSummary.trend), hotspot=$($sentruxEvolutionSummary.topHotspot), coupling=$($sentruxEvolutionSummary.topCoupling), busFactorRisk=$($sentruxEvolutionSummary.topBusFactorRisk))"
}
if ($null -ne $sentruxWhatIfSummary) {
    $summaryLines += "- What-if: $($sentruxWhatIfSummary.path) (scenarios=$($sentruxWhatIfSummary.scenarios), failing=$($sentruxWhatIfSummary.failing), primaryRisk=$($sentruxWhatIfSummary.primaryRisk), topScenario=$($sentruxWhatIfSummary.topScenario))"
}
if ($null -ne $codeNexusContextSummary) {
    $summaryLines += "- CodeNexus context: $($codeNexusContextSummary.path) (files=$($codeNexusContextSummary.files), references=$($codeNexusContextSummary.references), commits=$($codeNexusContextSummary.recentCommits), topFile=$($codeNexusContextSummary.topFile))"
}
if ($null -ne $runtimeCiSummary) {
    $summaryLines += "- Runtime/CI: $($runtimeCiSummary.path) (health=$($runtimeCiSummary.health), freshness=$($runtimeCiSummary.freshness), completeness=$($runtimeCiSummary.completeness))"
}
foreach ($metric in $sentruxMetrics) {
    $summaryLines += "- Metric $($metric.name): $($metric.before) -> $($metric.after) (delta $($metric.delta), regressed=$($metric.regressed))"
}
if ($sentruxScan.Count -gt 0) {
    $summaryLines += "- Scan: files=$($sentruxScan['files']), imports=$($sentruxScan['importEdges']), calls=$($sentruxScan['callEdges']), unresolvedImports=$($sentruxScan['unresolvedImports'])"
}
foreach ($action in $sentruxNextActions) {
    $summaryLines += "- Next: $action"
}
foreach ($hint in $sentruxCodeNexusHints) {
    $summaryLines += "- CodeNexus: $hint"
}
if ($failureClassifications.Count -gt 0) {
    $summaryLines += ""
    $summaryLines += "## Failure Classification"
    foreach ($item in $failureClassifications) {
        $summaryLines += "- $($item.category): $($item.step)"
    }
}
if ($notes.Count -gt 0) {
    $summaryLines += ""
    $summaryLines += "## Notes"
    foreach ($note in $notes) {
        $summaryLines += "- $note"
    }
}
$summaryLines | Set-Content -LiteralPath $summaryPath -Encoding UTF8

$passedSteps = @($steps | Where-Object { $_.status -eq "passed" })
$skippedSteps = @($steps | Where-Object { $_.status -eq "skipped" })
$problemSteps = @($steps | Where-Object { $_.status -eq "failed" -or $_.status -eq "manual_required" })
$graphStep = @($steps | Where-Object { $_.name -eq "understand graph" } | Select-Object -First 1)
$repowiseSteps = @($steps | Where-Object { $_.name -like "repowise*" })
$sentruxSteps = @($steps | Where-Object { $_.name -like "sentrux*" })
$nextAction = "No immediate action required; keep this artifact as the latest clean code-intel snapshot."
if ($failureCounts.providerQuota -gt 0) {
    $nextAction = "Provider quota blocked part of the run. Retry provider-backed docs/index work after quota resets."
}
elseif ($failureCounts.providerUnavailable -gt 0) {
    $nextAction = "The upstream provider route or selected model was unavailable. Verify the provider model catalog and retry without treating this as a local tool failure."
}
elseif ($failureCounts.configError -gt 0) {
    $nextAction = "Correct the provider endpoint, model, or credential configuration, then retry provider-backed docs."
}
elseif ($failureCounts.localToolError -gt 0) {
    $nextAction = "Fix the local tool error shown in report.json, then rerun the pipeline."
}
elseif ($failureCounts.graphMissing -gt 0) {
    $nextAction = "Run the emitted Understand Anything command in Claude, then rerun the pipeline."
}
elseif ($effectiveFailureCounts.sentruxFail -gt 0) {
    $nextAction = "Inspect blocking Sentrux debt in sentrux-debt-register.json before changing code or saving a baseline."
}
elseif ($failureCounts.sentruxFail -gt 0) {
    $nextAction = "Known or informational Sentrux debt is recorded in sentrux-debt-register.json; understanding artifacts are usable."
}
elseif (-not [bool]$sentruxInsight['rulesExists']) {
    $nextAction = "Add real Sentrux boundary rules at $($sentruxInsight['rulesPath']) before treating this scope as governed."
}
elseif ($manual.Count -gt 0) {
    $nextAction = "Resolve the manual_required step, then rerun if the team needs a fully clean artifact."
}

$understandingLines = @(
    "# Understanding Report",
    "",
    "## Key Assumptions",
    "- The repo path resolved to ``$repoPath``.",
    "- Mode ``$Mode`` reflects the intended confidence level for this run.",
    "- ``rg`` is exact inventory, ``repowise`` is semantic memory, Understand Anything is architecture graph context, and Sentrux is the structural gate.",
    "- Generated artifacts are local evidence, not a replacement for human review.",
    "",
    "## Verified",
    "- Artifact directory: ``$runDir``",
    "- Report: ``$reportPath``",
    "- Summary: ``$summaryPath``",
    "- Agent Code Slice: ``code-evidence/merged/agent/index.md``",
"- Code Evidence scorecard: ``code-evidence/merged/scorecard.json``",
"- Repomix pack: $(if (-not [string]::IsNullOrWhiteSpace([string]$repomixPack.path)) { '``' + $repomixPack.path + '`` status=' + $repomixPack.status + ', style=' + $repomixPack.style } else { 'not generated' })",
"- Hospital report: ``$hospitalReportPath``",
    "- Hospital markdown: ``$hospitalMarkdownPath``",
    "- Sentrux DSM: $(if ($null -ne $sentruxDsmSummary) { '``' + $sentruxDsmSummary.path + '``' } else { 'not generated' })",
    "- Sentrux file details: $(if ($null -ne $sentruxFileDetailsSummary) { '``' + $sentruxFileDetailsSummary.path + '``' } else { 'not generated' })",
    "- Sentrux hotspots: $(if ($null -ne $sentruxHotspotsSummary) { '``' + $sentruxHotspotsSummary.path + '``' } else { 'not generated' })",
"- Sentrux evolution: $(if ($null -ne $sentruxEvolutionSummary) { '``' + $sentruxEvolutionSummary.path + '``' } else { 'not generated' })",
"- Sentrux what-if: $(if ($null -ne $sentruxWhatIfSummary) { '``' + $sentruxWhatIfSummary.path + '``' } else { 'not generated' })",
"- CodeNexus context: $(if ($null -ne $codeNexusContextSummary) { '``' + $codeNexusContextSummary.path + '``' } else { 'not generated' })",
"- GitHub research: $githubResearchSummary",
    "- Tools: rg=$($toolState.rg), git=$($toolState.git), repowise=$($toolState.repowise), repomix=$($toolState.repomix), sentrux=$($toolState.sentrux)",
    "- Passed steps: $(Join-StatusNames $passedSteps)",
    "- Hospital: status=$($hospitalReport.triage.status), disposition=$($hospitalReport.triage.disposition), state=$($hospitalReport.state_machine.current_state), score=$($hospitalReport.triage.overall_score), next=$($hospitalReport.triage.next_protocol)",
    "",
    "## Read Order",
    "$(if ([string]$repomixPack.status -eq "ok" -and -not [string]::IsNullOrWhiteSpace([string]$repomixPack.path)) { '1. Start: ``' + $repomixPack.path + '`` complete repository pack.' + [Environment]::NewLine + '2. Then: ``summary.md`` run status, failures.' + [Environment]::NewLine + '3. Navigate: ``code-evidence/merged/agent/index.md`` ranked files, symbols.' + [Environment]::NewLine + '4. Govern: ``hospital.md`` plus ``surgery-plan.md``.' } else { '1. Start: ``summary.md`` run status, failures.' + [Environment]::NewLine + '2. Navigate: ``code-evidence/merged/agent/index.md`` ranked files, symbols.' + [Environment]::NewLine + '3. Govern: ``hospital.md`` plus ``surgery-plan.md``.' })",
    "",
    "## Unverified Or Inferred",
    "- Understand graph: $(if ($graphStep.Count -gt 0) { $graphStep[0].status } else { 'not checked' })",
"- Repowise state: $(Join-StatusNames $repowiseSteps)",
"- Sentrux state: $(Join-StatusNames $sentruxSteps)",
"- Sentrux gate insight: gate=$($sentruxInsight['gateStatus']), noDegradation=$($sentruxInsight['noDegradation']), rules=$($sentruxInsight['rulesExists'])",
"- Sentrux failures: ``$sentruxFailuresPath`` status=$($sentruxFailures.status), records=$(@($sentruxFailures.records).Count), conflicts=$(@($sentruxFailures.conflicts).Count)",
"- Sentrux debt register: ``$sentruxDebtRegisterPath`` known=$($sentruxDebtRegister.summary.knownDebt), new=$($sentruxDebtRegister.summary.newDebt), worsened=$($sentruxDebtRegister.summary.worsenedDebt), blocking=$($sentruxDebtRegister.summary.blocking), informational=$($sentruxDebtRegister.summary.informational)",
"- Sentrux authoritative primary: $(if ($null -ne $sentruxFailures.primary) { [string]$sentruxFailures.primary.id + ' target=' + [string]$sentruxFailures.primary.target.status } else { 'none' })",
"- Skipped steps: $(Join-StatusNames $skippedSteps)",
    "",
    "## Sentrux Structural Signal",
    "$(if ($sentruxMetrics.Count -gt 0) { ($sentruxMetrics | ForEach-Object { '- ' + $_.name + ': ' + $_.before + ' -> ' + $_.after + ' (delta ' + $_.delta + ', regressed=' + $_.regressed + ')' }) -join [Environment]::NewLine } else { '- no parsed metrics' })",
    "$(if ($sentruxNextActions.Count -gt 0) { ($sentruxNextActions | ForEach-Object { '- next: ' + $_ }) -join [Environment]::NewLine } else { '- next: none' })",
    "$(if ($sentruxCodeNexusHints.Count -gt 0) { ($sentruxCodeNexusHints | ForEach-Object { '- codenexus: ' + $_ }) -join [Environment]::NewLine } else { '- codenexus: none' })",
    "",
    "## Failure Categories",
    "- provider_quota: $($failureCounts.providerQuota)",
    "- provider_unavailable: $($failureCounts.providerUnavailable)",
    "- config_error: $($failureCounts.configError)",
    "- local_tool_error: $($failureCounts.localToolError)",
    "- graph_missing: $($failureCounts.graphMissing)",
    "- sentrux_fail: $($failureCounts.sentruxFail)",
    "- effective_sentrux_fail: $($effectiveFailureCounts.sentruxFail)",
    "- effective_failed: $($effectiveFailed.Count)",
    "",
    "## Human Inspection Required",
    "- If Repomix status is ``ok``, read ``repomix-output.*`` first for whole-repo orientation; otherwise read ``summary.md`` first.",
    "- If `graph_missing > 0`, run: ``$understandCommand``",
    "- If ``sentrux_fail > 0``, inspect Sentrux output in ``report.json`` before saving a new baseline.",
    "- If ``provider_quota > 0``, treat it as an upstream quota/rate issue, not a local indexing failure.",
    "- If ``provider_unavailable > 0``, verify the upstream model catalog/route; do not repair the local toolchain for an upstream 404.",
    "- If ``config_error > 0``, correct endpoint/model/credential configuration without writing secrets into repository files.",
    "- If ``local_tool_error > 0``, inspect command output and PATH/tool installation before changing repo code.",
    "- If automatic PR consent is ``pending``, present ``automatic-pr-consent.request.json`` to the user. A recommendation or response alone does not authorize PR creation.",
    "",
    "## Problem Steps",
    "$(if ($problemSteps.Count -gt 0) { ($problemSteps | ForEach-Object { '- ' + $_.name + ': ' + $_.status }) -join [Environment]::NewLine } else { '- none' })",
    "",
    "## Next Action",
    $nextAction
)
$understandingLines | Set-Content -LiteralPath $understandingPath -Encoding UTF8

# A run is authoritative only after every materialized view has been written,
# staging paths have been replaced with their published locations, the staged
# directory has been promoted, and run-complete.json is written last.
$textExtensions = @(".json", ".md", ".txt", ".yaml", ".yml", ".toml")
$escapedRunDir = ($runDir | ConvertTo-Json -Compress).Trim('"')
$escapedFinalRunDir = ($finalRunDir | ConvertTo-Json -Compress).Trim('"')
foreach ($file in Get-ChildItem -LiteralPath $runDir -File -ErrorAction Stop) {
    if ($file.Extension.ToLowerInvariant() -notin $textExtensions) { continue }
    if ($file.Name -like "repomix-output.*") { continue }
    $content = [System.IO.File]::ReadAllText($file.FullName)
    if ($content.Contains($runDir) -or $content.Contains($escapedRunDir)) {
        [System.IO.File]::WriteAllText(
            $file.FullName,
            $content.Replace($escapedRunDir, $escapedFinalRunDir).Replace($runDir, $finalRunDir),
            [System.Text.UTF8Encoding]::new($false)
        )
    }
}

[System.IO.Directory]::Move($runDir, $finalRunDir)
$runDir = $finalRunDir
$reportPath = Join-Path $runDir "report.json"
$summaryPath = Join-Path $runDir "summary.md"
$understandingPath = Join-Path $runDir "understanding.md"
$hospitalReportPath = Join-Path $runDir "hospital-report.json"
$hospitalMarkdownPath = Join-Path $runDir "hospital.md"
$reportDigest = (Get-FileHash -LiteralPath $reportPath -Algorithm SHA256).Hash.ToLowerInvariant()
$runCommit = [ordered]@{
    schema = "code-intel-run-commit.v1"
    generatedAt = (Get-Date).ToString("o")
    report = "report.json"
    reportSha256 = $reportDigest
}
$runCommit | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $runDir "run-complete.json") -Encoding UTF8

Write-Host "Code intel pipeline complete"
Write-Host "Repo: $repoPath"
Write-Host "Report: $reportPath"
Write-Host "Summary: $summaryPath"
Write-Host "Understanding: $understandingPath"
Write-Host "Hospital: $hospitalMarkdownPath"
Write-CodeIntelFollowUpPrompt -Automation $followUpAutomation -RunDirectory $runDir
if ($manual.Count -gt 0) {
    Write-Host "Manual step required: $understandCommand"
}
if ($effectiveFailed.Count -gt 0) {
    Write-Host "Failed steps: $($failed.Count)"
    Write-Host "Effective failed steps: $($effectiveFailed.Count)"
    exit 1
}
exit 0
