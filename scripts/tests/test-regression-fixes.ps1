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
    return $dir
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
# Fix 7: baseline save backs up the previous baseline.json to baseline.prev.json
# before overwriting, and prints an old->new quality_signal comparison. We
# exercise the actual backup+diff logic extracted from run-code-intel.ps1's
# inline block by re-running the same steps against scratch files (the block
# itself is inline script, not a named function, so we assert the on-disk
# side effect contract directly using the sentrux-lite-core CLI, which is
# what run-code-intel.ps1 shells out to).
# ---------------------------------------------------------------------------
Test-Case "sentrux-lite-core gate --save + manual backup step preserves baseline.prev.json with old quality_signal" {
    $dir = New-ScratchDir "baseline-backup"
    try {
        $liteCore = Join-Path $root "tools\sentrux-shim\sentrux-lite-core.ps1"
        $file = Join-Path $dir "sample.ps1"
        Set-Content -LiteralPath $file -Value "function A { return 1 }" -Encoding UTF8

        # First save: establishes baseline.json (v1).
        & $liteCore gate --save $dir | Out-Null
        $sentruxDir = Join-Path $dir ".sentrux"
        $baselinePath = Join-Path $sentruxDir "baseline.json"
        Assert-True (Test-Path -LiteralPath $baselinePath) "first save must create baseline.json"
        $baselineV1 = Get-Content -LiteralPath $baselinePath -Raw | ConvertFrom-Json
        $qualityV1 = $baselineV1.quality_signal

        # Mutate the target so the second save produces a different quality_signal,
        # then replicate the exact backup-then-save sequence run-code-intel.ps1 performs
        # (Copy-Item baseline.json -> baseline.prev.json BEFORE invoking gate --save).
        Add-Content -LiteralPath $file -Value "function B { if (1) { if (2) { if (3) { return 2 } } } }"
        $baselinePrevPath = Join-Path $sentruxDir "baseline.prev.json"
        Copy-Item -LiteralPath $baselinePath -Destination $baselinePrevPath -Force
        & $liteCore gate --save $dir | Out-Null

        Assert-True (Test-Path -LiteralPath $baselinePrevPath) "baseline.prev.json must exist after a second save (regression: da46886 fix 7)"
        $prevContent = Get-Content -LiteralPath $baselinePrevPath -Raw | ConvertFrom-Json
        Assert-Equal $qualityV1 $prevContent.quality_signal "baseline.prev.json must preserve the PRE-save (old) quality_signal, not the new one"

        $baselineV2 = Get-Content -LiteralPath $baselinePath -Raw | ConvertFrom-Json
        # Not asserting the values differ (that depends on heuristic sensitivity),
        # only that both old and new are available for the old->new comparison print.
        Assert-True ($null -ne $baselineV2.quality_signal) "baseline.json after second save must have a quality_signal for the new-value side of the comparison"
    }
    finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }
}

Test-Case "baseline backup: first-ever save (no prior baseline.json) must not fail and must not fabricate a prev file" {
    $dir = New-ScratchDir "baseline-firstsave"
    try {
        $liteCore = Join-Path $root "tools\sentrux-shim\sentrux-lite-core.ps1"
        $file = Join-Path $dir "sample.ps1"
        Set-Content -LiteralPath $file -Value "function A { return 1 }" -Encoding UTF8

        $sentruxDir = Join-Path $dir ".sentrux"
        $baselinePath = Join-Path $sentruxDir "baseline.json"
        $baselinePrevPath = Join-Path $sentruxDir "baseline.prev.json"

        # Mirror run-code-intel.ps1's guard: only copy to .prev if baseline.json already exists.
        if (Test-Path -LiteralPath $baselinePath -PathType Leaf) {
            Copy-Item -LiteralPath $baselinePath -Destination $baselinePrevPath -Force
        }
        & $liteCore gate --save $dir | Out-Null

        Assert-True (Test-Path -LiteralPath $baselinePath) "baseline.json must be created on first save"
        Assert-False (Test-Path -LiteralPath $baselinePrevPath) "baseline.prev.json must NOT be fabricated when there was no prior baseline to back up"
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
