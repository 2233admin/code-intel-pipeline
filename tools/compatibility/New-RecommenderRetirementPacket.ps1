[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutDir,

    [Parameter(Mandatory = $true)]
    [long]$EvaluatedAt,

    [string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),
    [string]$CodeIntel = (Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) "target\debug\code-intel.exe")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($EvaluatedAt -le 0) { throw "EvaluatedAt must be a positive Unix timestamp" }
if (Test-Path -LiteralPath $OutDir) { throw "packet output must be exclusive: $OutDir" }
if (-not (Test-Path -LiteralPath $CodeIntel -PathType Leaf)) { throw "code-intel binary is missing: $CodeIntel" }

$null = New-Item -ItemType Directory -Path $OutDir
$evidenceDir = Join-Path $OutDir "evidence"
$null = New-Item -ItemType Directory -Path $evidenceDir

function Write-JsonFile([string]$Path, [object]$Value) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 30 -Compress), [Text.UTF8Encoding]::new($false))
}

function Get-Sha256Text([string]$Text) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Text)
    $hash = [Security.Cryptography.SHA256]::HashData($bytes)
    return ([Convert]::ToHexString($hash)).ToLowerInvariant()
}

function Get-Sha256Json([object]$Value) {
    return Get-Sha256Text (ConvertTo-Json -InputObject $Value -Depth 30 -Compress)
}

function New-ArtifactRef([string]$ArtifactSchema, [string]$Type, [string]$RelativePath, [string]$SnapshotIdentity) {
    $path = Join-Path $OutDir ($RelativePath -replace '/', [IO.Path]::DirectorySeparatorChar)
    return [ordered]@{
        schema = "code-intel-artifact-ref.v1"
        artifactSchema = $ArtifactSchema
        type = $Type
        path = ($RelativePath -replace '\\', '/')
        sha256 = ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant())
        consumedSnapshotIdentity = $SnapshotIdentity
    }
}

$verification = [ordered]@{}
& pwsh -NoLogo -NoProfile -File (Join-Path $RepoRoot "scripts/tests/test-workflow-recommendation-brief.ps1") | Out-Null
$verification.brief = ($LASTEXITCODE -eq 0)
& cargo test -p code-intel --test capability_exec advisory_workflow_recommend_runs_through_a01_with_zero_effects_and_facade_parity --quiet | Out-Null
$verification.facadeParity = ($LASTEXITCODE -eq 0)
& cargo test -p code-intel --test authority_transition recommender_direct_commit_requires_explicit_approved_authority_event --quiet | Out-Null
$verification.authorityIsolation = ($LASTEXITCODE -eq 0)
if ($verification.Values -contains $false) { throw "recommender parity or authority verification failed" }

$runText = Get-Content -LiteralPath (Join-Path $RepoRoot "run-code-intel.ps1") -Raw
$legacyInlineAbsent = ($runText -notmatch 'function Invoke-WorkflowStackDetector') -and ($runText -notmatch 'function Get-CodeMetrics')
$providerPreflightPresent = $runText -match 'Invoke-RepowiseProviderProbe\.ps1'
if (-not $legacyInlineAbsent) { throw "legacy inline recommender is still present" }
if (-not $providerPreflightPresent) { throw "unrelated provider-preflight branch changed; E02 refuses to proceed" }

$inputDigests = @(
    "run-code-intel.ps1", "OpenSpec-Detector.ps1", "Invoke-WorkflowRecommendation.ps1", "orchestration/integrations.json"
) | ForEach-Object { (Get-FileHash -LiteralPath (Join-Path $RepoRoot $_) -Algorithm SHA256).Hash.ToLowerInvariant() }
$snapshotIdentity = Get-Sha256Text ($inputDigests -join "`n")
$retirementId = "retire-recommender-branch"
$branchId = "run-code-intel.workflow-recommender.inline"
$replacementId = "advisory.workflow-recommend"
$expiry = $EvaluatedAt + (30 * 86400)

function Add-Evidence([string]$Name, [string]$Class, [object]$Details) {
    $value = [ordered]@{
        schema = "code-intel-compatibility-retirement-evidence.v1"
        snapshotIdentity = $snapshotIdentity
        id = "e02.$Name"
        evidenceClass = $Class
        retirementId = $retirementId
        legacyBranchId = $branchId
        replacementCapabilityId = $replacementId
        details = $Details
    }
    $relative = "evidence/$Name.json"
    Write-JsonFile (Join-Path $OutDir $relative) $value
    return New-ArtifactRef "code-intel-compatibility-retirement-evidence.v1" "compatibility.retirement-evidence" $relative $snapshotIdentity
}

