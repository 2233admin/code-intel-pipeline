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
    Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
}
function Get-Sha256Text([string]$Text) {
    ([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Text)))).ToLowerInvariant()
}

$branch = "invoke-code-intel.doctor.direct-production"
$replacement = "doctor"
$callPath = "invoke-code-intel.ps1::$branch"
$ticket = Read-Packet "compatibility-retirement-ticket.json"
$manifest = Read-Packet "compatibility-retirement-manifest.json"
$decision = Read-Packet "gate-out/compatibility-retirement-decision.json"
$diff = Read-Packet "compatibility-retirement-deletion-diff.json"
$status = Read-Packet "status.json"
$snapshotInputs = @(
    "invoke-code-intel.ps1", "check-code-intel-tools.ps1",
    "crates/code-intel-cli/src/doctor_adapter.rs", "crates/code-intel-cli/tests/doctor_envelope.rs",
    "crates/code-intel-cli/src/dag_run.rs", "orchestration/integrations.json"
)
$currentSnapshotIdentity = Get-Sha256Text (($snapshotInputs | ForEach-Object {
    (Get-FileHash -LiteralPath (Join-Path $RepoRoot $_) -Algorithm SHA256).Hash.ToLowerInvariant()
}) -join "`n")
foreach ($artifact in @($ticket, $manifest, $decision, $diff)) {
    if ($artifact.snapshotIdentity -ne $currentSnapshotIdentity) { throw "E09 packet is stale relative to its frozen source set" }
}

if ($ticket.legacyBranch.branchId -ne $branch -or $ticket.legacyBranch.callPath -ne $callPath -or
    $ticket.legacyBranch.capabilityId -ne "doctor.bootstrap.direct-production") { throw "E09 ticket branch is not exact" }
if (@($ticket.affectedFiles).Count -ne 1 -or $ticket.affectedFiles[0] -ne "invoke-code-intel.ps1") {
    throw "E09 ticket includes another file"
}
if ($manifest.approvalSubject.replacement.capabilityId -ne $replacement -or
    $manifest.approvalSubject.replacement.implementationId -ne "doctor.envelope.compat" -or
    @($manifest.approvalSubject.replacement.dependencies) -notcontains "repo.snapshot") {
    throw "E09 replacement differs from B10"
}
if ($manifest.approvalSubject.legacyBranch.callPath -ne $callPath -or
    @($manifest.approvalSubject.legacyBranch.affectedFiles).Count -ne 1 -or
    $manifest.approvalSubject.legacyBranch.affectedFiles[0] -ne "invoke-code-intel.ps1") {
    throw "E00 subject does not bind the exact E09 branch"
}

if ($diff.legacyBranchId -ne $branch -or @($diff.affectedFiles).Count -ne 1 -or
    $diff.affectedFiles[0] -ne "invoke-code-intel.ps1" -or $diff.deletionsOnly -ne $true -or
    $diff.patch.algorithm -ne "replayable-delete-only-v1" -or @($diff.patch.files).Count -ne 1 -or
    @($diff.patch.files[0].hunks).Count -ne 3) {
    throw "E09 deletion proof is not one bounded replayable branch with three segments"
}
$patchFile = $diff.patch.files[0]
if ($patchFile.path -ne "invoke-code-intel.ps1" -or $patchFile.baseBlobSha256 -ne (Get-Sha256Text $patchFile.baseText)) {
    throw "E09 base text is not content bound"
}
$deletedText = @($patchFile.hunks | ForEach-Object {
    if ($_.newLines -ne 0 -or @($_.addedLines).Count -ne 0 -or $_.oldLines -le 0) { throw "E09 diff contains additions or an empty deletion" }
    $_.deletedLines -join "`n"
}) -join "`n"
if ($deletedText -notmatch '\$doctor = Join-Path \$root "check-code-intel-tools\.ps1"' -or
    $deletedText -notmatch '& \$doctor -Config \$Config' -or
    $deletedText -notmatch 'Doctor script missing: \$doctor') {
    throw "E09 diff omitted a direct production doctor route segment"
}
if ($deletedText -match 'run-code-intel|update-code-intel-index|Invoke-CodeNexusLite|New-Hospital|evidence.native-code') {
    throw "E09 diff crossed into another wrapper branch"
}
$baseLines = @($patchFile.baseText -split "`n")
$removed = [Collections.Generic.HashSet[int]]::new()
foreach ($hunk in @($patchFile.hunks)) {
    for ($line = [int]$hunk.oldStart; $line -lt ([int]$hunk.oldStart + [int]$hunk.oldLines); $line++) {
        if (-not $removed.Add($line)) { throw "E09 deletion hunks overlap" }
    }
}
$replayed = (@(for ($line = 1; $line -le $baseLines.Count; $line++) {
    if (-not $removed.Contains($line)) { $baseLines[$line - 1] }
}) -join "`n")
if ($replayed -cne $patchFile.resultText -or (Get-Sha256Text $replayed) -ne $patchFile.resultBlobSha256) {
    throw "E09 deletion patch does not replay to its declared result"
}

$liveInvoke = [IO.File]::ReadAllText((Join-Path $RepoRoot "invoke-code-intel.ps1")).Replace("`r`n", "`n").Replace("`r", "`n")
$rollback = [IO.File]::ReadAllText((Join-Path $PacketRoot "rollback-rehearsal/invoke-code-intel.ps1"))
if ($patchFile.baseText -cne $liveInvoke -or $rollback -cne $liveInvoke) {
    throw "E09 rollback rehearsal does not exactly replay the current normalized wrapper"
}

