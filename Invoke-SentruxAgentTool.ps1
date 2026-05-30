param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateSet("scan", "health", "session_start", "session_end", "rescan", "check_rules", "evolution", "dsm", "git_stats", "test_gaps", "what_if", "sentrux_scan", "sentrux_health", "sentrux_dsm", "sentrux_git_stats", "sentrux_test_gaps")]
    [string]$Tool,

    [Parameter(Position = 1)]
    [string]$Path = ".",

    [string]$SessionId = "",
    [int]$Recent = 10
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$OutputEncoding = [System.Text.UTF8Encoding]::new()
$env:PYTHONIOENCODING = "utf-8"
$env:PYTHONUTF8 = "1"
$env:NO_COLOR = "1"

function Resolve-Directory {
    param([string]$InputPath)

    $item = Get-Item -LiteralPath $InputPath -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        throw "Path is not a directory: $InputPath"
    }
    return $item.FullName
}

function Invoke-Native {
    param(
        [string]$Command,
        [string[]]$Arguments
    )

    $global:LASTEXITCODE = 0
    $started = Get-Date
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & $Command @Arguments 2>&1
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $finished = Get-Date

    return [pscustomobject][ordered]@{
        command = ($Command + " " + ($Arguments -join " ")).Trim()
        exitCode = $global:LASTEXITCODE
        output = ($output | ForEach-Object { $_.ToString() } | Out-String).Trim()
        durationMs = [int]($finished - $started).TotalMilliseconds
    }
}

function ConvertTo-NullableDouble {
    param([object]$Value)

    if ($null -eq $Value) { return $null }
    try { return [double]$Value } catch { return $null }
}

function Convert-QualitySignal {
    param([object]$Value)

    $number = ConvertTo-NullableDouble $Value
    if ($null -eq $number) { return $null }
    if ($number -le 1) { return [int][math]::Round($number * 10000) }
    return [int][math]::Round($number)
}

function Read-JsonFileSafe {
    param([string]$FilePath)

    if ([string]::IsNullOrWhiteSpace($FilePath) -or -not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        return $null
    }
    try {
        return Get-Content -LiteralPath $FilePath -Raw | ConvertFrom-Json
    }
    catch {
        return $null
    }
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

function Get-MetricPair {
    param(
        [string]$Text,
        [string]$Label
    )

    if ([string]::IsNullOrWhiteSpace($Text)) { return $null }
    $pattern = [regex]::Escape($Label) + ":\s+([0-9.]+)\s+\S+\s+([0-9.]+)"
    $match = [regex]::Match($Text, $pattern)
    if ($match.Success) {
        return [ordered]@{
            before = ConvertTo-NullableDouble $match.Groups[1].Value
            after = ConvertTo-NullableDouble $match.Groups[2].Value
        }
    }

    $singlePattern = [regex]::Escape($Label) + ":\s+([0-9.]+)"
    $single = [regex]::Match($Text, $singlePattern)
    if ($single.Success) {
        $value = ConvertTo-NullableDouble $single.Groups[1].Value
        return [ordered]@{
            before = $null
            after = $value
        }
    }

    return $null
}

function Get-ScanStats {
    param([string]$Text)

    $stats = [ordered]@{}

    $scanMatch = [regex]::Match($Text, "\[scan\]\s+git ls-files:\s+([0-9]+)\s+total,\s+([0-9]+)\s+kept,\s+([0-9]+)\s+dropped")
    if ($scanMatch.Success) {
        $stats["gitFilesTotal"] = [int]$scanMatch.Groups[1].Value
        $stats["gitFilesKept"] = [int]$scanMatch.Groups[2].Value
        $stats["gitFilesDropped"] = [int]$scanMatch.Groups[3].Value
    }

    $mapMatch = [regex]::Match($Text, "\[build_project_map\]\s+([0-9]+)\s+files,\s+([0-9]+)\s+unique dirs")
    if ($mapMatch.Success) {
        $stats["files"] = [int]$mapMatch.Groups[1].Value
        $stats["uniqueDirs"] = [int]$mapMatch.Groups[2].Value
    }

    $resolveMatch = [regex]::Match($Text, "\[resolve\]\s+([0-9]+)\s+resolved,\s+([0-9]+)\s+unresolved")
    if ($resolveMatch.Success) {
        $stats["resolvedImports"] = [int]$resolveMatch.Groups[1].Value
        $stats["unresolvedImports"] = [int]$resolveMatch.Groups[2].Value
    }

    $graphMatch = [regex]::Match($Text, "\[build_graphs\]\s+([0-9]+)\s+files.*\|\s+([0-9]+)\s+import,\s+([0-9]+)\s+call,\s+([0-9]+)\s+inherit edges")
    if ($graphMatch.Success) {
        $stats["files"] = [int]$graphMatch.Groups[1].Value
        $stats["importEdges"] = [int]$graphMatch.Groups[2].Value
        $stats["callEdges"] = [int]$graphMatch.Groups[3].Value
        $stats["inheritEdges"] = [int]$graphMatch.Groups[4].Value
    }

    return $stats
}

function Parse-SentruxOutput {
    param([string]$Text)

    $quality = Get-MetricPair $Text "Quality"
    $coupling = Get-MetricPair $Text "Coupling"
    $cycles = Get-MetricPair $Text "Cycles"
    $godFiles = Get-MetricPair $Text "God files"
    $violations = $null
    $violationMatch = [regex]::Match($Text, "([0-9]+)\s+violation\(s\)\s+found")
    if ($violationMatch.Success) {
        $violations = [int]$violationMatch.Groups[1].Value
    }

    $distance = $null
    $distanceMatch = [regex]::Match($Text, "Distance from Main Sequence:\s+([0-9.]+)")
    if ($distanceMatch.Success) {
        $distance = ConvertTo-NullableDouble $distanceMatch.Groups[1].Value
    }

    return [ordered]@{
        quality_before = if ($null -ne $quality) { Convert-QualitySignal $quality["before"] } else { $null }
        quality_signal = if ($null -ne $quality) { Convert-QualitySignal $quality["after"] } else { $null }
        coupling_before = if ($null -ne $coupling) { ConvertTo-NullableDouble $coupling["before"] } else { $null }
        coupling = if ($null -ne $coupling) { ConvertTo-NullableDouble $coupling["after"] } else { $null }
        cycles_before = if ($null -ne $cycles) { ConvertTo-NullableDouble $cycles["before"] } else { $null }
        cycles = if ($null -ne $cycles) { ConvertTo-NullableDouble $cycles["after"] } else { $null }
        god_files_before = if ($null -ne $godFiles) { ConvertTo-NullableDouble $godFiles["before"] } else { $null }
        god_files = if ($null -ne $godFiles) { ConvertTo-NullableDouble $godFiles["after"] } else { $null }
        distance_from_main_sequence = $distance
        no_degradation = ($Text -match "No degradation detected")
        violations = $violations
        scan = Get-ScanStats $Text
    }
}

function Get-BaselineMetrics {
    param([string]$TargetPath)

    $baselinePath = Join-Path $TargetPath ".sentrux\baseline.json"
    $baseline = Read-JsonFileSafe $baselinePath
    if ($null -eq $baseline) { return $null }

    return [ordered]@{
        path = $baselinePath
        quality_signal = Convert-QualitySignal (Get-JsonProperty $baseline "quality_signal")
        coupling = ConvertTo-NullableDouble (Get-JsonProperty $baseline "coupling_score")
        cycles = ConvertTo-NullableDouble (Get-JsonProperty $baseline "cycle_count")
        god_files = ConvertTo-NullableDouble (Get-JsonProperty $baseline "god_file_count")
        complex_functions = ConvertTo-NullableDouble (Get-JsonProperty $baseline "complex_fn_count")
        total_import_edges = ConvertTo-NullableDouble (Get-JsonProperty $baseline "total_import_edges")
        cross_module_edges = ConvertTo-NullableDouble (Get-JsonProperty $baseline "cross_module_edges")
    }
}

function Get-Bottleneck {
    param([object]$Metrics)

    $scan = $Metrics["scan"]
    if ($null -ne $Metrics["cycles"] -and $Metrics["cycles"] -gt 0) { return "cycles" }
    if ($null -ne $Metrics["god_files"] -and $Metrics["god_files"] -gt 0) { return "god_files" }
    if ($null -ne $Metrics["coupling"] -and $Metrics["coupling"] -ge 0.35) { return "modularity" }
    if ($null -ne $scan -and $scan.Contains("unresolvedImports") -and $scan.Contains("resolvedImports")) {
        $total = $scan["resolvedImports"] + $scan["unresolvedImports"]
        if ($total -gt 0 -and ($scan["unresolvedImports"] / $total) -gt 0.5) {
            return "import_resolution"
        }
    }
    if ($null -ne $Metrics["quality_signal"] -and $Metrics["quality_signal"] -lt 6000) { return "quality" }
    return "none"
}

function Find-ScopeCandidates {
    param([string]$TargetPath)

    $items = @()
    try {
        $items = Get-ChildItem -LiteralPath $TargetPath -Recurse -Filter "baseline.json" -File -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match "\\.sentrux\\baseline\.json$" } |
            Select-Object -First 12
    }
    catch {
        $items = @()
    }

    return @($items | ForEach-Object {
        $scope = Split-Path -Parent (Split-Path -Parent $_.FullName)
        [ordered]@{
            path = $scope
            relative_path = [System.IO.Path]::GetRelativePath($TargetPath, $scope)
            baseline = $_.FullName
        }
    })
}

function Get-PollutionSignals {
    param([string]$TargetPath)

    $noisyDirs = @(
        [ordered]@{ path = "node_modules"; reason = "dependency directory excluded from governed source graph"; inspectNestedGit = $false },
        [ordered]@{ path = ".pnpm"; reason = "dependency store excluded from governed source graph"; inspectNestedGit = $false },
        [ordered]@{ path = ".yarn"; reason = "dependency store excluded from governed source graph"; inspectNestedGit = $false },
        [ordered]@{ path = "vendor"; reason = "vendored code excluded from governed source graph"; inspectNestedGit = $true },
        [ordered]@{ path = "vendors"; reason = "vendored code excluded from governed source graph"; inspectNestedGit = $true },
        [ordered]@{ path = "third_party"; reason = "third-party code excluded from governed source graph"; inspectNestedGit = $true },
        [ordered]@{ path = "third-party"; reason = "third-party code excluded from governed source graph"; inspectNestedGit = $true },
        [ordered]@{ path = "external"; reason = "external code excluded from governed source graph"; inspectNestedGit = $true },
        [ordered]@{ path = "research"; reason = "research or reference tree excluded from governed source graph"; inspectNestedGit = $true },
        [ordered]@{ path = "sandbox"; reason = "sandbox tree excluded from governed source graph"; inspectNestedGit = $false },
        [ordered]@{ path = "dist"; reason = "build output excluded from governed source graph"; inspectNestedGit = $false },
        [ordered]@{ path = "build"; reason = "build output excluded from governed source graph"; inspectNestedGit = $false },
        [ordered]@{ path = "target"; reason = "build output excluded from governed source graph"; inspectNestedGit = $false },
        [ordered]@{ path = "static\assets"; reason = "bundled static assets excluded from governed source graph"; inspectNestedGit = $false },
        [ordered]@{ path = "public\assets"; reason = "bundled static assets excluded from governed source graph"; inspectNestedGit = $false },
        [ordered]@{ path = "tools"; reason = "common tool or generated-support directory"; inspectNestedGit = $true }
    )
    $signals = @()
    foreach ($entry in $noisyDirs) {
        $dir = [string]$entry["path"]
        $full = Join-Path $TargetPath $dir
        if (-not (Test-Path -LiteralPath $full -PathType Container)) { continue }
        $nestedGit = @()
        if ([bool]$entry["inspectNestedGit"]) {
            $nestedGit = @(Get-ChildItem -LiteralPath $full -Recurse -Directory -Force -Filter ".git" -ErrorAction SilentlyContinue | Select-Object -First 5)
        }
        $signals += [ordered]@{
            path = $dir
            nested_git_count_sample = $nestedGit.Count
            reason = if ($nestedGit.Count -gt 0) { "contains nested repositories" } else { [string]$entry["reason"] }
        }
    }
    return $signals
}

