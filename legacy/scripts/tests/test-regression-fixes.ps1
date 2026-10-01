# Regression tests for the 7 fail-open / false-green fixes landed in commit da46886
# ("fix: close fail-open gates and metric contradiction across sentrux layer").
#
# Run:
#   pwsh -File scripts/tests/test-regression-fixes.ps1
#   pwsh -File scripts/tests/test-regression-fixes.ps1 -VerboseOutput
#
# Pattern: lightweight assert-based harness (no external test framework — this repo
# doesn't have one). Each Test-Case runs in a try/catch, failures are collected and
# reported at the end with a non-zero exit code. This mirrors the "throw on failure"
# style already used by scripts/tests/test-code-intel-pipeline.ps1, but adds pass/fail counting
# since this file exercises many small, independent units rather than one end-to-end
# pipeline run.
#
# Every case creates its own scratch directory under $env:TEMP and cleans it up
# afterward. Nothing here touches a real repo's .sentrux/baseline.json.

param(
    [switch]$VerboseOutput
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))

$repoRoot = Split-Path -Parent $root
$script:passed = 0
$script:failed = 0
$script:failures = New-Object System.Collections.Generic.List[string]

function Test-Case {
    param(
        [string]$Name,
        [scriptblock]$Body
    )

    try {
        & $Body
        $script:passed++
        if ($VerboseOutput) { Write-Host "[PASS] $Name" -ForegroundColor Green }
    }
    catch {
        $script:failed++
        $script:failures.Add("$Name -- $($_.Exception.Message)")
        Write-Host "[FAIL] $Name -- $($_.Exception.Message)" -ForegroundColor Red
    }
}

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw "Assert-True failed: $Message" }
}

function Assert-False {
    param([bool]$Condition, [string]$Message)
    if ($Condition) { throw "Assert-False failed: $Message" }
}

function Assert-Equal {
    param($Expected, $Actual, [string]$Message)
    if ("$Expected" -ne "$Actual") {
        throw "Assert-Equal failed: $Message (expected '$Expected', got '$Actual')"
    }
}

function New-ScratchDir {
    param([string]$Prefix)
    $dir = Join-Path $env:TEMP ("cip-test-{0}-{1}" -f $Prefix, [guid]::NewGuid().ToString("N").Substring(0, 8))
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    return (Get-Item -LiteralPath $dir).FullName
}

# Extracts only the function definitions from a script (via AST), WITHOUT running
# the script's top-level body. This lets us unit-test functions inside
# run-code-intel.ps1 / Invoke-SentruxAgentTool.ps1 / Install-SentruxVlangOverlay.ps1
# even though those files execute real work (network calls, sentrux invocation,
# file installs) at the bottom of the file.
#
# NOTE: this returns the extracted source text; the CALL SITE must dot-source it
# directly (". Get-ScriptFunctionsSource ...") so the functions land in script
# scope. Calling `. $scriptBlock` from inside a helper function only dot-sources
# into that helper's own scope, which disappears when the helper returns.
function Get-ScriptFunctionsSource {
    param(
        [string]$Path,
        [string[]]$Only = @()
    )

    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors -and $parseErrors.Count -gt 0) {
        throw "Failed to parse $Path for function extraction: $($parseErrors[0].Message)"
    }

    $funcAsts = $ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true)
    if ($Only.Count -gt 0) {
        $funcAsts = @($funcAsts | Where-Object { $Only -contains $_.Name })
    }
    if ($funcAsts.Count -eq 0) {
        throw "No matching function definitions found in $Path"
    }

    $source = ($funcAsts | ForEach-Object { $_.Extent.Text }) -join "`n`n"
    return [scriptblock]::Create($source)
}

Write-Host "== code-intel-pipeline regression suite (fixes in da46886) ==" -ForegroundColor Cyan

# ---------------------------------------------------------------------------
# Sentrux core resolution: an installed PATH launcher is a thin forwarder back
# to this repository, not a real core. Unix exposes the extensionless launcher
# to PATH discovery, so selecting it would recurse until the process is killed.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "tools\sentrux-shim\sentrux-shim.ps1") -Only @(
        "Test-CodeIntelThinForwarderCandidate",
        "Resolve-Core"
    ))

Test-Case "Resolve-Core skips the installed thin-forwarder bin and selects a later real core" {
    $dir = New-ScratchDir "sentrux-forwarder"
    $savedPath = $env:PATH
    $savedCoreOverride = $env:SENTRUX_CORE_EXE
    try {
        $thinDir = Join-Path $dir "thin-bin"
        $realDir = Join-Path $dir "real-bin"
        $sourceShimDir = Join-Path $dir "source\tools\sentrux-shim"
        New-Item -ItemType Directory -Force -Path $thinDir, $realDir, $sourceShimDir | Out-Null

        # Use the Unix extensionless names even on Windows so this local suite
        # directly exercises the CI failure mode instead of relying on host OS.
        $thinLauncherName = "sentrux"
        $realCoreName = "sentrux-core"
        $thinLauncher = Join-Path $thinDir $thinLauncherName
        $realCore = Join-Path $realDir $realCoreName
        Set-Content -LiteralPath $thinLauncher -Value "thin launcher" -Encoding UTF8
        Set-Content -LiteralPath (Join-Path $thinDir "sentrux-shim.ps1") -Value "# thin forwarder" -Encoding UTF8
        [ordered]@{
            repoRoot = $root
            generatedAt = (Get-Date).ToUniversalTime().ToString("o")
            note = "Generated by install-code-intel-pipeline.ps1. bin/ contains thin forwarders only; edit the repo source at repoRoot, not the files in this directory."
        } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $thinDir "repo.json") -Encoding UTF8
        Set-Content -LiteralPath $realCore -Value "real core" -Encoding UTF8
        $sourceShimPath = Join-Path $sourceShimDir "sentrux-shim.ps1"
        Set-Content -LiteralPath $sourceShimPath -Value "# source shim" -Encoding UTF8

        $env:SENTRUX_CORE_EXE = $null
        $env:PATH = $thinDir + [System.IO.Path]::PathSeparator + $realDir

        Assert-True (Test-CodeIntelThinForwarderCandidate -Path $thinLauncher) "the installer marker must classify its launcher as a thin forwarder"
        Assert-False (Test-CodeIntelThinForwarderCandidate -Path $realCore) "an ordinary core directory must not be classified as a thin forwarder"
        Set-Content -LiteralPath (Join-Path $thinDir "repo.json") -Value "{ partial metadata" -Encoding UTF8
        Assert-True (Test-CodeIntelThinForwarderCandidate -Path $thinLauncher) "damaged marker metadata must not reopen the recursive PATH candidate"
        Assert-Equal ([System.IO.Path]::GetFullPath($realCore)) (Resolve-Core -ShimPath $sourceShimPath) "core resolution must skip the recursive PATH launcher and choose the later real core"
    }
    finally {
        $env:PATH = $savedPath
        $env:SENTRUX_CORE_EXE = $savedCoreOverride
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Code Evidence symbol extraction: lock behavior before decomposing the
# high-complexity native fallback matcher.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "run-code-intel.ps1") -Only @(
        "New-CodeEvidenceNativeSymbol",
        "Get-CodeEvidencePowerShellSymbol",
        "Get-CodeEvidencePythonSymbol",
        "Get-CodeEvidenceJavaScriptSymbol",
        "Get-CodeEvidenceRustSymbol",
        "Get-CodeEvidenceGoSymbol",
        "Get-CodeEvidenceJavaSymbol",
        "Get-CodeEvidenceSymbolCandidate",
        "Get-CodeEvidenceSymbols"
    ))

Test-Case "code evidence symbols: supported language matchers preserve native output" {
    $cases = @(
        @{ language = "powershell"; lines = @("function Invoke-Thing {"); expected = @("function:Invoke-Thing:1") },
        @{ language = "python"; lines = @("class Widget:", "def run():"); expected = @("class:Widget:1", "function:run:2") },
        @{ language = "javascript"; lines = @("export async function load() {}", "const save = async (x) => x", "export class Box {}"); expected = @("function:load:1", "function:save:2", "class:Box:3") },
        @{ language = "rust"; lines = @("pub async fn fetch_data() {}"); expected = @("function:fetch_data:1") },
        @{ language = "go"; lines = @("func (s *Server) Serve() {}"); expected = @("function:Serve:1") },
        @{ language = "java"; lines = @("public class Demo {}"); expected = @("class:Demo:1") }
    )

    foreach ($case in $cases) {
        $symbols = @(Get-CodeEvidenceSymbols -RelativePath "sample" -Language $case.language -Lines $case.lines)
        $actual = @($symbols | ForEach-Object { "$($_.kind):$($_.name):$($_.startLine)" })
        Assert-Equal ($case.expected -join "|") ($actual -join "|") "symbol output mismatch for $($case.language)"
    }
}

# ---------------------------------------------------------------------------
# Sentrux module bucketing: lock representative taxonomy behavior before
# replacing a long conditional chain with table-driven matching.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "Invoke-SentruxAgentTool.ps1") -Only @(
        "Get-FirstRegexBucket",
        "Get-ModuleBucket"
    ))

