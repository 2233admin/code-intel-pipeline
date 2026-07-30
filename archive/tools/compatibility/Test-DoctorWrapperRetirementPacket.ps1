[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string]$PacketRoot,
    [string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent)
)

Set-StrictMode -Version Latest
# $RepoRoot lands on archive/ after the PowerShell move; assets that
# stayed behind (orchestration/, crates/) live one level above it
$PipelineRepoRoot = Split-Path -Parent $RepoRoot
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
# E09 is a historical record, so it is not re-derived from the live tree.
# The packet was frozen against an explicit working-tree overlay rather than a
# commit: no revision in this repository reproduces $frozenSnapshotIdentity,
# because invoke-code-intel.ps1 was overlaid from f3a4e867 while three of the
# six frozen inputs did not exist at that commit. Re-hashing live sources would
# therefore always report drift and could never report anything else. The packet
# is anchored to its own content-bound artifacts and to the recorded deletion.
$frozenSnapshotIdentity = "ff712225ab6fea6d458a660b15350f3fcc75b8158c24ed7d995f6a786171365a"
foreach ($artifact in @($ticket, $manifest, $decision, $diff)) {
    if ($artifact.snapshotIdentity -ne $frozenSnapshotIdentity) { throw "E09 artifacts do not share one frozen snapshot identity" }
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
if ($rollback -cne $patchFile.baseText) {
    throw "E09 rollback rehearsal is not the exact pre-deletion wrapper the diff was cut from"
}
if ($rollback -ceq $liveInvoke) {
    throw "E09 is recorded as retired but the live wrapper still matches the pre-deletion text"
}

$evidence = @(Get-ChildItem -LiteralPath (Join-Path $PacketRoot "evidence") -Filter "*.json" -File | ForEach-Object {
    Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
})
if ($evidence.Count -ne 12) { throw "E09 requires the eleven E00 evidence artifacts plus the out-of-band deletion record" }
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
$outOfBand = $evidence | Where-Object evidenceClass -eq "out_of_band_deletion"
if ($null -eq $outOfBand) { throw "E09 is retired without an out-of-band deletion record" }
if ($outOfBand.details.gateAuthorityObtained -ne $false -or $outOfBand.details.governanceBypass -ne $true -or
    $outOfBand.details.gateDecisionAtDeletion -ne "blocked" -or $outOfBand.details.gateDecisionSuperseded -ne $false -or
    $outOfBand.details.packetBornStale -ne $true -or $outOfBand.details.snapshotIdentityCommitReachable -ne $false -or
    $outOfBand.details.bootstrapRetained -ne $true) {
    throw "E09 out-of-band deletion record overstates its authority"
}
foreach ($field in @("authoringCommit", "landedOnMainVia", "packetIntroducedBy", "baseTextCommit")) {
    $sha = [string]$outOfBand.details.$field
    if ($sha -notmatch '^[0-9a-f]{40}$') { throw "E09 out-of-band record has no resolvable $field commit" }
    & git -C $RepoRoot cat-file -e "$sha^{commit}" 2>$null
    if ($LASTEXITCODE -ne 0) { throw "E09 out-of-band record cites a commit that is not in this repository: $field=$sha" }
}
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
if ($registryEvidence.details.registryAuditOk -ne $true -or $registryEvidence.details.owner -ne "code-intel-pipeline") {
    throw "retained bootstrap ownership changed"
}
# E09 never had authority over check-code-intel-tools.ps1, so the historical
# record only requires that the retirement did not take the bootstrap with the
# branch. Its content has drifted since the packet was frozen, which is expected
# for a live script, so the frozen hash is not asserted against the working tree.
if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot "check-code-intel-tools.ps1") -PathType Leaf)) {
    throw "retained bootstrap was deleted; E09 never had authority over it"
}

