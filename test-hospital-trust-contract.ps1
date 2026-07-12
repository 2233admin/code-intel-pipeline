param(
    [switch]$VerboseOutput
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$script:passed = 0
$script:failed = 0
$script:failures = [System.Collections.Generic.List[string]]::new()

function Test-Case {
    param([string]$Name, [scriptblock]$Body)

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

function Get-ScriptFunctionsSource {
    param([string]$Path, [string[]]$Only)

    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors -and $parseErrors.Count -gt 0) {
        throw "Failed to parse ${Path}: $($parseErrors[0].Message)"
    }

    $functions = @($ast.FindAll({
                param($node)
                $node -is [System.Management.Automation.Language.FunctionDefinitionAst]
            }, $true) | Where-Object { $Only -contains $_.Name })
    if ($functions.Count -ne $Only.Count) {
        $found = @($functions | ForEach-Object Name)
        throw "Function extraction incomplete. Requested: $($Only -join ', '); found: $($found -join ', ')"
    }

    return [scriptblock]::Create(($functions | ForEach-Object { $_.Extent.Text }) -join "`n`n")
}

. (Get-ScriptFunctionsSource -Path (Join-Path $root "run-code-intel.ps1") -Only @(
        "Read-JsonFileSafe",
        "Get-StepScore",
        "Get-FailureCount",
        "Get-FirstLine",
        "New-Modality",
        "New-StateTransition",
        "New-HospitalStateMachine",
        "Get-HospitalDiagnosis",
        "Get-HospitalNextProtocol",
        "Get-HospitalAdmissionReason",
        "Get-HospitalTreatmentPlan",
        "New-HospitalDecisionBlock",
        "New-HospitalMeasurements",
        "Get-ImportResolutionScore",
        "Get-SourceCoverageScore",
        "New-HospitalScoreBlock",
        "Read-HospitalArtifactFile",
        "Read-HospitalArtifacts"
    ))

function New-CleanFailureCounts {
    return [pscustomobject]@{
        localToolError = 0
        graphMissing = 0
        sentruxFail = 0
        providerQuota = 0
    }
}

function New-Step {
    param([string]$Status = "passed")
    return [pscustomobject]@{ status = $Status; output = $Status }
}

function New-ScoreBlockForMeasurements {
    param(
        [object]$Measurements,
        [object]$Artifacts
    )

    $insight = @{
        rulesExists = $true
        rulesPath = ".sentrux/rules.toml"
        gateStatus = "passed"
        checkStatus = "passed"
    }
    $passedStep = New-Step
    if ($null -eq $Artifacts) {
        $Artifacts = [ordered]@{
            dsm = $null
            file_details = $null
            evolution = $null
            what_if = $null
            codenexus = $null
        }
    }
    return New-HospitalScoreBlock `
        -SentruxInsight $insight `
        -Measurements $Measurements `
        -UnderstandStep $passedStep `
        -RepowiseStep $passedStep `
        -SentruxCheckStep $passedStep `
        -SentruxGateStep $passedStep `
        -SentruxDsmObject $Artifacts.dsm `
        -SentruxFileDetailsObject $Artifacts.file_details `
        -SentruxEvolutionObject $Artifacts.evolution `
        -SentruxWhatIfObject $Artifacts.what_if `
        -CodeNexusContextObject $Artifacts.codenexus
}

function New-ArtifactSummary {
    param(
        [string]$Directory,
        [string]$Name,
        [string]$Json = '{"evidence":true}'
    )

    $path = Join-Path $Directory "$Name.json"
    Set-Content -LiteralPath $path -Value $Json -Encoding utf8
    return [pscustomobject]@{ path = $path }
}

Write-Host "== Hospital trust contract ==" -ForegroundColor Cyan

Test-Case "gate failure cannot produce green or discharge_ready" {
    $decision = New-HospitalDecisionBlock `
        -FailureCounts (New-CleanFailureCounts) `
        -RulesExists $true `
        -GateStatus "failed" `
        -CheckStatus "passed" `
        -FailingWhatIfCount 0 `
        -UnderstandCommand "ua refresh" `
        -TopContextFile "" `
        -StructuralEvidenceComplete $true `
        -SurgeryTarget "old hotspot" `
        -CurrentTopHotspot "new hotspot" `
        -GitHubResearch $null

    Assert-False ($decision.severity -eq "green") "a failed gate is incomplete/negative evidence"
    Assert-Equal "admit" $decision.disposition "failed gate must block discharge"
}

Test-Case "unknown check cannot produce green or discharge_ready" {
    $decision = New-HospitalDecisionBlock `
        -FailureCounts (New-CleanFailureCounts) `
        -RulesExists $true `
        -GateStatus "passed" `
        -CheckStatus "unknown" `
        -FailingWhatIfCount 0 `
        -UnderstandCommand "ua refresh" `
        -TopContextFile "" `
        -StructuralEvidenceComplete $true `
        -SurgeryTarget "old hotspot" `
        -CurrentTopHotspot "new hotspot" `
        -GitHubResearch $null

    Assert-False ($decision.severity -eq "green") "unknown check evidence must remain non-green"
    Assert-Equal "admit" $decision.disposition "unknown check must block discharge"
}

Test-Case "clean gate and check without affirmative post-op evidence stays green observation" {
    $decision = New-HospitalDecisionBlock `
        -FailureCounts (New-CleanFailureCounts) `
        -RulesExists $true `
        -GateStatus "passed" `
        -CheckStatus "passed" `
        -FailingWhatIfCount 0 `
        -UnderstandCommand "ua refresh" `
        -TopContextFile "" `
        -StructuralEvidenceComplete $true `
        -GitHubResearch $null

    Assert-Equal "green" $decision.severity "fully known clean evidence should remain green"
    Assert-Equal "observe" $decision.disposition "clean structural evidence without post-op proof requires observation"
    Assert-Equal "post_op" $decision.stateMachine.current_state "observation must stay in post-op"
}

Test-Case "discharge requires affirmative resolved post-op target evidence" {
    $decision = New-HospitalDecisionBlock `
        -FailureCounts (New-CleanFailureCounts) `
        -RulesExists $true `
        -GateStatus "passed" `
        -CheckStatus "passed" `
        -FailingWhatIfCount 0 `
        -UnderstandCommand "ua refresh" `
        -TopContextFile "" `
        -StructuralEvidenceComplete $true `
        -SurgeryTarget "old hotspot" `
        -CurrentTopHotspot "new hotspot" `
        -GitHubResearch $null

    Assert-Equal "green" $decision.severity "resolved post-op evidence remains green"
    Assert-Equal "discharge_ready" $decision.disposition "two known different targets prove post-op resolution"
}

Test-Case "missing structural summaries block green and route diagnosis" {
    $decision = New-HospitalDecisionBlock `
        -FailureCounts (New-CleanFailureCounts) `
        -RulesExists $true `
        -GateStatus "passed" `
        -CheckStatus "passed" `
        -FailingWhatIfCount 0 `
        -UnderstandCommand "ua refresh" `
        -TopContextFile "" `
        -StructuralEvidenceComplete $false `
        -SurgeryTarget "old hotspot" `
        -CurrentTopHotspot "new hotspot" `
        -GitHubResearch $null

    Assert-Equal "amber" $decision.severity "missing structural evidence is unknown, not green"
    Assert-Equal "admit" $decision.disposition "missing what-if and related summaries cannot become zero debt"
    Assert-Equal "diagnose" $decision.nextProtocol "missing structural evidence must route to diagnosis"
    Assert-False $decision.stateMachine.guards.structural_evidence_complete "state machine must expose the missing evidence guard"
}

Test-Case "provider quota failure blocks green and discharge" {
    $counts = New-CleanFailureCounts
    $counts.providerQuota = 1
    $decision = New-HospitalDecisionBlock `
        -FailureCounts $counts `
        -RulesExists $true `
        -GateStatus "passed" `
        -CheckStatus "passed" `
        -FailingWhatIfCount 0 `
        -UnderstandCommand "ua refresh" `
        -TopContextFile "" `
        -StructuralEvidenceComplete $true `
        -SurgeryTarget "old hotspot" `
        -CurrentTopHotspot "new hotspot" `
        -GitHubResearch $null

    Assert-False ($decision.severity -eq "green") "provider quota means required evidence may be incomplete"
    Assert-Equal "admit" $decision.disposition "provider quota must block discharge"
    Assert-Equal "triage" $decision.nextProtocol "provider availability failure routes to triage"
    Assert-True ($decision.admissionReason -like "*quota*") "admission reason should name provider quota"
}

Test-Case "missing surgery target is unresolved" {
    $machine = New-HospitalStateMachine `
        -FailureCounts (New-CleanFailureCounts) `
        -RulesExists $true `
        -GateStatus "passed" `
        -CheckStatus "passed" `
        -FailingWhatIfCount 0 `
        -Disposition "admit" `
        -NextProtocol "post_op" `
        -SurgeryTarget "" `
        -CurrentTopHotspot "other hotspot"

    Assert-False $machine.guards.surgery_target_resolved "absence of the target is unknown, not resolved"
    Assert-False $machine.guards.surgery_to_post_op_ok "unknown target must block post-op transition"
}

Test-Case "missing current hotspot is unresolved" {
    $machine = New-HospitalStateMachine `
        -FailureCounts (New-CleanFailureCounts) `
        -RulesExists $true `
        -GateStatus "passed" `
        -CheckStatus "passed" `
        -FailingWhatIfCount 0 `
        -Disposition "admit" `
        -NextProtocol "post_op" `
        -SurgeryTarget "old hotspot" `
        -CurrentTopHotspot ""

    Assert-False $machine.guards.surgery_target_resolved "absence of current evidence is unknown, not resolved"
    Assert-False $machine.guards.surgery_to_post_op_ok "unknown current hotspot must block post-op transition"
}

Test-Case "known changed hotspot retains resolved behavior" {
    $machine = New-HospitalStateMachine `
        -FailureCounts (New-CleanFailureCounts) `
        -RulesExists $true `
        -GateStatus "passed" `
        -CheckStatus "passed" `
        -FailingWhatIfCount 0 `
        -Disposition "admit" `
        -NextProtocol "post_op" `
        -SurgeryTarget "old hotspot" `
        -CurrentTopHotspot "new hotspot"

    Assert-True $machine.guards.surgery_target_resolved "two known, different targets prove resolution"
    Assert-True $machine.guards.surgery_to_post_op_ok "known resolution plus clean sentrux should pass"
}

Test-Case "unknown import, pollution, and source coverage are explicit zero-confidence evidence" {
    $measurements = New-HospitalMeasurements `
        -InventoryStep ([pscustomobject]@{ output = "inventory unavailable" }) `
        -SentruxInsight $null `
        -DsmObject ([pscustomobject]@{ scope = [pscustomobject]@{} })
    $scores = New-ScoreBlockForMeasurements $measurements

    Assert-Equal 0 $scores.resolution_score "unknown import resolution must not receive neutral credit"
    Assert-Equal "unknown" $scores.import_resolution_status "unknown import evidence must be explicit"
    Assert-Equal 0 $scores.pollution_score "unknown pollution must not receive clean credit"
    Assert-Equal "unknown" $scores.pollution_status "unknown pollution evidence must be explicit"
    Assert-Equal 0 $scores.source_coverage_score "unknown source coverage must not receive credit"
    Assert-Equal "unknown" $measurements.source_scope_status "unknown source coverage must be explicit"
    Assert-Equal 0 $scores.mri_score "missing MRI evidence must score zero"
    Assert-Equal 0 $scores.ct_score "missing CT evidence must score zero"
    Assert-Equal 0 $scores.pet_score "missing PET evidence must score zero"
    Assert-Equal "missing" $scores.mri_status "missing MRI evidence needs explicit status"
    Assert-Equal "missing" $scores.ct_status "missing CT evidence needs explicit status"
    Assert-Equal "missing" $scores.pet_status "missing PET evidence needs explicit status"
}

Test-Case "known complete measurements retain pass scores" {
    $measurements = New-HospitalMeasurements `
        -InventoryStep ([pscustomobject]@{ output = "files=10" }) `
        -SentruxInsight @{ scan = @{ files = 10; resolvedImports = 8; unresolvedImports = 0 } } `
        -DsmObject ([pscustomobject]@{ scope = [pscustomobject]@{ excluded_files = 0 } })
    $scores = New-ScoreBlockForMeasurements $measurements

    Assert-Equal 100 $scores.resolution_score "known 100% import resolution should remain full credit"
    Assert-Equal "100%" $scores.import_resolution_status "known import ratio should remain visible"
    Assert-Equal 80 $scores.pollution_score "known zero exclusions should preserve the existing conservative score"
    Assert-Equal "clean" $scores.pollution_status "known zero exclusions should be distinguishable from unknown"
    Assert-Equal 100 $scores.source_coverage_score "complete known scan coverage should remain full credit"
    Assert-Equal "measured" $measurements.source_scope_status "complete source measurements should remain measured"
}

Test-Case "known negative and partial measurements retain evidence-driven scores" {
    $measurements = New-HospitalMeasurements `
        -InventoryStep ([pscustomobject]@{ output = "files=10" }) `
        -SentruxInsight @{ scan = @{ files = 5; resolvedImports = 0; unresolvedImports = 8 } } `
        -DsmObject ([pscustomobject]@{ scope = [pscustomobject]@{ excluded_files = 3 } })
    $scores = New-ScoreBlockForMeasurements $measurements

    Assert-Equal 30 $scores.resolution_score "known 0% import resolution should retain the existing failure score"
    Assert-Equal "0%" $scores.import_resolution_status "known failed import ratio should remain visible"
    Assert-Equal 100 $scores.pollution_score "known quarantined files should retain full pollution-control credit"
    Assert-Equal "quarantined" $scores.pollution_status "known exclusions should remain explicit"
    Assert-Equal 50 $scores.source_coverage_score "known partial coverage should be scored proportionally"
    Assert-Equal "measured" $measurements.source_scope_status "known partial source measurements are measured, not unknown"
}

Test-Case "manual and skipped steps are unknown and receive zero confidence" {
    Assert-Equal 0 (Get-StepScore (New-Step "manual_required")) "manual-required is not affirmative evidence"
    Assert-Equal 0 (Get-StepScore (New-Step "skipped")) "skipped is not affirmative evidence"
    Assert-Equal 100 (Get-StepScore (New-Step "passed")) "known passed behavior remains full confidence"
}

Test-Case "known modality artifacts retain established scores and explicit status" {
    $measurements = New-HospitalMeasurements `
        -InventoryStep ([pscustomobject]@{ output = "files=10" }) `
        -SentruxInsight @{ scan = @{ files = 10; resolvedImports = 8; unresolvedImports = 0 } } `
        -DsmObject ([pscustomobject]@{ scope = [pscustomobject]@{ excluded_files = 0 } })
    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("hospital-trust-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tempDir | Out-Null
    try {
        $summaries = [ordered]@{
            dsm = New-ArtifactSummary $tempDir "dsm"
            file_details = New-ArtifactSummary $tempDir "file-details"
            hotspots = New-ArtifactSummary $tempDir "hotspots"
            evolution = New-ArtifactSummary $tempDir "evolution"
            what_if = New-ArtifactSummary $tempDir "what-if"
            codenexus = New-ArtifactSummary $tempDir "codenexus"
        }
        $artifacts = Read-HospitalArtifacts `
            $summaries.dsm `
            $summaries.file_details `
            $summaries.hotspots `
            $summaries.evolution `
            $summaries.what_if `
            $summaries.codenexus
        $scores = New-ScoreBlockForMeasurements $measurements $artifacts

        Assert-Equal 100 $scores.mri_score "known MRI evidence retains full confidence"
        Assert-Equal 100 $scores.ct_score "known CT evidence retains full confidence"
        Assert-Equal 70 $scores.pet_score "known PET proxy retains its established confidence"
        Assert-Equal "available" $scores.mri_status "known MRI evidence is explicit"
        Assert-Equal "available" $scores.ct_status "known CT evidence is explicit"
        Assert-Equal "available" $scores.pet_status "known PET evidence is explicit"
    }
    finally {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Test-Case "deleted and corrupt modality artifacts remain zero-confidence" {
    $measurements = New-HospitalMeasurements `
        -InventoryStep ([pscustomobject]@{ output = "files=10" }) `
        -SentruxInsight @{ scan = @{ files = 10; resolvedImports = 8; unresolvedImports = 0 } } `
        -DsmObject ([pscustomobject]@{ scope = [pscustomobject]@{ excluded_files = 0 } })
    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("hospital-trust-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tempDir | Out-Null
    try {
        $missing = [pscustomobject]@{ path = (Join-Path $tempDir "deleted.json") }
        $corrupt = New-ArtifactSummary $tempDir "corrupt" '{not-json'
        $valid = New-ArtifactSummary $tempDir "valid"
        $artifacts = Read-HospitalArtifacts $valid $missing $valid $corrupt $valid $missing
        $scores = New-ScoreBlockForMeasurements $measurements $artifacts

        Assert-Equal 0 $scores.mri_score "deleted CodeNexus artifact must score zero"
        Assert-Equal 0 $scores.ct_score "deleted file-details artifact must make CT incomplete"
        Assert-Equal 0 $scores.pet_score "corrupt evolution artifact must make PET incomplete"
        Assert-Equal "missing" $scores.mri_status "deleted MRI evidence must be explicit"
        Assert-Equal "missing" $scores.ct_status "partial CT evidence must be explicit"
        Assert-Equal "missing" $scores.pet_status "partial PET evidence must be explicit"
    }
    finally {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Test-Case "hospital report wires structural evidence completeness into the decision" {
    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile((Join-Path $root "run-code-intel.ps1"), [ref]$tokens, [ref]$parseErrors)
    $reportFunction = $ast.FindAll({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq "New-CodeIntelHospitalReport"
        }, $true) | Select-Object -First 1
    $source = $reportFunction.Extent.Text

    Assert-True ($source -like "*Read-HospitalArtifacts `$SentruxDsmSummary `$SentruxFileDetailsSummary `$SentruxHotspotsSummary `$SentruxEvolutionSummary `$SentruxWhatIfSummary `$CodeNexusContextSummary*") "report must read every artifact path before trusting evidence"
    Assert-True ($source -like "*`$artifacts.dsm*`$artifacts.file_details*`$artifacts.hotspots*`$artifacts.evolution*`$artifacts.what_if*") "structural completeness must use all five parsed artifacts"
    Assert-True ($source -like "*-SentruxDsmObject `$artifacts.dsm*-SentruxFileDetailsObject `$artifacts.file_details*-SentruxEvolutionObject `$artifacts.evolution*-SentruxWhatIfObject `$artifacts.what_if*-CodeNexusContextObject `$artifacts.codenexus*") "modality scoring must use parsed artifacts rather than summaries"
    Assert-True ($source -like "*-StructuralEvidenceComplete `$structuralEvidenceComplete*") "report must pass completeness into the decision seam"
}

Write-Host ""
Write-Host "== Results: $script:passed passed, $script:failed failed ==" -ForegroundColor $(if ($script:failed -eq 0) { "Green" } else { "Red" })
if ($script:failed -gt 0) {
    foreach ($failure in $script:failures) { Write-Host "  - $failure" -ForegroundColor Red }
    exit 1
}

exit 0