function Get-SessionDir {
    param([string]$TargetPath)

    return Join-Path $TargetPath ".sentrux\agent-sessions"
}

function New-SessionId {
    return (Get-Date -Format "yyyyMMdd-HHmmss")
}

function Get-LatestSessionId {
    param([string]$TargetPath)

    $dir = Get-SessionDir $TargetPath
    if (-not (Test-Path -LiteralPath $dir -PathType Container)) { return "" }
    $latest = Get-ChildItem -LiteralPath $dir -Filter "*.start.json" -File |
        Sort-Object Name -Descending |
        Select-Object -First 1
    if ($null -eq $latest) { return "" }
    return ($latest.BaseName -replace "\.start$", "")
}

function Write-JsonFile {
    param(
        [string]$FilePath,
        [object]$Data
    )

    $parent = Split-Path -Parent $FilePath
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $Data | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $FilePath -Encoding UTF8
}

function Invoke-Gate {
    param(
        [string]$TargetPath,
        [switch]$Save
    )

    $args = @("gate")
    if ($Save) { $args += "--save" }
    $args += $TargetPath
    $native = Invoke-Native "sentrux" $args
    $metrics = Parse-SentruxOutput $native.output
    $baseline = Get-BaselineMetrics $TargetPath

    if ($null -eq $metrics["quality_signal"] -and $null -ne $baseline) {
        $metrics["quality_signal"] = $baseline["quality_signal"]
    }
    if ($null -eq $metrics["coupling"] -and $null -ne $baseline) {
        $metrics["coupling"] = $baseline["coupling"]
    }
    if ($null -eq $metrics["cycles"] -and $null -ne $baseline) {
        $metrics["cycles"] = $baseline["cycles"]
    }
    if ($null -eq $metrics["god_files"] -and $null -ne $baseline) {
        $metrics["god_files"] = $baseline["god_files"]
    }

    return [ordered]@{
        pass = ($native.exitCode -eq 0)
        status = if ($native.exitCode -eq 0) { "passed" } else { "failed" }
        exit_code = $native.exitCode
        duration_ms = $native.durationMs
        metrics = $metrics
        baseline = $baseline
        bottleneck = Get-Bottleneck $metrics
        raw_output = $native.output
    }
}

function Invoke-ScanTool {
    param(
        [string]$TargetPath,
        [string]$ToolName = "scan"
    )

    $baselinePath = Join-Path $TargetPath ".sentrux\baseline.json"
    $rulesPath = Join-Path $TargetPath ".sentrux\rules.toml"
    $gate = Invoke-Gate $TargetPath
    $metrics = $gate["metrics"]
    $inventory = Get-SourceFileInventory $TargetPath
    $pollutionSignals = @(Get-PollutionSignals $TargetPath)
    $scopeCandidates = @(Find-ScopeCandidates $TargetPath)
    $baselineExists = Test-Path -LiteralPath $baselinePath -PathType Leaf
    $rulesExists = Test-Path -LiteralPath $rulesPath -PathType Leaf
    $status = if (-not $baselineExists) { "baseline_missing" } else { $gate["status"] }

    return [ordered]@{
        tool = $ToolName
        path = $TargetPath
        status = $status
        quality_signal = $metrics["quality_signal"]
        files = if ($metrics["scan"].Contains("files")) { $metrics["scan"]["files"] } else { $null }
        bottleneck = $gate["bottleneck"]
        baseline_exists = $baselineExists
        rules_exists = $rulesExists
        scope = $inventory["scope"]
        pollution_signals = $pollutionSignals
        scope_candidates = $scopeCandidates
        scope_hint = if ($pollutionSignals.Count -gt 0 -and $scopeCandidates.Count -gt 0) { "Use a scoped path instead of the repo root for Agent sessions." } else { $null }
        gate = $gate
    }
}

function Invoke-HealthTool {
    param([string]$TargetPath)

    $scan = Invoke-ScanTool $TargetPath
    $status = "ok"
    if (-not $scan["baseline_exists"]) { $status = "baseline_missing" }
    elseif (-not $scan["gate"]["pass"]) { $status = "degraded" }

    return [ordered]@{
        tool = "health"
        path = $TargetPath
        status = $status
        quality_signal = $scan["quality_signal"]
        bottleneck = $scan["bottleneck"]
        baseline_exists = $scan["baseline_exists"]
        rules_exists = $scan["rules_exists"]
        scope = $scan["scope"]
        pollution_signals = $scan["pollution_signals"]
        scope_candidates = $scan["scope_candidates"]
        scope_hint = $scan["scope_hint"]
        no_degradation = $scan["gate"]["metrics"]["no_degradation"]
    }
}

function Invoke-SessionStartTool {
    param(
        [string]$TargetPath,
        [string]$GivenSessionId
    )

    $id = if ([string]::IsNullOrWhiteSpace($GivenSessionId)) { New-SessionId } else { $GivenSessionId }
    $gate = Invoke-Gate $TargetPath -Save
    $metrics = $gate["metrics"]
    $record = [ordered]@{
        tool = "session_start"
        session_id = $id
        path = $TargetPath
        status = if ($gate["pass"]) { "Baseline saved" } else { "failed" }
        quality_signal = $metrics["quality_signal"]
        bottleneck = $gate["bottleneck"]
        started_at = (Get-Date).ToString("o")
        gate = $gate
    }
    Write-JsonFile (Join-Path (Get-SessionDir $TargetPath) "$id.start.json") $record
    return $record
}

function Invoke-SessionEndTool {
    param(
        [string]$TargetPath,
        [string]$GivenSessionId
    )

    $id = if ([string]::IsNullOrWhiteSpace($GivenSessionId)) { Get-LatestSessionId $TargetPath } else { $GivenSessionId }
    if ([string]::IsNullOrWhiteSpace($id)) {
        throw "No prior session_start record found. Pass -SessionId or run session_start first."
    }

    $startPath = Join-Path (Get-SessionDir $TargetPath) "$id.start.json"
    $start = Read-JsonFileSafe $startPath
    $gate = Invoke-Gate $TargetPath
    $metrics = $gate["metrics"]
    $rulesPath = Join-Path $TargetPath ".sentrux\rules.toml"
    $rules = $null
    if (Test-Path -LiteralPath $rulesPath -PathType Leaf) {
        $rules = Invoke-CheckRulesTool $TargetPath
    }
    $before = Convert-QualitySignal (Get-JsonProperty $start "quality_signal")
    $after = Convert-QualitySignal $metrics["quality_signal"]
    $delta = if ($null -ne $before -and $null -ne $after) { $after - $before } else { $null }
    $rulesPass = ($null -eq $rules -or [bool]$rules["pass"])
    $pass = ($gate["pass"] -and ($null -eq $delta -or $delta -ge 0) -and $rulesPass)
    $summary = "No structural degradation during this session"
    if (-not $gate["pass"] -or ($null -ne $delta -and $delta -lt 0)) {
        $summary = "Quality degraded during this session"
    }
    elseif (-not $rulesPass) {
        $summary = "Architecture rules failed during this session"
    }

    $record = [ordered]@{
        tool = "session_end"
        session_id = $id
        path = $TargetPath
        pass = $pass
        signal_before = $before
        signal_after = $after
        delta = $delta
        summary = $summary
        ended_at = (Get-Date).ToString("o")
        gate = $gate
        rules = $rules
    }
    Write-JsonFile (Join-Path (Get-SessionDir $TargetPath) "$id.end.json") $record
    return $record
}

function Invoke-CheckRulesTool {
    param([string]$TargetPath)

    $rulesPath = Join-Path $TargetPath ".sentrux\rules.toml"
    $templatePath = Join-Path $PSScriptRoot "templates\sentrux-rules.example.toml"
    if (-not (Test-Path -LiteralPath $rulesPath -PathType Leaf)) {
        return [ordered]@{
            tool = "check_rules"
            path = $TargetPath
            pass = $false
            status = "rules_missing"
            rules_path = $rulesPath
            template_path = $templatePath
            summary = "No .sentrux/rules.toml found"
            next_action = "Copy the template into this scope, encode real layer/boundary rules, then rerun check_rules."
        }
    }

    $native = Invoke-Native "sentrux" @("check", $TargetPath)
    $metrics = Parse-SentruxOutput $native.output
    return [ordered]@{
        tool = "check_rules"
        path = $TargetPath
        pass = ($native.exitCode -eq 0)
        status = if ($native.exitCode -eq 0) { "passed" } else { "failed" }
        quality_signal = $metrics["quality_signal"]
        violations = $metrics["violations"]
        bottleneck = Get-Bottleneck $metrics
        raw_output = $native.output
    }
}

function Get-SourceExtensions {
    return @(".ps1", ".psm1", ".py", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".rs", ".go", ".java", ".cs")
}