$registry = Get-Content -LiteralPath (Join-Path $PipelineRepoRoot "orchestration/integrations.json") -Raw | ConvertFrom-Json
$doctor = @($registry.integrations | Where-Object id -eq "doctor")
if ($doctor.Count -ne 1 -or $doctor[0].owner -ne "code-intel-pipeline" -or
    [string]$doctor[0].extensionPoint -notmatch 'observation-only bootstrap') {
    throw "B07 doctor registration no longer declares the bootstrap non-authoritative"
}
# The E00 gate output is never rewritten. It still reads blocked, and that
# disagreement with the retired status is the point of the historical record:
# the branch was removed without the gate, not because the gate was satisfied.
if ($decision.decision -ne "blocked") { throw "E09 gate decision must remain the unaltered blocked record" }
if ($status.decision -ne "retired_out_of_band" -or $status.gateDecision -ne "blocked" -or
    $status.gateAuthorityObtained -ne $false -or $status.governanceBypass -ne $true -or
    $status.retirementBasis -ne "historical" -or $status.retired -ne $true -or
    $status.deletionExecuted -ne $true -or $status.liveDirectDoctorRoute -ne $false -or
    $status.publicPreflightUsesB10 -ne $false -or
    $status.bootstrapRetained -ne $true -or $status.bootstrapNonAuthoritative -ne $true -or
    $status.bootstrapRegistered -ne $true -or $status.bootstrapOwned -ne $true -or
    $status.bootstrapExpiring -ne $false) {
    throw "E09 historical retirement state is misrecorded"
}
if ($status.deletionExecutedBy -ne $outOfBand.details.authoringCommit -or
    $status.deletionLandedOnMainVia -ne $outOfBand.details.landedOnMainVia) {
    throw "E09 status and out-of-band evidence disagree about which commit removed the branch"
}
if (@($status.blockers).Count -ne 0) { throw "a retired packet cannot still carry live gate blockers" }
$unmetBlockers = @("unproven_compatibility_window", "unproven_independent_approval", "unproven_replacement_atom", "unproven_usage_observation")
if (@($status.unmetGateBlockersAtDeletion).Count -ne $unmetBlockers.Count) {
    throw "E09 must preserve exactly the gate blockers that were unmet when the branch was deleted"
}
foreach ($blocker in $unmetBlockers) {
    if (@($status.unmetGateBlockersAtDeletion) -notcontains $blocker) { throw "E09 missing unmet gate blocker: $blocker" }
}
$e01 = Get-Content -LiteralPath (Join-Path $PacketRoot "e01-stderr.txt") -Raw
if ($e01 -notmatch '"exitCode":65' -or $e01 -notmatch 'ticket requires an approved E00 decision') {
    throw "E01 did not reject only at the blocked E00 authority boundary"
}
$directAssignmentCount = [regex]::Matches($liveInvoke, '(?m)^\$doctor = Join-Path \$root "check-code-intel-tools\.ps1"$').Count
$directInvocationCount = [regex]::Matches($liveInvoke, '(?m)^        & \$doctor -Config \$Config').Count
$missingGuardCount = [regex]::Matches($liveInvoke, '(?m)^    throw "Doctor script missing: \$doctor"$').Count
if ($directAssignmentCount -ne 0 -or $directInvocationCount -ne 0 -or $missingGuardCount -ne 0) {
    throw "E09 is recorded as retired but the direct production doctor route is still present"
}
if ($liveInvoke -match '(?i)doctor') { throw "E09 is recorded as retired but the live wrapper still mentions doctor" }
$liveInvokeHash = Get-Sha256Text $liveInvoke
if ($outOfBand.details.liveWrapperSha256 -ne $liveInvokeHash) {
    throw "the live wrapper changed since the out-of-band deletion was recorded; re-verify and update the record"
}

[ordered]@{
    ok = $true; retirementId = "retire-doctor-wrapper-branch"; decision = $status.decision
    gateDecision = $status.gateDecision; gateAuthorityObtained = $status.gateAuthorityObtained
    governanceBypass = $status.governanceBypass; retirementBasis = $status.retirementBasis
    liveDirectDoctorRoute = $status.liveDirectDoctorRoute; publicPreflightUsesB10 = $status.publicPreflightUsesB10
    bootstrapRetained = $status.bootstrapRetained; bootstrapExpiring = $status.bootstrapExpiring
    deletionExecuted = $status.deletionExecuted; deletionExecutedBy = $status.deletionExecutedBy
    retired = $status.retired; evidenceCount = $evidence.Count
} | ConvertTo-Json -Compress
