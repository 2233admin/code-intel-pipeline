Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))

$repoRoot = Split-Path -Parent $root
$runner = Join-Path $root "run-code-intel.ps1"
$runnerText = Get-Content -LiteralPath $runner -Raw
$marker = "`$configData = `$null"
$prefixLength = $runnerText.IndexOf($marker)
if ($prefixLength -lt 0) {
    throw "Could not find runner execution marker."
}
$runnerPrefix = $runnerText.Substring(0, $prefixLength).Replace('$PSScriptRoot', '$root')
Invoke-Expression $runnerPrefix

function New-TestStep {
    param(
        [string]$Name,
        [string]$Status,
        [string]$Output = "",
        [string]$ErrorText = ""
    )

    [pscustomobject][ordered]@{
        name = $Name
        status = $Status
        output = $Output
        error = $ErrorText
    }
}

$base = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-sentrux-normalization-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $base | Out-Null

try {
    $hotspotsPath = Join-Path $base "sentrux-hotspots.json"
    [ordered]@{
        functions = @(
            [ordered]@{
                file = "Invoke-SentruxAgentTool.ps1"
                name = "Get-ModuleBucket"
                complexity = 86
            }
        )
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $hotspotsPath -Encoding UTF8

    $named = New-CodeIntelSentruxFailures `
        -Steps @(
            (New-TestStep "sentrux check" "failed" "run-code-intel.ps1:Get-CodeEvidenceSymbols (cc=311)")
        ) `
        -HotspotsPath $hotspotsPath `
        -OutputPath (Join-Path $base "named.json")
    $namedDebt = New-CodeIntelSentruxDebtRegister -Failures $named -RepoPath $root -RunTimestamp "2026-07-02T00:00:00Z" -OutputPath (Join-Path $base "named-debt.json")
    $knownEffectiveFailures = @(Get-CodeIntelEffectiveFailedSteps `
        -FailedSteps @((New-TestStep "sentrux check" "failed" "run-code-intel.ps1:Get-CodeEvidenceSymbols (cc=311)")) `
        -BlockingSentruxDebt ([int]$namedDebt.summary.blocking))

    if ([string]$named.schema -ne "code-intel-sentrux-failures.v1") { throw "Unexpected failure schema." }
    if ([string]$named.status -ne "failed") { throw "Named offender should produce failed status." }
    if ([string]$named.primary.target.status -ne "resolved") { throw "Named offender should resolve target." }
    if ([string]$named.primary.target.file -ne "run-code-intel.ps1") { throw "Named offender file not preserved." }
    if ([string]$named.primary.target.symbol -ne "Get-CodeEvidenceSymbols") { throw "Named offender symbol not preserved." }
    if ([int]$named.primary.value -ne 311) { throw "Named offender cc value not preserved." }
    if (@($named.conflicts).Count -ne 1) { throw "Hotspot disagreement should emit one metric_conflict." }
    if ([string]$named.conflicts[0].kind -ne "metric_conflict") { throw "Conflict kind should be metric_conflict." }
    if ([string]$namedDebt.schema -ne "code-intel-sentrux-debt-register.v1") { throw "Unexpected debt schema." }
    if ([int]$namedDebt.summary.knownDebt -ne 1) { throw "Named current offender should be known debt." }
    if ([int]$namedDebt.summary.blocking -ne 0) { throw "Known debt should not block." }
    if ($knownEffectiveFailures.Count -ne 0) { throw "Known non-blocking Sentrux debt should not produce an effective failure." }
    if ((Get-Content -LiteralPath (Join-Path $base "named-debt.json") -Raw | ConvertFrom-Json).schema -ne "code-intel-sentrux-debt-register.v1") {
        throw "Written debt artifact should parse as JSON."
    }

    $aggregate = New-CodeIntelSentruxFailures -Steps @(
        (New-TestStep "sentrux check" "failed" "max_cc exceeded: threshold 70, actual 311")
    )
    $aggregateDebt = New-CodeIntelSentruxDebtRegister -Failures $aggregate -RepoPath $root
    if ([string]$aggregate.primary.target.status -ne "unresolved") { throw "Aggregate max_cc should be unresolved." }
    if ([int]$aggregateDebt.summary.informational -ne 1) { throw "Aggregate unresolved max_cc should be informational." }
    if ([int]$aggregateDebt.summary.blocking -ne 0) { throw "Aggregate unresolved max_cc should not block." }

    $worsened = New-CodeIntelSentruxFailures -Steps @(
        (New-TestStep "sentrux gate" "failed" "Complex functions increased: 7 -> 11`nComplex functions increased: 11 → 12")
    )
    $worsenedDebt = New-CodeIntelSentruxDebtRegister -Failures $worsened -RepoPath $root
    # Issue #130: a "sentrux gate" step that produced a parseable gate:* finding is gate
    # OUTPUT (architecture/quality debt reporting), not a process failure -- it must NOT
    # remain an effective failure regardless of its blocking/worsened classification. Pass
    # -Failures so Get-CodeIntelEffectiveFailedSteps can use $worsened.gate as the
    # finding-vs-crash discriminator (see the companion crash-path test below).
    $blockingEffectiveFailures = @(Get-CodeIntelEffectiveFailedSteps `
        -FailedSteps @((New-TestStep "sentrux gate" "failed" "Complex functions increased: 7 -> 11")) `
        -BlockingSentruxDebt ([int]$worsenedDebt.summary.blocking) `
        -Failures $worsened)
    if ([string]$worsened.gate.target.status -ne "aggregate") { throw "Gate regression should be aggregate." }
    if ([int]$worsenedDebt.summary.worsenedDebt -lt 1) { throw "Gate increase should be worsened debt." }
    if ([int]$worsenedDebt.summary.blocking -lt 1) { throw "Worsened debt should block." }
    if ($blockingEffectiveFailures.Count -ne 0) { throw "A sentrux gate finding (even blocking/worsened debt) must not remain an effective failure -- it is gate output, not a process failure." }

    # Crash path: a "sentrux gate" step that failed WITHOUT producing any parseable gate:*
    # record at all (stdout matched no known regression format) has $Failures.gate -eq $null
    # -- that is a genuine tool crash, not a finding, and must still count as an effective
    # failure so it is not silently swallowed alongside real gate findings.
    $gateCrash = New-CodeIntelSentruxFailures -Steps @(
        (New-TestStep "sentrux gate" "failed" "sentrux.exe: unhandled panic, no structured output produced")
    )
    $gateCrashDebt = New-CodeIntelSentruxDebtRegister -Failures $gateCrash -RepoPath $root
    $gateCrashEffectiveFailures = @(Get-CodeIntelEffectiveFailedSteps `
        -FailedSteps @((New-TestStep "sentrux gate" "failed" "sentrux.exe: unhandled panic, no structured output produced")) `
        -BlockingSentruxDebt ([int]$gateCrashDebt.summary.blocking) `
        -Failures $gateCrash)
    if ($null -ne $gateCrash.gate) { throw "Unparseable gate crash should not produce a gate record." }
    if ($gateCrashEffectiveFailures.Count -ne 1) { throw "A sentrux gate step that crashed without a parseable finding must remain an effective failure." }

    $localToolEffectiveFailures = @(Get-CodeIntelEffectiveFailedSteps `
        -FailedSteps @((New-TestStep "inventory" "failed" "" "tool failed")) `
        -BlockingSentruxDebt 0)
    if ($localToolEffectiveFailures.Count -ne 1) { throw "Non-Sentrux failures must remain effective." }

    # Backward compatibility: callers that omit -Failures entirely (old call shape) must
    # keep the pre-#130 behavior for a "sentrux gate" step -- gated purely by blocking debt.
    $legacyShapeEffectiveFailures = @(Get-CodeIntelEffectiveFailedSteps `
        -FailedSteps @((New-TestStep "sentrux gate" "failed" "Complex functions increased: 7 -> 11")) `
        -BlockingSentruxDebt ([int]$worsenedDebt.summary.blocking))
    if ($legacyShapeEffectiveFailures.Count -ne 1) { throw "Omitting -Failures must preserve the pre-#130 blocking-debt-gated behavior." }

    $mixedDirection = New-CodeIntelSentruxFailures -Steps @(
        (New-TestStep "sentrux gate" "failed" "Quality: 4726 -> 5389`nComplex functions increased: 20 -> 22")
    )
    $mixedDirectionDebt = New-CodeIntelSentruxDebtRegister -Failures $mixedDirection -RepoPath $root
    $qualityDebt = @($mixedDirectionDebt.entries | Where-Object { [string]$_.kind -eq "quality" })
    if ($qualityDebt.Count -ne 1) { throw "Mixed-direction gate output should preserve one quality record." }
    if ([string]$qualityDebt[0].classification -ne "informational") { throw "Increasing quality must not be classified as worsened debt." }
    if ([int]$mixedDirectionDebt.summary.worsenedDebt -ne 1) { throw "Only complex_functions should be worsened debt." }
    if ([int]$mixedDirectionDebt.summary.blocking -ne 1) { throw "Only the complex_functions regression should block." }

    $otherNamed = New-CodeIntelSentruxFailures -Steps @(
        (New-TestStep "sentrux check" "failed" "other.ps1:New-BigFunction (cc=101)")
    )
    $otherDebt = New-CodeIntelSentruxDebtRegister -Failures $otherNamed -RepoPath $root
    if ([int]$otherDebt.summary.newDebt -ne 1) { throw "Unknown named failure should be new debt." }
    if ([int]$otherDebt.summary.blocking -ne 1) { throw "New debt should block." }

    $manual = New-CodeIntelSentruxFailures -Steps @(
        (New-TestStep "sentrux gate" "manual_required" "Sentrux baseline missing at .sentrux/baseline.json.")
    )
    $manualDebt = New-CodeIntelSentruxDebtRegister -Failures $manual -RepoPath $root
    if ([string]$manual.status -ne "manual_required") { throw "Manual-required gate should produce manual_required status." }
    if ([int]$manualDebt.summary.informational -ne 1) { throw "Manual-required should be informational debt state." }
    if ([int]$manualDebt.summary.knownDebt -ne 0) { throw "Manual-required should not be known debt." }

    $skipped = New-CodeIntelSentruxFailures -Steps @(
        (New-TestStep "sentrux check" "skipped" "No .sentrux/rules.toml found"),
        (New-TestStep "sentrux gate" "skipped" "Skipped by -SkipSentruxGate")
    )
    $skippedDebt = New-CodeIntelSentruxDebtRegister -Failures $skipped -RepoPath $root
    if ([string]$skipped.status -ne "skipped") { throw "Skipped Sentrux steps should produce skipped status." }
    if ([int]$skippedDebt.summary.informational -ne 1) { throw "Skipped should be informational." }

    $unparsed = New-CodeIntelSentruxFailures -Steps @(
        (New-TestStep "sentrux check" "failed" "unexpected sentrux output")
    )
    $unparsedDebt = New-CodeIntelSentruxDebtRegister -Failures $unparsed -RepoPath $root
    if ([string]$unparsed.status -ne "unparsed") { throw "Malformed failed stdout should produce unparsed status." }
    if ([int]$unparsedDebt.summary.informational -ne 1) { throw "Unparsed should be informational." }

    Write-Host "Sentrux failure normalization tests passed."
}
finally {
    if (Test-Path -LiteralPath $base) {
        Remove-Item -LiteralPath $base -Recurse -Force
    }
}