function Get-ExcludedSourceReason {
    param([string]$RelativePath)

    $normalized = Normalize-RelativeFilePath $RelativePath
    $lower = $normalized.ToLowerInvariant()
    $parts = @($lower -split "/" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $excludedParts = @(
        ".git", ".repowise", ".understand-anything", ".sentrux",
        "node_modules", ".pnpm", ".yarn",
        "target", "dist", "build", "out", "coverage",
        ".venv", "venv", "env", ".tox", "__pycache__",
        ".next", ".nuxt", ".turbo", ".cache"
    )
    foreach ($part in $parts) {
        if ($excludedParts -contains $part) {
            return "excluded_dir:$part"
        }
    }

    if ($lower -match "^(static|public|wwwroot)/assets/") {
        return "bundled_static_assets"
    }

    $leaf = [System.IO.Path]::GetFileName($normalized)
    $leafLower = $leaf.ToLowerInvariant()
    if ($leafLower -match "(\.min|\.bundle)\.(js|jsx|mjs|cjs)$") {
        return "bundled_or_minified_file"
    }
    if ($leaf -match ".+-[A-Za-z0-9_]{6,}\.(js|jsx|mjs|cjs)$" -and $leaf -match "[0-9]" -and $leaf -cmatch "[A-Z]") {
        return "hashed_bundle_file"
    }
    if ($lower -match "/assets/" -and $leaf -match "^(chunk-|vendor-|index-|assets?).+-[A-Za-z0-9_-]{6,}\.(js|jsx|mjs|cjs)$") {
        return "hashed_static_asset"
    }

    return ""
}

function Get-SourceFileInventory {
    param([string]$TargetPath)

    Push-Location $TargetPath
    try {
        $files = rg --files 2>$null
    }
    finally {
        Pop-Location
    }

    $sourceExt = Get-SourceExtensions
    $included = New-Object System.Collections.Generic.List[string]
    $excluded = @{}

    foreach ($file in @($files)) {
        $normalized = Normalize-RelativeFilePath $file
        $ext = [System.IO.Path]::GetExtension($normalized).ToLowerInvariant()
        if ($sourceExt -notcontains $ext) { continue }

        $reason = Get-ExcludedSourceReason $normalized
        if ([string]::IsNullOrWhiteSpace($reason) -and $ext -in @(".js", ".jsx", ".mjs", ".cjs")) {
            $fullPath = Join-Path $TargetPath $normalized
            $item = Get-Item -LiteralPath $fullPath -ErrorAction SilentlyContinue
            if ($null -ne $item -and $item.Length -gt 2097152) {
                $reason = "oversized_generated_or_bundle"
            }
        }
        if (-not [string]::IsNullOrWhiteSpace($reason)) {
            if (-not $excluded.ContainsKey($reason)) {
                $excluded[$reason] = [ordered]@{
                    files = 0
                    samples = New-Object System.Collections.Generic.List[string]
                }
            }
            $excluded[$reason]["files"] = [int]$excluded[$reason]["files"] + 1
            if ($excluded[$reason]["samples"].Count -lt 8) {
                $excluded[$reason]["samples"].Add($normalized)
            }
            continue
        }

        $included.Add($normalized)
    }

    $excludedTotal = 0
    $excludedByReason = @($excluded.Keys | ForEach-Object {
        $excludedTotal += [int]$excluded[$_]["files"]
        [ordered]@{
            reason = $_
            files = [int]$excluded[$_]["files"]
            samples = @($excluded[$_]["samples"])
        }
    } | Sort-Object { $_["files"] } -Descending)

    return [ordered]@{
        files = @($included | Sort-Object)
        scope = [ordered]@{
            mode = "auto_governed_source"
            included_files = $included.Count
            excluded_files = $excludedTotal
            excluded_by_reason = $excludedByReason
            source_extensions = $sourceExt
            note = "Root paths are allowed. Dependency, build-output, cache, and bundled static-asset code is excluded from governed source metrics."
        }
    }
}

function Get-SourceFiles {
    param([string]$TargetPath)

    $inventory = Get-SourceFileInventory $TargetPath
    return @($inventory["files"])
}

function Get-ModuleName {
    param([string]$RelativePath)

    $normalized = $RelativePath -replace "\\", "/"
    $parts = @($normalized -split "/" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($parts.Count -eq 0) { return "root" }
    if ($parts[0] -eq "src" -and $parts.Count -gt 1) { return "src/$($parts[1])" }
    return $parts[0]
}

function Normalize-RelativeFilePath {
    param([string]$RelativePath)

    $normalized = ($RelativePath -replace "\\", "/").Trim()
    while ($normalized.StartsWith("./")) {
        $normalized = $normalized.Substring(2)
    }
    return $normalized
}

function Get-StableId {
    param([string]$Text)

    $sha1 = [System.Security.Cryptography.SHA1]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
        $hash = $sha1.ComputeHash($bytes)
        return (($hash | ForEach-Object { $_.ToString("x2") }) -join "").Substring(0, 16)
    }
    finally {
        $sha1.Dispose()
    }
}

function Resolve-GitFileKey {
    param(
        [string]$PathFromGit,
        [string]$Prefix,
        [hashtable]$KnownFiles
    )

    $candidate = Normalize-RelativeFilePath $PathFromGit
    if (-not [string]::IsNullOrWhiteSpace($Prefix)) {
        $prefix = Normalize-RelativeFilePath $Prefix
        if ($candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            $candidate = $candidate.Substring($prefix.Length).TrimStart("/")
        }
    }
    if ($KnownFiles.ContainsKey($candidate)) { return $candidate }

    foreach ($key in $KnownFiles.Keys) {
        if ($candidate.EndsWith("/$key", [System.StringComparison]::OrdinalIgnoreCase) -or
            $key.EndsWith("/$candidate", [System.StringComparison]::OrdinalIgnoreCase)) {
            return $key
        }
    }
    return ""
}

function Invoke-GitLsFilesForKnownFiles {
    param(
        [string]$TargetPath,
        [string[]]$ExtraArgs,
        [string[]]$Files,
        [int]$BatchSize = 80
    )

    $result = @()
    $known = @($Files | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    for ($i = 0; $i -lt $known.Count; $i += $BatchSize) {
        $take = [math]::Min($BatchSize, $known.Count - $i)
        $batch = @($known[$i..($i + $take - 1)])
        $args = @("-C", $TargetPath, "ls-files") + $ExtraArgs + @("--") + $batch
        $native = Invoke-Native "git" $args
        if ($native.exitCode -eq 0 -and -not [string]::IsNullOrWhiteSpace($native.output)) {
            $result += @($native.output -split "\r?\n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        }
    }
    return $result
}

function Invoke-GitLogForKnownFiles {
    param(
        [string]$TargetPath,
        [string[]]$Files,
        [int]$BatchSize = 80
    )

    $result = @()
    $known = @($Files | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    for ($i = 0; $i -lt $known.Count; $i += $BatchSize) {
        $take = [math]::Min($BatchSize, $known.Count - $i)
        $batch = @($known[$i..($i + $take - 1)])
        $args = @("-C", $TargetPath, "log", "--format=__SENTRUX_COMMIT__%ct", "--name-only", "--") + $batch
        $native = Invoke-Native "git" $args
        if ($native.exitCode -eq 0 -and -not [string]::IsNullOrWhiteSpace($native.output)) {
            $result += @($native.output -split "\r?\n")
        }
    }
    return $result
}

function Invoke-GitAuthorLogForKnownFiles {
    param(
        [string]$TargetPath,
        [string[]]$Files,
        [int]$BatchSize = 80
    )

    $result = @()
    $known = @($Files | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    for ($i = 0; $i -lt $known.Count; $i += $BatchSize) {
        $take = [math]::Min($BatchSize, $known.Count - $i)
        $batch = @($known[$i..($i + $take - 1)])
        $args = @("-C", $TargetPath, "log", "--format=__SENTRUX_COMMIT__%ct`t%an <%ae>", "--name-only", "--") + $batch
        $native = Invoke-Native "git" $args
        if ($native.exitCode -eq 0 -and -not [string]::IsNullOrWhiteSpace($native.output)) {
            $result += @($native.output -split "\r?\n")
        }
    }
    return $result
}

function Get-GitFileSignals {
    param(
        [string]$TargetPath,
        [string[]]$Files
    )

    $signals = @{}
    foreach ($file in $Files) {
        $key = Normalize-RelativeFilePath $file
        $signals[$key] = [ordered]@{
            age_days = $null
            churn = 0
            status = "clean"
            dirty = $false
            untracked = $false
            last_commit_unix = $null
        }
    }
    if ($signals.Count -eq 0) { return $signals }

    $prefix = ""
    $prefixNative = Invoke-Native "git" @("-C", $TargetPath, "rev-parse", "--show-prefix")
    if ($prefixNative.exitCode -eq 0) {
        $prefix = $prefixNative.output.Trim()
    }

    $tracked = @{}
    $knownFiles = @($signals.Keys)
    foreach ($line in (Invoke-GitLsFilesForKnownFiles $TargetPath @() $knownFiles)) {
        $key = Resolve-GitFileKey $line $prefix $signals
        if (-not [string]::IsNullOrWhiteSpace($key)) {
            $tracked[$key] = $true
        }
    }

    foreach ($line in (Invoke-GitLsFilesForKnownFiles $TargetPath @("--modified") $knownFiles)) {
        $key = Resolve-GitFileKey $line $prefix $signals
        if ([string]::IsNullOrWhiteSpace($key)) { continue }
        $signals[$key]["status"] = "dirty"
        $signals[$key]["dirty"] = $true
    }

    foreach ($line in (Invoke-GitLsFilesForKnownFiles $TargetPath @("--others", "--exclude-standard") $knownFiles)) {
        $key = Resolve-GitFileKey $line $prefix $signals
        if ([string]::IsNullOrWhiteSpace($key)) { continue }
        $signals[$key]["status"] = "untracked"
        $signals[$key]["untracked"] = $true
    }

    foreach ($key in $signals.Keys) {
        if (-not $tracked.ContainsKey($key) -and -not [bool]$signals[$key]["dirty"] -and -not [bool]$signals[$key]["untracked"]) {
            $signals[$key]["status"] = "untracked"
            $signals[$key]["untracked"] = $true
        }
    }

    $logLines = @(Invoke-GitLogForKnownFiles $TargetPath $knownFiles)
    if ($logLines.Count -gt 0) {
        $currentCommitUnix = $null
        foreach ($line in $logLines) {
            $trimmed = $line.Trim()
            if ([string]::IsNullOrWhiteSpace($trimmed)) { continue }
            $commitMatch = [regex]::Match($trimmed, "^__SENTRUX_COMMIT__([0-9]+)$")
            if ($commitMatch.Success) {
                $currentCommitUnix = [int64]$commitMatch.Groups[1].Value
                continue
            }
            if ($null -eq $currentCommitUnix) { continue }
            $key = Resolve-GitFileKey $trimmed $prefix $signals
            if ([string]::IsNullOrWhiteSpace($key)) { continue }
            $signals[$key]["churn"] = [int]$signals[$key]["churn"] + 1
            if ($null -eq $signals[$key]["last_commit_unix"] -or $currentCommitUnix -gt [int64]$signals[$key]["last_commit_unix"]) {
                $signals[$key]["last_commit_unix"] = $currentCommitUnix
            }
        }
    }

    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    foreach ($key in $signals.Keys) {
        if ($null -ne $signals[$key]["last_commit_unix"]) {
            $ageSeconds = [math]::Max(0, $now - [int64]$signals[$key]["last_commit_unix"])
            $signals[$key]["age_days"] = [int][math]::Floor($ageSeconds / 86400)
        }
    }

    return $signals
}

function Get-GitAuthorSignals {
    param(
        [string]$TargetPath,
        [string[]]$Files
    )

    $signals = @{}
    foreach ($file in $Files) {
        $key = Normalize-RelativeFilePath $file
        $signals[$key] = [ordered]@{
            authors = @{}
            touches = 0
            last_author = $null
            last_commit_unix = $null
        }
    }
    if ($signals.Count -eq 0) { return $signals }

    $prefix = ""
    $prefixNative = Invoke-Native "git" @("-C", $TargetPath, "rev-parse", "--show-prefix")
    if ($prefixNative.exitCode -eq 0) {
        $prefix = $prefixNative.output.Trim()
    }

    $knownFiles = @($signals.Keys)
    $currentCommitUnix = $null
    $currentAuthor = $null
    foreach ($line in (Invoke-GitAuthorLogForKnownFiles $TargetPath $knownFiles)) {
        $trimmed = $line.Trim()
        if ([string]::IsNullOrWhiteSpace($trimmed)) { continue }
        $commitMatch = [regex]::Match($trimmed, "^__SENTRUX_COMMIT__([0-9]+)\t(.+)$")
        if ($commitMatch.Success) {
            $currentCommitUnix = [int64]$commitMatch.Groups[1].Value
            $currentAuthor = $commitMatch.Groups[2].Value.Trim()
            continue
        }
        if ($null -eq $currentCommitUnix -or [string]::IsNullOrWhiteSpace($currentAuthor)) { continue }
        $key = Resolve-GitFileKey $trimmed $prefix $signals
        if ([string]::IsNullOrWhiteSpace($key)) { continue }

        if (-not $signals[$key]["authors"].ContainsKey($currentAuthor)) {
            $signals[$key]["authors"][$currentAuthor] = 0
        }
        $signals[$key]["authors"][$currentAuthor] = [int]$signals[$key]["authors"][$currentAuthor] + 1
        $signals[$key]["touches"] = [int]$signals[$key]["touches"] + 1
        if ($null -eq $signals[$key]["last_commit_unix"] -or $currentCommitUnix -gt [int64]$signals[$key]["last_commit_unix"]) {
            $signals[$key]["last_commit_unix"] = $currentCommitUnix
            $signals[$key]["last_author"] = $currentAuthor
        }
    }

    return $signals
}

function Convert-AuthorCounts {
    param([hashtable]$Authors)

    if ($null -eq $Authors -or $Authors.Count -eq 0) { return @() }
    return @($Authors.Keys | ForEach-Object {
        [ordered]@{
            author = $_
            touches = [int]$Authors[$_]
        }
    } | Sort-Object { $_["touches"] } -Descending)
}

function Get-BusFactorRisk {
    param(
        [int]$AuthorCount,
        [double]$TopAuthorShare
    )

    if ($AuthorCount -le 0) { return 100 }
    $share = [math]::Max(0, [math]::Min(1, $TopAuthorShare))
    $risk = ((1.0 / $AuthorCount) * 55.0) + ($share * 45.0)
    return [int][math]::Round([math]::Max(0, [math]::Min(100, $risk)))
}

function New-BusFactorEntry {
    param(
        [string]$Id,
        [string]$Name,
        [hashtable]$Authors,
        [int]$Files = 1,
        [object]$Extra = $null
    )

    $authorList = @(Convert-AuthorCounts $Authors)
    $touches = 0
    foreach ($author in $authorList) {
        $touches += [int]$author["touches"]
    }
    $topTouches = if ($authorList.Count -gt 0) { [int]$authorList[0]["touches"] } else { 0 }
    $topShare = if ($touches -gt 0) { [math]::Round($topTouches / $touches, 4) } else { 1.0 }
    $entry = [ordered]@{
        id = $Id
        name = $Name
        files = $Files
        bus_factor = $authorList.Count
        bus_factor_risk = Get-BusFactorRisk $authorList.Count $topShare
        touches = $touches
        top_author = if ($authorList.Count -gt 0) { $authorList[0]["author"] } else { $null }
        top_author_share = $topShare
        authors = $authorList
    }
    if ($null -ne $Extra) {
        foreach ($prop in $Extra.GetEnumerator()) {
            $entry[$prop.Key] = $prop.Value
        }
    }
    return $entry
}

function Add-DsmEdge {
    param(
        [hashtable]$Edges,
        [string]$From,
        [string]$To
    )

    if ([string]::IsNullOrWhiteSpace($From) -or [string]::IsNullOrWhiteSpace($To) -or $From -eq $To) {
        return
    }
    $key = "$From->$To"
    if (-not $Edges.ContainsKey($key)) {
        $Edges[$key] = [ordered]@{ from = $From; to = $To; count = 0 }
    }
    $Edges[$key]["count"]++
}

function ConvertTo-HeatColor {
    param([double]$Score)

    $bounded = [math]::Max(0, [math]::Min(100, $Score))
    if ($bounded -le 50) {
        $t = $bounded / 50.0
        $r = [int][math]::Round(34 + ((245 - 34) * $t))
        $g = [int][math]::Round(197 + ((158 - 197) * $t))
        $b = [int][math]::Round(94 + ((11 - 94) * $t))
    }
    else {
        $t = ($bounded - 50) / 50.0
        $r = [int][math]::Round(245 + ((239 - 245) * $t))
        $g = [int][math]::Round(158 + ((68 - 158) * $t))
        $b = [int][math]::Round(11 + ((68 - 11) * $t))
    }
    return "#{0:X2}{1:X2}{2:X2}" -f $r, $g, $b
}

function ConvertTo-HeatScore {
    param(
        [object]$Value,
        [double]$MaxValue
    )

    $number = ConvertTo-NullableDouble $Value
    if ($null -eq $number -or $MaxValue -le 0) { return 0 }
    return [int][math]::Round([math]::Max(0, [math]::Min(100, ($number / $MaxValue) * 100)))
}

function New-ColorEntry {
    param(
        [object]$Value,
        [double]$Score
    )

    $bounded = [int][math]::Round([math]::Max(0, [math]::Min(100, $Score)))
    return [ordered]@{
        value = $Value
        score = $bounded
        color = ConvertTo-HeatColor $bounded
    }
}

function Get-MaxMetric {
    param(
        [hashtable]$Modules,
        [string]$Key
    )

    $max = 0.0
    foreach ($name in $Modules.Keys) {
        $value = ConvertTo-NullableDouble $Modules[$name][$Key]
        if ($null -ne $value -and $value -gt $max) {
            $max = $value
        }
    }
    return $max
}

function Get-ReachableDependentsCount {
    param(
        [string]$ModuleName,
        [hashtable]$ReverseAdjacency
    )

    $seen = @{}
    $queue = New-Object System.Collections.Generic.Queue[string]
    if ($ReverseAdjacency.ContainsKey($ModuleName)) {
        foreach ($item in $ReverseAdjacency[$ModuleName]) {
            $queue.Enqueue($item)
        }
    }
    while ($queue.Count -gt 0) {
        $current = $queue.Dequeue()
        if ($seen.ContainsKey($current)) { continue }
        $seen[$current] = $true
        if ($ReverseAdjacency.ContainsKey($current)) {
            foreach ($next in $ReverseAdjacency[$current]) {
                if (-not $seen.ContainsKey($next)) {
                    $queue.Enqueue($next)
                }
            }
        }
    }
    return $seen.Count
}

function Get-LeadingWhitespaceCount {
    param([string]$Line)

    $match = [regex]::Match($Line, "^\s*")
    if ($match.Success) { return $match.Value.Length }
    return 0
}

function Get-LineNumberFromOffset {
    param(
        [string]$Text,
        [int]$Offset
    )

    if ($Offset -le 0) { return 1 }
    return ([regex]::Matches($Text.Substring(0, [math]::Min($Offset, $Text.Length)), "\r\n|\n|\r")).Count + 1
}

function Get-ParamCount {
    param([string]$ParamText)

    if ([string]::IsNullOrWhiteSpace($ParamText)) { return 0 }
    $clean = $ParamText.Trim()
    if ([string]::IsNullOrWhiteSpace($clean)) { return 0 }
    return @($clean -split "," | Where-Object {
        $item = $_.Trim()
        -not [string]::IsNullOrWhiteSpace($item) -and $item -notin @("self", "&self", "mut self", "&mut self", "cls")
    }).Count
}

function Measure-FunctionComplexity {
    param([string[]]$Lines)

    $complexity = 1
    foreach ($line in $Lines) {
        $trimmed = $line.Trim()
        if ([string]::IsNullOrWhiteSpace($trimmed)) { continue }
        if ($trimmed.StartsWith("#") -or $trimmed.StartsWith("//")) { continue }
        $complexity += [regex]::Matches($trimmed, "\b(if|elif|for|while|except|case|catch|match|guard|when|with)\b").Count
        $complexity += [regex]::Matches($trimmed, "&&|\|\|").Count
        if ($trimmed -match "\s=>\s") { $complexity++ }
    }
    return $complexity
}

function Measure-LineStats {
    param([string[]]$Lines)

    $blank = 0
    $comment = 0
    foreach ($line in $Lines) {
        $trimmed = $line.Trim()
        if ([string]::IsNullOrWhiteSpace($trimmed)) {
            $blank++
        }
        elseif ($trimmed.StartsWith("#") -or $trimmed.StartsWith("//") -or $trimmed.StartsWith("/*") -or $trimmed.StartsWith("*")) {
            $comment++
        }
    }
    return [ordered]@{
        lines = $Lines.Count
        blank_lines = $blank
        comment_lines = $comment
        loc = [math]::Max(0, $Lines.Count - $blank - $comment)
    }
}

function New-FunctionMetric {
    param(
        [string]$Name,
        [string]$Kind,
        [int]$StartLine,
        [int]$EndLine,
        [string[]]$BodyLines,
        [string]$Params = "",
        [bool]$IsAsync = $false,
        [bool]$IsPublic = $false
    )

    $stats = Measure-LineStats $BodyLines
    return [ordered]@{
        name = $Name
        kind = $Kind
        start_line = $StartLine
        end_line = $EndLine
        lines = $stats["lines"]
        loc = $stats["loc"]
        complexity = Measure-FunctionComplexity $BodyLines
        params = Get-ParamCount $Params
        async = $IsAsync
        public = $IsPublic
    }
}

function Get-CLikeFunctionEndLine {
    param(
        [string[]]$Lines,
        [int]$StartIndex
    )

    $braceDepth = 0
    $seenBrace = $false
    for ($i = $StartIndex; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        foreach ($char in $line.ToCharArray()) {
            if ($char -eq "{") {
                $braceDepth++
                $seenBrace = $true
            }
            elseif ($char -eq "}") {
                $braceDepth--
                if ($seenBrace -and $braceDepth -le 0) {
                    return $i
                }
            }
        }
    }
    return $StartIndex
}

function Get-FunctionsFromPython {
    param([string[]]$Lines)

    $functions = @()
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        $match = [regex]::Match($line, "^(\s*)(async\s+def|def)\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)")
        if (-not $match.Success) { continue }
        $indent = $match.Groups[1].Value.Length
        $end = $Lines.Count - 1
        for ($j = $i + 1; $j -lt $Lines.Count; $j++) {
            $candidate = $Lines[$j]
            $trimmed = $candidate.Trim()
            if ([string]::IsNullOrWhiteSpace($trimmed) -or $trimmed.StartsWith("#") -or $trimmed.StartsWith("@")) { continue }
            if ((Get-LeadingWhitespaceCount $candidate) -le $indent) {
                $end = [math]::Max($i, $j - 1)
                break
            }
        }
        $body = @($Lines[$i..$end])
        $functions += New-FunctionMetric `
            -Name $match.Groups[3].Value `
            -Kind "function" `
            -StartLine ($i + 1) `
            -EndLine ($end + 1) `
            -BodyLines $body `
            -Params $match.Groups[4].Value `
            -IsAsync ($match.Groups[2].Value -match "async")
    }
    return $functions
}

function Get-FunctionsFromRust {
    param([string[]]$Lines)

    $functions = @()
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        $match = [regex]::Match($line, "^\s*(pub(?:\([^)]*\))?\s+)?(async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)")
        if (-not $match.Success) { continue }
        $end = Get-CLikeFunctionEndLine $Lines $i
        $body = @($Lines[$i..$end])
        $functions += New-FunctionMetric `
            -Name $match.Groups[3].Value `
            -Kind "function" `
            -StartLine ($i + 1) `
            -EndLine ($end + 1) `
            -BodyLines $body `
            -Params $match.Groups[4].Value `
            -IsAsync (-not [string]::IsNullOrWhiteSpace($match.Groups[2].Value)) `
            -IsPublic (-not [string]::IsNullOrWhiteSpace($match.Groups[1].Value))
    }
    return $functions
}

function Get-FunctionsFromJavaScriptLike {
    param([string[]]$Lines)

    $functions = @()
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        $match = [regex]::Match($line, "^\s*(export\s+)?(async\s+)?function\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)")
        if (-not $match.Success) {
            $match = [regex]::Match($line, "^\s*(export\s+)?(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(async\s+)?\(?([^)=]*)\)?\s*=>")
            if ($match.Success) {
                $end = Get-CLikeFunctionEndLine $Lines $i
                $body = @($Lines[$i..$end])
                $functions += New-FunctionMetric `
                    -Name $match.Groups[2].Value `
                    -Kind "function" `
                    -StartLine ($i + 1) `
                    -EndLine ($end + 1) `
                    -BodyLines $body `
                    -Params $match.Groups[4].Value `
                    -IsAsync (-not [string]::IsNullOrWhiteSpace($match.Groups[3].Value)) `
                    -IsPublic (-not [string]::IsNullOrWhiteSpace($match.Groups[1].Value))
            }
            continue
        }
        $end = Get-CLikeFunctionEndLine $Lines $i
        $body = @($Lines[$i..$end])
        $functions += New-FunctionMetric `
            -Name $match.Groups[3].Value `
            -Kind "function" `
            -StartLine ($i + 1) `
            -EndLine ($end + 1) `
            -BodyLines $body `
            -Params $match.Groups[4].Value `
            -IsAsync (-not [string]::IsNullOrWhiteSpace($match.Groups[2].Value)) `
            -IsPublic (-not [string]::IsNullOrWhiteSpace($match.Groups[1].Value))
    }
    return $functions
}

function Get-FunctionsFromPowerShell {
    param([string[]]$Lines)

    $functions = @()
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        $match = [regex]::Match($line, "^\s*function\s+([A-Za-z_][A-Za-z0-9_:-]*)")
        if (-not $match.Success) { continue }
        $end = Get-CLikeFunctionEndLine $Lines $i
        $body = @($Lines[$i..$end])
        $paramText = ""
        $paramMatch = $body | Select-String -Pattern "^\s*param\s*\((.*)" -CaseSensitive:$false | Select-Object -First 1
        if ($null -ne $paramMatch -and $paramMatch.Matches.Count -gt 0) {
            $paramText = $paramMatch.Matches[0].Groups[1].Value
        }
        $functions += New-FunctionMetric `
            -Name $match.Groups[1].Value `
            -Kind "function" `
            -StartLine ($i + 1) `
            -EndLine ($end + 1) `
            -BodyLines $body `
            -Params $paramText `
            -IsAsync $false `
            -IsPublic $true
    }
    return $functions
}

function Get-FileDetail {
    param(
        [string]$TargetPath,
        [string]$RelativePath,
        [hashtable]$GitSignals
    )

    $normalized = Normalize-RelativeFilePath $RelativePath
    $fullPath = Join-Path $TargetPath $normalized
    $content = Get-Content -LiteralPath $fullPath -Raw -ErrorAction SilentlyContinue
    if ($null -eq $content) { $content = "" }
    $lines = @($content -split "\r\n|\n|\r")
    if ($lines.Count -eq 1 -and [string]::IsNullOrEmpty($lines[0])) { $lines = @() }
    $ext = [System.IO.Path]::GetExtension($normalized).ToLowerInvariant()
    $language = switch ($ext) {
        ".py" { "python" }
        ".rs" { "rust" }
        ".ts" { "typescript" }
        ".tsx" { "typescript" }
        ".js" { "javascript" }
        ".jsx" { "javascript" }
        ".mjs" { "javascript" }
        ".cjs" { "javascript" }
        ".ps1" { "powershell" }
        ".psm1" { "powershell" }
        ".go" { "go" }
        ".java" { "java" }
        ".cs" { "csharp" }
        default { "unknown" }
    }

    $functions = @(switch ($language) {
        "python" { Get-FunctionsFromPython $lines }
        "rust" { Get-FunctionsFromRust $lines }
        "typescript" { Get-FunctionsFromJavaScriptLike $lines }
        "javascript" { Get-FunctionsFromJavaScriptLike $lines }
        "powershell" { Get-FunctionsFromPowerShell $lines }
        default { @() }
    })

    $lineStats = Measure-LineStats $lines
    $maxComplexity = 0
    $totalComplexity = 0
    foreach ($fn in $functions) {
        $totalComplexity += [int]$fn["complexity"]
        if ([int]$fn["complexity"] -gt $maxComplexity) {
            $maxComplexity = [int]$fn["complexity"]
        }
        $functionKey = "function:{0}:{1}:{2}:{3}" -f $normalized, $fn["name"], $fn["start_line"], $fn["end_line"]
        $fn["id"] = Get-StableId $functionKey
        $fn["source_anchor"] = [ordered]@{
            path = $normalized
            line = $fn["start_line"]
            label = "$normalized`:$($fn["start_line"])"
        }
    }
    $avgComplexity = if ($functions.Count -gt 0) { [math]::Round($totalComplexity / $functions.Count, 2) } else { 0 }
    $git = if ($GitSignals.ContainsKey($normalized)) { $GitSignals[$normalized] } else { $null }

    return [ordered]@{
        id = Get-StableId "file:$normalized"
        path = $normalized
        module = Get-ModuleName $normalized
        language = $language
        source_anchor = [ordered]@{
            path = $normalized
            line = 1
            label = "$normalized`:1"
        }
        lines = $lineStats["lines"]
        loc = $lineStats["loc"]
        blank_lines = $lineStats["blank_lines"]
        comment_lines = $lineStats["comment_lines"]
        function_count = $functions.Count
        max_complexity = $maxComplexity
        avg_complexity = $avgComplexity
        total_complexity = $totalComplexity
        git = if ($null -ne $git) {
            [ordered]@{
                status = $git["status"]
                dirty = $git["dirty"]
                untracked = $git["untracked"]
                age_days = $git["age_days"]
                churn = $git["churn"]
            }
        } else { $null }
        functions = @($functions | Sort-Object { $_["complexity"] } -Descending)
    }
}

function Invoke-DsmTool {
    param([string]$TargetPath)

    $inventory = Get-SourceFileInventory $TargetPath
    $files = @($inventory["files"])
    $gitSignals = Get-GitFileSignals $TargetPath $files
    $fileDetails = @($files | ForEach-Object { Get-FileDetail $TargetPath $_ $gitSignals })
    $modules = [ordered]@{}
    $moduleFiles = @{}
    foreach ($file in $files) {
        $module = Get-ModuleName $file
        if (-not $modules.Contains($module)) {
            $modules[$module] = [ordered]@{
                files = 0
                source_files = 0
                test_files = 0
                test_gap = 0
                avg_age_days = $null
                max_age_days = $null
                churn = 0
                dirty_files = 0
                untracked_files = 0
                git_files = 0
                inbound_edges = 0
                outbound_edges = 0
                coupling = 0
                exec_depth = 0
                blast_radius = 0
                risk = 0
            }
            $moduleFiles[$module] = New-Object System.Collections.Generic.List[string]
        }
        $modules[$module]["files"]++
        $moduleFiles[$module].Add((Normalize-RelativeFilePath $file))
        if (Test-IsTestFile $file) {
            $modules[$module]["test_files"]++
        }
        else {
            $modules[$module]["source_files"]++
        }

        $signalKey = Normalize-RelativeFilePath $file
        if ($gitSignals.ContainsKey($signalKey)) {
            $signal = $gitSignals[$signalKey]
            $modules[$module]["churn"] = [int]$modules[$module]["churn"] + [int]$signal["churn"]
            if ([bool]$signal["dirty"]) { $modules[$module]["dirty_files"]++ }
            if ([bool]$signal["untracked"]) { $modules[$module]["untracked_files"]++ }
            if ([bool]$signal["dirty"] -or [bool]$signal["untracked"]) { $modules[$module]["git_files"]++ }
        }
    }

    $edges = @{}
    foreach ($file in $files) {
        $ext = [System.IO.Path]::GetExtension($file).ToLowerInvariant()
        if (@(".py", ".rs") -notcontains $ext) { continue }
        $from = Get-ModuleName $file
        $content = Get-Content -LiteralPath (Join-Path $TargetPath $file) -Raw -ErrorAction SilentlyContinue
        if ([string]::IsNullOrWhiteSpace($content)) { continue }
        if ($ext -eq ".py") {
            foreach ($match in [regex]::Matches($content, "(?m)^\s*(?:from|import)\s+([A-Za-z_][A-Za-z0-9_\.]*)")) {
                $root = ($match.Groups[1].Value -split "\.")[0]
                if (-not $modules.Contains($root)) { continue }
                Add-DsmEdge $edges $from $root
            }
        }
        elseif ($ext -eq ".rs") {
            foreach ($match in [regex]::Matches($content, "(?m)^\s*mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")) {
                $target = "src/$($match.Groups[1].Value).rs"
                if ($modules.Contains($target)) {
                    Add-DsmEdge $edges $from $target
                }
            }
            foreach ($match in [regex]::Matches($content, "use\s+crate::([A-Za-z_][A-Za-z0-9_]*)")) {
                $target = "src/$($match.Groups[1].Value).rs"
                if ($modules.Contains($target)) {
                    Add-DsmEdge $edges $from $target
                }
            }
        }
    }

    $adjacency = @{}
    $reverseAdjacency = @{}
    foreach ($edge in $edges.Values) {
        $from = [string]$edge["from"]
        $to = [string]$edge["to"]
        if (-not $adjacency.ContainsKey($from)) { $adjacency[$from] = New-Object System.Collections.Generic.List[string] }
        if (-not $reverseAdjacency.ContainsKey($to)) { $reverseAdjacency[$to] = New-Object System.Collections.Generic.List[string] }
        if (-not $adjacency[$from].Contains($to)) { $adjacency[$from].Add($to) }
        if (-not $reverseAdjacency[$to].Contains($from)) { $reverseAdjacency[$to].Add($from) }
        if ($modules.Contains($from)) { $modules[$from]["outbound_edges"] = [int]$modules[$from]["outbound_edges"] + [int]$edge["count"] }
        if ($modules.Contains($to)) { $modules[$to]["inbound_edges"] = [int]$modules[$to]["inbound_edges"] + [int]$edge["count"] }
    }

    $depths = @{}
    foreach ($name in $modules.Keys) { $depths[$name] = 0 }
    for ($i = 0; $i -lt [math]::Max(1, $modules.Keys.Count); $i++) {
        $changed = $false
        foreach ($edge in $edges.Values) {
            $from = [string]$edge["from"]
            $to = [string]$edge["to"]
            if (-not $depths.ContainsKey($from) -or -not $depths.ContainsKey($to)) { continue }
            $candidate = [int]$depths[$to] + 1
            if ($candidate -gt [int]$depths[$from]) {
                $depths[$from] = [math]::Min(99, $candidate)
                $changed = $true
            }
        }
        if (-not $changed) { break }
    }

    foreach ($name in $modules.Keys) {
        $ageSum = 0
        $ageCount = 0
        $maxAge = $null
        foreach ($file in $moduleFiles[$name]) {
            if (-not $gitSignals.ContainsKey($file)) { continue }
            $age = $gitSignals[$file]["age_days"]
            if ($null -eq $age) { continue }
            $ageSum += [int]$age
            $ageCount++
            if ($null -eq $maxAge -or [int]$age -gt [int]$maxAge) {
                $maxAge = [int]$age
            }
        }
        if ($ageCount -gt 0) {
            $modules[$name]["avg_age_days"] = [int][math]::Round($ageSum / $ageCount)
            $modules[$name]["max_age_days"] = $maxAge
        }
        $modules[$name]["test_gap"] = [math]::Max(0, [int]$modules[$name]["source_files"] - [int]$modules[$name]["test_files"])
        $modules[$name]["coupling"] = [int]$modules[$name]["inbound_edges"] + [int]$modules[$name]["outbound_edges"]
        $modules[$name]["exec_depth"] = [int]$depths[$name]
        $reachable = Get-ReachableDependentsCount $name $reverseAdjacency
        $modules[$name]["blast_radius"] = $reachable + [int]$modules[$name]["inbound_edges"] + [int]$modules[$name]["outbound_edges"]
    }

    $maxFiles = Get-MaxMetric $modules "files"
    $maxCoupling = Get-MaxMetric $modules "coupling"
    $maxTestGap = Get-MaxMetric $modules "test_gap"
    $maxAge = Get-MaxMetric $modules "avg_age_days"
    $maxChurn = Get-MaxMetric $modules "churn"
    $maxGit = Get-MaxMetric $modules "git_files"
    $maxExecDepth = Get-MaxMetric $modules "exec_depth"
    $maxBlastRadius = Get-MaxMetric $modules "blast_radius"

    foreach ($name in $modules.Keys) {
        $riskScore =
            (ConvertTo-HeatScore $modules[$name]["coupling"] $maxCoupling) * 0.18 +
            (ConvertTo-HeatScore $modules[$name]["blast_radius"] $maxBlastRadius) * 0.18 +
            (ConvertTo-HeatScore $modules[$name]["exec_depth"] $maxExecDepth) * 0.14 +
            (ConvertTo-HeatScore $modules[$name]["churn"] $maxChurn) * 0.14 +
            (ConvertTo-HeatScore $modules[$name]["git_files"] $maxGit) * 0.12 +
            (ConvertTo-HeatScore $modules[$name]["test_gap"] $maxTestGap) * 0.12 +
            (ConvertTo-HeatScore $modules[$name]["avg_age_days"] $maxAge) * 0.07 +
            (ConvertTo-HeatScore $modules[$name]["files"] $maxFiles) * 0.05
        $modules[$name]["risk"] = [int][math]::Round($riskScore)
    }

    $colorModes = @(
        [ordered]@{ name = "Size"; key = "size"; metric = "files"; meaning = "module file count" },
        [ordered]@{ name = "Coupling"; key = "coupling"; metric = "coupling"; meaning = "incoming plus outgoing dependency edges" },
        [ordered]@{ name = "TestGap"; key = "test_gap"; metric = "test_gap"; meaning = "source files without matching test density" },
        [ordered]@{ name = "Age"; key = "age"; metric = "avg_age_days"; meaning = "average days since last git commit touching files in the module" },
        [ordered]@{ name = "Churn"; key = "churn"; metric = "churn"; meaning = "git commit touches for files in the module" },
        [ordered]@{ name = "Risk"; key = "risk"; metric = "risk"; meaning = "composite score from coupling, blast radius, execution depth, churn, git dirtiness, test gap, age, and size" },
        [ordered]@{ name = "Git"; key = "git"; metric = "git_files"; meaning = "dirty or untracked files in the module" },
        [ordered]@{ name = "ExecDepth"; key = "exec_depth"; metric = "exec_depth"; meaning = "approximate dependency-chain depth from this module" },
        [ordered]@{ name = "BlastRadius"; key = "blast_radius"; metric = "blast_radius"; meaning = "reachable dependents plus incident dependency edges" }
    )

    $moduleOutput = @($modules.Keys | ForEach-Object {
        $m = $modules[$_]
        $colors = [ordered]@{
            Size = New-ColorEntry $m["files"] (ConvertTo-HeatScore $m["files"] $maxFiles)
            Coupling = New-ColorEntry $m["coupling"] (ConvertTo-HeatScore $m["coupling"] $maxCoupling)
            TestGap = New-ColorEntry $m["test_gap"] (ConvertTo-HeatScore $m["test_gap"] $maxTestGap)
            Age = New-ColorEntry $m["avg_age_days"] (ConvertTo-HeatScore $m["avg_age_days"] $maxAge)
            Churn = New-ColorEntry $m["churn"] (ConvertTo-HeatScore $m["churn"] $maxChurn)
            Risk = New-ColorEntry $m["risk"] $m["risk"]
            Git = New-ColorEntry $m["git_files"] (ConvertTo-HeatScore $m["git_files"] $maxGit)
            ExecDepth = New-ColorEntry $m["exec_depth"] (ConvertTo-HeatScore $m["exec_depth"] $maxExecDepth)
            BlastRadius = New-ColorEntry $m["blast_radius"] (ConvertTo-HeatScore $m["blast_radius"] $maxBlastRadius)
        }
        [ordered]@{
            id = Get-StableId "module:$_"
            name = $_
            files = $m["files"]
            metrics = $m
            colors = $colors
        }
    } | Sort-Object { $_["colors"]["Risk"]["score"] } -Descending)

    $edgeOutput = @($edges.Values | ForEach-Object {
        [ordered]@{
            id = Get-StableId ("edge:{0}->{1}" -f $_["from"], $_["to"])
            from = $_["from"]
            to = $_["to"]
            count = $_["count"]
        }
    } | Sort-Object { $_["count"] } -Descending)

    return [ordered]@{
        tool = "dsm"
        path = $TargetPath
        scope = $inventory["scope"]
        default_color_mode = "Risk"
        color_modes = $colorModes
        modules = $moduleOutput
        file_details = @($fileDetails | Sort-Object { $_["max_complexity"] } -Descending)
        edges = $edgeOutput
        note = "Lightweight DSM with 9 color modes. Git-derived modes depend on local git history; use Sentrux/CodeNexus for authoritative graph detail."
    }
}

function Test-IsTestFile {
    param([string]$RelativePath)

    $normalized = ($RelativePath -replace "\\", "/").ToLowerInvariant()
    $name = [System.IO.Path]::GetFileName($normalized)
    return (
        $normalized -match "(^|/)(test|tests|__tests__)/" -or
        $name -match "^test_" -or
        $name -match "(\.test|\.spec)\."
    )
}

function Invoke-TestGapsTool {
    param([string]$TargetPath)

    $inventory = Get-SourceFileInventory $TargetPath
    $files = @($inventory["files"])
    $byModule = [ordered]@{}
    foreach ($file in $files) {
        $module = Get-ModuleName $file
        if (-not $byModule.Contains($module)) {
            $byModule[$module] = [ordered]@{ source = 0; tests = 0 }
        }
        if (Test-IsTestFile $file) {
            $byModule[$module]["tests"]++
        }
        else {
            $byModule[$module]["source"]++
        }
    }

    $gaps = @($byModule.Keys | ForEach-Object {
        $source = $byModule[$_]["source"]
        $tests = $byModule[$_]["tests"]
        [ordered]@{
            module = $_
            source_files = $source
            test_files = $tests
            gap = [math]::Max(0, $source - $tests)
        }
    })
    $gaps = @($gaps | Sort-Object { $_["gap"] } -Descending)

    $sourceTotal = 0
    $testTotal = 0
    foreach ($gap in $gaps) {
        $sourceTotal += [int]$gap["source_files"]
        $testTotal += [int]$gap["test_files"]
    }

    return [ordered]@{
        tool = "test_gaps"
        path = $TargetPath
        scope = $inventory["scope"]
        modules = $gaps
        summary = [ordered]@{
            source_files = $sourceTotal
            test_files = $testTotal
            largest_gap = if ($gaps.Count -gt 0) { $gaps[0] } else { $null }
        }
    }
}

function Invoke-GitStatsTool {
    param([string]$TargetPath)

    $inventory = Get-SourceFileInventory $TargetPath
    $files = @($inventory["files"])
    $gitSignals = Get-GitFileSignals $TargetPath $files
    $authorSignals = Get-GitAuthorSignals $TargetPath $files

    $dirtyFiles = 0
    $untrackedFiles = 0
    $trackedFiles = 0
    $totalChurn = 0
    $ageValues = @()
    $authorTotals = @{}
    $moduleStats = @{}
    $fileStats = @()

    foreach ($file in $files) {
        $key = Normalize-RelativeFilePath $file
        $git = if ($gitSignals.ContainsKey($key)) { $gitSignals[$key] } else { $null }
        $authors = if ($authorSignals.ContainsKey($key)) { $authorSignals[$key] } else { $null }
        $module = Get-ModuleName $key

        if (-not $moduleStats.ContainsKey($module)) {
            $moduleStats[$module] = [ordered]@{
                module = $module
                files = 0
                churn = 0
                dirty_files = 0
                untracked_files = 0
                authors = @{}
            }
        }

        $churn = if ($null -ne $git) { [int]$git["churn"] } else { 0 }
        $dirty = $null -ne $git -and [bool]$git["dirty"]
        $untracked = $null -ne $git -and [bool]$git["untracked"]
        $ageDays = if ($null -ne $git) { $git["age_days"] } else { $null }

        if ($dirty) { $dirtyFiles++ }
        if ($untracked) { $untrackedFiles++ } else { $trackedFiles++ }
        if ($null -ne $ageDays) { $ageValues += [int]$ageDays }
        $totalChurn += $churn

        $moduleStats[$module]["files"] = [int]$moduleStats[$module]["files"] + 1
        $moduleStats[$module]["churn"] = [int]$moduleStats[$module]["churn"] + $churn
        if ($dirty) { $moduleStats[$module]["dirty_files"] = [int]$moduleStats[$module]["dirty_files"] + 1 }
        if ($untracked) { $moduleStats[$module]["untracked_files"] = [int]$moduleStats[$module]["untracked_files"] + 1 }

        $fileAuthors = @()
        if ($null -ne $authors) {
            $fileAuthors = @(Convert-AuthorCounts $authors["authors"])
            foreach ($author in $fileAuthors) {
                $name = [string]$author["author"]
                $touches = [int]$author["touches"]
                if (-not $authorTotals.ContainsKey($name)) { $authorTotals[$name] = 0 }
                if (-not $moduleStats[$module]["authors"].ContainsKey($name)) { $moduleStats[$module]["authors"][$name] = 0 }
                $authorTotals[$name] = [int]$authorTotals[$name] + $touches
                $moduleStats[$module]["authors"][$name] = [int]$moduleStats[$module]["authors"][$name] + $touches
            }
        }

        $busEntry = New-BusFactorEntry (Get-StableId $key) $key $(if ($null -ne $authors) { $authors["authors"] } else { @{} }) 1
        $fileStats += [ordered]@{
            path = $key
            module = $module
            status = if ($null -ne $git) { [string]$git["status"] } else { "unknown" }
            dirty = $dirty
            untracked = $untracked
            age_days = $ageDays
            churn = $churn
            last_commit_unix = if ($null -ne $git) { $git["last_commit_unix"] } else { $null }
            last_author = if ($null -ne $authors) { $authors["last_author"] } else { $null }
            author_count = $busEntry["bus_factor"]
            bus_factor_risk = $busEntry["bus_factor_risk"]
            authors = $fileAuthors
        }
    }

    $moduleEntries = @($moduleStats.Keys | ForEach-Object {
        $entry = $moduleStats[$_]
        $bus = New-BusFactorEntry (Get-StableId $_) $_ $entry["authors"] ([int]$entry["files"]) ([ordered]@{
            churn = [int]$entry["churn"]
            dirty_files = [int]$entry["dirty_files"]
            untracked_files = [int]$entry["untracked_files"]
        })
        [pscustomobject]$bus
    } | Sort-Object { $_.bus_factor_risk }, { $_.churn } -Descending)

    $authorEntries = @(Convert-AuthorCounts $authorTotals)
    $avgAge = if ($ageValues.Count -gt 0) { [math]::Round((($ageValues | Measure-Object -Average).Average), 2) } else { $null }
    $oldestAge = if ($ageValues.Count -gt 0) { [int](($ageValues | Measure-Object -Maximum).Maximum) } else { $null }
    $newestAge = if ($ageValues.Count -gt 0) { [int](($ageValues | Measure-Object -Minimum).Minimum) } else { $null }

    return [ordered]@{
        tool = "git_stats"
        path = $TargetPath
        scope = $inventory["scope"]
        summary = [ordered]@{
            files = $files.Count
            tracked_files = $trackedFiles
            dirty_files = $dirtyFiles
            untracked_files = $untrackedFiles
            total_churn = $totalChurn
            avg_age_days = $avgAge
            newest_age_days = $newestAge
            oldest_age_days = $oldestAge
            authors = $authorEntries.Count
            modules = $moduleEntries.Count
        }
        hotspots = [ordered]@{
            churn_files = @($fileStats | Sort-Object { $_["churn"] } -Descending | Select-Object -First 25)
            stale_files = @($fileStats | Where-Object { $null -ne $_["age_days"] } | Sort-Object { $_["age_days"] } -Descending | Select-Object -First 25)
            dirty_files = @($fileStats | Where-Object { $_["dirty"] -or $_["untracked"] } | Select-Object -First 25)
            bus_factor_files = @($fileStats | Sort-Object { $_["bus_factor_risk"] } -Descending | Select-Object -First 25)
        }
        modules = $moduleEntries
        authors = $authorEntries
    }
}

function New-EvolutionHotspots {
    param([object]$Dsm)

    $functionHotspots = @()
    foreach ($file in @($Dsm["file_details"])) {
        foreach ($fn in @($file["functions"])) {
            $functionHotspots += [ordered]@{
                id = $fn["id"]
                fileId = $file["id"]
                file = $file["path"]
                name = $fn["name"]
                complexity = $fn["complexity"]
                loc = $fn["loc"]
                params = $fn["params"]
                sourceAnchor = $fn["source_anchor"]
            }
        }
    }

    return [ordered]@{
        modules = @($Dsm["modules"] |
            Sort-Object { $_["colors"]["Risk"]["score"] } -Descending |
            Select-Object -First 20 |
            ForEach-Object {
                [ordered]@{
                    id = $_["id"]
                    name = $_["name"]
                    risk = $_["metrics"]["risk"]
                    riskScore = $_["colors"]["Risk"]["score"]
                    files = $_["files"]
                    coupling = $_["metrics"]["coupling"]
                    blastRadius = $_["metrics"]["blast_radius"]
                    gitFiles = $_["metrics"]["git_files"]
                }
            })
        files = @($Dsm["file_details"] |
            Sort-Object { $_["max_complexity"] } -Descending |
            Select-Object -First 30 |
            ForEach-Object {
                [ordered]@{
                    id = $_["id"]
                    path = $_["path"]
                    sourceAnchor = $_["source_anchor"]
                    functionCount = $_["function_count"]
                    maxComplexity = $_["max_complexity"]
                    avgComplexity = $_["avg_complexity"]
                    loc = $_["loc"]
                    git = $_["git"]
                }
            })
        functions = @($functionHotspots |
            Sort-Object { $_["complexity"] } -Descending |
            Select-Object -First 50)
    }
}

function New-EvolutionCouplingDetails {
    param([object]$Dsm)

    $modules = @($Dsm["modules"] |
        Sort-Object { $_["metrics"]["coupling"] } -Descending |
        Select-Object -First 30 |
        ForEach-Object {
            [ordered]@{
                id = $_["id"]
                name = $_["name"]
                coupling = $_["metrics"]["coupling"]
                inbound = $_["metrics"]["inbound_edges"]
                outbound = $_["metrics"]["outbound_edges"]
                blastRadius = $_["metrics"]["blast_radius"]
                execDepth = $_["metrics"]["exec_depth"]
                risk = $_["metrics"]["risk"]
            }
        })
    $edges = @($Dsm["edges"] |
        Sort-Object { $_["count"] } -Descending |
        Select-Object -First 50 |
        ForEach-Object {
            [ordered]@{
                id = $_["id"]
                from = $_["from"]
                to = $_["to"]
                count = $_["count"]
            }
        })
    $maxCoupling = if ($modules.Count -gt 0) { $modules[0]["coupling"] } else { 0 }
    return [ordered]@{
        summary = [ordered]@{
            modules = @($Dsm["modules"]).Count
            edges = @($Dsm["edges"]).Count
            maxCoupling = $maxCoupling
            topModule = if ($modules.Count -gt 0) { $modules[0]["name"] } else { $null }
        }
        modules = $modules
        edges = $edges
    }
}

function New-EvolutionBusFactorDetails {
    param(
        [string]$TargetPath,
        [object]$Dsm
    )

    $fileDetails = @($Dsm["file_details"])
    $files = @($fileDetails | ForEach-Object { $_["path"] })
    $authorSignals = Get-GitAuthorSignals $TargetPath $files

    $fileEntries = @()
    $moduleAuthorState = @{}
    foreach ($file in $fileDetails) {
        $path = [string]$file["path"]
        $module = [string]$file["module"]
        if (-not $moduleAuthorState.ContainsKey($module)) {
            $moduleAuthorState[$module] = [ordered]@{
                authors = @{}
                files = 0
                paths = New-Object System.Collections.Generic.List[string]
            }
        }
        $moduleAuthorState[$module]["files"] = [int]$moduleAuthorState[$module]["files"] + 1
        $moduleAuthorState[$module]["paths"].Add($path)

        $authors = @{}
        if ($authorSignals.ContainsKey($path)) {
            foreach ($author in $authorSignals[$path]["authors"].Keys) {
                $authors[$author] = [int]$authorSignals[$path]["authors"][$author]
                if (-not $moduleAuthorState[$module]["authors"].ContainsKey($author)) {
                    $moduleAuthorState[$module]["authors"][$author] = 0
                }
                $moduleAuthorState[$module]["authors"][$author] = [int]$moduleAuthorState[$module]["authors"][$author] + [int]$authorSignals[$path]["authors"][$author]
            }
        }

        $fileEntries += New-BusFactorEntry `
            -Id $file["id"] `
            -Name $path `
            -Authors $authors `
            -Files 1 `
            -Extra ([ordered]@{
                path = $path
                module = $module
                sourceAnchor = $file["source_anchor"]
                functionCount = $file["function_count"]
                maxComplexity = $file["max_complexity"]
                git = $file["git"]
            })
    }

    $moduleEntries = @()
    foreach ($module in $moduleAuthorState.Keys) {
        $moduleEntries += New-BusFactorEntry `
            -Id (Get-StableId "module:$module") `
            -Name $module `
            -Authors $moduleAuthorState[$module]["authors"] `
            -Files ([int]$moduleAuthorState[$module]["files"]) `
            -Extra ([ordered]@{
                paths = @($moduleAuthorState[$module]["paths"])
            })
    }

    $moduleEntries = @($moduleEntries | Sort-Object { $_["bus_factor_risk"] } -Descending)
    $fileEntries = @($fileEntries | Sort-Object { $_["bus_factor_risk"] } -Descending)
    return [ordered]@{
        summary = [ordered]@{
            modules = $moduleEntries.Count
            files = $fileEntries.Count
            highestModuleRisk = if ($moduleEntries.Count -gt 0) { $moduleEntries[0]["bus_factor_risk"] } else { 0 }
            highestFileRisk = if ($fileEntries.Count -gt 0) { $fileEntries[0]["bus_factor_risk"] } else { 0 }
        }
        modules = @($moduleEntries | Select-Object -First 30)
        files = @($fileEntries | Select-Object -First 50)
    }
}

function New-SessionTrend {
    param([object[]]$Sessions)

    $completed = @($Sessions | Where-Object { $null -ne $_["end_signal"] })
    $failed = @($Sessions | Where-Object { $_["pass"] -eq $false })
    $deltas = @($completed | ForEach-Object {
        if ($null -ne $_["start_signal"] -and $null -ne $_["end_signal"]) {
            [int]$_["end_signal"] - [int]$_["start_signal"]
        }
    })
    $totalDelta = 0
    foreach ($delta in $deltas) { $totalDelta += [int]$delta }
    return [ordered]@{
        sessions = $Sessions.Count
        completed = $completed.Count
        failed = $failed.Count
        totalSignalDelta = $totalDelta
        lastSignalDelta = if ($deltas.Count -gt 0) { [int]$deltas[0] } else { $null }
        direction = if ($totalDelta -gt 0) { "improving" } elseif ($totalDelta -lt 0) { "degrading" } else { "stable" }
    }
}

function Read-SentruxRuleHints {
    param([string]$TargetPath)

    $rulesPath = Join-Path $TargetPath ".sentrux\rules.toml"
    $constraints = [ordered]@{
        max_cycles = $null
        max_coupling = $null
        max_cc = $null
        no_god_files = $null
    }
    if (-not (Test-Path -LiteralPath $rulesPath -PathType Leaf)) {
        return [ordered]@{
            exists = $false
            path = $rulesPath
            constraints = $constraints
        }
    }

    $text = Get-Content -LiteralPath $rulesPath -Raw
    foreach ($key in @("max_cycles", "max_cc")) {
        $match = [regex]::Match($text, "(?m)^\s*$key\s*=\s*([0-9]+)\s*$")
        if ($match.Success) {
            $constraints[$key] = [int]$match.Groups[1].Value
        }
    }
    $couplingMatch = [regex]::Match($text, '(?m)^\s*max_coupling\s*=\s*"?([A-Za-z0-9_.-]+)"?\s*$')
    if ($couplingMatch.Success) {
        $constraints["max_coupling"] = $couplingMatch.Groups[1].Value
    }
    $godMatch = [regex]::Match($text, '(?m)^\s*no_god_files\s*=\s*(true|false)\s*$', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)
    if ($godMatch.Success) {
        $constraints["no_god_files"] = [bool]::Parse($godMatch.Groups[1].Value)
    }

    return [ordered]@{
        exists = $true
        path = $rulesPath
        constraints = $constraints
    }
}

function Convert-CouplingGradeToLimit {
    param([object]$Grade)

    if ($null -eq $Grade) { return 5 }
    $value = ([string]$Grade).Trim().ToUpperInvariant()
    switch ($value) {
        "A" { return 2 }
        "B" { return 5 }
        "C" { return 8 }
        "D" { return 12 }
        default {
            $parsed = 0
            if ([int]::TryParse($value, [ref]$parsed)) { return $parsed }
            return 5
        }
    }
}

function New-WhatIfScenario {
    param(
        [string]$Name,
        [string]$Question,
        [bool]$Pass,
        [object[]]$Affected,
        [string]$Severity,
        [string]$RecommendedRule,
        [string]$Action
    )

    $affectedList = @($Affected)
    return [ordered]@{
        name = $Name
        question = $Question
        pass = $Pass
        severity = $Severity
        impact_count = $affectedList.Count
        affected = @($affectedList | Select-Object -First 20)
        recommended_rule = $RecommendedRule
        action = $Action
    }
}

function Get-WhatIfFunctionViolations {
    param(
        [object]$Dsm,
        [int]$MaxComplexity
    )

    $items = @()
    foreach ($file in @($Dsm["file_details"])) {
        foreach ($fn in @($file["functions"])) {
            if ([int]$fn["complexity"] -le $MaxComplexity) { continue }
            $items += [ordered]@{
                id = $fn["id"]
                name = $fn["name"]
                file = $file["path"]
                sourceAnchor = $fn["source_anchor"]
                complexity = [int]$fn["complexity"]
                limit = $MaxComplexity
                over_by = [int]$fn["complexity"] - $MaxComplexity
            }
        }
    }
    return @($items | Sort-Object { $_["over_by"] } -Descending)
}

function Get-WhatIfGodFileViolations {
    param(
        [object]$Dsm,
        [int]$MaxLoc,
        [int]$MaxFunctions
    )

    return @($Dsm["file_details"] |
        Where-Object { [int]$_["loc"] -gt $MaxLoc -or [int]$_["function_count"] -gt $MaxFunctions } |
        Sort-Object { [math]::Max(([int]$_["loc"] - $MaxLoc), ([int]$_["function_count"] - $MaxFunctions)) } -Descending |
        ForEach-Object {
            [ordered]@{
                id = $_["id"]
                path = $_["path"]
                sourceAnchor = $_["source_anchor"]
                loc = $_["loc"]
                functionCount = $_["function_count"]
                maxLoc = $MaxLoc
                maxFunctions = $MaxFunctions
            }
        })
}

function Get-WhatIfModuleMetricViolations {
    param(
        [object]$Dsm,
        [string]$Metric,
        [int]$Limit
    )

    return @($Dsm["modules"] |
        Where-Object { [int]$_["metrics"][$Metric] -gt $Limit } |
        Sort-Object { [int]$_["metrics"][$Metric] } -Descending |
        ForEach-Object {
            [ordered]@{
                id = $_["id"]
                name = $_["name"]
                metric = $Metric
                value = [int]$_["metrics"][$Metric]
                limit = $Limit
                files = $_["files"]
                risk = $_["metrics"]["risk"]
                coupling = $_["metrics"]["coupling"]
                blastRadius = $_["metrics"]["blast_radius"]
                testGap = $_["metrics"]["test_gap"]
            }
        })
}

function Invoke-WhatIfTool {
    param([string]$TargetPath)

    $dsm = Invoke-DsmTool $TargetPath
    $ruleHints = Read-SentruxRuleHints $TargetPath
    $constraints = $ruleHints["constraints"]
    $maxCc = if ($null -ne $constraints["max_cc"]) { [int]$constraints["max_cc"] } else { 25 }
    $strictCc = if ($maxCc -gt 15) { 15 } else { $maxCc }
    $hardCc = if ($strictCc -gt 10) { 10 } else { $strictCc }
    $couplingLimit = Convert-CouplingGradeToLimit $constraints["max_coupling"]
    $blastLimit = [math]::Max(3, $couplingLimit + 2)
    $godLocLimit = 800
    $godFunctionLimit = 40
    $busRiskLimit = 85

    $complexityAtRule = @(Get-WhatIfFunctionViolations $dsm $maxCc)
    $complexityStrict = @(Get-WhatIfFunctionViolations $dsm $strictCc)
    $complexityHard = @(Get-WhatIfFunctionViolations $dsm $hardCc)
    $godFiles = @(Get-WhatIfGodFileViolations $dsm $godLocLimit $godFunctionLimit)
    $coupling = @(Get-WhatIfModuleMetricViolations $dsm "coupling" $couplingLimit)
    $blast = @(Get-WhatIfModuleMetricViolations $dsm "blast_radius" $blastLimit)
    $testGaps = @(Get-WhatIfModuleMetricViolations $dsm "test_gap" 0)
    $busFactor = New-EvolutionBusFactorDetails $TargetPath $dsm
    $busFactorRisk = @($busFactor["modules"] |
        Where-Object { [int]$_["bus_factor_risk"] -ge $busRiskLimit } |
        Select-Object -First 20)
    $pollution = @(Get-PollutionSignals $TargetPath)
    $scopeCandidates = @(Find-ScopeCandidates $TargetPath)

    $scenarios = @(
        (New-WhatIfScenario `
            -Name "current_max_cc_gate" `
            -Question "Would the current or default max_cc gate pass?" `
            -Pass ($complexityAtRule.Count -eq 0) `
            -Affected $complexityAtRule `
            -Severity $(if ($complexityAtRule.Count -gt 0) { "high" } else { "ok" }) `
            -RecommendedRule "max_cc = $maxCc" `
            -Action "Split or simplify functions above the max_cc gate before raising the baseline.")
        (New-WhatIfScenario `
            -Name "strict_max_cc_gate" `
            -Question "What breaks if max_cc is tightened for agent-written code?" `
            -Pass ($complexityStrict.Count -eq 0) `
            -Affected $complexityStrict `
            -Severity $(if ($complexityStrict.Count -gt 0) { "medium" } else { "ok" }) `
            -RecommendedRule "max_cc = $strictCc" `
            -Action "Use this as the target for touched/new code; do not require legacy cleanup in one pass.")
        (New-WhatIfScenario `
            -Name "hard_max_cc_gate" `
            -Question "What breaks under a very strict complexity ceiling?" `
            -Pass ($complexityHard.Count -eq 0) `
            -Affected $complexityHard `
            -Severity $(if ($complexityHard.Count -gt 0) { "medium" } else { "ok" }) `
            -RecommendedRule "max_cc = $hardCc" `
            -Action "Use only for greenfield modules or narrow critical paths.")
        (New-WhatIfScenario `
            -Name "module_coupling_cap" `
            -Question "Which modules would fail a coupling cap?" `
            -Pass ($coupling.Count -eq 0) `
            -Affected $coupling `
            -Severity $(if ($coupling.Count -gt 0) { "high" } else { "ok" }) `
            -RecommendedRule "max_coupling = `"B`"" `
            -Action "Inspect top edges and carve adapters before adding new cross-module dependencies.")
        (New-WhatIfScenario `
            -Name "blast_radius_cap" `
            -Question "Which modules would fail a blast-radius cap?" `
            -Pass ($blast.Count -eq 0) `
            -Affected $blast `
            -Severity $(if ($blast.Count -gt 0) { "high" } else { "ok" }) `
            -RecommendedRule "max_blast_radius = $blastLimit" `
            -Action "Reduce incident dependencies or split fan-out responsibilities.")
        (New-WhatIfScenario `
            -Name "test_gap_gate" `
            -Question "Which modules would fail if every source-heavy module needed tests?" `
            -Pass ($testGaps.Count -eq 0) `
            -Affected $testGaps `
            -Severity $(if ($testGaps.Count -gt 0) { "medium" } else { "ok" }) `
            -RecommendedRule "require_tests_for_source_modules = true" `
            -Action "Add targeted smoke or contract tests around the highest-risk untested modules.")
        (New-WhatIfScenario `
            -Name "bus_factor_gate" `
            -Question "Which modules are one-person or no-history risks?" `
            -Pass ($busFactorRisk.Count -eq 0) `
            -Affected $busFactorRisk `
            -Severity $(if ($busFactorRisk.Count -gt 0) { "medium" } else { "ok" }) `
            -RecommendedRule "max_bus_factor_risk = $busRiskLimit" `
            -Action "Require review notes, ownership backup, or tests before large changes in these modules.")
        (New-WhatIfScenario `
            -Name "scope_pollution_guard" `
            -Question "Would this scope stay clean if root pollution were disallowed?" `
            -Pass ($pollution.Count -eq 0) `
            -Affected $pollution `
            -Severity $(if ($pollution.Count -gt 0) { "high" } else { "ok" }) `
            -RecommendedRule "governed_scope = explicit" `
            -Action "Keep scanning the root, but keep dependency, generated, and bundled asset code outside governed source metrics.")
    )

    $failed = @($scenarios | Where-Object { -not $_["pass"] })
    $primary = if ($failed.Count -gt 0) { $failed[0]["name"] } else { "none" }
    $recommendations = @()
    if (-not [bool]$ruleHints["exists"]) {
        $recommendations += "Add .sentrux/rules.toml before treating this scope as governed."
    }
    foreach ($scenario in $failed | Select-Object -First 5) {
        $recommendations += "$($scenario["name"]): $($scenario["action"])"
    }
    if ($recommendations.Count -eq 0) {
        $recommendations += "Current scope passes the default what-if gates; keep using session_start/session_end for drift."
    }

    return [ordered]@{
        tool = "what_if"
        path = $TargetPath
        generated_at = (Get-Date).ToString("o")
        rules = $ruleHints
        thresholds = [ordered]@{
            max_cc = $maxCc
            strict_max_cc = $strictCc
            hard_max_cc = $hardCc
            max_module_coupling = $couplingLimit
            max_blast_radius = $blastLimit
            god_file_loc = $godLocLimit
            god_file_functions = $godFunctionLimit
            max_bus_factor_risk = $busRiskLimit
        }
        summary = [ordered]@{
            scenarios = $scenarios.Count
            passing = @($scenarios | Where-Object { $_["pass"] }).Count
            failing = $failed.Count
            primary_risk = $primary
            scope_candidates = $scopeCandidates
            source_scope = $dsm["scope"]
        }
        scenarios = $scenarios
        recommendations = $recommendations
        assumptions = @(
            "This is deterministic static analysis, not a runtime prediction.",
            "Git-derived bus factor is conservative when files are untracked or have shallow history.",
            "Strict gates are meant for new or touched code unless the team explicitly schedules legacy cleanup."
        )
    }
}

function Invoke-EvolutionTool {
    param(
        [string]$TargetPath,
        [int]$Limit
    )

    $dir = Get-SessionDir $TargetPath
    $sessions = @()
    if (Test-Path -LiteralPath $dir -PathType Container) {
        $starts = Get-ChildItem -LiteralPath $dir -Filter "*.start.json" -File |
            Sort-Object Name -Descending |
            Select-Object -First $Limit
        foreach ($startFile in $starts) {
            $id = ($startFile.BaseName -replace "\.start$", "")
            $start = Read-JsonFileSafe $startFile.FullName
            $end = Read-JsonFileSafe (Join-Path $dir "$id.end.json")
            $sessions += [ordered]@{
                session_id = $id
                start_signal = Convert-QualitySignal (Get-JsonProperty $start "quality_signal")
                end_signal = Convert-QualitySignal (Get-JsonProperty $end "signal_after")
                pass = Get-JsonProperty $end "pass"
                started_at = Get-JsonProperty $start "started_at"
                ended_at = Get-JsonProperty $end "ended_at"
            }
        }
    }

    $dsm = Invoke-DsmTool $TargetPath
    $hotspots = New-EvolutionHotspots $dsm
    $coupling = New-EvolutionCouplingDetails $dsm
    $busFactor = New-EvolutionBusFactorDetails $TargetPath $dsm
    $trend = New-SessionTrend $sessions

    return [ordered]@{
        tool = "evolution"
        path = $TargetPath
        sessions = $sessions
        count = $sessions.Count
        trend = $trend
        hotspots = $hotspots
        coupling = $coupling
        bus_factor = $busFactor
    }
}

$targetPath = Resolve-Directory $Path
$normalizedTool = if ($Tool.StartsWith("sentrux_")) { $Tool.Substring("sentrux_".Length) } else { $Tool }
$result = switch ($normalizedTool) {
    "scan" { Invoke-ScanTool $targetPath }
    "health" { Invoke-HealthTool $targetPath }
    "session_start" { Invoke-SessionStartTool $targetPath $SessionId }
    "session_end" { Invoke-SessionEndTool $targetPath $SessionId }
    "rescan" { Invoke-ScanTool $targetPath "rescan" }
    "check_rules" { Invoke-CheckRulesTool $targetPath }
    "evolution" { Invoke-EvolutionTool $targetPath $Recent }
    "dsm" { Invoke-DsmTool $targetPath }
    "git_stats" { Invoke-GitStatsTool $targetPath }
    "test_gaps" { Invoke-TestGapsTool $targetPath }
    "what_if" { Invoke-WhatIfTool $targetPath }
}

$result | ConvertTo-Json -Depth 14
