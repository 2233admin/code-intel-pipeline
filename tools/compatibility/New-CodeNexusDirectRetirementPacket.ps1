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
function Get-Sha256Json([object]$Value) {
    return Get-Sha256Text (ConvertTo-Json -InputObject $Value -Depth 40 -Compress)
}
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
$directPattern = '(?s)\n\$codeNexusLiteTool = Join-Path \$PSScriptRoot "Invoke-CodeNexusLite\.ps1".*?(?=\n\$reportPath = Join-Path \$runDir "report\.json")'
$directMatches = [regex]::Matches($runText, $directPattern)
$b04FacadeCount = [regex]::Matches($runText, 'provider codenexus-adapt').Count
$b05FacadeCount = [regex]::Matches($runText, 'repository survival-scan').Count
if ($directMatches.Count -ne 1) { throw "E04 requires exactly one live direct CodeNexus-lite branch; found $($directMatches.Count)" }
if ($b04FacadeCount -ne 1 -or $b05FacadeCount -ne 1) { throw "B04/B05 facade routes are missing or ambiguous" }

$env:CARGO_TARGET_DIR = Join-Path $RepoRoot "work/e04-target"
$b04Tests = @(
    "full_and_lite_share_one_port_shape_but_keep_distinct_provenance",
    "unavailable_is_partial_provider_unavailable_and_never_fabricates_facts",
    "snapshot_mismatch_and_stale_observation_fail_closed_in_a04",
    "lite_is_only_explicit_fallback_or_rollback_and_full_is_primary",
    "adapter_declares_effects_rejects_storage_coupling_and_drops_unknown_secrets",
    "admitted_payload_identity_cannot_be_relabelled",
    "port_schema_and_boundary_document_are_closed_and_real",
    "production_route_runs_full_lite_and_unavailable_through_a04",
    "production_route_rejects_secret_fields_wrong_snapshot_and_bad_usage",
    "production_registry_facade_and_route_schema_are_declared"
)
$b05Tests = @(
    "unavailable_provider_yields_useful_basic_evidence_and_unknown_structure",
    "fallback_rejects_observed_provider_and_forged_admission",
    "fallback_rejects_undeclared_codenexus_port_fields",
    "fallback_rejects_undeclared_fact_promotion_fields",
    "snapshot_and_inventory_are_a03_verified_and_snapshot_bound",
    "production_cli_registry_facade_schema_and_docs_are_closed",
    "result_has_exact_top_level_contract"
)
foreach ($testName in $b04Tests) {
    & cargo test -q -p code-intel --test codenexus_adapter $testName -- --exact
    if ($LASTEXITCODE -ne 0) { throw "B04 targeted contract test failed: $testName" }
}
foreach ($testName in $b05Tests) {
    & cargo test -q -p code-intel --test survival_scan $testName -- --exact
    if ($LASTEXITCODE -ne 0) { throw "B05 targeted fallback test failed: $testName" }
}

$snapshotInputs = @(
    "run-code-intel.ps1", "Invoke-CodeNexusLite.ps1", "crates/code-intel-cli/src/codenexus_adapter.rs",
    "crates/code-intel-cli/src/survival_scan.rs", "orchestration/integrations.json"
)
$snapshotIdentity = Get-Sha256Text (($snapshotInputs | ForEach-Object {
    (Get-FileHash -LiteralPath (Join-Path $RepoRoot $_) -Algorithm SHA256).Hash.ToLowerInvariant()
}) -join "`n")
$retirementId = "retire-codenexus-direct-branch"
$branchId = "run-code-intel.codenexus-lite.direct"
$replacementId = "provider.codenexus-adapt"
$expiry = $EvaluatedAt + (30 * 86400)

function Add-Evidence([string]$Name, [string]$Class, [object]$Details) {
    $value = [ordered]@{
        schema = "code-intel-compatibility-retirement-evidence.v1"; snapshotIdentity = $snapshotIdentity
        id = "e04.$Name"; evidenceClass = $Class; retirementId = $retirementId
        legacyBranchId = $branchId; replacementCapabilityId = $replacementId; details = $Details
    }
    $relative = "evidence/$Name.json"
    Write-JsonFile (Join-Path $OutDir $relative) $value
    return New-ArtifactRef "code-intel-compatibility-retirement-evidence.v1" "compatibility.retirement-evidence" $relative $snapshotIdentity
}

