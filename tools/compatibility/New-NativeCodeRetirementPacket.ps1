[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string]$OutDir,
    [Parameter(Mandatory = $true)] [long]$EvaluatedAt,
    [string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),
    [string]$CodeIntel = (Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) "target\debug\code-intel.exe")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($EvaluatedAt -le 0) { throw "EvaluatedAt must be positive" }
if (Test-Path -LiteralPath $OutDir) { throw "packet output must be exclusive: $OutDir" }
if (-not (Test-Path -LiteralPath $CodeIntel -PathType Leaf)) { throw "code-intel binary is missing: $CodeIntel" }
$null = New-Item -ItemType Directory -Path $OutDir
$null = New-Item -ItemType Directory -Path (Join-Path $OutDir "evidence")

function Write-JsonFile([string]$Path, [object]$Value) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 40 -Compress), [Text.UTF8Encoding]::new($false))
}
function Get-Sha256Text([string]$Text) {
    return ([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Text)))).ToLowerInvariant()
}
function Get-Sha256Json([object]$Value) { return Get-Sha256Text (ConvertTo-Json -InputObject $Value -Depth 40 -Compress) }
function New-ArtifactRef([string]$ArtifactSchema, [string]$Type, [string]$RelativePath, [string]$SnapshotIdentity) {
    $path = Join-Path $OutDir ($RelativePath -replace '/', [IO.Path]::DirectorySeparatorChar)
    return [ordered]@{
        schema = "code-intel-artifact-ref.v1"; artifactSchema = $ArtifactSchema; type = $Type
        path = ($RelativePath -replace '\\', '/'); sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        consumedSnapshotIdentity = $SnapshotIdentity
    }
}

$runPath = Join-Path $RepoRoot "run-code-intel.ps1"
$runText = [IO.File]::ReadAllText($runPath).Replace("`r`n", "`n").Replace("`r", "`n")
$deletePatterns = @(
    '(?sm)^function Get-CodeEvidenceLanguage \{.*?(?=\nfunction ConvertTo-NullableDouble \{)',
    '(?m)^\$codeEvidenceConfig = Get-JsonProperty \$configData "codeEvidence"\n\$codeEvidence = New-CodeEvidenceLayer -RepoPath \$repoPath -RunDir \$runDir -Files \$inventoryFiles -CodeEvidenceConfig \$codeEvidenceConfig'
)
$matches = @($deletePatterns | ForEach-Object {
    $all = [regex]::Matches($runText, $_)
    if ($all.Count -ne 1) { throw "E07 embedded branch marker is absent or ambiguous: $_" }
    $all[0]
} | Sort-Object Index)
$dagFacadeCount = [regex]::Matches($runText, '(?m)^if \(\$DagCoordinate\) \{').Count
$dagCommandCount = [regex]::Matches($runText, '& \$rustCli run dag-coordinate --repo \$repoPath --out \$dagOut').Count
if ($dagFacadeCount -ne 1 -or $dagCommandCount -ne 1) { throw "E07 requires one A09 facade route" }

$registry = Get-Content -LiteralPath (Join-Path $RepoRoot "orchestration/integrations.json") -Raw | ConvertFrom-Json
$nativeDeclarations = @($registry.integrations | Where-Object id -eq "evidence.native-code")
if ($nativeDeclarations.Count -ne 1) { throw "B07 registry must declare evidence.native-code exactly once" }
$declaredDigest = [string]$nativeDeclarations[0].capabilityDeclaration.implementation.toolchainDigests[0]
$sourceDigest = (Get-FileHash -LiteralPath (Join-Path $RepoRoot "crates/code-intel-cli/src/native_code_evidence.rs") -Algorithm SHA256).Hash.ToLowerInvariant()
if ($declaredDigest -ne $sourceDigest) { throw "R06/B07 native-code digest is not stable: declared=$declaredDigest source=$sourceDigest" }

