#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArgs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$RemainingArgs = @($RemainingArgs)
$script:SentruxLiteExitCode = 0

function Show-Help {
    Write-Output "Live codebase visualization and structural quality gate"
    Write-Output ""
    Write-Output "Usage: sentrux <COMMAND> [PATH]"
    Write-Output ""
    Write-Output "Commands:"
    Write-Output "  scan       Scan a project and print structural metrics as JSON"
    Write-Output "  health     Print a compact health signal"
    Write-Output "  check      Enforce architectural rules from .sentrux/rules.toml"
    Write-Output "  gate       Compare current structure with .sentrux/baseline.json"
    Write-Output "  plugin     List and validate local language plugins"
    Write-Output "  pro        Manage local open-source Pro activation"
    Write-Output ""
    Write-Output "check: Enforce architectural rules"
}

function Resolve-TargetPath {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        $Path = (Get-Location).Path
    }
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        throw "target path is not a directory: $Path"
    }
    return $item.FullName
}

function Test-SkippedPath {
    param([string]$Path)

    $normalized = ($Path -replace "\\", "/").ToLowerInvariant()
    if ($normalized -match "/(static|public|wwwroot)/assets/") {
        return $true
    }
    $leaf = [System.IO.Path]::GetFileName(($Path -replace "\\", "/"))
    $leafLower = $leaf.ToLowerInvariant()
    if ($leafLower -match "(\.min|\.bundle)\.(js|jsx|mjs|cjs)$") {
        return $true
    }
    if ($leaf -match ".+-[A-Za-z0-9_]{6,}\.(js|jsx|mjs|cjs)$" -and $leaf -match "[0-9]" -and $leaf -cmatch "[A-Z]") {
        return $true
    }

    $parts = @($normalized -split "/" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    foreach ($part in $parts) {
        if ($part -in @(
            ".git", ".repowise", ".understand-anything", ".sentrux",
            "tools", "vendor", "third_party", "external",
            "node_modules", ".pnpm", ".yarn",
            "target", "dist", "build", "out", "coverage",
            ".venv", "venv", "env", ".tox", "__pycache__",
            ".next", ".nuxt", ".turbo", ".cache"
        )) {
            return $true
        }
        if ($part -match "^(\.venv|venv|env)-") {
            return $true
        }
    }
    return $false
}

function Get-CodeFiles {
    param([string]$TargetPath)

    $extensions = @(".ps1", ".psm1", ".py", ".rs", ".go", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".java", ".cs", ".cpp", ".c", ".h", ".hpp", ".v")
    # Skip rules must apply to the path *relative to the target*, otherwise a
    # target rooted inside a skip-listed dir name (e.g. <repo>\tools) matches nothing.
    $files = Get-ChildItem -LiteralPath $TargetPath -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object {
            -not (Test-SkippedPath (Get-RelativePathSafe $TargetPath $_.FullName)) -and
            $_.Length -le 2097152 -and
            $extensions -contains $_.Extension.ToLowerInvariant()
        }
    return @($files)
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

function Measure-File {
    param(
        [string]$TargetPath,
        [System.IO.FileInfo]$File
    )

    $text = ""
    try {
        $text = Get-Content -LiteralPath $File.FullName -Raw -ErrorAction Stop
        if ($null -eq $text) {
            $text = ""
        }
    }
    catch {
        $text = ""
    }
    # Get-Content -Raw returns $null (without throwing) for empty files
    if ($null -eq $text) { $text = "" }

    $lines = if ([string]::IsNullOrWhiteSpace($text)) { @() } else { @($text -split "\r?\n") }
    $loc = @($lines | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }).Count
    $functionMatches = [regex]::Matches($text, "(?m)^\s*(function\s+[\w:-]+|def\s+\w+|fn\s+\w+|pub\s+fn\s+\w+|func\s+\w+|class\s+\w+|interface\s+\w+|export\s+function\s+\w+|const\s+\w+\s*=\s*(async\s*)?\(|public\s+[\w<>\[\]]+\s+\w+\s*\()")
    $functionCount = $functionMatches.Count

    $complexityTerms = [regex]::Matches($text, "\b(if|else\s+if|for|foreach|while|switch|case|catch|except|match|and|or|\&\&|\|\|)\b")
    $maxComplexity = 1
    if ($functionCount -gt 0) {
        $maxComplexity = [Math]::Max(1, [int][Math]::Ceiling($complexityTerms.Count / [Math]::Max(1, $functionCount)) + 1)
    }
    elseif ($complexityTerms.Count -gt 0) {
        $maxComplexity = [Math]::Max(1, [int]$complexityTerms.Count + 1)
    }

    if ($File.Extension.ToLowerInvariant() -eq ".v") {
        $importMatches = @([regex]::Matches($text, "(?m)^\s*import\s+([A-Za-z_][\w.]*)") |
            Where-Object { $_.Groups[1].Value -notin @("os", "time", "json", "strings", "arrays", "maps", "math") })
        $callMatches = @([regex]::Matches($text, "\b(?:([A-Za-z_]\w*)\.)?([A-Za-z_]\w*)\s*\(") |
            Where-Object { $_.Groups[1].Value -notin @("os", "time", "json", "strings", "arrays", "maps", "math") })
    }
    else {
        $importMatches = @([regex]::Matches($text, "(?m)^\s*(import\s+|from\s+|use\s+|mod\s+|require\(|#include\s+|using\s+)"))
        $callMatches = @([regex]::Matches($text, "\b\w+\s*\("))
    }

    return [ordered]@{
        path = Get-RelativePathSafe $TargetPath $File.FullName
        loc = $loc
        functions = $functionCount
        max_complexity = $maxComplexity
        imports = $importMatches.Count
        calls = $callMatches.Count
        is_god_file = ($loc -gt 800 -or ($functionCount -gt 25 -and $loc -gt 400))
        is_complex = ($maxComplexity -gt 25)
    }
}

function Get-SafeSum {
    param(
        [object[]]$Items,
        [string]$Property
    )
    $result = $Items | Measure-Object -Property $Property -Sum -ErrorAction SilentlyContinue
    if ($null -ne $result -and $null -ne $result.PSObject.Properties['Sum'] -and $null -ne $result.Sum) {
        return [int]$result.Sum
    }
    return 0
}

function Get-SafeMaximum {
    param(
        [object[]]$Items,
        [string]$Property
    )
    $result = $Items | Measure-Object -Property $Property -Maximum -ErrorAction SilentlyContinue
    if ($null -ne $result -and $null -ne $result.PSObject.Properties['Maximum'] -and $null -ne $result.Maximum) {
        return [int]$result.Maximum
    }
    return 0
}

function Measure-Project {
    param([string]$TargetPath)

    $files = Get-CodeFiles $TargetPath
    $fileMetrics = @($files | ForEach-Object { [pscustomobject](Measure-File $TargetPath $_) })
    $fileCount = $fileMetrics.Count
    $importEdges = Get-SafeSum $fileMetrics 'imports'
    $callEdges = Get-SafeSum $fileMetrics 'calls'
    $godFiles = @($fileMetrics | Where-Object { $_.is_god_file }).Count
    $complexFiles = @($fileMetrics | Where-Object { $_.is_complex }).Count
    $functions = Get-SafeSum $fileMetrics 'functions'
    $maxComplexity = Get-SafeMaximum $fileMetrics 'max_complexity'
    $couplingScore = if ($fileCount -gt 0) { [Math]::Round(($importEdges / [Math]::Max(1, $fileCount)) * 10, 2) } else { 0 }
    $quality = [int][Math]::Max(0, 10000 - ($couplingScore * 8) - ($complexFiles * 60) - ($godFiles * 120) - [Math]::Max(0, $maxComplexity - 15) * 10)

    return [ordered]@{
        tool = "sentrux-lite"
        path = $TargetPath
        quality_signal = $quality
        files = $fileCount
        functions = $functions
        coupling_score = $couplingScore
        cycle_count = 0
        god_file_count = $godFiles
        complex_fn_count = $complexFiles
        max_complexity = $maxComplexity
        total_import_edges = $importEdges
        cross_module_edges = $importEdges
        call_edges = $callEdges
        unresolved_imports = 0
        files_detail = $fileMetrics
    }
}

function Get-BaselinePath {
    param([string]$TargetPath)
    return (Join-Path (Join-Path $TargetPath ".sentrux") "baseline.json")
}

function Write-Baseline {
    param(
        [string]$TargetPath,
        [object]$Metrics
    )

    $baselinePath = Get-BaselinePath $TargetPath
    $baselineDir = Split-Path -Parent $baselinePath
    New-Item -ItemType Directory -Force -Path $baselineDir | Out-Null
    $payload = [ordered]@{
        tool = "sentrux-lite"
        saved_at = (Get-Date).ToUniversalTime().ToString("o")
        path = $TargetPath
        quality_signal = $Metrics.quality_signal
        coupling_score = $Metrics.coupling_score
        cycle_count = $Metrics.cycle_count
        god_file_count = $Metrics.god_file_count
        complex_fn_count = $Metrics.complex_fn_count
        cross_module_edges = $Metrics.cross_module_edges
        total_import_edges = $Metrics.total_import_edges
        files = $Metrics.files
        functions = $Metrics.functions
        max_complexity = $Metrics.max_complexity
    }
    $payload | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $baselinePath -Encoding UTF8
    return $baselinePath
}

function Write-GateOutput {
    param(
        [object]$Before,
        [object]$After,
        [bool]$Saved = $false,
        [string]$BaselinePath = ""
    )

    Write-Output ("[resolve] {0} resolved, {1} unresolved" -f $After.total_import_edges, $After.unresolved_imports)
    Write-Output ("[build_graphs] {0} files | {1} import, {2} call, 0 inherit edges" -f $After.files, $After.total_import_edges, $After.call_edges)
    Write-Output ("Quality: {0} -> {1}" -f $Before.quality_signal, $After.quality_signal)
    Write-Output ("Coupling: {0} -> {1}" -f $Before.coupling_score, $After.coupling_score)
    Write-Output ("Cycles: {0} -> {1}" -f $Before.cycle_count, $After.cycle_count)
    Write-Output ("God files: {0} -> {1}" -f $Before.god_file_count, $After.god_file_count)
    Write-Output ("Distance from Main Sequence: {0}" -f ([Math]::Round($After.coupling_score / 100, 4)))
    if ($Saved) {
        Write-Output "Baseline saved: $BaselinePath"
    }
}

function Invoke-Scan {
    param([string]$Path)
    $target = Resolve-TargetPath $Path
    $metrics = Measure-Project $target
    $metrics | ConvertTo-Json -Depth 8
}

function Invoke-Health {
    param([string]$Path)
    $target = Resolve-TargetPath $Path
    $metrics = Measure-Project $target
    [ordered]@{
        status = "ok"
        tool = "sentrux-lite"
        quality_signal = $metrics.quality_signal
        files = $metrics.files
        bottleneck = if ($metrics.god_file_count -gt 0) { "god_files" } elseif ($metrics.complex_fn_count -gt 0) { "complexity" } elseif ($metrics.coupling_score -gt 20) { "coupling" } else { "none" }
    } | ConvertTo-Json -Depth 4
}

function Invoke-Check {
    param([string]$Path)
    $target = Resolve-TargetPath $Path
    $rulesPath = Join-Path (Join-Path $target ".sentrux") "rules.toml"
    if (-not (Test-Path -LiteralPath $rulesPath -PathType Leaf)) {
        Write-Output "No .sentrux/rules.toml found"
        Write-Output "Quality: not gated"
        $script:SentruxLiteExitCode = 0
        return
    }

    $rules = Get-Content -LiteralPath $rulesPath -Raw
    $metrics = Measure-Project $target
    Write-Output ("[resolve] {0} resolved, {1} unresolved" -f $metrics.total_import_edges, $metrics.unresolved_imports)
    Write-Output ("[build_graphs] {0} files | {1} import, {2} call, 0 inherit edges" -f $metrics.files, $metrics.total_import_edges, $metrics.call_edges)
    $violations = New-Object System.Collections.Generic.List[string]
    $maxCcMatch = [regex]::Match($rules, "(?m)^\s*max_cc\s*=\s*([0-9]+)")
    if ($maxCcMatch.Success -and $metrics.max_complexity -gt [int]$maxCcMatch.Groups[1].Value) {
        $violations.Add("max_cc exceeded: $($metrics.max_complexity) > $($maxCcMatch.Groups[1].Value)")
    }
    if ($rules -match "(?m)^\s*no_god_files\s*=\s*true" -and $metrics.god_file_count -gt 0) {
        $violations.Add("god files detected: $($metrics.god_file_count)")
    }
    $maxCyclesMatch = [regex]::Match($rules, "(?m)^\s*max_cycles\s*=\s*([0-9]+)")
    if ($maxCyclesMatch.Success -and $metrics.cycle_count -gt [int]$maxCyclesMatch.Groups[1].Value) {
        $violations.Add("cycles exceeded: $($metrics.cycle_count) > $($maxCyclesMatch.Groups[1].Value)")
    }
    $maxCouplingMatch = [regex]::Match($rules, "(?m)^\s*max_coupling\s*=\s*""?([A-D])""?")
    if ($maxCouplingMatch.Success) {
        $thresholds = @{ A = 5; B = 15; C = 30; D = 60 }
        $grade = $maxCouplingMatch.Groups[1].Value
        if ($metrics.coupling_score -gt $thresholds[$grade]) {
            $violations.Add("coupling grade exceeded: $($metrics.coupling_score) > $($thresholds[$grade]) for $grade")
        }
    }

    if ($violations.Count -gt 0) {
        Write-Output "Sentrux Lite check failed"
        foreach ($violation in $violations) {
            Write-Output "- $violation"
        }
        $script:SentruxLiteExitCode = 1
        return
    }

    Write-Output "All rules passed - Quality: $($metrics.quality_signal)"
    $script:SentruxLiteExitCode = 0
}

function Invoke-Gate {
    param([string[]]$GateArgs)

    $save = $false
    $paths = New-Object System.Collections.Generic.List[string]
    foreach ($arg in $GateArgs) {
        if ($arg -eq "--save") {
            $save = $true
        }
        else {
            $paths.Add($arg)
        }
    }

    $target = Resolve-TargetPath ($(if ($paths.Count -gt 0) { $paths[0] } else { "" }))
    $metrics = [pscustomobject](Measure-Project $target)
    $baselinePath = Get-BaselinePath $target
    if ($save) {
        $savedPath = Write-Baseline $target $metrics
        Write-GateOutput $metrics $metrics $true $savedPath
        exit 0
    }

    if (-not (Test-Path -LiteralPath $baselinePath -PathType Leaf)) {
        Write-Output "Sentrux baseline missing at $baselinePath"
        Write-Output "Run sentrux gate --save $target"
        exit 1
    }

    $baseline = Get-Content -LiteralPath $baselinePath -Raw | ConvertFrom-Json
    Write-GateOutput $baseline $metrics $false $baselinePath
    $regressed = $false
    if ([double]$metrics.quality_signal -lt [double]$baseline.quality_signal) { $regressed = $true }
    if ([double]$metrics.coupling_score -gt [double]$baseline.coupling_score) { $regressed = $true }
    if ([double]$metrics.cycle_count -gt [double]$baseline.cycle_count) { $regressed = $true }
    if ([double]$metrics.god_file_count -gt [double]$baseline.god_file_count) { $regressed = $true }

    if ($regressed) {
        Write-Output "Quality degraded during this session"
        exit 1
    }

    Write-Output "No degradation detected"
    exit 0
}

function Get-PluginRoot {
    if (-not [string]::IsNullOrWhiteSpace($env:SENTRUX_PLUGIN_ROOT)) {
        return $env:SENTRUX_PLUGIN_ROOT
    }
    $userProfile = if (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) { $env:USERPROFILE } else { $HOME }
    return (Join-Path $userProfile (Join-Path ".sentrux" "plugins"))
}

function Read-PluginTomlField {
    param(
        [string]$Toml,
        [string]$Name
    )

    $match = [regex]::Match($Toml, "(?m)^\s*$([regex]::Escape($Name))\s*=\s*['""]([^'""]+)['""]")
    if ($match.Success) { return $match.Groups[1].Value }
    return ""
}

function Read-PluginExtensions {
    param([string]$Toml)

    $match = [regex]::Match($Toml, "(?m)^\s*extensions\s*=\s*\[([^\]]*)\]")
    if (-not $match.Success) { return @() }
    return @([regex]::Matches($match.Groups[1].Value, "['""]([^'""]+)['""]") | ForEach-Object { $_.Groups[1].Value })
}

function Test-PluginPath {
    param([string]$PluginPath)

    $target = try {
        Resolve-TargetPath $PluginPath
    }
    catch {
        Write-Output "plugin invalid: path does not exist or is not a directory: $PluginPath"
        $script:SentruxLiteExitCode = 1
        return
    }
    $pluginToml = Join-Path $target "plugin.toml"
    $queryPath = Join-Path $target (Join-Path "queries" "tags.scm")
    $grammarDir = Join-Path $target "grammars"

    foreach ($required in @($pluginToml, $queryPath)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            Write-Output "plugin invalid: missing $required"
            $script:SentruxLiteExitCode = 1
            return
        }
    }

    $toml = Get-Content -LiteralPath $pluginToml -Raw
    if ($toml -match "(?m)^\s*\[grammar\]\s*$") {
        $grammarFiles = @()
        if (Test-Path -LiteralPath $grammarDir -PathType Container) {
            $grammarFiles = @(Get-ChildItem -LiteralPath $grammarDir -File -ErrorAction SilentlyContinue)
        }
        if ($grammarFiles.Count -eq 0) {
            Write-Output "plugin invalid: grammar artifact missing under $grammarDir"
            $script:SentruxLiteExitCode = 1
            return
        }
    }

    $name = Read-PluginTomlField $toml "name"
    $version = Read-PluginTomlField $toml "version"
    $extensions = @(Read-PluginExtensions $toml)
    if ([string]::IsNullOrWhiteSpace($name) -or [string]::IsNullOrWhiteSpace($version) -or $extensions.Count -eq 0) {
        Write-Output "plugin invalid: plugin.toml is missing name, version, or extensions"
        $script:SentruxLiteExitCode = 1
        return
    }

    Write-Output ("plugin valid: {0} v{1} [{2}]" -f $name, $version, ($extensions -join ","))
    $script:SentruxLiteExitCode = 0
}