$evidence = @(Get-ChildItem -LiteralPath (Join-Path $PacketRoot "evidence") -Filter "*.json" -File | ForEach-Object {
    Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
})
if ($evidence.Count -ne 11) { throw "E09 requires exactly eleven E00 evidence artifacts" }
foreach ($item in $evidence) {
    if ($item.legacyBranchId -ne $branch -or $item.replacementCapabilityId -ne $replacement) {
        throw "E09 evidence crossed a branch boundary"
    }
}
$atom = $evidence | Where-Object evidenceClass -eq "replacement_atom"
$golden = $evidence | Where-Object evidenceClass -eq "golden_parity"
$contract = $evidence | Where-Object evidenceClass -eq "contract_parity"
$effects = $evidence | Where-Object evidenceClass -eq "effect_parity"
$registryEvidence = $evidence | Where-Object evidenceClass -eq "registry_reconciliation"
if ($atom.details.outcome -ne "blocked" -or $atom.details.b10EnvelopeAvailable -ne $true -or
    $atom.details.publicWrapperUsesDirectDoctor -ne $true -or $atom.details.bootstrapNonAuthoritative -ne $true -or
    $atom.details.bootstrapRegistered -ne $true -or $atom.details.bootstrapOwner -ne "code-intel-pipeline" -or
    $atom.details.bootstrapExpiryDeclared -ne $false) { throw "E09 replacement/bootstrap boundary is overstated" }
if ($golden.details.outcome -ne "passed" -or $golden.details.executedTestCount -ne 3 -or
    $golden.details.singleResultDocument -ne $true -or $golden.details.manifestDriftFixture -ne $true -or
    $golden.details.presentNonconformingFixture -ne $true) { throw "B10 route fixtures are not frozen" }
if ($contract.details.readinessConformanceSeparated -ne $true -or $contract.details.stdoutDocumentCount -ne 1 -or
    $contract.details.admissibilityNotPromoted -ne $true -or $effects.details.secretRedacted -ne $true) {
    throw "B10 envelope, domain, or redaction contract is missing"
}
$bootstrapHash = (Get-FileHash -LiteralPath (Join-Path $RepoRoot "check-code-intel-tools.ps1") -Algorithm SHA256).Hash.ToLowerInvariant()
if ($registryEvidence.details.registryAuditOk -ne $true -or $registryEvidence.details.owner -ne "code-intel-pipeline" -or
    $registryEvidence.details.bootstrapHash -ne $bootstrapHash) { throw "retained bootstrap ownership or hash changed" }

$registry = Get-Content -LiteralPath (Join-Path $RepoRoot "orchestration/integrations.json") -Raw | ConvertFrom-Json
$doctor = @($registry.integrations | Where-Object id -eq "doctor")
if ($doctor.Count -ne 1 -or $doctor[0].owner -ne "code-intel-pipeline" -or
    [string]$doctor[0].extensionPoint -notmatch 'observation-only bootstrap') {
    throw "B07 doctor registration no longer declares the bootstrap non-authoritative"
}
if ($decision.decision -ne "blocked" -or $status.decision -ne "blocked" -or
    $status.liveDirectDoctorRoute -ne $true -or $status.publicPreflightUsesB10 -ne $false -or
    $status.bootstrapRetained -ne $true -or $status.bootstrapNonAuthoritative -ne $true -or
    $status.bootstrapRegistered -ne $true -or $status.bootstrapOwned -ne $true -or
    $status.bootstrapExpiring -ne $false -or $status.deletionExecuted -ne $false -or $status.retired -ne $false) {
    throw "E09 blocked state was overstated"
}
$requiredBlockers = @("unproven_compatibility_window", "unproven_independent_approval", "unproven_replacement_atom", "unproven_usage_observation")
if (@($status.blockers).Count -ne $requiredBlockers.Count) { throw "E09 contains an unexpected blocker or unproven parity claim" }
foreach ($blocker in $requiredBlockers) {
    if (@($status.blockers) -notcontains $blocker) { throw "E09 missing blocker: $blocker" }
}
$e01 = Get-Content -LiteralPath (Join-Path $PacketRoot "e01-stderr.txt") -Raw
if ($e01 -notmatch '"exitCode":65' -or $e01 -notmatch 'ticket requires an approved E00 decision') {
    throw "E01 did not reject only at the blocked E00 authority boundary"
}
$directAssignmentCount = [regex]::Matches($liveInvoke, '(?m)^\$doctor = Join-Path \$root "check-code-intel-tools\.ps1"$').Count
$directInvocationCount = [regex]::Matches($liveInvoke, '(?m)^        & \$doctor -Config \$Config').Count
$missingGuardCount = [regex]::Matches($liveInvoke, '(?m)^    throw "Doctor script missing: \$doctor"$').Count
if ($directAssignmentCount -ne 1 -or $directInvocationCount -ne 2 -or $missingGuardCount -ne 1) {
    throw "current E09 public doctor route changed after packet generation"
}

[ordered]@{
    ok = $true; retirementId = "retire-doctor-wrapper-branch"; decision = $status.decision
    liveDirectDoctorRoute = $status.liveDirectDoctorRoute; publicPreflightUsesB10 = $status.publicPreflightUsesB10
    bootstrapRetained = $status.bootstrapRetained; bootstrapExpiring = $status.bootstrapExpiring
    deletionExecuted = $status.deletionExecuted; retired = $status.retired; evidenceCount = $evidence.Count
} | ConvertTo-Json -Compress