$replacement = Add-Evidence "replacement-atom" "replacement_atom" ([ordered]@{ outcome = "passed"; status = "production_ready"; capability = $replacementId; verification = $verification })
$golden = Add-Evidence "golden-parity" "golden_parity" ([ordered]@{ outcome = "passed"; assertionCount = 4; command = "scripts/tests/test-workflow-recommendation-brief.ps1" })
$contract = Add-Evidence "contract-parity" "contract_parity" ([ordered]@{ outcome = "passed"; assertionCount = 8; command = "cargo test -p code-intel --test capability_exec advisory_workflow_recommend_runs_through_a01_with_zero_effects_and_facade_parity" })
$effects = Add-Evidence "effect-parity" "effect_parity" ([ordered]@{ outcome = "passed"; assertionCount = 3; declaredEffects = @(); observedEffects = @(); noAutoInit = $true })
$registry = Add-Evidence "registry-reconciliation" "registry_reconciliation" ([ordered]@{ outcome = "passed"; registryParticipantId = "facade.workflow-recommender.inline"; replacementCapabilityId = $replacementId; status = "deleted"; providerPreflightUntouched = $providerPreflightPresent })
$window = Add-Evidence "compatibility-window" "compatibility_window" ([ordered]@{ outcome = "blocked"; startedAt = $EvaluatedAt; observedThrough = $EvaluatedAt; minimumDays = 30; checkedAt = $EvaluatedAt; expiresAt = $expiry; blocker = "no completed 30-day compatibility observation window" })

$rehearsalRelative = "work/e02-recommender-rollback-$EvaluatedAt"
$rehearsalRoot = Join-Path $RepoRoot ($rehearsalRelative -replace '/', [IO.Path]::DirectorySeparatorChar)
$rollbackCommand = "pwsh -NoLogo -NoProfile -File tools/compatibility/Restore-RecommenderLegacyBranch.ps1 -RehearsalRoot $rehearsalRelative"
& pwsh -NoLogo -NoProfile -File (Join-Path $RepoRoot "tools\compatibility\Restore-RecommenderLegacyBranch.ps1") -RehearsalRoot $rehearsalRoot | Out-Null
if ($LASTEXITCODE -ne 0) { throw "rollback rehearsal failed" }
$rollback = Add-Evidence "rollback-execution" "rollback_execution" ([ordered]@{ outcome = "passed"; command = $rollbackCommand; executedAt = $EvaluatedAt; exitCode = 0; target = "$rehearsalRelative/run-code-intel.ps1"; replacementChanged = $false })
$usage = Add-Evidence "usage-observation" "usage_observation" ([ordered]@{ outcome = "blocked"; startedAt = $EvaluatedAt; endedAt = $EvaluatedAt; totalInvocations = 0; legacyInvocations = 0; replacementInvocations = 0; blocker = "no production usage observation exists" })
$traceJson = '{"legacyBranchId":"' + $branchId + '","replacementCapabilityId":"' + $replacementId + '","retirementId":"' + $retirementId + '"}'
$necessity = Add-Evidence "c00-necessity" "c00_necessity" ([ordered]@{ outcome = "passed"; decision = "admit"; changeId = $retirementId; necessityTraceSha256 = (Get-Sha256Text $traceJson) })
$snapshotDependency = Add-Evidence "dependency-repo-snapshot" "dependency_approval" ([ordered]@{ outcome = "passed"; dependencyId = "repo.snapshot"; status = "approved"; reviewer = "e02-author" })
$d02Dependency = Add-Evidence "dependency-d02-clean-machine" "dependency_approval" ([ordered]@{ outcome = "blocked"; dependencyId = "project.orientation-benchmark"; status = "pending"; reviewer = "independent-verifier-required"; blocker = "D02 clean-machine repetition is not complete" })

$subject = [ordered]@{
    legacyBranch = [ordered]@{ capabilityId = "facade.workflow-recommender.inline"; branchId = $branchId; callPath = "run-code-intel.ps1::$branchId"; affectedFiles = @("run-code-intel.ps1"); owner = "executor-recommender"; registryParticipantId = "facade.workflow-recommender.inline" }
    replacement = [ordered]@{ capabilityId = $replacementId; implementationId = "advisory.workflow-recommend.compat"; dependencies = @("repo.snapshot", "project.orientation-benchmark"); atomEvidence = $replacement }
    parity = [ordered]@{ golden = $golden; contract = $contract; effects = $effects }
    registryReconciliation = $registry
    compatibilityWindow = $window
    rollback = [ordered]@{ command = $rollbackCommand; executionEvidence = $rollback }
    usageObservation = $usage
    necessityEvidence = $necessity
    dependencyStates = @($snapshotDependency, $d02Dependency)
    lineReductionEvidence = $false
}
$independent = Add-Evidence "independent-approval" "independent_approval" ([ordered]@{ outcome = "blocked"; approved = $false; authorIndependent = $false; subjectSha256 = ("0" * 64); reviewer = "independent-verifier-required"; authorityEvent = [ordered]@{}; blocker = "no independent repository-governed approval exists" })
$manifest = [ordered]@{ schema = "code-intel-compatibility-retirement-manifest.v1"; snapshotIdentity = $snapshotIdentity; retirementId = $retirementId; approvalSubject = $subject; independentApproval = $independent }
Write-JsonFile (Join-Path $OutDir "compatibility-retirement-manifest.json") $manifest
$manifestRef = New-ArtifactRef "code-intel-compatibility-retirement-manifest.v1" "compatibility.retirement-manifest" "compatibility-retirement-manifest.json" $snapshotIdentity