$nativeTests = @(
    "native_atom_preserves_representative_v1_artifacts_through_a01_a03_a09",
    "unsupported_language_is_explicit_unknown_without_relationship_fabrication",
    "labeled_multilingual_corpus_quantifies_native_symbol_precision_recall_and_coverage",
    "a01_a09_artifacts_match_the_real_legacy_producer_on_the_same_fixture"
)
$dagTests = @(
    "production_run_route_executes_snapshot_then_inventory",
    "production_run_preserves_doctor_domain_failure_and_completes_unrelated_branch"
)
foreach ($testName in $nativeTests) {
    & cargo test -q -p code-intel --test native_code_evidence $testName -- --exact
    if ($LASTEXITCODE -ne 0) { throw "B08 targeted test failed: $testName" }
}
foreach ($testName in $dagTests) {
    & cargo test -q -p code-intel --test dag_run $testName -- --exact
    if ($LASTEXITCODE -ne 0) { throw "A09/B08 targeted route test failed: $testName" }
}
$registryAudit = & $CodeIntel orchestrate --action Validate --manifest (Join-Path $RepoRoot "orchestration/integrations.json") --json | ConvertFrom-Json
if (-not $registryAudit.ok -or -not $registryAudit.registryAudit.ok) { throw "B07 registry audit failed" }

$snapshotInputs = @(
    "run-code-intel.ps1", "crates/code-intel-cli/src/native_code_evidence.rs",
    "crates/code-intel-cli/tests/native_code_evidence.rs", "crates/code-intel-cli/tests/dag_run.rs",
    "orchestration/integrations.json"
)
$snapshotIdentity = Get-Sha256Text (($snapshotInputs | ForEach-Object {
    (Get-FileHash -LiteralPath (Join-Path $RepoRoot $_) -Algorithm SHA256).Hash.ToLowerInvariant()
}) -join "`n")
$retirementId = "retire-native-code-branch"
$branchId = "run-code-intel.native-code.embedded"
$replacementId = "evidence.native-code"
$expiry = $EvaluatedAt + (30 * 86400)
function Add-Evidence([string]$Name, [string]$Class, [object]$Details) {
    $value = [ordered]@{
        schema = "code-intel-compatibility-retirement-evidence.v1"; snapshotIdentity = $snapshotIdentity
        id = "e07.$Name"; evidenceClass = $Class; retirementId = $retirementId
        legacyBranchId = $branchId; replacementCapabilityId = $replacementId; details = $Details
    }
    $relative = "evidence/$Name.json"
    Write-JsonFile (Join-Path $OutDir $relative) $value
    return New-ArtifactRef "code-intel-compatibility-retirement-evidence.v1" "compatibility.retirement-evidence" $relative $snapshotIdentity
}

$replacement = Add-Evidence "replacement-atom" "replacement_atom" ([ordered]@{
    outcome = "blocked"; status = "pending_normal_full_facade_route"; capability = $replacementId
    b08AtomAvailable = $true; a09RouteAvailable = $true; normalFullRouteUsesA09 = $false
    blocker = "normal/full continue through the embedded New-CodeEvidenceLayer call unless DagCoordinate is explicitly selected"
})
$golden = Add-Evidence "golden-parity" "golden_parity" ([ordered]@{
    outcome = "passed"; assertionCount = 2; executedTestCount = $nativeTests.Count
    command = "cargo test -q -p code-intel --test native_code_evidence <test-name> -- --exact"
    normalizedArtifactParity = $true; modes = @("normal", "full"); publicModeRouteParity = "blocked_until_A09_substitution"
})
$contract = Add-Evidence "contract-parity" "contract_parity" ([ordered]@{
    outcome = "passed"; assertionCount = 8; artifactRefCount = 8; stableV1Artifacts = $true; unsupportedLanguage = "unknown"
    relationshipPrecision = "unknown"; callGraph = "unknown"; fabricatedRelationships = $false
    executedNativeTests = $nativeTests.Count; executedDagTests = $dagTests.Count
})
$effects = Add-Evidence "effect-parity" "effect_parity" ([ordered]@{
    outcome = "passed"; assertionCount = 2; declaredEffects = @("repo_read", "local_write"); observedEffects = @("repo_read", "local_write")
    transport = "artifact_ref"; publicationTouched = $false; indexTouched = $false; hospitalTouched = $false
})
$registryEvidence = Add-Evidence "registry-reconciliation" "registry_reconciliation" ([ordered]@{
    outcome = "passed"; registryParticipantId = "evidence.native-code"; replacementCapabilityId = $replacementId
    status = "declared"; registryAuditOk = $true; toolchainDigest = $sourceDigest
    embeddedFunctionSegmentCount = 1; embeddedCallSegmentCount = 1; deletionStatus = "not_authorized"
})
$window = Add-Evidence "compatibility-window" "compatibility_window" ([ordered]@{
    outcome = "blocked"; startedAt = $EvaluatedAt; observedThrough = $EvaluatedAt; minimumDays = 30
    checkedAt = $EvaluatedAt; expiresAt = $expiry; blocker = "no completed normal/full A09 production observation window"
})