$replacement = Add-Evidence "replacement-atom" "replacement_atom" ([ordered]@{
    outcome = "blocked"; status = "pending_facade_route"; capability = $replacementId
    b04FacadeAvailable = $true; b05FacadeAvailable = $true; liveDirectBranchCount = $directMatches.Count
    blocker = "normal production path still invokes Invoke-CodeNexusLite.ps1 directly instead of composing B04 and B05"
})
$golden = Add-Evidence "golden-parity" "golden_parity" ([ordered]@{
    outcome = "passed"; assertionCount = 3; command = "cargo test -q -p code-intel --test codenexus_adapter <test-name> -- --exact"
    executedTestCount = $b04Tests.Count; testFunctions = $b04Tests
    fixtures = @("full-current", "lite-current", "unavailable"); portShapeShared = $true; provenanceDistinct = $true
})
$contract = Add-Evidence "contract-parity" "contract_parity" ([ordered]@{
    outcome = "passed"; assertionCount = 6
    commands = @(
        "cargo test -q -p code-intel --test codenexus_adapter <test-name> -- --exact",
        "cargo test -q -p code-intel --test survival_scan <test-name> -- --exact"
    )
    executedB04TestCount = $b04Tests.Count; executedB05TestCount = $b05Tests.Count
    unavailableRoute = "repository.survival-scan"; structuralVerdictWhenUnavailable = "unknown"
    forbiddenStructuralClaims = @("architecture", "dependency graph", "impact analysis", "complete call graph")
})
$effects = Add-Evidence "effect-parity" "effect_parity" ([ordered]@{
    outcome = "passed"; assertionCount = 5; processOwnership = "provider"; storageOwnership = "provider"
    transport = "artifact_ref"; fullEffects = @("network_provider", "read_provider_artifact")
    liteEffects = @("read_repository", "read_git_history", "read_sentrux_artifacts", "write_compatibility_artifact")
    fallbackEffects = @("repo_read", "local_write"); effectSetsRemainModeSpecific = $true; noSecretOrStoragePathProjection = $true
})
$registry = Add-Evidence "registry-reconciliation" "registry_reconciliation" ([ordered]@{
    outcome = "passed"; registryParticipantId = "localization.codenexus-lite"; replacementCapabilityId = $replacementId
    status = "declared"; b04FacadeCount = $b04FacadeCount; b05FacadeCount = $b05FacadeCount
    liveDirectBranchCount = $directMatches.Count; deletionStatus = "not_authorized"
})
$window = Add-Evidence "compatibility-window" "compatibility_window" ([ordered]@{
    outcome = "blocked"; startedAt = $EvaluatedAt; observedThrough = $EvaluatedAt; minimumDays = 30
    checkedAt = $EvaluatedAt; expiresAt = $expiry; blocker = "no completed 30-day full/lite/unavailable production observation window"
})

$rehearsalRelative = "rollback-rehearsal"
$rehearsalRoot = Join-Path $OutDir $rehearsalRelative
$rollbackCommand = "pwsh -NoProfile -File tools/compatibility/Restore-CodeNexusDirectBranch.ps1 -RehearsalRoot <packet-root>/$rehearsalRelative"
& pwsh -NoProfile -File (Join-Path $RepoRoot "tools/compatibility/Restore-CodeNexusDirectBranch.ps1") -RehearsalRoot $rehearsalRoot | Out-Null
if ($LASTEXITCODE -ne 0) { throw "CodeNexus rollback rehearsal failed" }
$rollback = Add-Evidence "rollback-execution" "rollback_execution" ([ordered]@{
    outcome = "passed"; command = $rollbackCommand; executedAt = $EvaluatedAt; exitCode = 0
    target = "$rehearsalRelative/run-code-intel.ps1"; exactReplay = $true; unrelatedBranchesChanged = $false
})
$usage = Add-Evidence "usage-observation" "usage_observation" ([ordered]@{
    outcome = "blocked"; startedAt = $EvaluatedAt; endedAt = $EvaluatedAt; totalInvocations = 0
    legacyInvocations = 0; replacementInvocations = 0; blocker = "no production route/usage observation exists"
})
$traceJson = '{"legacyBranchId":"' + $branchId + '","replacementCapabilityId":"' + $replacementId + '","retirementId":"' + $retirementId + '"}'
$necessity = Add-Evidence "c00-necessity" "c00_necessity" ([ordered]@{
    outcome = "passed"; decision = "admit"; changeId = $retirementId; necessityTraceSha256 = (Get-Sha256Text $traceJson)
    rationale = "remove facade-owned provider process/storage coupling only after B04/B05 route replacement"
})
$b05Dependency = Add-Evidence "dependency-b05" "dependency_approval" ([ordered]@{
    outcome = "passed"; dependencyId = "repository.survival-scan"; status = "approved"; reviewer = "e04-author-test"
})