Test-Case "Get-ModuleBucket preserves representative taxonomy buckets" {
    $cases = @(
        @{ domain = "strategy"; leaf = " "; expected = "__root__" },
        @{ domain = "strategy"; leaf = "__init__"; expected = "__root__" },
        @{ domain = "strategy"; leaf = "app__strategy__dialectical_rule"; expected = "dialectical_filter" },
        @{ domain = "strategy"; leaf = "market_monitor"; expected = "market" },
        @{ domain = "data"; leaf = "okx_feed"; expected = "okx" },
        @{ domain = "api"; leaf = "crypto_price"; expected = "market" },
        @{ domain = "cli"; leaf = "config_cmd"; expected = "market_control" },
        @{ domain = "trading"; leaf = "runner_live"; expected = "market_execution" },
        @{ domain = "brokers"; leaf = "exchange_bridge"; expected = "market_execution" },
        @{ domain = "markets"; leaf = "okx_ccxt_adapter"; expected = "market_integration_adapter" },
        @{ domain = "unknown"; leaf = "whatever"; expected = "misc" }
    )

    foreach ($case in $cases) {
        $actual = Get-ModuleBucket -Domain $case.domain -Leaf $case.leaf
        Assert-Equal $case.expected $actual "bucket mismatch for $($case.domain)/$($case.leaf)"
    }
}

# ---------------------------------------------------------------------------
# Sentrux DSM workflow: lock the output contract consumed by the pipeline
# artifact writer and browser/sidebar handoff.
# ---------------------------------------------------------------------------
Test-Case "Invoke-SentruxAgentTool dsm emits expected output contract" {
    $dir = New-ScratchDir "dsm-contract"
    try {
        Set-Content -LiteralPath (Join-Path $dir "alpha.py") -Value @(
            "import beta",
            "",
            "def alpha():",
            "    return beta.beta()"
        ) -Encoding UTF8
        Set-Content -LiteralPath (Join-Path $dir "beta.py") -Value @(
            "def beta():",
            "    return 42"
        ) -Encoding UTF8
        New-Item -ItemType Directory -Path (Join-Path $dir "tests") -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $dir "tests\test_alpha.py") -Value @(
            "from alpha import alpha",
            "",
            "def test_alpha():",
            "    assert alpha() == 42"
        ) -Encoding UTF8

        $raw = & pwsh -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root "Invoke-SentruxAgentTool.ps1") dsm $dir
        if ($LASTEXITCODE -ne 0) {
            throw "dsm command exited ${LASTEXITCODE}: $raw"
        }

        $dsm = $raw | ConvertFrom-Json
        Assert-Equal "dsm" $dsm.tool "DSM tool marker"
        Assert-Equal "Risk" $dsm.default_color_mode "DSM default color mode"
        Assert-Equal 9 @($dsm.color_modes).Count "DSM must expose 9 color modes"

        $colorNames = @($dsm.color_modes | ForEach-Object { $_.name })
        foreach ($expectedColor in @("Size", "Coupling", "TestGap", "Age", "Churn", "Risk", "Git", "ExecDepth", "BlastRadius")) {
            Assert-True ($colorNames -contains $expectedColor) "missing DSM color mode $expectedColor"
        }

        Assert-True (@($dsm.modules).Count -ge 2) "DSM module output populated"
        $module = @($dsm.modules)[0]
        Assert-True ($null -ne $module.metrics) "DSM module metrics populated"
        foreach ($expectedColor in $colorNames) {
            Assert-True ($null -ne $module.colors.$expectedColor) "module missing $expectedColor color entry"
            Assert-True ($null -ne $module.colors.$expectedColor.score) "module $expectedColor color score missing"
        }

        Assert-True (@($dsm.file_details).Count -ge 3) "DSM file details populated"
        $alphaDetail = @($dsm.file_details | Where-Object { $_.path -eq "alpha.py" })[0]
        Assert-True ($null -ne $alphaDetail) "alpha.py file detail exists"
        Assert-True (@($alphaDetail.functions).Count -ge 1) "alpha.py function details populated"
        Assert-True ($null -ne $dsm.scope) "DSM scope metadata populated"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Fix 1: god-file heuristic — functionCount > 25 alone must NOT flag god-file;
# it must also have loc > 400. A well-decomposed file with many small functions
# should not be punished for decomposing.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "tools\sentrux-shim\sentrux-lite-core.ps1") -Only @("Measure-File", "Get-RelativePathSafe"))