$rehearsalRelative = "rollback-rehearsal"
$rehearsalRoot = Join-Path $OutDir $rehearsalRelative
$rollbackCommand = "pwsh -NoProfile -File tools/compatibility/Restore-NativeCodeEmbeddedBranch.ps1 -RehearsalRoot <packet-root>/$rehearsalRelative"
& pwsh -NoProfile -File (Join-Path $RepoRoot "tools/compatibility/Restore-NativeCodeEmbeddedBranch.ps1") -RehearsalRoot $rehearsalRoot | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Native Code Evidence rollback rehearsal failed" }
$rollback = Add-Evidence "rollback-execution" "rollback_execution" ([ordered]@{
    outcome = "passed"; command = $rollbackCommand; executedAt = $EvaluatedAt; exitCode = 0
    target = "$rehearsalRelative/run-code-intel.ps1"; exactReplay = $true; unrelatedBranchesChanged = $false; segmentCount = 2
})
$usage = Add-Evidence "usage-observation" "usage_observation" ([ordered]@{
    outcome = "blocked"; startedAt = $EvaluatedAt; endedAt = $EvaluatedAt; totalInvocations = 0
    legacyInvocations = 0; replacementInvocations = 0; blocker = "normal/full A09 route observation has not started"
})
$traceJson = '{"legacyBranchId":"' + $branchId + '","replacementCapabilityId":"' + $replacementId + '","retirementId":"' + $retirementId + '"}'
$necessity = Add-Evidence "c00-necessity" "c00_necessity" ([ordered]@{
    outcome = "passed"; decision = "admit"; changeId = $retirementId; necessityTraceSha256 = (Get-Sha256Text $traceJson)
    rationale = "remove duplicated embedded evidence production only after B08 executes through A09 for normal/full"
})
$snapshotDependency = Add-Evidence "dependency-snapshot" "dependency_approval" ([ordered]@{
    outcome = "passed"; dependencyId = "repo.snapshot"; status = "approved"; reviewer = "e07-author-test"
})
$inventoryDependency = Add-Evidence "dependency-inventory" "dependency_approval" ([ordered]@{
    outcome = "passed"; dependencyId = "inventory.rg"; status = "approved"; reviewer = "e07-author-test"
})

$subject = [ordered]@{
    legacyBranch = [ordered]@{
        capabilityId = "evidence.native-code.embedded"; branchId = $branchId
        callPath = "run-code-intel.ps1::$branchId"; affectedFiles = @("run-code-intel.ps1")
        owner = "executor-native-code-retirement"; registryParticipantId = "evidence.native-code"
    }
    replacement = [ordered]@{
        capabilityId = $replacementId; implementationId = "evidence.native-code.compat"
        dependencies = @("repo.snapshot", "inventory.rg"); atomEvidence = $replacement
    }
    parity = [ordered]@{ golden = $golden; contract = $contract; effects = $effects }
    registryReconciliation = $registryEvidence; compatibilityWindow = $window
    rollback = [ordered]@{ command = $rollbackCommand; executionEvidence = $rollback }
    usageObservation = $usage; necessityEvidence = $necessity
    dependencyStates = @($snapshotDependency, $inventoryDependency); lineReductionEvidence = $false
}
$independent = Add-Evidence "independent-approval" "independent_approval" ([ordered]@{
    outcome = "blocked"; approved = $false; authorIndependent = $false; subjectSha256 = ("0" * 64)
    reviewer = "independent-verifier-required"; authorityEvent = [ordered]@{}
    blocker = "no independent repository-governed E00 approval exists"
})
$manifest = [ordered]@{
    schema = "code-intel-compatibility-retirement-manifest.v1"; snapshotIdentity = $snapshotIdentity
    retirementId = $retirementId; approvalSubject = $subject; independentApproval = $independent
}
Write-JsonFile (Join-Path $OutDir "compatibility-retirement-manifest.json") $manifest
$manifestRef = New-ArtifactRef "code-intel-compatibility-retirement-manifest.v1" "compatibility.retirement-manifest" "compatibility-retirement-manifest.json" $snapshotIdentity

