param(
    [string]$Repo = "",
    [string]$RepoPath = "",

    [string]$Config = "",

    [ValidateSet("lite", "normal", "full")]
    [string]$Mode = "normal",

    [string]$Language = "",

    [string]$ArtifactRoot = "",
    [string]$SentruxPath = "",
    [string]$RepowiseWorkspaceRoot = "",
    [string]$RepowiseShadowRoot = "",
    [string[]]$RepowiseScopePaths = @(),
    [string[]]$RepowiseRootFiles = @(),
    [string[]]$InventoryExclude = @(),

    [switch]$SaveSentruxBaseline,
    [switch]$AutoSaveMissingSentruxBaseline,
    [switch]$SkipRepowise,
    [switch]$RepowiseDocs,
    [switch]$SkipSentrux,
    [switch]$SkipSentruxCheck,
    [switch]$SkipSentruxGate,
    [switch]$RequireUnderstandGraph,
    [switch]$WorkspaceAdd
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-Repo {
    param([string]$Path)

    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        throw "Repo path is not a directory: $Path"
    }
    return $item.FullName
}

function Test-CommandAvailable {
    param([string]$Name)
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

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

function Get-DefaultArtifactRoot {
    $fromEnv = [Environment]::GetEnvironmentVariable("CODE_INTEL_ARTIFACT_ROOT", "User")
    if (-not [string]::IsNullOrWhiteSpace($fromEnv)) { return $fromEnv }
    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_ARTIFACT_ROOT)) { return $env:CODE_INTEL_ARTIFACT_ROOT }
    $base = if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { $env:LOCALAPPDATA } else { (Join-Path $HOME ".code-intel") }
    return (Join-Path $base "code-intel\artifacts")
}

function Get-DefaultShadowRoot {
    $fromEnv = [Environment]::GetEnvironmentVariable("CODE_INTEL_SHADOW_ROOT", "User")
    if (-not [string]::IsNullOrWhiteSpace($fromEnv)) { return $fromEnv }
    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_SHADOW_ROOT)) { return $env:CODE_INTEL_SHADOW_ROOT }
    $base = if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { $env:LOCALAPPDATA } else { (Join-Path $HOME ".code-intel") }
    return (Join-Path $base "code-intel\repowise")
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
        $output = & $Body 2>&1
        $entry.output = ($output | Out-String).Trim()
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