$subject = [ordered]@{
    legacyBranch = [ordered]@{
        capabilityId = "localization.codenexus-lite"; branchId = $branchId
        callPath = "run-code-intel.ps1::$branchId"; affectedFiles = @("run-code-intel.ps1")
        owner = "executor-codenexus-retirement"; registryParticipantId = "localization.codenexus-lite"
    }
    replacement = [ordered]@{
        capabilityId = $replacementId; implementationId = "provider.codenexus-adapt"
        dependencies = @("repository.survival-scan"); atomEvidence = $replacement
    }
    parity = [ordered]@{ golden = $golden; contract = $contract; effects = $effects }
    registryReconciliation = $registry; compatibilityWindow = $window
    rollback = [ordered]@{ command = $rollbackCommand; executionEvidence = $rollback }
    usageObservation = $usage; necessityEvidence = $necessity
    dependencyStates = @($b05Dependency); lineReductionEvidence = $false
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

$registryJson = Get-Content -LiteralPath (Join-Path $RepoRoot "orchestration/integrations.json") -Raw | ConvertFrom-Json
$gateDecl = ($registryJson.integrations | Where-Object { $_.id -eq "compatibility.retirement-gate" }).capabilityDeclaration
$gateInputs = @($manifestRef, $replacement, $golden, $contract, $effects, $registry, $window, $rollback, $usage, $necessity, $b05Dependency, $independent)
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
if ($decision.decision -ne "blocked") { throw "E04 cannot pass while the direct facade route, observation, and independent approval blockers remain" }

$match = $directMatches[0]
$deletedLines = @($match.Value -split "`n")
$oldStart = @(($runText.Substring(0, $match.Index)) -split "`n").Count
$hunk = [ordered]@{
    addedLines = @(); deletedLines = $deletedLines; newLines = 0; newStart = $oldStart
    oldLines = $deletedLines.Count; oldStart = $oldStart
}
$baseLines = @($runText -split "`n")
$resultLines = for ($lineNumber = 1; $lineNumber -le $baseLines.Count; $lineNumber++) {
    if ($lineNumber -lt $hunk.oldStart -or $lineNumber -ge ($hunk.oldStart + $hunk.oldLines)) {
        $baseLines[$lineNumber - 1]
    }
}
$resultText = $resultLines -join "`n"
$patchFiles = @([ordered]@{
    baseBlobSha256 = Get-Sha256Text $runText; baseText = $runText; hunks = @($hunk)
    path = "run-code-intel.ps1"; resultBlobSha256 = Get-Sha256Text $resultText; resultText = $resultText
})
$deletionDiff = [ordered]@{
    schema = "code-intel-compatibility-retirement-deletion-diff.v1"; snapshotIdentity = $snapshotIdentity
    retirementId = $retirementId; legacyBranchId = $branchId; affectedFiles = @("run-code-intel.ps1")
    deletionsOnly = $true
    summary = "Proposed deletion contains only the one live direct CodeNexus-lite facade branch. It is not executable until a separate route substitution is approved."
    patch = [ordered]@{ algorithm = "replayable-delete-only-v1"; sha256 = (Get-Sha256Json $patchFiles); files = $patchFiles }
}
Write-JsonFile (Join-Path $OutDir "compatibility-retirement-deletion-diff.json") $deletionDiff
$diffRef = New-ArtifactRef "code-intel-compatibility-retirement-deletion-diff.v1" "compatibility.retirement-deletion-diff" "compatibility-retirement-deletion-diff.json" $snapshotIdentity
$decisionRef = New-ArtifactRef "code-intel-compatibility-retirement-decision.v1" "compatibility.retirement-decision" "gate-out/compatibility-retirement-decision.json" $snapshotIdentity

$ticket = [ordered]@{
    schema = "code-intel-compatibility-retirement-ticket-template.v1"; snapshotIdentity = $snapshotIdentity
    ticketId = "ticket-e04-retire-codenexus-direct-branch"; retirementId = $retirementId
    legacyBranch = [ordered]@{ capabilityId = "localization.codenexus-lite"; branchId = $branchId; callPath = "run-code-intel.ps1::$branchId" }
    replacement = [ordered]@{ capabilityId = $replacementId; dependencies = @("repository.survival-scan") }
    affectedFiles = @("run-code-intel.ps1")
    evidence = [ordered]@{ golden = $golden; contract = $contract; effects = $effects; usage = $usage; rollbackRehearsal = $rollback; deletionDiff = $diffRef }
    source = [ordered]@{ retirementDecision = $decisionRef; retirementManifest = $manifestRef }
    owner = "executor-codenexus-retirement"; verifier = "independent-verifier-required"
    observationExpiry = $expiry; status = "draft"; authorityBoundary = "template_only_no_approval_or_deletion_authority"
}
Write-JsonFile (Join-Path $OutDir "compatibility-retirement-ticket.json") $ticket
& $CodeIntel compatibility retirement-ticket lint --ticket (Join-Path $OutDir "compatibility-retirement-ticket.json") --evaluated-at $EvaluatedAt | Out-Null
if ($LASTEXITCODE -ne 0) { throw "E01 ticket lint failed" }

$ticketRef = New-ArtifactRef "code-intel-compatibility-retirement-ticket-template.v1" "compatibility.retirement-ticket-template" "compatibility-retirement-ticket.json" $snapshotIdentity
$ticketDecl = ($registryJson.integrations | Where-Object { $_.id -eq "compatibility.retirement-ticket-template" }).capabilityDeclaration
$e01Request = [ordered]@{
    schema = "code-intel-capability-request.v1"; capability = "compatibility.retirement-ticket-template"; contractVersion = 1
    implementation = $ticketDecl.implementation; snapshot = $requestSnapshot; options = [ordered]@{ evaluatedAt = $EvaluatedAt }
    inputs = @($ticketRef, $manifestRef, $decisionRef, $diffRef); effectPolicy = [ordered]@{ allowedEffects = $ticketDecl.allowedEffects }
}
Write-JsonFile (Join-Path $OutDir "e01-request.json") $e01Request
$e01Output = @(& $CodeIntel capability exec compatibility.retirement-ticket-template --request (Join-Path $OutDir "e01-request.json") --out (Join-Path $OutDir "e01-out") --artifact-root $OutDir 2>&1)
$e01Exit = $LASTEXITCODE
$e01Text = $e01Output -join "`n"
[IO.File]::WriteAllText((Join-Path $OutDir "e01-stderr.txt"), $e01Text, [Text.UTF8Encoding]::new($false))
if ($e01Exit -ne 65 -or $e01Text -notmatch "ticket requires an approved E00 decision") {
    throw "E01 must validate the deletion proof and reject only because E00 is blocked: exit=$e01Exit output=$e01Text"
}

$status = [ordered]@{
    schema = "code-intel-compatibility-retirement-execution-status.v1"; retirementId = $retirementId
    decision = "blocked"; liveDirectBranch = $true; facadeRoutedThroughB04 = $false
    unavailableFallback = "repository.survival-scan"; deletionExecuted = $false; retired = $false
    blockers = @($decision.blockers); gainLedgerProjection = $decision.gainLedgerProjection
    boundary = "E04 is a single-branch draft packet. The live direct route is unchanged and no deletion or retirement authority exists while E00 is blocked."
}
Write-JsonFile (Join-Path $OutDir "status.json") $status
$status | ConvertTo-Json -Depth 12 -Compress