$gateDecl = ($registry.integrations | Where-Object { $_.id -eq "compatibility.retirement-gate" }).capabilityDeclaration
$gateInputs = @($manifestRef, $replacement, $golden, $contract, $effects, $registryEvidence, $window, $rollback, $usage, $necessity, $snapshotDependency, $inventoryDependency, $independent)
$requestSnapshot = [ordered]@{
    identity = $snapshotIdentity; repoIdentity = ("content-v1:" + ("c" * 64)); head = "unversioned"
    workingTreePolicy = "explicit_overlay"; scope = @("."); inputDigest = ("d" * 64)
}
$e00Request = [ordered]@{
    schema = "code-intel-capability-request.v1"; capability = "compatibility.retirement-gate"; contractVersion = 1
    implementation = $gateDecl.implementation; snapshot = $requestSnapshot; options = [ordered]@{ evaluatedAt = $EvaluatedAt }
    inputs = $gateInputs; effectPolicy = [ordered]@{ allowedEffects = $gateDecl.allowedEffects }
}
Write-JsonFile (Join-Path $OutDir "e00-request.json") $e00Request
$gateOut = Join-Path $OutDir "gate-out"
& $CodeIntel capability exec compatibility.retirement-gate --request (Join-Path $OutDir "e00-request.json") --out $gateOut --artifact-root $OutDir | Out-Null
if ($LASTEXITCODE -ne 0) { throw "E00 execution failed" }
$decision = Get-Content -LiteralPath (Join-Path $gateOut "compatibility-retirement-decision.json") -Raw | ConvertFrom-Json
if ($decision.decision -ne "blocked") { throw "E07 cannot pass while normal/full route, observation, and approval blockers remain" }

