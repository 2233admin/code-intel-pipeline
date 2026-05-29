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

[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$OutputEncoding = [System.Text.UTF8Encoding]::new()
$env:PYTHONIOENCODING = "utf-8"
$env:PYTHONUTF8 = "1"
$env:TERM = "xterm"
$env:NO_COLOR = "1"
$env:RICH_FORCE_TERMINAL = "0"

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

function New-SentruxInsight {
    param(
        [string]$RepoName,
        [string]$TargetPath,
        [string]$BaselinePath,
        [object[]]$Steps
    )

    $gateStep = @($Steps | Where-Object { $_.name -like "sentrux gate*" } | Select-Object -Last 1)
    $checkStep = @($Steps | Where-Object { $_.name -eq "sentrux check" } | Select-Object -First 1)
    $rulesPath = if ([string]::IsNullOrWhiteSpace($TargetPath)) { "" } else { Join-Path $TargetPath ".sentrux\rules.toml" }
    $baseline = Read-JsonFileSafe $BaselinePath
    $gateOutput = if ($gateStep.Count -gt 0) { [string]$gateStep[0].output } else { "" }

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
        noDegradation = ($gateOutput -match "No degradation detected")
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
                    repowise status --no-workspace
                }))

                if ($Mode -ne "lite") {
                    $repowiseDir = Join-Path $repoPath ".repowise"
                    $repowiseStatePath = Join-Path $repowiseDir "state.json"
                    $repowiseDbPath = Join-Path $repowiseDir "wiki.db"
                    if ((Test-Path -LiteralPath $repowiseStatePath -PathType Leaf) -or (Test-Path -LiteralPath $repowiseDbPath -PathType Leaf)) {
                        $steps.Add((Invoke-LoggedStep "repowise update" {
                            repowise update --no-workspace --index-only
                        }))
                    }
                    else {
                        $steps.Add((Invoke-LoggedStep "repowise init" {
                            @("n") | repowise init . --index-only -y --no-claude-md --no-onboarding --embedder mock --provider mock -x "tmp/**" -x "**/tmp/**" -x "**/*.egg-info/**" -x "uv.lock" -x "**/uv.lock" -x "*.bak" -x "**/*.bak"
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

$sentruxInsight = New-SentruxInsight -RepoName $repoName -TargetPath $sentruxTargetPath -BaselinePath $baselinePath -Steps $steps
$sentruxDsmPath = Join-Path $runDir "sentrux-dsm.json"
$sentruxFileDetailsPath = Join-Path $runDir "sentrux-file-details.json"
$sentruxHotspotsPath = Join-Path $runDir "sentrux-hotspots.json"
$sentruxEvolutionPath = Join-Path $runDir "sentrux-evolution.json"
$sentruxWhatIfPath = Join-Path $runDir "sentrux-what-if.json"
$sentruxDsmSummary = $null
$sentruxFileDetailsSummary = $null
$sentruxHotspotsSummary = $null
$sentruxEvolutionSummary = $null
$sentruxWhatIfSummary = $null
$sentruxAgentTool = Join-Path $PSScriptRoot "Invoke-SentruxAgentTool.ps1"
if (-not [string]::IsNullOrWhiteSpace($sentruxTargetPath) -and (Test-Path -LiteralPath $sentruxAgentTool -PathType Leaf)) {
    try {
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
    sentruxInsight = $sentruxInsight
    sentruxDsm = $sentruxDsmSummary
    sentruxFileDetails = $sentruxFileDetailsSummary
    sentruxHotspots = $sentruxHotspotsSummary
    sentruxEvolution = $sentruxEvolutionSummary
    sentruxWhatIf = $sentruxWhatIfSummary
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
$understandingPath = Join-Path $runDir "understanding.md"
$report["understanding"] = $understandingPath
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8

$summaryPath = Join-Path $runDir "summary.md"
$sentruxMetrics = @($sentruxInsight['metrics'])
$sentruxNextActions = @($sentruxInsight['nextActions'])
$sentruxCodeNexusHints = @($sentruxInsight['codeNexusHints'])
$sentruxScan = $sentruxInsight['scan']
$summaryLines = @(
    "# Code Intel Pipeline",
    "",
    "- Repo: $repoPath",
    "- Mode: $Mode",
    "- Report: $reportPath",
    "- Understanding: $understandingPath",
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
$summaryLines += ""
$summaryLines += "## Sentrux Insight"
$summaryLines += "- Target: $($sentruxInsight['targetPath'])"
$summaryLines += "- Baseline: $($sentruxInsight['baselinePath'])"
$summaryLines += "- Rules: $($sentruxInsight['rulesPath'])"
$summaryLines += "- Gate: $($sentruxInsight['gateStatus'])"
$summaryLines += "- Check: $($sentruxInsight['checkStatus'])"
$summaryLines += "- No degradation: $($sentruxInsight['noDegradation'])"
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
elseif ($failureCounts.localToolError -gt 0) {
    $nextAction = "Fix the local tool error shown in report.json, then rerun the pipeline."
}
elseif ($failureCounts.graphMissing -gt 0) {
    $nextAction = "Run the emitted Understand Anything command in Claude, then rerun the pipeline."
}
elseif ($failureCounts.sentruxFail -gt 0) {
    $nextAction = "Inspect the Sentrux violation or regression before changing or saving a baseline."
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
    "- Sentrux DSM: $(if ($null -ne $sentruxDsmSummary) { '``' + $sentruxDsmSummary.path + '``' } else { 'not generated' })",
    "- Sentrux file details: $(if ($null -ne $sentruxFileDetailsSummary) { '``' + $sentruxFileDetailsSummary.path + '``' } else { 'not generated' })",
    "- Sentrux hotspots: $(if ($null -ne $sentruxHotspotsSummary) { '``' + $sentruxHotspotsSummary.path + '``' } else { 'not generated' })",
    "- Sentrux evolution: $(if ($null -ne $sentruxEvolutionSummary) { '``' + $sentruxEvolutionSummary.path + '``' } else { 'not generated' })",
    "- Sentrux what-if: $(if ($null -ne $sentruxWhatIfSummary) { '``' + $sentruxWhatIfSummary.path + '``' } else { 'not generated' })",
    "- Tools: rg=$($toolState.rg), git=$($toolState.git), repowise=$($toolState.repowise), sentrux=$($toolState.sentrux)",
    "- Passed steps: $(Join-StatusNames $passedSteps)",
    "",
    "## Unverified Or Inferred",
    "- Understand graph: $(if ($graphStep.Count -gt 0) { $graphStep[0].status } else { 'not checked' })",
    "- Repowise state: $(Join-StatusNames $repowiseSteps)",
    "- Sentrux state: $(Join-StatusNames $sentruxSteps)",
    "- Sentrux gate insight: gate=$($sentruxInsight['gateStatus']), noDegradation=$($sentruxInsight['noDegradation']), rules=$($sentruxInsight['rulesExists'])",
    "- Skipped steps: $(Join-StatusNames $skippedSteps)",
    "",
    "## Sentrux Structural Signal",
    "$(if ($sentruxMetrics.Count -gt 0) { ($sentruxMetrics | ForEach-Object { '- ' + $_.name + ': ' + $_.before + ' -> ' + $_.after + ' (delta ' + $_.delta + ', regressed=' + $_.regressed + ')' }) -join [Environment]::NewLine } else { '- no parsed metrics' })",
    "$(if ($sentruxNextActions.Count -gt 0) { ($sentruxNextActions | ForEach-Object { '- next: ' + $_ }) -join [Environment]::NewLine } else { '- next: none' })",
    "$(if ($sentruxCodeNexusHints.Count -gt 0) { ($sentruxCodeNexusHints | ForEach-Object { '- codenexus: ' + $_ }) -join [Environment]::NewLine } else { '- codenexus: none' })",
    "",
    "## Failure Categories",
    "- provider_quota: $($failureCounts.providerQuota)",
    "- local_tool_error: $($failureCounts.localToolError)",
    "- graph_missing: $($failureCounts.graphMissing)",
    "- sentrux_fail: $($failureCounts.sentruxFail)",
    "",
    "## Human Inspection Required",
    "- Read `summary.md` first.",
    "- If `graph_missing > 0`, run: ``$understandCommand``",
    "- If ``sentrux_fail > 0``, inspect Sentrux output in ``report.json`` before saving a new baseline.",
    "- If ``provider_quota > 0``, treat it as an upstream quota/rate issue, not a local indexing failure.",
    "- If ``local_tool_error > 0``, inspect command output and PATH/tool installation before changing repo code.",
    "",
    "## Problem Steps",
    "$(if ($problemSteps.Count -gt 0) { ($problemSteps | ForEach-Object { '- ' + $_.name + ': ' + $_.status }) -join [Environment]::NewLine } else { '- none' })",
    "",
    "## Next Action",
    $nextAction
)
$understandingLines | Set-Content -LiteralPath $understandingPath -Encoding UTF8

Write-Host "Code intel pipeline complete"
Write-Host "Repo: $repoPath"
Write-Host "Report: $reportPath"
Write-Host "Summary: $summaryPath"
Write-Host "Understanding: $understandingPath"
if ($manual.Count -gt 0) {
    Write-Host "Manual step required: $understandCommand"
}
if ($failed.Count -gt 0) {
    Write-Host "Failed steps: $($failed.Count)"
    exit 1
}