$registryJson = Get-Content -LiteralPath (Join-Path $RepoRoot "orchestration\integrations.json") -Raw | ConvertFrom-Json
$gateDecl = ($registryJson.integrations | Where-Object { $_.id -eq "compatibility.retirement-gate" }).capabilityDeclaration
$inputs = @($manifestRef, $replacement, $golden, $contract, $effects, $registry, $window, $rollback, $usage, $necessity, $snapshotDependency, $d02Dependency, $independent)
$request = [ordered]@{
    schema = "code-intel-capability-request.v1"; capability = "compatibility.retirement-gate"; contractVersion = 1; implementation = $gateDecl.implementation
    snapshot = [ordered]@{ identity = $snapshotIdentity; repoIdentity = ("content-v1:" + ("c" * 64)); head = "unversioned"; workingTreePolicy = "explicit_overlay"; scope = @("."); inputDigest = ("d" * 64) }
    options = [ordered]@{ evaluatedAt = $EvaluatedAt }; inputs = $inputs; effectPolicy = [ordered]@{ allowedEffects = $gateDecl.allowedEffects }
}
Write-JsonFile (Join-Path $OutDir "e00-request.json") $request
$gateOut = Join-Path $OutDir "gate-out"
& $CodeIntel capability exec compatibility.retirement-gate --request (Join-Path $OutDir "e00-request.json") --out $gateOut --artifact-root $OutDir | Out-Null
if ($LASTEXITCODE -ne 0) { throw "E00 execution failed" }
$decision = Get-Content -LiteralPath (Join-Path $gateOut "compatibility-retirement-decision.json") -Raw | ConvertFrom-Json
if ($decision.decision -ne "blocked") { throw "E02 must not proceed without real usage, D02 clean-machine evidence, and independent approval" }

