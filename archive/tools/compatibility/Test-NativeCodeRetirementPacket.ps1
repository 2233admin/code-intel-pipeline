[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string]$PacketRoot,
    [string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
function Read-Packet([string]$Relative) {
    $path = Join-Path $PacketRoot $Relative
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "packet file missing: $Relative" }
    return Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
}
function Get-Sha256Text([string]$Text) {
    return ([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Text)))).ToLowerInvariant()
}

$branch = "run-code-intel.native-code.embedded"
$replacement = "evidence.native-code"
$callPath = "run-code-intel.ps1::$branch"
$ticket = Read-Packet "compatibility-retirement-ticket.json"
$manifest = Read-Packet "compatibility-retirement-manifest.json"
$decision = Read-Packet "gate-out/compatibility-retirement-decision.json"
$diff = Read-Packet "compatibility-retirement-deletion-diff.json"
$status = Read-Packet "status.json"
$snapshotInputs = @(
    "run-code-intel.ps1", "crates/code-intel-cli/src/native_code_evidence.rs",
    "crates/code-intel-cli/tests/native_code_evidence.rs", "crates/code-intel-cli/tests/dag_run.rs",
    "orchestration/integrations.json"
)
$currentSnapshotIdentity = Get-Sha256Text (($snapshotInputs | ForEach-Object {
    (Get-FileHash -LiteralPath (Join-Path $RepoRoot $_) -Algorithm SHA256).Hash.ToLowerInvariant()
}) -join "`n")
if ($manifest.snapshotIdentity -ne $currentSnapshotIdentity -or $ticket.snapshotIdentity -ne $currentSnapshotIdentity -or
    $decision.snapshotIdentity -ne $currentSnapshotIdentity -or $diff.snapshotIdentity -ne $currentSnapshotIdentity) {
    throw "E07 packet is stale relative to its frozen source set"
}
$rollbackText = [IO.File]::ReadAllText((Join-Path $PacketRoot "rollback-rehearsal/run-code-intel.ps1"))
$currentRunText = [IO.File]::ReadAllText((Join-Path $RepoRoot "run-code-intel.ps1")).Replace("`r`n", "`n").Replace("`r", "`n")
if ($rollbackText -cne $currentRunText) { throw "E07 rollback rehearsal no longer exactly replays the current normalized facade" }

if ($ticket.legacyBranch.branchId -ne $branch -or $ticket.legacyBranch.callPath -ne $callPath -or
    $ticket.legacyBranch.capabilityId -ne "evidence.native-code.embedded") { throw "E07 ticket branch is not exact" }
if (@($ticket.affectedFiles).Count -ne 1 -or $ticket.affectedFiles[0] -ne "run-code-intel.ps1") { throw "E07 ticket includes another file" }
if ($ticket.replacement.capabilityId -ne $replacement -or $manifest.approvalSubject.replacement.capabilityId -ne $replacement) {
    throw "E07 replacement differs from B08"
}
if ($manifest.approvalSubject.legacyBranch.callPath -ne $callPath -or
    @($manifest.approvalSubject.legacyBranch.affectedFiles).Count -ne 1 -or
    $manifest.approvalSubject.legacyBranch.affectedFiles[0] -ne "run-code-intel.ps1") {
    throw "E00 subject does not bind the exact E07 branch"
}

if ($diff.legacyBranchId -ne $branch -or @($diff.affectedFiles).Count -ne 1 -or
    $diff.affectedFiles[0] -ne "run-code-intel.ps1" -or $diff.deletionsOnly -ne $true -or
    $diff.patch.algorithm -ne "replayable-delete-only-v1" -or @($diff.patch.files).Count -ne 1 -or
    @($diff.patch.files[0].hunks).Count -ne 2) { throw "E07 deletion proof is not one bounded replayable branch with two segments" }
$deletedText = @($diff.patch.files[0].hunks | ForEach-Object {
    if ($_.newLines -ne 0 -or @($_.addedLines).Count -ne 0 -or $_.oldLines -le 0) { throw "E07 diff contains additions or an empty deletion" }
    $_.deletedLines -join "`n"
}) -join "`n"
if ($deletedText -notmatch 'function Get-CodeEvidenceLanguage \{' -or
    $deletedText -notmatch 'function New-CodeEvidenceLayer \{' -or
    $deletedText -notmatch '\$codeEvidence = New-CodeEvidenceLayer') { throw "E07 diff omitted an embedded Native Code Evidence segment" }
