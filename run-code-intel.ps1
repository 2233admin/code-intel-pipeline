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
    [int]$RepowiseTimeoutSeconds = 600,
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
        "manual_required" { return 55 }
        "skipped" { return 35 }
        default { return 0 }
    }
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

    $status = if ($null -ne $Step) {
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
        [string]$SurgeryTarget = "",
        [string]$CurrentTopHotspot = ""
    )

    $toolsOk = ([int]$FailureCounts.localToolError -eq 0)
    $graphOk = ([int]$FailureCounts.graphMissing -eq 0)
    $sentruxOk = ([int]$FailureCounts.sentruxFail -eq 0 -and $RulesExists -and $GateStatus -eq "passed" -and $CheckStatus -eq "passed")
    $surgeryDebtCleared = ($FailingWhatIfCount -eq 0)
    $postOpOk = ($toolsOk -and $graphOk -and $sentruxOk -and $surgeryDebtCleared)

    # surgery_plan -> post_op: the surgery target has actually been treated
    # (it no longer shows up as the current top hotspot) and sentrux confirms
    # the governed scope is clean, so it is safe to move on to post-op review.
    $surgeryTargetResolved = ([string]::IsNullOrWhiteSpace($SurgeryTarget) -or
        [string]::IsNullOrWhiteSpace($CurrentTopHotspot) -or
        ($SurgeryTarget -ne $CurrentTopHotspot))
    $surgeryToPostOpOk = ($sentruxOk -and $surgeryTargetResolved)

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
            graph_ok = $graphOk
            rules_exists = $RulesExists
            sentrux_check = $CheckStatus
            sentrux_gate = $GateStatus
            sentrux_ok = $sentruxOk
            failing_what_if = $FailingWhatIfCount
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
            "test-code-intel-pipeline.ps1 -RepoPath `"$RepoPath`" -SentruxPath `"$((Get-RelativePathSafe $RepoPath $SentruxTargetPath) -replace '\\', '/')`" -SkipRepowise -Mode normal"
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

    if ($FailureCounts.localToolError -gt 0) {
        return [ordered]@{ severity = "red"; primaryDiagnosis = "local tool failure" }
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
        [int]$FailingWhatIfCount
    )

    if ($FailureCounts.localToolError -gt 0) { return "triage" }
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

    $treatment = @()
    if ($FailureCounts.localToolError -gt 0) { $treatment += "Fix local tool errors before interpreting architecture signals." }
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
        [string]$SurgeryTarget = "",
        [string]$CurrentTopHotspot = ""
    )

    $diagnosis = Get-HospitalDiagnosis $FailureCounts $RulesExists $FailingWhatIfCount
    $nextProtocol = Get-HospitalNextProtocol $FailureCounts $RulesExists $FailingWhatIfCount
    $disposition = if ($diagnosis.severity -eq "green") { "discharge_ready" } else { "admit" }
    $admissionReason = Get-HospitalAdmissionReason $diagnosis.primaryDiagnosis
    $dischargeCriteria = @(
        "failure category counters are zero",
        "Sentrux check and gate pass for the governed scope",
        "hospital triage status is green or explicitly accepted for observation",
        "session_end reports no quality regression after Agent edits"
    )
    $treatment = Get-HospitalTreatmentPlan $FailureCounts $RulesExists $FailingWhatIfCount $UnderstandCommand $TopContextFile

    $stateMachine = New-HospitalStateMachine `
        -FailureCounts $FailureCounts `
        -RulesExists $RulesExists `
        -GateStatus $GateStatus `
        -CheckStatus $CheckStatus `
        -FailingWhatIfCount $FailingWhatIfCount `
        -Disposition $disposition `
        -NextProtocol $nextProtocol `
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
        [int]$CtScore,
        [int]$PetScore,
        [int]$GovernanceScore,
        [string]$RunDir,
        [string]$RepoPath,
        [int]$InventoryFiles,
        [object]$SentruxDsmSummary,
        [object]$SentruxFileDetailsSummary,
        [object]$CodeNexusContextSummary,
        [object]$SentruxWhatIfSummary,
        [string]$GovernanceArtifact,
        [string]$GovernanceFinding
    )

    $xrayFinding = if ($InventoryFiles -gt 0) { "$InventoryFiles files inventoried" } else { "no inventory" }
    $ctArtifact = if ($null -ne $SentruxDsmSummary) { [string]$SentruxDsmSummary.path } else { "" }
    $ctFinding = if ($null -ne $SentruxDsmSummary) { "$($SentruxDsmSummary.modules) modules, $($SentruxFileDetailsSummary.functions) functions" } else { "not generated" }
    $mriArtifact = if ($null -ne $CodeNexusContextSummary) { [string]$CodeNexusContextSummary.path } else { "" }
    $mriFinding = if ($null -ne $CodeNexusContextSummary) { "$($CodeNexusContextSummary.files) files, $($CodeNexusContextSummary.references) references" } else { "not generated" }
    $petArtifact = if ($null -ne $SentruxWhatIfSummary) { [string]$SentruxWhatIfSummary.path } else { "" }
    $petFinding = if ($null -ne $SentruxWhatIfSummary) { "$($SentruxWhatIfSummary.failing) failing what-if scenarios" } else { "not generated" }
    $chartFinding = if ($null -ne $RepowiseStep) { [string]$RepowiseStep.status } else { "not run" }

    return @(
        (New-Modality "xray" "fast file inventory and repo surface" $InventoryStep (Get-StepScore $InventoryStep) (Join-Path $RunDir "files.txt") $xrayFinding "Sees files, not semantic impact.")
        (New-Modality "anatomy" "Understand Anything architecture graph" $UnderstandStep $GraphScore (Join-Path $RepoPath ".understand-anything\knowledge-graph.json") (Get-FirstLine ([string]$UnderstandStep.output)) "Requires a prebuilt graph from the Understand tool.")
        (New-Modality "ct" "Sentrux DSM, hotspots, and structural slices" $SentruxGateStep $CtScore $ctArtifact $ctFinding "Static structure is not runtime truth.")
        (New-Modality "mri" "CodeNexus context and impact localization" $null $MriScore $mriArtifact $mriFinding "Lite mode is local evidence, not a full semantic backend.")
        (New-Modality "pet" "execution proxy: test gaps, evolution, and what-if risk" $null $PetScore $petArtifact $petFinding "No live runtime trace is captured yet.")
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
        [object]$SentruxHotspotsSummary,
        [object]$SentruxWhatIfSummary,
        [object]$CodeNexusContextSummary
    )

    return [ordered]@{
        dsm = Read-HospitalArtifactFile $SentruxDsmSummary
        hotspots = Read-HospitalArtifactFile $SentruxHotspotsSummary
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
    $excludedFiles = if ($null -ne $DsmObject -and $null -ne $DsmObject.scope -and $null -ne $DsmObject.scope.excluded_files) { [int]$DsmObject.scope.excluded_files } else { 0 }
    $sourceScopeStatus = if ($inventoryFiles -gt 0 -and $scanFiles -gt 0) { "measured" } elseif ($inventoryFiles -gt 0) { "inventory_only" } else { "missing" }

    return [ordered]@{
        inventory_files = $inventoryFiles
        scan_files = $scanFiles
        unresolved_imports = $unresolvedImports
        resolved_imports = $resolvedImports
        resolved_ratio = $resolvedRatio
        excluded_files = $excludedFiles
        source_scope_status = $sourceScopeStatus
    }
}