function Get-RelativePathSafe {
    param(
        [string]$Base,
        [string]$Path
    )

    try {
        return [System.IO.Path]::GetRelativePath($Base, $Path)
    }
    catch {
        return $Path
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
    if ($status -eq "failed") {
        return "local_tool_error"
    }
    return $null
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
if ($null -ne $reposConfig -and -not [string]::IsNullOrWhiteSpace($Repo)) {
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
$runDir = Join-Path $artifactRoot $timestamp
New-Item -ItemType Directory -Force -Path $runDir | Out-Null

$steps = New-Object System.Collections.Generic.List[object]
$notes = New-Object System.Collections.Generic.List[string]

$toolState = [ordered]@{
    rg = Test-CommandAvailable "rg"
    repowise = Test-CommandAvailable "repowise"
    sentrux = Test-CommandAvailable "sentrux"
    git = Test-CommandAvailable "git"
}

if (-not $toolState.rg) {
    throw "Missing required tool: rg"
}

if ($RepowiseDocs -and -not $SkipRepowise) {
    $providerPreflightScript = Join-Path $PSScriptRoot "test-code-intel-provider.ps1"
    $preflightStep = Invoke-LoggedStep "provider preflight" {
        if (-not (Test-Path -LiteralPath $providerPreflightScript -PathType Leaf)) {
            throw "provider preflight script not found: $providerPreflightScript"
        }
        & $providerPreflightScript -Json
    }
    $steps.Add($preflightStep)
    if ($preflightStep.status -ne "passed") {
        $RepowiseDocs = $false
        $notes.Add("Repowise docs disabled because provider preflight failed. Index-only repowise will still run.")
    }
}

$steps.Add((Invoke-LoggedStep "git status" {
    if (-not $toolState.git) { throw "git not found" }
    git -C $repoPath status --short --branch
}))

$steps.Add((Invoke-LoggedStep "rg file inventory" {
    $rgArgs = @("--files", "--hidden")
    foreach ($pattern in $allInventoryExclude) {
        $rgArgs += @("-g", $pattern)
    }
    $rgArgs += $repoPath
    $files = & rg @rgArgs

    $fileListPath = Join-Path $runDir "files.txt"
    $files | Set-Content -LiteralPath $fileListPath -Encoding UTF8
    "files=$($files.Count)"
}))

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
        if ($RepowiseScopePaths.Count -gt 0 -or $RepowiseRootFiles.Count -gt 0) {
            $scopedRepowiseScript = Join-Path $PSScriptRoot "Invoke-ScopedRepowise.ps1"
            if ($RepowiseDocs -and $Mode -ne "lite") {
                $steps.Add((Invoke-LoggedStep "repowise scoped docs" {
                    & $scopedRepowiseScript `
                        -RepoPath $repoPath `
                        -ShadowRoot $RepowiseShadowRoot `
                        -ScopePaths $RepowiseScopePaths `
                        -RootFiles $RepowiseRootFiles `
                        -Docs
                }))
            }
            else {
                $steps.Add((Invoke-LoggedStep "repowise scoped index" {
                    & $scopedRepowiseScript `
                        -RepoPath $repoPath `
                        -ShadowRoot $RepowiseShadowRoot `
                        -ScopePaths $RepowiseScopePaths `
                        -RootFiles $RepowiseRootFiles
                }))
            }
        }
        else {
            Push-Location $repoPath
            try {
                $steps.Add((Invoke-LoggedStep "repowise status" {
                    repowise status
                }))

                if ($Mode -ne "lite") {
                    if (Test-Path -LiteralPath (Join-Path $repoPath ".repowise")) {
                        $steps.Add((Invoke-LoggedStep "repowise update" {
                            repowise update
                        }))
                    }
                    else {
                        $steps.Add((Invoke-LoggedStep "repowise init" {
                            repowise init
                        }))
                    }

                    if ($RepowiseDocs) {
                        $steps.Add((Invoke-LoggedStep "repowise docs" {
                            repowise update --docs --no-workspace
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
            $steps.Add((Invoke-LoggedStep "sentrux gate save" {
                sentrux gate --save $sentruxTargetPath
            }))
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
    localToolError = @($failureClassifications | Where-Object { $_.category -eq "local_tool_error" }).Count
    graphMissing = @($failureClassifications | Where-Object { $_.category -eq "graph_missing" }).Count
    sentruxFail = @($failureClassifications | Where-Object { $_.category -eq "sentrux_fail" }).Count
}

$report = [ordered]@{
    repo = $repoPath
    repoInput = $Repo
    repoName = $repoName
    mode = $Mode
    language = $Language
    artifactDir = $runDir
    sentruxPath = if ([string]::IsNullOrWhiteSpace($SentruxPath)) { $repoPath } else { (Resolve-ChildPath $repoPath $SentruxPath) }
    tools = $toolState
    understandCommand = $understandCommand
    steps = $steps
    notes = $notes
    failureClassifications = $failureClassifications
    summary = [ordered]@{
        failed = $failed.Count
        manualRequired = $manual.Count
        passed = @($steps | Where-Object { $_.status -eq "passed" }).Count
        skipped = @($steps | Where-Object { $_.status -eq "skipped" }).Count
        failureCategories = $failureCounts
    }
}

$reportPath = Join-Path $runDir "report.json"
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8

$summaryPath = Join-Path $runDir "summary.md"
$summaryLines = @(
    "# Code Intel Pipeline",
    "",
    "- Repo: $repoPath",
    "- Mode: $Mode",
    "- Report: $reportPath",
    "- Understand command: ``$understandCommand``",
    "",
    "## Summary",
    "",
    "- Passed: $(@($steps | Where-Object { $_.status -eq 'passed' }).Count)",
    "- Failed: $($failed.Count)",
    "- Manual required: $($manual.Count)",
    "- Skipped: $(@($steps | Where-Object { $_.status -eq 'skipped' }).Count)",
    "- Provider quota: $($failureCounts.providerQuota)",
    "- Local tool error: $($failureCounts.localToolError)",
    "- Graph missing: $($failureCounts.graphMissing)",
    "- Sentrux fail: $($failureCounts.sentruxFail)",
    "",
    "## Steps"
)
foreach ($step in $steps) {
    $summaryLines += "- $($step.status): $($step.name)"
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

Write-Host "Code intel pipeline complete"
Write-Host "Repo: $repoPath"
Write-Host "Report: $reportPath"
Write-Host "Summary: $summaryPath"
if ($manual.Count -gt 0) {
    Write-Host "Manual step required: $understandCommand"
}
if ($failed.Count -gt 0) {
    Write-Host "Failed steps: $($failed.Count)"
    exit 1
}