if ($deletedText -match 'run dag-coordinate|RunCommit|hospital|update-code-intel-index|Invoke-ScopedRepowise|Invoke-CodeNexusLite') {
    throw "E07 diff crossed into A09/publication/index/Hospital/provider branches"
}

$evidence = @(Get-ChildItem -LiteralPath (Join-Path $PacketRoot "evidence") -Filter "*.json" -File | ForEach-Object {
    Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
})
if ($evidence.Count -ne 12) { throw "E07 requires exactly twelve E00 evidence artifacts" }
foreach ($item in $evidence) {
    if ($item.legacyBranchId -ne $branch -or $item.replacementCapabilityId -ne $replacement) { throw "E07 evidence crossed a branch boundary" }
}
$contract = $evidence | Where-Object evidenceClass -eq "contract_parity"
$effects = $evidence | Where-Object evidenceClass -eq "effect_parity"
$registry = $evidence | Where-Object evidenceClass -eq "registry_reconciliation"
if ($contract.details.artifactRefCount -ne 8 -or $contract.details.unsupportedLanguage -ne "unknown" -or
    $contract.details.relationshipPrecision -ne "unknown" -or $contract.details.fabricatedRelationships -ne $false) {
    throw "B08 artifact or unsupported-language boundary is missing"
}
if (@($effects.details.declaredEffects) -join ',' -ne 'repo_read,local_write' -or
    @($effects.details.observedEffects) -join ',' -ne 'repo_read,local_write' -or $effects.details.transport -ne "artifact_ref" -or
    $effects.details.publicationTouched -ne $false -or $effects.details.indexTouched -ne $false -or $effects.details.hospitalTouched -ne $false) {
    throw "B08 effect boundary is not frozen"
}
if ($registry.details.registryAuditOk -ne $true -or $registry.details.status -ne "declared") { throw "B07 registry path is not reconciled" }

if ($decision.decision -ne "blocked" -or $status.decision -ne "blocked" -or
    $status.liveEmbeddedBranch -ne $true -or $status.normalFullRoutedThroughA09 -ne $false -or
    $status.deletionExecuted -ne $false -or $status.retired -ne $false) { throw "E07 blocked state was overstated" }
$requiredBlockers = @("unproven_compatibility_window", "unproven_independent_approval", "unproven_replacement_atom", "unproven_usage_observation")
foreach ($blocker in $requiredBlockers) {
    if (@($status.blockers) -notcontains $blocker) { throw "E07 missing blocker: $blocker" }
}
if (@($status.blockers).Count -ne $requiredBlockers.Count) { throw "E07 contains an unexpected blocker or unproven parity claim" }
$e01 = @(Get-Content -LiteralPath (Join-Path $PacketRoot "e01-stderr.txt"))
$e01EnvelopeLine = $e01 | Where-Object { $_ -match '^\{.*"exitCode"' } | Select-Object -First 1
if ([string]::IsNullOrWhiteSpace([string]$e01EnvelopeLine)) {
    throw "E01 result envelope is missing"
}
$e01Envelope = $e01EnvelopeLine | ConvertFrom-Json
if ([int]$e01Envelope.exitCode -ne 65 -or
    @($e01Envelope.diagnostics).Count -ne 1 -or
    [string]$e01Envelope.diagnostics[0] -ne 'ticket requires an approved E00 decision') {
    throw "E01 did not reject only at the blocked E00 authority boundary"
}

$directFunctionCount = [regex]::Matches($currentRunText, '(?m)^function New-CodeEvidenceLayer \{').Count
$directCallCount = [regex]::Matches($currentRunText, '(?m)^\$codeEvidence = New-CodeEvidenceLayer ').Count
$dagCount = [regex]::Matches($currentRunText, '& \$rustCli run dag-coordinate --repo \$repoPath --out \$dagOut').Count
if ($directFunctionCount -ne 1 -or $directCallCount -ne 1 -or $dagCount -ne 1) { throw "current E07 call graph changed after packet generation" }

[ordered]@{
    ok = $true; retirementId = "retire-native-code-branch"; decision = $status.decision
    liveEmbeddedBranch = $status.liveEmbeddedBranch; normalFullRoutedThroughA09 = $status.normalFullRoutedThroughA09
    deletionExecuted = $status.deletionExecuted; retired = $status.retired; evidenceCount = $evidence.Count
} | ConvertTo-Json -Compress