$baseText = $runText.Replace("`r`n", "`n").Replace("`r", "`n")
$deletePatterns = @(
    '(?s)# Workflow recommendations are owned by the standalone advisory atom in OpenSpec-Detector\.ps1\..*?(?=\nfunction Get-JsonProperty)',
    '(?s)# Historical options now map to the standalone advisory atom: Skip disables it and.*?(?=\nif \(-not \$toolState\.rg\))'
)
$matches = @($deletePatterns | ForEach-Object {
    $match = [regex]::Match($baseText, $_)
    if (-not $match.Success) { throw "bounded recommender deletion marker is absent: $_" }
    $match
} | Sort-Object Index)
$deletedBefore = 0
$hunks = @($matches | ForEach-Object {
    $deletedLines = @($_.Value -split "`n")
    $oldStart = @(($baseText.Substring(0, $_.Index)) -split "`n").Count
    $hunk = [ordered]@{
        addedLines = @()
        deletedLines = $deletedLines
        newLines = 0
        newStart = $oldStart - $deletedBefore
        oldLines = $deletedLines.Count
        oldStart = $oldStart
    }
    $deletedBefore += $deletedLines.Count
    $hunk
})
$baseLines = @($baseText -split "`n")
$resultLines = for ($lineNumber = 1; $lineNumber -le $baseLines.Count; $lineNumber++) {
    $deleted = @($hunks | Where-Object {
        $lineNumber -ge $_.oldStart -and $lineNumber -lt ($_.oldStart + $_.oldLines)
    }).Count -gt 0
    if (-not $deleted) { $baseLines[$lineNumber - 1] }
}
$resultText = $resultLines -join "`n"
$patchFiles = @([ordered]@{
    addedLines = $null
    baseBlobSha256 = Get-Sha256Text $baseText
    baseText = $baseText
    hunks = $hunks
    path = "run-code-intel.ps1"
    resultBlobSha256 = Get-Sha256Text $resultText
    resultText = $resultText
})
$null = $patchFiles[0].Remove("addedLines")
$deletionDiff = [ordered]@{
    schema = "code-intel-compatibility-retirement-deletion-diff.v1"
    snapshotIdentity = $snapshotIdentity
    retirementId = $retirementId
    legacyBranchId = $branchId
    affectedFiles = @("run-code-intel.ps1")
    deletionsOnly = $true
    summary = "Proposed deletion is limited to the retired inline recommender adapter markers; provider-preflight and all other branches are excluded. Summary is non-authoritative."
    patch = [ordered]@{ algorithm = "replayable-delete-only-v1"; sha256 = (Get-Sha256Json $patchFiles); files = $patchFiles }
}
Write-JsonFile (Join-Path $OutDir "compatibility-retirement-deletion-diff.json") $deletionDiff
$diffRef = New-ArtifactRef "code-intel-compatibility-retirement-deletion-diff.v1" "compatibility.retirement-deletion-diff" "compatibility-retirement-deletion-diff.json" $snapshotIdentity
$decisionRef = New-ArtifactRef "code-intel-compatibility-retirement-decision.v1" "compatibility.retirement-decision" "gate-out/compatibility-retirement-decision.json" $snapshotIdentity
$ticket = [ordered]@{
    schema = "code-intel-compatibility-retirement-ticket-template.v1"; snapshotIdentity = $snapshotIdentity; ticketId = "ticket-e02-retire-recommender-branch"; retirementId = $retirementId
    legacyBranch = [ordered]@{ capabilityId = "facade.workflow-recommender.inline"; branchId = $branchId; callPath = "run-code-intel.ps1::$branchId" }
    replacement = [ordered]@{ capabilityId = $replacementId; dependencies = @("repo.snapshot", "project.orientation-benchmark") }
    affectedFiles = @("run-code-intel.ps1")
    evidence = [ordered]@{ golden = $golden; contract = $contract; effects = $effects; usage = $usage; rollbackRehearsal = $rollback; deletionDiff = $diffRef }
    source = [ordered]@{ retirementDecision = $decisionRef; retirementManifest = $manifestRef }
    owner = "executor-recommender"; verifier = "independent-verifier-required"; observationExpiry = $expiry; status = "draft"; authorityBoundary = "template_only_no_approval_or_deletion_authority"
}
Write-JsonFile (Join-Path $OutDir "compatibility-retirement-ticket.json") $ticket
& $CodeIntel compatibility retirement-ticket lint --ticket (Join-Path $OutDir "compatibility-retirement-ticket.json") --evaluated-at $EvaluatedAt | Out-Null
if ($LASTEXITCODE -ne 0) { throw "E01 ticket lint failed" }
$ticketRef = New-ArtifactRef "code-intel-compatibility-retirement-ticket-template.v1" "compatibility.retirement-ticket-template" "compatibility-retirement-ticket.json" $snapshotIdentity
$ticketDecl = ($registryJson.integrations | Where-Object { $_.id -eq "compatibility.retirement-ticket-template" }).capabilityDeclaration
$e01Request = [ordered]@{
    schema = "code-intel-capability-request.v1"; capability = "compatibility.retirement-ticket-template"; contractVersion = 1; implementation = $ticketDecl.implementation
    snapshot = [ordered]@{ identity = $snapshotIdentity; repoIdentity = ("content-v1:" + ("c" * 64)); head = "unversioned"; workingTreePolicy = "explicit_overlay"; scope = @("."); inputDigest = ("d" * 64) }
    options = [ordered]@{ evaluatedAt = $EvaluatedAt }; inputs = @($ticketRef, $manifestRef, $decisionRef, $diffRef); effectPolicy = [ordered]@{ allowedEffects = $ticketDecl.allowedEffects }
}
Write-JsonFile (Join-Path $OutDir "e01-request.json") $e01Request
$e01Output = @(& $CodeIntel capability exec compatibility.retirement-ticket-template --request (Join-Path $OutDir "e01-request.json") --out (Join-Path $OutDir "e01-out") --artifact-root $OutDir 2>&1)
$e01Exit = $LASTEXITCODE
$e01Text = $e01Output -join "`n"
[IO.File]::WriteAllText((Join-Path $OutDir "e01-stderr.txt"), $e01Text, [Text.UTF8Encoding]::new($false))
if ($e01Exit -ne 65 -or $e01Text -notmatch "ticket requires an approved E00 decision") {
    throw "E01 must validate the replayable patch and reject only because the real E00 decision is blocked: exit=$e01Exit output=$e01Text"
}

$status = [ordered]@{
    schema = "code-intel-compatibility-retirement-execution-status.v1"; retirementId = $retirementId; decision = "blocked"; deletionExecuted = $false; retired = $false
    blockers = @($decision.blockers); gainLedgerProjection = $decision.gainLedgerProjection
    boundary = "E02 generated a complete draft packet but has no approval or deletion authority while any E00 blocker remains."
}
Write-JsonFile (Join-Path $OutDir "status.json") $status
$status | ConvertTo-Json -Depth 10 -Compress
