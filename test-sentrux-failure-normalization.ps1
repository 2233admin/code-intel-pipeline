Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$runner = Join-Path $root "run-code-intel.ps1"
$runnerText = Get-Content -LiteralPath $runner -Raw
$marker = "`$configData = `$null"
$prefixLength = $runnerText.IndexOf($marker)
if ($prefixLength -lt 0) {
    throw "Could not find runner execution marker."
}
Invoke-Expression $runnerText.Substring(0, $prefixLength)

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
    if ([string]$worsened.gate.target.status -ne "aggregate") { throw "Gate regression should be aggregate." }
    if ([int]$worsenedDebt.summary.worsenedDebt -lt 1) { throw "Gate increase should be worsened debt." }
    if ([int]$worsenedDebt.summary.blocking -lt 1) { throw "Worsened debt should block." }

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