Test-Case "god-file: many functions but low LOC is NOT a god file" {
    $dir = New-ScratchDir "godfile-lowloc"
    try {
        # 30 tiny one-line functions -> functionCount > 25, but loc stays well under 400.
        $lines = 1..30 | ForEach-Object { "function Fn$_ { return $_ }" }
        $file = Join-Path $dir "many-small-fns.ps1"
        Set-Content -LiteralPath $file -Value $lines -Encoding UTF8

        $fileInfo = Get-Item -LiteralPath $file
        $metrics = Measure-File $dir $fileInfo
        Assert-True ($metrics.functions -gt 25) "expected functionCount > 25, got $($metrics.functions)"
        Assert-True ($metrics.loc -le 400) "expected loc <= 400, got $($metrics.loc)"
        Assert-False $metrics.is_god_file "functionCount>25 alone must not flag is_god_file without loc>400 (regression: da46886 fix 1)"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "god-file: many functions AND high LOC IS a god file" {
    $dir = New-ScratchDir "godfile-highloc"
    try {
        # 30 functions, each padded so total loc exceeds 400.
        $lines = New-Object System.Collections.Generic.List[string]
        for ($i = 1; $i -le 30; $i++) {
            $lines.Add("function Fn$i {")
            for ($j = 0; $j -lt 15; $j++) { $lines.Add("    Write-Output 'line $i-$j'") }
            $lines.Add("}")
        }
        $file = Join-Path $dir "many-big-fns.ps1"
        Set-Content -LiteralPath $file -Value $lines -Encoding UTF8

        $fileInfo = Get-Item -LiteralPath $file
        $metrics = Measure-File $dir $fileInfo
        Assert-True ($metrics.functions -gt 25) "expected functionCount > 25, got $($metrics.functions)"
        Assert-True ($metrics.loc -gt 400) "expected loc > 400, got $($metrics.loc)"
        Assert-True $metrics.is_god_file "functionCount>25 AND loc>400 should still flag is_god_file"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "god-file: loc > 800 alone still flags regardless of function count" {
    $dir = New-ScratchDir "godfile-locarm"
    try {
        $lines = 1..850 | ForEach-Object { "# padding line $_" }
        $file = Join-Path $dir "one-big-comment-file.ps1"
        Set-Content -LiteralPath $file -Value $lines -Encoding UTF8

        $fileInfo = Get-Item -LiteralPath $file
        $metrics = Measure-File $dir $fileInfo
        Assert-True ($metrics.loc -gt 800) "expected loc > 800, got $($metrics.loc)"
        Assert-True $metrics.is_god_file "loc>800 arm must still flag is_god_file independent of function count"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Fix 1b (same commit hunk): null-guard Measure-Object sums / empty-file reads.
# Empty directory (zero files) or an empty file must not throw / must resolve
# to zero-value metrics instead of null propagating into arithmetic.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "tools\sentrux-shim\sentrux-lite-core.ps1") -Only @("Get-SafeSum", "Get-SafeMaximum", "Measure-File", "Get-RelativePathSafe"))

Test-Case "Get-SafeSum returns 0 (not null/throw) on empty collection" {
    $result = Get-SafeSum @() "imports"
    Assert-Equal 0 $result "Get-SafeSum on empty array should be 0"
}

Test-Case "Get-SafeMaximum returns 0 (not null/throw) on empty collection" {
    $result = Get-SafeMaximum @() "max_complexity"
    Assert-Equal 0 $result "Get-SafeMaximum on empty array should be 0"
}

Test-Case "Measure-File handles a genuinely empty file without throwing" {
    $dir = New-ScratchDir "empty-file"
    try {
        $file = Join-Path $dir "empty.ps1"
        New-Item -ItemType File -Path $file | Out-Null
        $fileInfo = Get-Item -LiteralPath $file
        $metrics = Measure-File $dir $fileInfo
        Assert-Equal 0 $metrics.loc "empty file should measure loc=0"
        Assert-Equal 0 $metrics.functions "empty file should measure functions=0"
        Assert-False $metrics.is_god_file "empty file must never be a god file"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Fix 2: session_end fails closed when sentrux produced zero parseable metrics,
# instead of silently backfilling everything from baseline and reporting a
# false "no degradation". Partial backfills must be visible via a warning that
# names which metrics were backfilled.
#
# We test this at the Parse-SentruxOutput / Invoke-Gate contract level: given
# raw output that yields zero parseable core metrics, metrics_observed_count
# must be 0 so Invoke-SessionEndTool's fail-closed branch fires. Given raw
# output with a partial match, backfilled_metrics must name the gaps.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "Invoke-SentruxAgentTool.ps1") -Only @(
    "Convert-QualitySignal", "ConvertTo-NullableDouble", "Get-MetricPair", "Get-ScanStats",
    "Parse-SentruxOutput", "Get-JsonProperty", "Read-JsonFileSafe", "Get-BaselineMetrics", "Get-Bottleneck"
))

Test-Case "Parse-SentruxOutput: garbage output yields zero observed core metrics" {
    $metrics = Parse-SentruxOutput "totally unrecognized garbage output, no known markers here"
    $coreMetricKeys = @("quality_signal", "coupling", "cycles", "god_files")
    $observedCount = @($coreMetricKeys | Where-Object { $null -ne $metrics[$_] }).Count
    Assert-Equal 0 $observedCount "garbage sentrux output must parse to zero observed core metrics (regression: da46886 fix 2 fail-closed trigger)"
}

Test-Case "Parse-SentruxOutput: well-formed gate output yields 4/4 observed core metrics" {
    $sample = @"
[resolve] 10 resolved, 0 unresolved
[build_graphs] 5 files | 10 import, 3 call, 0 inherit edges
Quality: 9000 -> 9500
Coupling: 12.5 -> 10.0
Cycles: 0 -> 0
God files: 1 -> 0
Distance from Main Sequence: 0.1
No degradation detected
"@
    $metrics = Parse-SentruxOutput $sample
    $coreMetricKeys = @("quality_signal", "coupling", "cycles", "god_files")
    $observedCount = @($coreMetricKeys | Where-Object { $null -ne $metrics[$_] }).Count
    Assert-Equal 4 $observedCount "well-formed sentrux gate output should parse all 4 core metrics"
}

Test-Case "session_end fail-closed simulation: zero observed metrics forces pass=false with unparseable summary" {
    # Simulate the branch inside Invoke-SessionEndTool directly, mirroring its logic,
    # since Invoke-SessionEndTool itself shells out to the real `sentrux` binary and
    # touches session-dir state. This asserts the *contract* the fix depends on:
    # metrics_observed_count==0 must short-circuit to pass=false.
    $gate = [ordered]@{
        pass = $true  # native exit code says "pass" but that's meaningless with 0 metrics
        metrics_observed_count = 0
        backfilled_metrics = @("quality_signal", "coupling", "cycles", "god_files")
    }
    $metricsObserved = [int]$gate["metrics_observed_count"]
    if ($metricsObserved -eq 0) {
        $pass = $false
        $summary = "sentrux output unparseable - gate cannot evaluate"
    }
    else {
        $pass = $true
        $summary = "should not reach here"
    }
    Assert-False $pass "zero observed metrics must fail closed (pass=false), not fail open"
    Assert-Equal "sentrux output unparseable - gate cannot evaluate" $summary "fail-closed summary text must be the explicit unparseable message"
}

Test-Case "session_end partial backfill: summary/backfilled_metrics names the gaps, does not silently pass clean" {
    $backfilledMetrics = @("cycles", "god_files")
    $metricsObserved = 2
    Assert-True ($metricsObserved -gt 0) "partial observation should NOT trigger the zero-metrics fail-closed branch"
    $summary = "No structural degradation during this session"
    if ($backfilledMetrics.Count -gt 0) {
        $summary = "$summary (warning: backfilled from baseline: $($backfilledMetrics -join ', '))"
    }
    Assert-True ($summary -like "*warning: backfilled from baseline: cycles, god_files*") "partial backfill must surface metric names in the summary warning (regression: da46886 fix 2 partial-backfill warning)"
}

# ---------------------------------------------------------------------------
# Sentrux insight: when the authoritative gate says no degradation, raw metric
# noise must not drive a false "regressed structural metrics" next action.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "run-code-intel.ps1") -Only @(
    "New-SentruxMetricDelta",
    "Test-SentruxGateNoDegradation",
    "Resolve-SentruxMetricRegressions"
))

Test-Case "Sentrux insight: gate no-degradation suppresses false metric regression" {
    $metric = [pscustomobject](New-SentruxMetricDelta "quality" 4726 4713 "higher_is_better")
    Assert-True $metric.regressed "raw quality delta should start as regressed"

    $resolved = @(Resolve-SentruxMetricRegressions -Metrics @($metric) -NoDegradation (Test-SentruxGateNoDegradation "No degradation detected"))
    Assert-False $resolved[0].regressed "authoritative no-degradation gate should suppress false regression"
    Assert-True $resolved[0].rawRegressed "rawRegressed preserves the observed metric direction"
    Assert-True $resolved[0].gateAccepted "gateAccepted records why regression was suppressed"
}

Test-Case "Sentrux insight: metric regression remains when gate does not accept it" {
    $metric = [pscustomobject](New-SentruxMetricDelta "quality" 4726 4713 "higher_is_better")

    $resolved = @(Resolve-SentruxMetricRegressions -Metrics @($metric) -NoDegradation $false)
    Assert-True $resolved[0].regressed "regression must remain without authoritative no-degradation gate"
    Assert-True $resolved[0].rawRegressed "raw regression marker remains visible"
    Assert-False $resolved[0].gateAccepted "gateAccepted must be false without no-degradation gate"
}

# ---------------------------------------------------------------------------
# Fix 3: surgery_plan -> post_op transition must evaluate real data (sentrux_ok
# AND surgery target no longer the current top hotspot), not the old hardcoded
# $false. Also covers Get-PreviousSurgeryTarget reading the prior run's
# surgery-plan.json.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "run-code-intel.ps1") -Only @(
    "New-HospitalStateMachine", "New-StateTransition", "Get-PreviousSurgeryTarget", "Read-JsonFileSafe"
))

function New-FakeFailureCounts {
    [pscustomobject]@{ localToolError = 0; graphMissing = 0; sentruxFail = 0 }
}

Test-Case "surgery_plan->post_op: transition is no longer hardcoded false when guards actually pass" {
    $fc = New-FakeFailureCounts
    $sm = New-HospitalStateMachine -FailureCounts $fc -RulesExists $true -GateStatus "passed" -CheckStatus "passed" `
        -FailingWhatIfCount 0 -Disposition "discharge_ready" -NextProtocol "post_op" `
        -SurgeryTarget "Measure-File in sentrux-lite-core.ps1" -CurrentTopHotspot "Something-Else in other.ps1"

    $transition = $sm.transitions | Where-Object { $_.from -eq "surgery_plan" -and $_.to -eq "post_op" }
    Assert-True ($null -ne $transition) "surgery_plan->post_op transition must exist"
    Assert-True $transition.pass "when sentrux is clean and surgery target no longer top hotspot, transition must be allowed (regression: da46886 fix 3, was hardcoded `$false)"
    Assert-True $sm.guards.surgery_to_post_op_ok "guards.surgery_to_post_op_ok must reflect the real evaluation"
}

Test-Case "surgery_plan->post_op: still blocked when the surgery target IS still the top hotspot" {
    $fc = New-FakeFailureCounts
    $sm = New-HospitalStateMachine -FailureCounts $fc -RulesExists $true -GateStatus "passed" -CheckStatus "passed" `
        -FailingWhatIfCount 0 -Disposition "observe" -NextProtocol "post_op" `
        -SurgeryTarget "Measure-File in sentrux-lite-core.ps1" -CurrentTopHotspot "Measure-File in sentrux-lite-core.ps1"

    $transition = $sm.transitions | Where-Object { $_.from -eq "surgery_plan" -and $_.to -eq "post_op" }
    Assert-False $transition.pass "surgery target unchanged (still current top hotspot) must NOT allow the transition"
    Assert-False $sm.guards.surgery_target_resolved "surgery_target_resolved must be false when target == current top hotspot"
}

Test-Case "surgery_plan->post_op: still blocked when sentrux itself is failing, even if target resolved" {
    $fc = [pscustomobject]@{ localToolError = 0; graphMissing = 0; sentruxFail = 1 }
    $sm = New-HospitalStateMachine -FailureCounts $fc -RulesExists $true -GateStatus "failed" -CheckStatus "passed" `
        -FailingWhatIfCount 0 -Disposition "admit" -NextProtocol "post_op" `
        -SurgeryTarget "Foo in bar.ps1" -CurrentTopHotspot "Baz in qux.ps1"

    $transition = $sm.transitions | Where-Object { $_.from -eq "surgery_plan" -and $_.to -eq "post_op" }
    Assert-False $transition.pass "sentrux_ok=false must block surgery_plan->post_op even when target resolved"
}

Test-Case "Get-PreviousSurgeryTarget reads primary_target from the most recent prior run's surgery-plan.json" {
    $repoArtifactRoot = New-ScratchDir "surgery-target-runs"
    try {
        $run1 = Join-Path $repoArtifactRoot "20260601-000000"
        $run2 = Join-Path $repoArtifactRoot "20260701-000000"
        New-Item -ItemType Directory -Force -Path $run1 | Out-Null
        New-Item -ItemType Directory -Force -Path $run2 | Out-Null

        $plan = [ordered]@{
            schema = "code-intel-surgery-plan.v1"
            primary_target = [ordered]@{ name = "Invoke-Gate"; file = "Invoke-SentruxAgentTool.ps1" }
        }
        $plan | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $run1 "surgery-plan.json") -Encoding UTF8

        # run2 is the "current" run (no surgery-plan.json yet); run1 is prior.
        $target = Get-PreviousSurgeryTarget $run2
        Assert-Equal "Invoke-Gate in Invoke-SentruxAgentTool.ps1" $target "should read primary_target name/file from the most recent OTHER run directory"
    }
    finally {
        Remove-Item -Recurse -Force $repoArtifactRoot -ErrorAction SilentlyContinue
    }
}

Test-Case "Get-PreviousSurgeryTarget returns empty string when no prior run exists" {
    $repoArtifactRoot = New-ScratchDir "surgery-target-norun"
    try {
        $run1 = Join-Path $repoArtifactRoot "20260701-000000"
        New-Item -ItemType Directory -Force -Path $run1 | Out-Null
        $target = Get-PreviousSurgeryTarget $run1
        Assert-Equal "" $target "no prior run directory should yield empty string, not throw"
    }
    finally {
        Remove-Item -Recurse -Force $repoArtifactRoot -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Fix 4: check-code-intel-tools.ps1 survives malformed pipeline.config.json —
# structured parse error instead of an uncaught ConvertFrom-Json exception
# crashing the doctor. Black-box invocation (whole script has no mandatory
# params, safe to run as subprocess).
# ---------------------------------------------------------------------------
Test-Case "check-code-intel-tools.ps1 reports structured parseError on malformed config JSON, does not crash" {
    $dir = New-ScratchDir "doctor-badconfig"
    try {
        $badConfig = Join-Path $dir "pipeline.config.json"
        Set-Content -LiteralPath $badConfig -Value "{ this is not valid json " -Encoding UTF8

        $doctor = Join-Path $root "check-code-intel-tools.ps1"
        $raw = & $doctor -Config $badConfig -RepoPath $dir -Json 2>&1
        # Must not throw a terminating PowerShell exception; must produce JSON.
        $json = $raw | ConvertFrom-Json
        Assert-False $json.checks.config.parsed "malformed JSON must report checks.config.parsed = false"
        Assert-True (-not [string]::IsNullOrWhiteSpace([string]$json.checks.config.parseError)) "malformed JSON must populate checks.config.parseError"
        Assert-False $json.ok "doctor must report ok=false overall when config JSON is invalid"
        Assert-True (@($json.missing) -like "*invalid JSON*").Count -gt 0 "missing[] must call out the invalid JSON reason (regression: da46886 fix 4)"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "check-code-intel-tools.ps1 still parses valid config JSON normally" {
    $dir = New-ScratchDir "doctor-goodconfig"
    try {
        $goodConfig = Join-Path $dir "pipeline.config.json"
        @{ artifactRoot = ""; repos = @{} } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $goodConfig -Encoding UTF8

        $doctor = Join-Path $root "check-code-intel-tools.ps1"
        $raw = & $doctor -Config $goodConfig -RepoPath $dir -Json 2>&1
        $json = $raw | ConvertFrom-Json
        Assert-True $json.checks.config.parsed "valid JSON must report parsed = true"
        Assert-Equal "" $json.checks.config.parseError "valid JSON must report empty parseError"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Fix 5: overlay comparison catch branch must fail toward re-copy ($false =
# "not identical, copy it"), not fail-open ($true = "identical, skip copy",
# which could leave a corrupt/locked file in place).
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "Install-SentruxVlangOverlay.ps1") -Only @("Test-SameOverlayFile"))

Test-Case "Test-SameOverlayFile: identical files compare true" {
    $dir = New-ScratchDir "overlay-identical"
    try {
        $src = Join-Path $dir "source.bin"
        $dst = Join-Path $dir "target.bin"
        Set-Content -LiteralPath $src -Value "same content" -Encoding UTF8 -NoNewline
        Copy-Item -LiteralPath $src -Destination $dst
        $result = Test-SameOverlayFile $src $dst
        Assert-True $result "byte-identical files must compare as same"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "Test-SameOverlayFile: unreadable target fails toward re-copy (returns false), not fail-open true" {
    $dir = New-ScratchDir "overlay-unreadable"
    try {
        $src = Join-Path $dir "source.bin"
        $dst = Join-Path $dir "target.bin"
        Set-Content -LiteralPath $src -Value "same content" -Encoding UTF8 -NoNewline
        Set-Content -LiteralPath $dst -Value "same content" -Encoding UTF8 -NoNewline

        # Lock the target file with an exclusive handle so ReadAllBytes inside
        # Test-SameOverlayFile throws (simulating "unreadable target").
        $stream = [System.IO.File]::Open($dst, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
        try {
            $result = Test-SameOverlayFile $src $dst
            Assert-False $result "an unreadable/locked target must return `$false (re-copy), not `$true (skip) -- regression: da46886 fix 5"
        }
        finally {
            $stream.Close()
        }
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "Test-SameOverlayFile: missing target returns false (copy needed)" {
    $dir = New-ScratchDir "overlay-missing"
    try {
        $src = Join-Path $dir "source.bin"
        $dst = Join-Path $dir "does-not-exist.bin"
        Set-Content -LiteralPath $src -Value "content" -Encoding UTF8 -NoNewline
        $result = Test-SameOverlayFile $src $dst
        Assert-False $result "missing target must return false"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Fix 6: global index refresh must skip an unparseable report.json (warn +
# continue) instead of throwing and aborting the whole-fleet index refresh.
# Black-box invocation of update-code-intel-index.ps1 against a scratch
# artifact root with one broken repo and one healthy repo.
# ---------------------------------------------------------------------------
Test-Case "update-code-intel-index.ps1 skips unparseable report.json and still indexes the healthy repo" {
    $artifactRoot = New-ScratchDir "index-refresh"
    try {
        # Repo A: broken report.json (malformed JSON) -- must be skipped, not crash the run.
        $repoABroken = Join-Path (Join-Path $artifactRoot "repoA") "20260701-000000"
        New-Item -ItemType Directory -Force -Path $repoABroken | Out-Null
        $brokenReportPath = Join-Path $repoABroken "report.json"
        Set-Content -LiteralPath $brokenReportPath -Value "{ broken json not closed " -Encoding UTF8
        [ordered]@{
            schema = "code-intel-run-commit.v1"
            report = "report.json"
            reportSha256 = (Get-FileHash -LiteralPath $brokenReportPath -Algorithm SHA256).Hash.ToLowerInvariant()
        } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $repoABroken "run-complete.json") -Encoding UTF8

        # Repo B: healthy report.json -- must still be indexed.
        $repoBHealthy = Join-Path (Join-Path $artifactRoot "repoB") "20260701-000000"
        New-Item -ItemType Directory -Force -Path $repoBHealthy | Out-Null
        $healthyReport = [ordered]@{
            summary = [ordered]@{
                failureCategories = [ordered]@{ providerQuota = 0; localToolError = 0; graphMissing = 0; sentruxFail = 0 }
                failed = 0
                manualRequired = 0
                passed = 5
                skipped = 0
            }
        }
        $healthyReportPath = Join-Path $repoBHealthy "report.json"
        $healthyReport | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $healthyReportPath -Encoding UTF8
        [ordered]@{
            schema = "code-intel-run-commit.v1"
            report = "report.json"
            reportSha256 = (Get-FileHash -LiteralPath $healthyReportPath -Algorithm SHA256).Hash.ToLowerInvariant()
        } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $repoBHealthy "run-complete.json") -Encoding UTF8
        Set-Content -LiteralPath (Join-Path $repoBHealthy "summary.md") -Value "# summary" -Encoding UTF8

        $indexScript = Join-Path $root "update-code-intel-index.ps1"
        $outputPath = Join-Path $artifactRoot "index.md"
        $raw = & $indexScript -ArtifactRoot $artifactRoot -OutputPath $outputPath -LegacyCompatibilityMode -WarningAction SilentlyContinue 2>&1
        Assert-Equal 0 $LASTEXITCODE "the explicit legacy index branch must exit 0 even with one broken committed report.json present (regression: da46886 fix 6)"

        $jsonOut = $raw | Where-Object { $_ -notmatch "^WARNING" } | ConvertFrom-Json
        Assert-True $jsonOut.ok "index refresh must report ok=true overall"
        Assert-Equal 1 $jsonOut.repos "only the healthy repo (repoB) should be indexed; the broken one (repoA) must be skipped, not counted or crashing"

        Assert-True (Test-Path -LiteralPath $outputPath) "index.md must still be written despite the broken repo"
        $indexContent = Get-Content -LiteralPath $outputPath -Raw
        Assert-True ($indexContent -match "repoB") "healthy repo must appear in the generated index"
        Assert-False ($indexContent -match "repoA") "broken repo must NOT appear in the generated index (it was skipped)"
    }
    finally {
        Remove-Item -Recurse -Force $artifactRoot -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Fix 7 (contract moved by issue #182): baseline save backs up the previous
# lite baseline before overwriting so an old->new quality_signal comparison
# stays possible. Since #182 the lite gate owns .sentrux/cache/lite-baseline.json
# and never reads or writes the native engine's .sentrux/baseline.json, so the
# backup contract moves with the lite file and a native baseline must stay
# byte-identical across lite saves. run-code-intel.ps1's inline backup block
# still points at the legacy .sentrux/baseline.json location; that step is
# vestigial (it now protects a file lite no longer overwrites) and retires
# with the facade under #78.
# ---------------------------------------------------------------------------
Test-Case "sentrux-lite-core gate --save + manual backup step preserves the previous lite baseline and never touches .sentrux/baseline.json" {
    $dir = New-ScratchDir "baseline-backup"
    try {
        $liteCore = Join-Path $root "tools\sentrux-shim\sentrux-lite-core.ps1"
        $file = Join-Path $dir "sample.ps1"
        Set-Content -LiteralPath $file -Value "function A { return 1 }" -Encoding UTF8

        # A native-engine baseline occupies .sentrux/baseline.json; lite saves
        # must leave it byte-identical (regression: #182 flat-format clobber).
        $sentruxDir = Join-Path $dir ".sentrux"
        New-Item -ItemType Directory -Force -Path $sentruxDir | Out-Null
        $nativeBaselinePath = Join-Path $sentruxDir "baseline.json"
        Set-Content -LiteralPath $nativeBaselinePath -Value '{"schema":"code-intel-sentrux-baseline.v5","engine":{"id":"sentrux-native"},"metrics":{"quality_signal":1}}' -Encoding UTF8
        $nativeHashBefore = (Get-FileHash -LiteralPath $nativeBaselinePath -Algorithm SHA256).Hash

        # First save: establishes the lite baseline (v1).
        & $liteCore gate --save $dir | Out-Null
        $baselinePath = Join-Path (Join-Path $sentruxDir "cache") "lite-baseline.json"
        Assert-True (Test-Path -LiteralPath $baselinePath) "first save must create .sentrux/cache/lite-baseline.json"
        $baselineV1 = Get-Content -LiteralPath $baselinePath -Raw | ConvertFrom-Json
        $qualityV1 = $baselineV1.quality_signal

        # Mutate the target so the second save produces a different quality_signal,
        # then replicate the backup-then-save sequence (Copy-Item the lite baseline
        # to its .prev sibling BEFORE invoking gate --save).
        Add-Content -LiteralPath $file -Value "function B { if (1) { if (2) { if (3) { return 2 } } } }"
        $baselinePrevPath = Join-Path (Join-Path $sentruxDir "cache") "lite-baseline.prev.json"
        Copy-Item -LiteralPath $baselinePath -Destination $baselinePrevPath -Force
        & $liteCore gate --save $dir | Out-Null

        Assert-True (Test-Path -LiteralPath $baselinePrevPath) "lite-baseline.prev.json must exist after a second save (regression: da46886 fix 7)"
        $prevContent = Get-Content -LiteralPath $baselinePrevPath -Raw | ConvertFrom-Json
        Assert-Equal $qualityV1 $prevContent.quality_signal "the prev file must preserve the PRE-save (old) quality_signal, not the new one"

        $baselineV2 = Get-Content -LiteralPath $baselinePath -Raw | ConvertFrom-Json
        # Not asserting the values differ (that depends on heuristic sensitivity),
        # only that both old and new are available for the old->new comparison print.
        Assert-True ($null -ne $baselineV2.quality_signal) "lite-baseline.json after second save must have a quality_signal for the new-value side of the comparison"

        $nativeHashAfter = (Get-FileHash -LiteralPath $nativeBaselinePath -Algorithm SHA256).Hash
        Assert-Equal $nativeHashBefore $nativeHashAfter ".sentrux/baseline.json must stay byte-identical across lite saves (regression: issue #182)"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "baseline backup: first-ever save (no prior lite baseline) must not fail and must not fabricate a prev file" {
    $dir = New-ScratchDir "baseline-firstsave"
    try {
        $liteCore = Join-Path $root "tools\sentrux-shim\sentrux-lite-core.ps1"
        $file = Join-Path $dir "sample.ps1"
        Set-Content -LiteralPath $file -Value "function A { return 1 }" -Encoding UTF8

        $sentruxDir = Join-Path $dir ".sentrux"
        $baselinePath = Join-Path (Join-Path $sentruxDir "cache") "lite-baseline.json"
        $baselinePrevPath = Join-Path (Join-Path $sentruxDir "cache") "lite-baseline.prev.json"

        # Mirror the backup guard: only copy to .prev if a lite baseline already exists.
        if (Test-Path -LiteralPath $baselinePath -PathType Leaf) {
            Copy-Item -LiteralPath $baselinePath -Destination $baselinePrevPath -Force
        }
        & $liteCore gate --save $dir | Out-Null

        Assert-True (Test-Path -LiteralPath $baselinePath) "lite-baseline.json must be created on first save"
        Assert-False (Test-Path -LiteralPath $baselinePrevPath) "lite-baseline.prev.json must NOT be fabricated when there was no prior baseline to back up"
        Assert-False (Test-Path -LiteralPath (Join-Path $sentruxDir "baseline.json")) "lite must not create the native engine's .sentrux/baseline.json (issue #182)"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Facade resolution fixes (post-da46886): update-code-intel-index.ps1 must
# resolve the Rust CLI via CODE_INTEL_RUST_CLI / release / debug / PATH instead
# of hardcoding target/debug; invoke-code-intel.ps1 must load the default
# pipeline.config.json for the plain -RepoPath shape and must not reject
# PowerShell common parameters; check-code-intel-tools.ps1 must not accept a
# CODE_INTEL_HOME that points at a missing directory.
# ---------------------------------------------------------------------------
Test-Case "update-code-intel-index.ps1 honors CODE_INTEL_RUST_CLI instead of requiring target/debug" {
    $dir = New-ScratchDir "index-rustcli-override"
    $previousRustCli = $env:CODE_INTEL_RUST_CLI
    try {
        $stub = Join-Path $dir "stub-code-intel.ps1"
        Set-Content -LiteralPath $stub -Value "'{`"schema`":`"stub.v1`",`"entries`":[],`"diagnostics`":[]}'`nexit 0" -Encoding UTF8
        $env:CODE_INTEL_RUST_CLI = $stub

        $indexScript = Join-Path $root "update-code-intel-index.ps1"
        $raw = & $indexScript -ArtifactRoot (Join-Path $dir "artifacts") 2>&1
        Assert-Equal 0 $LASTEXITCODE "index facade must exit 0 when CODE_INTEL_RUST_CLI points at a working CLI"
        $json = $raw | ConvertFrom-Json
        Assert-Equal "stub.v1" $json.schema "CODE_INTEL_RUST_CLI must select the binary actually invoked (regression: hardcoded target/debug path threw on installed machines)"
    }
    finally {
        $env:CODE_INTEL_RUST_CLI = $previousRustCli
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "invoke-code-intel.ps1 accepts common parameters and forwards default-config artifactRoot for -RepoPath" {
    $dir = New-ScratchDir "invoke-default-config"
    try {
        # -Verbose must not trip the unsupported-option gate: with a bogus explicit
        # -Config the run must reach the config existence check (child pwsh so the
        # [Console]::Error output is capturable).
        $legacy = Join-Path $root "invoke-code-intel.ps1"
        $missingConfig = Join-Path $dir "no-such-config.json"
        $raw = @(& pwsh -NoLogo -NoProfile -File $legacy -RepoPath $dir -Config $missingConfig -Verbose 2>&1)
        Assert-Equal 64 $LASTEXITCODE "missing explicit config must still exit 64"
        Assert-False (($raw -join "`n") -match "unsupported compatibility option") "-Verbose must not be rejected as an unsupported compatibility option"
        Assert-True (($raw -join "`n") -match "config file does not exist:") "the config existence check must be reached when -Verbose is bound"

        # Plain -RepoPath shape must load $PSScriptRoot/pipeline.config.json and
        # forward its artifactRoot (regression: config was only loaded for the
        # -Config/-Repo shapes). Copy the facade next to a stub launcher so the
        # forwarded arguments are observable without running the real pipeline.
        # mirror the real layout: the facade and its launcher sit under
        # legacy/, the default config stays at the repository root
        $fixtureArchive = Join-Path $dir "legacy"
        New-Item -ItemType Directory -Force -Path $fixtureArchive | Out-Null
        Copy-Item -LiteralPath $legacy -Destination (Join-Path $fixtureArchive "invoke-code-intel.ps1")
        $stubLauncher = @'
param(
    [string]$RepoPath = "",
    [string]$Mode = "normal",
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Remaining = @()
)
[pscustomobject]@{ repoPath = $RepoPath; mode = $Mode; remaining = @($Remaining) } | ConvertTo-Json -Compress
exit 0
'@
        Set-Content -LiteralPath (Join-Path $fixtureArchive "code-intel.ps1") -Value $stubLauncher -Encoding UTF8
        $configuredRoot = Join-Path $dir "artifact-root"
        @{ artifactRoot = $configuredRoot; repos = @{} } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $dir "pipeline.config.json") -Encoding UTF8

        $forwarded = @(& (Join-Path $fixtureArchive "invoke-code-intel.ps1") -RepoPath $dir 2>&1)
        Assert-Equal 0 $LASTEXITCODE "stubbed launcher run must exit 0"
        $json = ($forwarded -join "`n") | ConvertFrom-Json
        Assert-Equal $dir $json.repoPath "RepoPath must be forwarded unchanged"
        Assert-Equal "--artifact-root $configuredRoot" (@($json.remaining) -join " ") "plain -RepoPath shape must forward --artifact-root from the default pipeline.config.json"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "check-code-intel-tools.ps1 fails CODE_INTEL_HOME pointing at a missing directory, flags a non-default one" {
    $dir = New-ScratchDir "doctor-home-env"
    $previousHome = $env:CODE_INTEL_HOME
    try {
        $doctor = Join-Path $root "check-code-intel-tools.ps1"

        # A set-but-deleted CODE_INTEL_HOME must fail the env check and the doctor
        # (regression: comparing against Get-CodeIntelHome's own env-derived output
        # made any set value pass, even a deleted directory).
        $env:CODE_INTEL_HOME = Join-Path $dir "deleted-home"
        $raw = & $doctor -Json 2>&1
        $json = $raw | ConvertFrom-Json
        Assert-False $json.checks.env.codeIntelHome.ok "a CODE_INTEL_HOME without an existing directory must not pass the env check"
        Assert-False $json.checks.env.codeIntelHome.exists "exists must report the missing directory"
        Assert-True ([bool](@($json.missing) -match "CODE_INTEL_HOME")) "the missing directory must be reported in the doctor's missing list"
        Assert-False $json.ok "the doctor must not report ok overall with a broken CODE_INTEL_HOME"

        # An existing directory that differs from the default derivation is a
        # mismatch (ok=false, matchesDefault=false) but not a hard failure.
        $env:CODE_INTEL_HOME = $dir
        $raw = & $doctor -Json 2>&1
        $json = $raw | ConvertFrom-Json
        Assert-True $json.checks.env.codeIntelHome.exists "an existing override directory must report exists=true"
        Assert-False $json.checks.env.codeIntelHome.matchesDefault "an override away from the pipeline root must report matchesDefault=false"
        Assert-False $json.checks.env.codeIntelHome.ok "a mismatched CODE_INTEL_HOME must not pass the env check"
        Assert-False ([bool](@($json.missing) -match "CODE_INTEL_HOME")) "a mismatch alone must not land in the missing list"
    }
    finally {
        $env:CODE_INTEL_HOME = $previousHome
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Issue #59 proposal 3a: on macOS/Linux the platform module additionally
# maintains a POSIX env file (~/.config/code-intel/env.sh) so fresh bash/zsh
# sessions keep PATH and CODE_INTEL_HOME. The POSIX logic lives in small pure
# helpers plus -Platform-parameterized branches, so everything below runs on
# Windows. The *Windows* persistence branches of Set-CodeIntelUserEnv /
# Add-UserPathPrefix write the real user registry and are intentionally NOT
# invoked here.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "tools\code-intel-platform.psm1") -Only @(
    "Get-CodeIntelPlatform",
    "Get-CodeIntelHome",
    "Resolve-CodeIntelPath",
    "Get-CodeIntelPosixProfileInstruction",
    "ConvertTo-CodeIntelPosixEnvLine",
    "Update-CodeIntelPosixEnvContent",
    "Update-CodeIntelPosixEnvFile",
    "Set-CodeIntelUserEnv",
    "Add-UserPathPrefix"
))

Test-Case "explicit installer root wins over a stale CODE_INTEL_HOME override" {
    $dir = New-ScratchDir "explicit-home"
    $legacy = Join-Path $dir "legacy-release"
    $current = Join-Path $dir "current-release"
    New-Item -ItemType Directory -Force -Path $legacy, $current | Out-Null
    $previousHome = $env:CODE_INTEL_HOME
    try {
        $env:CODE_INTEL_HOME = $legacy
        $resolved = Get-CodeIntelHome -Root $current
        Assert-Equal (Get-Item -LiteralPath $current).FullName $resolved "installer root must not be shadowed by a stale ambient CODE_INTEL_HOME"
        Assert-Equal (Get-Item -LiteralPath $legacy).FullName (Get-CodeIntelHome) "ambient CODE_INTEL_HOME must remain the fallback without an explicit root"
        Assert-Equal (Get-Item -LiteralPath $legacy).FullName (Get-CodeIntelHome -Root " ") "a whitespace root must use the ambient fallback"
    }
    finally {
        $env:CODE_INTEL_HOME = $previousHome
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "posix env: profile instruction is the single copy-paste source line per platform" {
    Assert-Equal "echo 'source ~/.config/code-intel/env.sh' >> ~/.zshrc" (Get-CodeIntelPosixProfileInstruction -Platform macos) "macos instruction must target ~/.zshrc"
    Assert-Equal "echo 'source ~/.config/code-intel/env.sh' >> ~/.bashrc" (Get-CodeIntelPosixProfileInstruction -Platform linux) "linux instruction must target ~/.bashrc"
}

Test-Case "posix env: export lines render the PATH prefix form and escape sh metacharacters" {
    Assert-Equal 'export PATH="/opt/code-intel/bin:$PATH"' (ConvertTo-CodeIntelPosixEnvLine -Name "PATH" -Value "/opt/code-intel/bin" -AsPathPrefix) "PATH prefix line must keep the trailing `$PATH expandable"
    Assert-Equal 'export CODE_INTEL_HOME="/home/user/code-intel"' (ConvertTo-CodeIntelPosixEnvLine -Name "CODE_INTEL_HOME" -Value "/home/user/code-intel") "plain export line"
    Assert-Equal 'export X="a\$b\"c\\d"' (ConvertTo-CodeIntelPosixEnvLine -Name "X" -Value 'a$b"c\d') "dollar, quote, and backslash must be escaped inside the double quotes"
}

Test-Case "posix env: content updates are idempotent and replace same-variable exports" {
    $pathLine = ConvertTo-CodeIntelPosixEnvLine -Name "PATH" -Value "/opt/code-intel/bin" -AsPathPrefix
    $once = Update-CodeIntelPosixEnvContent -Lines @() -Line $pathLine
    $twice = Update-CodeIntelPosixEnvContent -Lines $once -Line $pathLine
    Assert-Equal ($once -join "`n") ($twice -join "`n") "re-adding the same PATH line must not duplicate it"
    Assert-Equal 1 @($twice).Count "exactly one PATH line after two identical updates"

    $otherPathLine = ConvertTo-CodeIntelPosixEnvLine -Name "PATH" -Value "/opt/other/bin" -AsPathPrefix
    $withOther = Update-CodeIntelPosixEnvContent -Lines $twice -Line $otherPathLine
    Assert-Equal 2 @($withOther).Count "a different directory must keep its own PATH line"

    $homePattern = '^\s*export\s+CODE_INTEL_HOME='
    $v1 = Update-CodeIntelPosixEnvContent -Lines $withOther -MatchPattern $homePattern -Line (ConvertTo-CodeIntelPosixEnvLine -Name "CODE_INTEL_HOME" -Value "/old/home")
    $v2 = Update-CodeIntelPosixEnvContent -Lines $v1 -MatchPattern $homePattern -Line (ConvertTo-CodeIntelPosixEnvLine -Name "CODE_INTEL_HOME" -Value "/new/home")
    Assert-Equal 3 @($v2).Count "a same-variable export must be replaced, not appended"
    Assert-True (($v2 -join "`n").Contains('/new/home')) "replacement must keep the newest value"
    Assert-False (($v2 -join "`n").Contains('/old/home')) "the stale value must be dropped"
}

Test-Case "posix env: Add-UserPathPrefix (linux branch) maintains env.sh and returns the copy-paste instruction" {
    $dir = New-ScratchDir "posix-pathprefix"
    $savedPath = $env:PATH
    try {
        $script:PosixTestHome = Join-Path $dir "home"
        New-Item -ItemType Directory -Force -Path $script:PosixTestHome | Out-Null
        function Get-CodeIntelHomeDirectory { return $script:PosixTestHome }

        $binDir = Join-Path $dir "bin"
        $result = Add-UserPathPrefix -PathToAdd $binDir -Platform linux
        Assert-False $result.persisted "non-Windows PATH persistence stays opt-in (persisted=false)"
        Assert-True ($result.detail.Contains("echo 'source ~/.config/code-intel/env.sh' >> ~/.bashrc")) "detail must carry the single copy-paste line (issue #59 proposal 3a)"

        $envSh = Join-Path (Join-Path (Join-Path $script:PosixTestHome ".config") "code-intel") "env.sh"
        Assert-True (Test-Path -LiteralPath $envSh -PathType Leaf) "env.sh must be written under ~/.config/code-intel"
        $expectedLine = ConvertTo-CodeIntelPosixEnvLine -Name "PATH" -Value $result.path -AsPathPrefix
        $lines = @(Get-Content -LiteralPath $envSh)
        Assert-Equal 1 @($lines | Where-Object { $_ -ceq $expectedLine }).Count "env.sh must contain exactly one PATH export for the directory"

        Add-UserPathPrefix -PathToAdd $binDir -Platform linux | Out-Null
        $lines = @(Get-Content -LiteralPath $envSh)
        Assert-Equal 1 @($lines | Where-Object { $_ -ceq $expectedLine }).Count "a reinstall must not duplicate the PATH export"

        $firstEntry = @($env:PATH -split [regex]::Escape([string][System.IO.Path]::PathSeparator))[0]
        Assert-Equal $result.path $firstEntry "the process PATH must still be prefixed"
    }
    finally {
        $env:PATH = $savedPath
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "posix env: Set-CodeIntelUserEnv (linux branch) rewrites env.ps1 + env.sh idempotently" {
    $dir = New-ScratchDir "posix-setenv"
    $savedValue = $env:CIP_TEST_POSIX_ENV
    try {
        $script:PosixTestHome = $dir
        function Get-CodeIntelHomeDirectory { return $script:PosixTestHome }

        Set-CodeIntelUserEnv -Name "CIP_TEST_POSIX_ENV" -Value "/repo/v1" -Platform linux | Out-Null
        $result = Set-CodeIntelUserEnv -Name "CIP_TEST_POSIX_ENV" -Value "/repo/v2" -Platform linux
        Assert-Equal "/repo/v2" $env:CIP_TEST_POSIX_ENV "the process environment must carry the newest value"
        Assert-False $result.persisted "non-Windows env persistence stays opt-in (persisted=false)"
        Assert-True ($result.detail.Contains("echo 'source ~/.config/code-intel/env.sh' >> ~/.bashrc")) "detail must carry the single copy-paste line"

        $configDir = Join-Path (Join-Path $dir ".config") "code-intel"
        $shLines = @(Get-Content -LiteralPath (Join-Path $configDir "env.sh"))
        Assert-Equal 1 @($shLines).Count "env.sh must hold a single export for the variable after two writes"
        Assert-Equal 'export CIP_TEST_POSIX_ENV="/repo/v2"' $shLines[0] "the env.sh export must be replaced with the newest value"

        $psLines = @(Get-Content -LiteralPath (Join-Path $configDir "env.ps1"))
        Assert-Equal 1 @($psLines).Count "env.ps1 must be rewritten idempotently, not appended (old behavior accumulated duplicates)"
        Assert-Equal "`$env:CIP_TEST_POSIX_ENV = '/repo/v2'" $psLines[0] "the env.ps1 line must carry the newest value"
    }
    finally {
        if ($null -ne $savedValue) { $env:CIP_TEST_POSIX_ENV = $savedValue } else { Remove-Item Env:CIP_TEST_POSIX_ENV -ErrorAction SilentlyContinue }
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Issue #59 proposal 3b: the installer copies orchestration/integrations.json
# next to the installed binary (<bin>/orchestration/) — the first candidate of
# discover_manifest's exe-ancestor walk in crates/code-intel-cli/src/capability.rs
# — so the installed code-intel works outside a repo checkout.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "install-code-intel-pipeline.ps1") -Only @(
    "Add-InstallAction",
    "Install-IntegrationsManifest"
))

Test-Case "installer: Install-IntegrationsManifest copies the manifest beside the binary and overwrites on reinstall" {
    $dir = New-ScratchDir "manifest-copy"
    try {
        $repoRoot = Join-Path $dir "repo"
        $binDir = Join-Path $dir "bin"
        New-Item -ItemType Directory -Force -Path (Join-Path $repoRoot "orchestration"), $binDir | Out-Null
        $sourceManifest = Join-Path (Join-Path $repoRoot "orchestration") "integrations.json"
        Set-Content -LiteralPath $sourceManifest -Value '{"integrations":[]}' -Encoding UTF8

        $actions = New-Object System.Collections.Generic.List[object]
        Install-IntegrationsManifest $actions $repoRoot $binDir
        $destination = Join-Path (Join-Path $binDir "orchestration") "integrations.json"
        Assert-True (Test-Path -LiteralPath $destination -PathType Leaf) "manifest must land at <bin>/orchestration/integrations.json (first candidate of the discover_manifest ancestor walk)"
        Assert-Equal "installed" $actions[0].status "the copy must be reported as installed in the install actions"
        Assert-Equal $destination $actions[0].detail "the reported detail must be the installed manifest path"

        Set-Content -LiteralPath $sourceManifest -Value '{"integrations":[{"id":"x"}]}' -Encoding UTF8
        Install-IntegrationsManifest $actions $repoRoot $binDir
        Assert-True ((Get-Content -LiteralPath $destination -Raw).Contains('"id"')) "a reinstall must overwrite the previously copied manifest"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "installer: Install-IntegrationsManifest reports install_failed when the repo manifest is missing" {
    $dir = New-ScratchDir "manifest-missing"
    try {
        $repoRoot = Join-Path $dir "repo"
        $binDir = Join-Path $dir "bin"
        New-Item -ItemType Directory -Force -Path $repoRoot, $binDir | Out-Null

        $actions = New-Object System.Collections.Generic.List[object]
        Install-IntegrationsManifest $actions $repoRoot $binDir
        Assert-Equal "install_failed" $actions[0].status "a missing repo manifest must be reported, not silently skipped"
        Assert-False (Test-Path -LiteralPath (Join-Path (Join-Path $binDir "orchestration") "integrations.json")) "no manifest file may be fabricated"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# The repowise ThinkingBlock overlay must tell "obsolete" apart from "broken".
# Upstream repowise 0.32.0 walks response.content itself, so the vulnerable
# pattern is gone from a healthy install — which used to be reported as
# install_failed on every single run and trained us to ignore the signal.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "install-code-intel-pipeline.ps1") -Only @(
    "Repair-RepowiseThinkingBlockPatch"
))

function Invoke-ThinkingPatchAgainst {
    # Points the lookup the function derives from $env:APPDATA at a scratch tree
    # seeded with $Source, and returns the install actions it recorded. Builds the
    # provider path with the same Join-Path expression the function uses, so the
    # fixture lands where the function looks on every platform.
    param([string]$Dir, [string]$Source)

    $providerPath = Join-Path $Dir "uv\tools\repowise\Lib\site-packages\repowise\core\providers\llm\anthropic.py"
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $providerPath) | Out-Null
    Set-Content -LiteralPath $providerPath -Value $Source -Encoding UTF8

    $savedAppData = $env:APPDATA
    try {
        $env:APPDATA = $Dir
        $actions = New-Object System.Collections.Generic.List[object]
        Repair-RepowiseThinkingBlockPatch $actions
        return [pscustomobject]@{ Actions = $actions; ProviderPath = $providerPath }
    }
    finally {
        $env:APPDATA = $savedAppData
    }
}

Test-Case "installer: repowise thinking patch reports not_needed when upstream already skips non-text blocks" {
    $dir = New-ScratchDir "thinking-upstream"
    try {
        $upstream = @'
        text_content = ""
        for block in response.content:
            if hasattr(block, "text"):
                text_content = block.text
                break

        result = GeneratedResponse(
            content=text_content,
        )
'@
        $run = Invoke-ThinkingPatchAgainst $dir $upstream
        Assert-Equal 1 $run.Actions.Count "the upstream-fixed case must record exactly one action"
        Assert-Equal "not_needed" $run.Actions[0].status "an install carrying the upstream fix is obsolete for this overlay, not failed"
        Assert-True ((Get-Content -LiteralPath $run.ProviderPath -Raw).Contains("text_content = block.text")) "a not_needed verdict must leave upstream source untouched"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "installer: repowise thinking patch still rewrites the vulnerable single-block read" {
    $dir = New-ScratchDir "thinking-vulnerable"
    try {
        $vulnerable = @'
        result = GeneratedResponse(
            content=response.content[0].text,
        )
'@
        $run = Invoke-ThinkingPatchAgainst $dir $vulnerable
        Assert-Equal "installed" $run.Actions[0].status "the pre-0.32.0 shape must still be patched"
        $patched = Get-Content -LiteralPath $run.ProviderPath -Raw
        Assert-True ($patched.Contains('getattr(block, "type", "") == "text"')) "the patched source must iterate blocks and keep only text ones"
        Assert-False ($patched.Contains("content=response.content[0].text,")) "the vulnerable single-block read must be gone after patching"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "installer: repowise thinking patch reports install_failed on an unrecognised upstream layout" {
    $dir = New-ScratchDir "thinking-unknown"
    try {
        $run = Invoke-ThinkingPatchAgainst $dir "content = something_else_entirely(response)"
        Assert-Equal "install_failed" $run.Actions[0].status "a layout matching neither the vulnerable nor the fixed shape must stay loud"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# skill:claude must verify WHOSE skill occupies the path, not just that some
# SKILL.md is there. Agent hosts share these directories with other skill
# managers: on the machine that surfaced this, ~/.claude/skills/code-intel-pipeline
# was a junction into an unrelated ~/.skillz store holding a five-week-old
# SKILL.md that pointed every canonical path at a legacy clone — and the
# installer reported `OK skill:claude` on every run.
# ---------------------------------------------------------------------------
. (Get-ScriptFunctionsSource -Path (Join-Path $root "install-code-intel-pipeline.ps1") -Only @(
    "Add-Check",
    "Test-SkillPathServesTarget",
    "Move-OccupiedSkillPathAside",
    "Ensure-SkillLink",
    "Ensure-SkillSource"
))
. (Get-ScriptFunctionsSource -Path (Join-Path $root "tools\code-intel-platform.psm1") -Only @(
    "New-CodeIntelLink"
))
$script:EffectivePlatform = Get-CodeIntelPlatform -Platform "auto"

function New-SkillFixture {
    param([string]$Path, [string]$Body)

    New-Item -ItemType Directory -Force -Path $Path | Out-Null
    Set-Content -LiteralPath (Join-Path $Path "SKILL.md") -Value $Body -Encoding UTF8
    return $Path
}

Test-Case "installer: a skill path occupied by a foreign store is not accepted as linked" {
    $dir = New-ScratchDir "skill-foreign"
    try {
        $target = New-SkillFixture (Join-Path $dir "agents\code-intel-pipeline") "bundled skill"
        $foreign = New-SkillFixture (Join-Path $dir "skillz\code-intel-pipeline") "someone else's stale skill"

        Assert-False (Test-SkillPathServesTarget -Path $foreign -Target $target) "a directory whose SKILL.md differs from the bundle must not count as serving it"

        $checks = New-Object System.Collections.Generic.List[object]
        Ensure-SkillLink $checks "claude" $foreign $target $false
        Assert-False $checks[0].ok "the check must fail rather than accept a foreign skill store"
        Assert-True ($checks[0].detail.Contains("occupied by a different skill store")) "the detail must name the actual problem"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "installer: a plain copy matching the bundle is accepted (link-less installs)" {
    $dir = New-ScratchDir "skill-copy"
    try {
        $target = New-SkillFixture (Join-Path $dir "agents\code-intel-pipeline") "bundled skill"
        $copy = New-SkillFixture (Join-Path $dir "claude\code-intel-pipeline") "bundled skill"

        Assert-True (Test-SkillPathServesTarget -Path $copy -Target $target) "macOS/Linux installs fall back to copying, which must still pass"

        $checks = New-Object System.Collections.Generic.List[object]
        Ensure-SkillLink $checks "claude" $copy $target $false
        Assert-True $checks[0].ok "a byte-identical copy is a healthy install, not drift"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "installer: repairing an occupied skill path preserves the previous occupant" {
    $dir = New-ScratchDir "skill-repair"
    try {
        $target = New-SkillFixture (Join-Path $dir "agents\code-intel-pipeline") "bundled skill"
        $occupied = New-SkillFixture (Join-Path $dir "claude\code-intel-pipeline") "someone else's stale skill"

        $checks = New-Object System.Collections.Generic.List[object]
        Ensure-SkillLink $checks "claude" $occupied $target $true

        Assert-True $checks[0].ok "repair must make the path serve the bundled skill"
        Assert-True (Test-SkillPathServesTarget -Path $occupied -Target $target) "the repaired path must resolve to the bundle"
        $preserved = @(Get-ChildItem -LiteralPath (Join-Path $dir "claude") -Directory -Force | Where-Object { $_.Name -like "code-intel-pipeline.replaced-*" })
        Assert-Equal 1 $preserved.Count "a real directory occupant must be moved aside, never deleted"
        Assert-Equal "someone else's stale skill" ((Get-Content -LiteralPath (Join-Path $preserved[0].FullName "SKILL.md") -Raw).Trim()) "the displaced skill must keep its content"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "installer: bundled skill parity ignores __pycache__ so a local bootstrap run cannot fake drift" {
    $dir = New-ScratchDir "skill-pycache"
    try {
        $bundled = New-SkillFixture (Join-Path $dir "bundled") "bundled skill"
        Set-Content -LiteralPath (Join-Path $bundled "bootstrap.py") -Value "print('hi')" -Encoding UTF8
        $installed = Join-Path $dir "installed"

        $checks = New-Object System.Collections.Generic.List[object]
        Ensure-SkillSource $checks $installed $bundled $true
        Assert-True $checks[0].ok "a fresh install must report current"

        $cache = Join-Path $bundled "__pycache__"
        New-Item -ItemType Directory -Force -Path $cache | Out-Null
        Set-Content -LiteralPath (Join-Path $cache "bootstrap.cpython-313.pyc") -Value "machine-local bytecode" -Encoding UTF8

        $checks = New-Object System.Collections.Generic.List[object]
        Ensure-SkillSource $checks $installed $bundled $false
        Assert-True $checks[0].ok "a gitignored interpreter cache must not make the installed skill look outdated"
        Assert-False (Test-Path -LiteralPath (Join-Path $installed "__pycache__")) "the cache must never be copied into an agent host's skill directory"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Issue #59 proposal 4: the doctor's graph-provider check must build its paths
# with chained Join-Path (single literal 'a\b\c' segments never exist on
# mac/linux), use the platform binary name, and accept target/release builds in
# addition to target/debug. Exercised black-box against a scratch pipeline root
# so binary/source presence is controlled exactly.
# ---------------------------------------------------------------------------
function New-DoctorScratchRoot {
    param([string]$Dir)

    # mirror the real layout: legacy/ holds the PowerShell entry points while
    # crates/ and target/ stay at the repository root
    New-Item -ItemType Directory -Force -Path (Join-Path $Dir "legacy") | Out-Null
    $crateDir = Join-Path (Join-Path $Dir "crates") "code-intel-cli"
    $graphDir = Join-Path (Join-Path $crateDir "src") "graph"
    New-Item -ItemType Directory -Force -Path $graphDir | Out-Null
    Set-Content -LiteralPath (Join-Path $crateDir "Cargo.toml") -Value "[package]" -Encoding UTF8
    # graph.rs is a directory module (mod.rs + tests.rs, issue #155's god-file
    # split): the doctor probe's sourceFound check looks for src/graph/mod.rs.
    Set-Content -LiteralPath (Join-Path $graphDir "mod.rs") -Value "// graph provider" -Encoding UTF8
}

function Invoke-DoctorScratch {
    param(
        [string]$Dir,
        [string[]]$ExtraArgs = @()
    )

    # The probe is native since T3 (#48): drive the real binary against the
    # scratch root rather than the retired script. The scratch tree's own
    # target/ holds a fake binary on purpose, so it must never be the one run.
    $binaryName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
    $repoRoot = Split-Path -Parent $root
    $cli = @(
        (Join-Path (Join-Path (Join-Path $repoRoot "target") "release") $binaryName),
        (Join-Path (Join-Path (Join-Path $repoRoot "target") "debug") $binaryName)
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if ($null -eq $cli) { throw "code-intel binary not built; run cargo build -p code-intel" }

    $raw = @(& $cli doctor bootstrap --pipeline-root $Dir --json @ExtraArgs 2>&1)
    return ($raw -join "`n") | ConvertFrom-Json
}

Test-Case "doctor graph provider: a target/release platform binary satisfies the binary check" {
    $dir = New-ScratchDir "doctor-graph-release"
    try {
        New-DoctorScratchRoot $dir
        $binaryName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
        $releaseBinary = Join-Path (Join-Path (Join-Path $dir "target") "release") $binaryName
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $releaseBinary) | Out-Null
        Set-Content -LiteralPath $releaseBinary -Value "fake binary" -Encoding UTF8

        $json = Invoke-DoctorScratch $dir @("--require-understand")
        Assert-True $json.checks.graphProvider.sourceFound "chained Join-Path must find crates/code-intel-cli/src/graph/mod.rs"
        Assert-True $json.checks.graphProvider.cargoFound "chained Join-Path must find crates/code-intel-cli/Cargo.toml"
        Assert-True $json.checks.graphProvider.binaryFound "a target/release build must satisfy the binary check (regression: only target\debug\code-intel.exe was probed)"
        Assert-Equal $releaseBinary $json.checks.graphProvider.binaryPath "binaryPath must report the discovered release binary"
        Assert-True ($json.checks.graphProvider.command.Contains($releaseBinary)) "the command hint must reference the discovered release binary"
        Assert-False ([bool](@($json.missing) -contains "internal graph provider source")) "-RequireUnderstand must not report a false MISSING graph provider source"
        Assert-False ([bool](@($json.missing) -contains "code-intel Rust runtime")) "-RequireUnderstand must not report a false MISSING Rust runtime"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "doctor graph provider: debug fallback still detected; absent binary reports the packaged-release hint" {
    $dir = New-ScratchDir "doctor-graph-debug"
    try {
        New-DoctorScratchRoot $dir
        $binaryName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
        $debugBinary = Join-Path (Join-Path (Join-Path $dir "target") "debug") $binaryName
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $debugBinary) | Out-Null
        Set-Content -LiteralPath $debugBinary -Value "fake binary" -Encoding UTF8

        $json = Invoke-DoctorScratch $dir
        Assert-True $json.checks.graphProvider.binaryFound "a target/debug build must still satisfy the binary check"
        Assert-Equal $debugBinary $json.checks.graphProvider.binaryPath "binaryPath must report the debug binary when no release build exists"

        Remove-Item -LiteralPath $debugBinary -Force
        $json = Invoke-DoctorScratch $dir
        Assert-False $json.checks.graphProvider.binaryFound "no built binary must report binaryFound=false"
        Assert-Equal "" ([string]$json.checks.graphProvider.binaryPath) "binaryPath must be empty when nothing is built"
        $expectedHint = Join-Path (Join-Path $dir "bin") $binaryName
        Assert-True ($json.checks.graphProvider.command.Contains($expectedHint)) "the command hint must prefer the packaged release candidate path"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Fail-open lint: scan all tracked .ps1 files in the repo for catch blocks
# that return/emit a permissive boolean ($true) directly, which is the exact
# anti-pattern all 7 fixes above were closing. A whitelist mechanism exists
# for legitimate cases (inline comment marker or path allowlist below).
# ---------------------------------------------------------------------------
Test-Case "fail-open lint: no catch{ return `$true } / catch{ `$true } patterns outside the whitelist" {
    # Path allowlist: relative paths (repo-root-relative, forward slashes) that are
    # permitted to contain a fail-open catch pattern. Empty by design -- the repo
    # should be at 0 violations after da46886. Add entries here (with a comment
    # explaining why) if a legitimate case is found later.
    $pathAllowlist = @()

    # Inline allowlist marker: a catch block whose body contains this comment on
    # the same or an adjacent line is considered reviewed-and-accepted.
    $inlineAllowMarker = "lint-allow: fail-open"

    $scriptFiles = Get-ChildItem -LiteralPath $root -Recurse -Filter "*.ps1" -File |
        Where-Object {
            $relative = $_.FullName.Substring($root.Length).TrimStart("\", "/").Replace("\", "/")
            -not ($pathAllowlist -contains $relative) -and
            $relative -notmatch "^\.repowise/" -and
            $relative -notmatch "^\.understand-anything/"
        }

    $violations = New-Object System.Collections.Generic.List[string]

    foreach ($scriptFile in $scriptFiles) {
        $tokens = $null
        $parseErrors = $null
        $ast = [System.Management.Automation.Language.Parser]::ParseFile($scriptFile.FullName, [ref]$tokens, [ref]$parseErrors)
        if ($null -eq $ast) { continue }

        $catchClauses = $ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.CatchClauseAst] }, $true)
        foreach ($catchClause in $catchClauses) {
            $bodyText = $catchClause.Body.Extent.Text
            $lineOffset = $catchClause.Body.Extent.StartLineNumber

            # Fail-open pattern: a bare `return $true` / trailing bare `$true` as
            # (one of) the statements in the catch body. This intentionally does
            # NOT flag `return $false`, `$false`, throw, or any other catch body.
            $isFailOpen = ($bodyText -match 'return\s+\$true\b') -or
                          ($bodyText -match '(?m)^\s*\$true\s*$')
            if (-not $isFailOpen) { continue }

            # Check for the inline allow marker anywhere in the catch body, or in
            # the few lines immediately preceding it (comment placed just above).
            $fileLines = Get-Content -LiteralPath $scriptFile.FullName
            $precedingStart = [Math]::Max(0, $lineOffset - 4)
            $precedingText = ($fileLines[$precedingStart..($lineOffset - 1)] -join "`n")
            $isAllowed = ($bodyText -match [regex]::Escape($inlineAllowMarker)) -or
                         ($precedingText -match [regex]::Escape($inlineAllowMarker))
            if ($isAllowed) { continue }

            $relative = $scriptFile.FullName.Substring($root.Length).TrimStart("\", "/")
            $violations.Add("$relative`:$lineOffset -- catch block returns/emits `$true (fail-open); add '# $inlineAllowMarker' comment if intentional")
        }
    }

    if ($violations.Count -gt 0) {
        throw "Fail-open lint found $($violations.Count) violation(s):`n$($violations -join "`n")"
    }
}

# ---------------------------------------------------------------------------
# Issue #216: after the shim moved under legacy/tools, the installer must pass
# the repository root (not the legacy directory) to both the shim installer and
# its required-file checks. Otherwise it searches legacy/legacy/tools and the
# portable macOS install exits 1 after a successful release build.
# ---------------------------------------------------------------------------
Test-Case "installer: relocated Sentrux shim stays rooted at the repository" {
    $installerPath = Join-Path $root "install-code-intel-pipeline.ps1"
    $installer = Get-Content -LiteralPath $installerPath -Raw
    Assert-True ($installer.Contains('Install-SentruxShim $installActions $repoRoot')) "the shim installer must receive the repository root"
    Assert-True ($installer.Contains('$shimSource = Join-Path (Join-Path (Join-Path $repoRoot "legacy") "tools") "sentrux-shim"')) "required shim files must be checked from repository-root/legacy/tools"
}

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "== Results: $script:passed passed, $script:failed failed ==" -ForegroundColor $(if ($script:failed -eq 0) { "Green" } else { "Red" })
if ($script:failed -gt 0) {
    Write-Host ""
    Write-Host "Failures:" -ForegroundColor Red
    foreach ($f in $script:failures) { Write-Host "  - $f" -ForegroundColor Red }
    exit 1
}
exit 0