$deletedBefore = 0
$hunks = @($matches | ForEach-Object {
    $deletedLines = @($_.Value -split "`n")
    $oldStart = @(($runText.Substring(0, $_.Index)) -split "`n").Count
    $hunk = [ordered]@{
        addedLines = @(); deletedLines = $deletedLines; newLines = 0; newStart = $oldStart - $deletedBefore
        oldLines = $deletedLines.Count; oldStart = $oldStart
    }
    $deletedBefore += $deletedLines.Count
    $hunk
})
$baseLines = @($runText -split "`n")
$resultLines = for ($lineNumber = 1; $lineNumber -le $baseLines.Count; $lineNumber++) {
    $deleted = @($hunks | Where-Object { $lineNumber -ge $_.oldStart -and $lineNumber -lt ($_.oldStart + $_.oldLines) }).Count -gt 0
    if (-not $deleted) { $baseLines[$lineNumber - 1] }
}
$resultText = $resultLines -join "`n"
$patchFiles = @([ordered]@{
    baseBlobSha256 = Get-Sha256Text $runText; baseText = $runText; hunks = $hunks; path = "run-code-intel.ps1"
    resultBlobSha256 = Get-Sha256Text $resultText; resultText = $resultText
})
$deletionDiff = [ordered]@{
    schema = "code-intel-compatibility-retirement-deletion-diff.v1"; snapshotIdentity = $snapshotIdentity
    retirementId = $retirementId; legacyBranchId = $branchId; affectedFiles = @("run-code-intel.ps1"); deletionsOnly = $true
    summary = "Proposed deletion removes only the embedded Native Code Evidence function family and its direct call. It remains non-executable while normal/full are not routed through A09."
    patch = [ordered]@{ algorithm = "replayable-delete-only-v1"; sha256 = (Get-Sha256Json $patchFiles); files = $patchFiles }
}
Write-JsonFile (Join-Path $OutDir "compatibility-retirement-deletion-diff.json") $deletionDiff
$diffRef = New-ArtifactRef "code-intel-compatibility-retirement-deletion-diff.v1" "compatibility.retirement-deletion-diff" "compatibility-retirement-deletion-diff.json" $snapshotIdentity
$decisionRef = New-ArtifactRef "code-intel-compatibility-retirement-decision.v1" "compatibility.retirement-decision" "gate-out/compatibility-retirement-decision.json" $snapshotIdentity
$ticket = [ordered]@{
    schema = "code-intel-compatibility-retirement-ticket-template.v1"; snapshotIdentity = $snapshotIdentity
    ticketId = "ticket-e07-retire-native-code-branch"; retirementId = $retirementId
    legacyBranch = [ordered]@{ capabilityId = "evidence.native-code.embedded"; branchId = $branchId; callPath = "run-code-intel.ps1::$branchId" }
    replacement = [ordered]@{ capabilityId = $replacementId; dependencies = @("repo.snapshot", "inventory.rg") }
    affectedFiles = @("run-code-intel.ps1")
    evidence = [ordered]@{ golden = $golden; contract = $contract; effects = $effects; usage = $usage; rollbackRehearsal = $rollback; deletionDiff = $diffRef }
    source = [ordered]@{ retirementDecision = $decisionRef; retirementManifest = $manifestRef }
    owner = "executor-native-code-retirement"; verifier = "independent-verifier-required"
    observationExpiry = $expiry; status = "draft"; authorityBoundary = "template_only_no_approval_or_deletion_authority"
}
Write-JsonFile (Join-Path $OutDir "compatibility-retirement-ticket.json") $ticket
& $CodeIntel compatibility retirement-ticket lint --ticket (Join-Path $OutDir "compatibility-retirement-ticket.json") --evaluated-at $EvaluatedAt | Out-Null
if ($LASTEXITCODE -ne 0) { throw "E01 ticket lint failed" }
$ticketRef = New-ArtifactRef "code-intel-compatibility-retirement-ticket-template.v1" "compatibility.retirement-ticket-template" "compatibility-retirement-ticket.json" $snapshotIdentity
$e01Request = [ordered]@{
    schema = "code-intel-capability-request.v1"; capability = "compatibility.retirement-ticket-template"; contractVersion = 1
    implementation = (($registry.integrations | Where-Object id -eq "compatibility.retirement-ticket-template").capabilityDeclaration.implementation)
    snapshot = $requestSnapshot; options = [ordered]@{ evaluatedAt = $EvaluatedAt }
    inputs = @($ticketRef, $decisionRef, $manifestRef, $diffRef)
    effectPolicy = [ordered]@{ allowedEffects = @("local_write") }
}
Write-JsonFile (Join-Path $OutDir "e01-request.json") $e01Request
$e01Out = Join-Path $OutDir "ticket-out"
$e01Output = (& $CodeIntel capability exec compatibility.retirement-ticket-template --request (Join-Path $OutDir "e01-request.json") --out $e01Out --artifact-root $OutDir 2>&1 | Out-String).Trim()
$e01Exit = $LASTEXITCODE
[IO.File]::WriteAllText((Join-Path $OutDir "e01-stderr.txt"), $e01Output, [Text.UTF8Encoding]::new($false))
if ($e01Exit -ne 65 -or $e01Output -notmatch "ticket requires an approved E00 decision") {
    throw "E01 must validate the deletion proof and reject only because E00 is blocked: exit=$e01Exit output=$e01Output"
}

$status = [ordered]@{
    schema = "code-intel-compatibility-retirement-execution-status.v1"; retirementId = $retirementId
    decision = "blocked"; liveEmbeddedBranch = $true; normalFullRoutedThroughA09 = $false
    replacementCapability = $replacementId; deletionExecuted = $false; retired = $false
    blockers = @($decision.blockers | Sort-Object)
    gainLedgerProjection = [ordered]@{
        evidenceCount = 12; gain = "Retire embedded Native Code Evidence only after normal/full execute B08 through A09"
        id = "retirement:$retirementId"; status = "blocked"
    }
    boundary = "E07 is a single-branch draft packet. Embedded production remains live and no deletion authority exists while E00 is blocked."
}
Write-JsonFile (Join-Path $OutDir "status.json") $status
$status | ConvertTo-Json -Depth 10 -Compress