function Get-ImportResolutionScore {
    param([object]$ResolvedRatio)

    if ($null -eq $ResolvedRatio) { return 50 }
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

    if ($ScanFiles -gt 0) { return 100 }
    if ($InventoryFiles -gt 0) { return 70 }

    return 0
}

function New-HospitalScoreBlock {
    param(
        [object]$SentruxInsight,
        [object]$Measurements,
        [object]$UnderstandStep,
        [object]$RepowiseStep,
        [object]$SentruxCheckStep,
        [object]$SentruxGateStep,
        [object]$SentruxDsmSummary,
        [object]$SentruxFileDetailsSummary,
        [object]$SentruxEvolutionSummary,
        [object]$SentruxWhatIfSummary,
        [object]$CodeNexusContextSummary
    )

    $rulesExists = [bool]$SentruxInsight["rulesExists"]
    $rulesScore = if ($rulesExists) { 100 } else { 45 }
    $gateScore = Get-StepScore $SentruxGateStep
    $checkScore = Get-StepScore $SentruxCheckStep
    $graphScore = Get-StepScore $UnderstandStep
    $memoryScore = Get-StepScore $RepowiseStep
    $mriScore = if ($null -ne $CodeNexusContextSummary) { 100 } else { 35 }
    $ctScore = if ($null -ne $SentruxDsmSummary -and $null -ne $SentruxFileDetailsSummary) { 100 } else { 35 }
    $petScore = if ($null -ne $SentruxWhatIfSummary -and $null -ne $SentruxEvolutionSummary) { 70 } else { 25 }
    $resolutionScore = Get-ImportResolutionScore $Measurements.resolved_ratio
    $pollutionScore = if ($Measurements.excluded_files -gt 0) { 100 } else { 80 }
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
        ct_score = $ctScore
        pet_score = $petScore
        resolution_score = $resolutionScore
        pollution_score = $pollutionScore
        governance_score = $governanceScore
        diagnostic_score = $diagnosticScore
        overall_score = $overallScore
        source_coverage_score = Get-SourceCoverageScore $Measurements.scan_files $Measurements.inventory_files
        import_resolution_status = if ($null -eq $resolvedRatio) { "unknown" } else { "$resolvedRatio%" }
        pollution_status = if ($Measurements.excluded_files -gt 0) { "quarantined" } else { "clean_or_unknown" }
        governance_status = if ($rulesExists) { "rules_present" } else { "rules_missing" }
        governance_artifact = $governanceArtifact
        governance_finding = "rules=$($SentruxInsight['rulesExists']); gate=$($SentruxInsight['gateStatus']); check=$($SentruxInsight['checkStatus'])"
        governance_evidence = "gate=$($SentruxInsight['gateStatus']); check=$($SentruxInsight['checkStatus'])"
        localization_status = if ($null -ne $CodeNexusContextSummary) { "available" } else { "missing" }
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
        [string]$UnderstandCommand,
        [object]$ToolState
    )

    $gitStep = Get-StepMatch $Steps "git status"
    $inventoryStep = Get-StepMatch $Steps "rg file inventory"
    $understandStep = Get-StepMatch $Steps "understand graph"
    $repowiseStep = Get-StepMatch $Steps "repowise*" -Last
    $sentruxCheckStep = Get-StepMatch $Steps "sentrux check"
    $sentruxGateStep = Get-StepMatch $Steps "sentrux gate*" -Last

    $artifacts = Read-HospitalArtifacts $SentruxDsmSummary $SentruxHotspotsSummary $SentruxWhatIfSummary $CodeNexusContextSummary
    $measurements = New-HospitalMeasurements $inventoryStep $SentruxInsight $artifacts.dsm
    $scores = New-HospitalScoreBlock `
        -SentruxInsight $SentruxInsight `
        -Measurements $measurements `
        -UnderstandStep $understandStep `
        -RepowiseStep $repowiseStep `
        -SentruxCheckStep $sentruxCheckStep `
        -SentruxGateStep $sentruxGateStep `
        -SentruxDsmSummary $SentruxDsmSummary `
        -SentruxFileDetailsSummary $SentruxFileDetailsSummary `
        -SentruxEvolutionSummary $SentruxEvolutionSummary `
        -SentruxWhatIfSummary $SentruxWhatIfSummary `
        -CodeNexusContextSummary $CodeNexusContextSummary
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
        -SurgeryTarget $surgeryTarget `
        -CurrentTopHotspot $currentTopHotspot

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
        -CtScore $scores.ct_score `
        -PetScore $scores.pet_score `
        -GovernanceScore $scores.governance_score `
        -RunDir $RunDir `
        -RepoPath $RepoPath `
        -InventoryFiles $measurements.inventory_files `
        -SentruxDsmSummary $SentruxDsmSummary `
        -SentruxFileDetailsSummary $SentruxFileDetailsSummary `
        -CodeNexusContextSummary $CodeNexusContextSummary `
        -SentruxWhatIfSummary $SentruxWhatIfSummary `
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
        }
        triage = [ordered]@{
            status = $decision.severity
            disposition = $decision.disposition
            primary_diagnosis = $decision.primaryDiagnosis
            overall_score = $scores.overall_score
            next_protocol = $decision.nextProtocol
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
        "- Current state: $($Hospital.state_machine.current_state)",
        "",
        "## Imaging Modalities"
    )
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
                $repowiseStep = Invoke-LoggedStep "repowise scoped docs" {
                    & $scopedRepowiseScript `
                        -RepoPath $repoPath `
                        -ShadowRoot $RepowiseShadowRoot `
                        -ScopePaths $RepowiseScopePaths `
                        -RootFiles $RepowiseRootFiles `
                        -TimeoutSeconds $RepowiseTimeoutSeconds `
                        -Docs
                }
                $steps.Add((Convert-OptionalRepowiseTimeout $repowiseStep))
            }
            else {
                $repowiseStep = Invoke-LoggedStep "repowise scoped index" {
                    & $scopedRepowiseScript `
                        -RepoPath $repoPath `
                        -ShadowRoot $RepowiseShadowRoot `
                        -ScopePaths $RepowiseScopePaths `
                        -RootFiles $RepowiseRootFiles `
                        -TimeoutSeconds $RepowiseTimeoutSeconds
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
                    $repowiseStatePath = Join-Path $repowiseDir "state.json"
                    $repowiseDbPath = Join-Path $repowiseDir "wiki.db"
                    if ((Test-Path -LiteralPath $repowiseStatePath -PathType Leaf) -or (Test-Path -LiteralPath $repowiseDbPath -PathType Leaf)) {
                        $steps.Add((Invoke-LoggedStep "repowise update" {
                            repowise update --no-workspace --index-only
                        }))
                    }
                    else {
                        $steps.Add((Invoke-LoggedStep "repowise init" {
                            @("all", "1") | repowise init . --index-only -y --no-claude-md --embedder mock --provider mock
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
$hospitalReport = New-CodeIntelHospitalReport `
    -RepoPath $repoPath `
    -Mode $Mode `
    -RunDir $runDir `
    -ReportPath $reportPath `
    -SummaryPath $summaryPath `
    -UnderstandingPath $understandingPath `
    -Steps $steps `
    -FailureCounts $failureCounts `
    -SentruxInsight $sentruxInsight `
    -SentruxDsmSummary $sentruxDsmSummary `
    -SentruxFileDetailsSummary $sentruxFileDetailsSummary `
    -SentruxHotspotsSummary $sentruxHotspotsSummary `
    -SentruxEvolutionSummary $sentruxEvolutionSummary `
    -SentruxWhatIfSummary $sentruxWhatIfSummary `
    -CodeNexusContextSummary $codeNexusContextSummary `
    -UnderstandCommand $understandCommand `
    -ToolState $toolState
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
$surgeryPlan | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $surgeryPlanPath -Encoding UTF8
Convert-SurgeryPlanToMarkdown $surgeryPlan | Set-Content -LiteralPath $surgeryMarkdownPath -Encoding UTF8
$hospitalReport["artifacts"]["surgeryPlan"] = $surgeryPlanPath
$hospitalReport["artifacts"]["surgeryPlanMarkdown"] = $surgeryMarkdownPath
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
    codeNexusContext = $codeNexusContextSummary
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
    }
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

$report["understanding"] = $understandingPath
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8

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
    "- Hospital: $hospitalMarkdownPath",
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
    "- Hospital status: $($hospitalReport.triage.status)",
    "- Hospital disposition: $($hospitalReport.triage.disposition)",
    "- Hospital state: $($hospitalReport.state_machine.current_state)",
    "- Hospital score: $($hospitalReport.triage.overall_score)",
    "- Next protocol: $($hospitalReport.triage.next_protocol)",
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
if ($null -ne $codeNexusContextSummary) {
    $summaryLines += "- CodeNexus context: $($codeNexusContextSummary.path) (files=$($codeNexusContextSummary.files), references=$($codeNexusContextSummary.references), commits=$($codeNexusContextSummary.recentCommits), topFile=$($codeNexusContextSummary.topFile))"
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
    "- Hospital report: ``$hospitalReportPath``",
    "- Hospital markdown: ``$hospitalMarkdownPath``",
    "- Sentrux DSM: $(if ($null -ne $sentruxDsmSummary) { '``' + $sentruxDsmSummary.path + '``' } else { 'not generated' })",
    "- Sentrux file details: $(if ($null -ne $sentruxFileDetailsSummary) { '``' + $sentruxFileDetailsSummary.path + '``' } else { 'not generated' })",
    "- Sentrux hotspots: $(if ($null -ne $sentruxHotspotsSummary) { '``' + $sentruxHotspotsSummary.path + '``' } else { 'not generated' })",
    "- Sentrux evolution: $(if ($null -ne $sentruxEvolutionSummary) { '``' + $sentruxEvolutionSummary.path + '``' } else { 'not generated' })",
    "- Sentrux what-if: $(if ($null -ne $sentruxWhatIfSummary) { '``' + $sentruxWhatIfSummary.path + '``' } else { 'not generated' })",
    "- CodeNexus context: $(if ($null -ne $codeNexusContextSummary) { '``' + $codeNexusContextSummary.path + '``' } else { 'not generated' })",
    "- Tools: rg=$($toolState.rg), git=$($toolState.git), repowise=$($toolState.repowise), sentrux=$($toolState.sentrux)",
    "- Passed steps: $(Join-StatusNames $passedSteps)",
    "- Hospital: status=$($hospitalReport.triage.status), disposition=$($hospitalReport.triage.disposition), state=$($hospitalReport.state_machine.current_state), score=$($hospitalReport.triage.overall_score), next=$($hospitalReport.triage.next_protocol)",
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
Write-Host "Hospital: $hospitalMarkdownPath"
if ($manual.Count -gt 0) {
    Write-Host "Manual step required: $understandCommand"
}
if ($failed.Count -gt 0) {
    Write-Host "Failed steps: $($failed.Count)"
    exit 1
}