function Invoke-Plugin {
    param([string[]]$PluginArgs)

    $subcommand = if ($PluginArgs.Count -gt 0) { $PluginArgs[0] } else { "list" }
    switch ($subcommand) {
        "list" {
            $pluginRoot = Get-PluginRoot
            if (-not (Test-Path -LiteralPath $pluginRoot -PathType Container)) {
                $script:SentruxLiteExitCode = 0
                return
            }
            $plugins = Get-ChildItem -LiteralPath $pluginRoot -Directory -ErrorAction SilentlyContinue
            foreach ($plugin in @($plugins)) {
                $tomlPath = Join-Path $plugin.FullName "plugin.toml"
                if (-not (Test-Path -LiteralPath $tomlPath -PathType Leaf)) { continue }
                $toml = Get-Content -LiteralPath $tomlPath -Raw
                $name = Read-PluginTomlField $toml "name"
                $version = Read-PluginTomlField $toml "version"
                $extensions = @(Read-PluginExtensions $toml)
                if ([string]::IsNullOrWhiteSpace($name)) { $name = $plugin.Name }
                Write-Output ("{0} v{1} [{2}]" -f $name, $version, ($extensions -join ","))
            }
            $script:SentruxLiteExitCode = 0
            return
        }
        "validate" {
            $pluginPath = if ($PluginArgs.Count -gt 1) { $PluginArgs[1] } else { "" }
            if ([string]::IsNullOrWhiteSpace($pluginPath)) {
                Write-Output "plugin validate requires <plugin-path>"
                $script:SentruxLiteExitCode = 1
                return
            }
            Test-PluginPath $pluginPath
            return
        }
        default {
            Write-Output "sentrux-lite: unknown plugin command '$subcommand'"
            Write-Output "Usage: sentrux plugin <list|validate> [plugin-path]"
            $script:SentruxLiteExitCode = 1
            return
        }
    }
}

if ($RemainingArgs.Count -eq 0 -or $RemainingArgs[0] -in @("-h", "--help", "help")) {
    Show-Help
    exit 0
}

$command = $RemainingArgs[0]
$tail = @($RemainingArgs | Select-Object -Skip 1)
switch ($command) {
    "scan" {
        Invoke-Scan ($(if ($tail.Count -gt 0) { $tail[0] } else { "" }))
        exit 0
    }
    "health" {
        Invoke-Health ($(if ($tail.Count -gt 0) { $tail[0] } else { "" }))
        exit 0
    }
    "check" {
        if ($tail.Count -gt 0 -and $tail[0] -in @("-h", "--help", "help")) {
            Show-Help
            exit 0
        }
        Invoke-Check ($(if ($tail.Count -gt 0) { $tail[0] } else { "" }))
        exit $script:SentruxLiteExitCode
    }
    "gate" {
        Invoke-Gate $tail
    }
    "plugin" {
        Invoke-Plugin $tail
        exit $script:SentruxLiteExitCode
    }
    default {
        Write-Output "sentrux-lite: unknown command '$command'"
        Show-Help
        exit 1
    }
}
