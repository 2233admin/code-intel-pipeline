[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$PacketRoot,
    [string]$RepoRoot=(Split-Path (Split-Path $PSScriptRoot -Parent) -Parent)
)
Set-StrictMode -Version Latest
$ErrorActionPreference="Stop"
function J([string]$r){
    $p=Join-Path $PacketRoot $r
    if(-not(Test-Path $p -PathType Leaf)){throw "missing $r"}
    Get-Content $p -Raw|ConvertFrom-Json
}
$t=J "compatibility-retirement-ticket.json"
$m=J "compatibility-retirement-manifest.json"
$d=J "compatibility-retirement-deletion-diff.json"
$g=J "gate-out/compatibility-retirement-decision.json"
$s=J "status.json"
$b="run-code-intel.publication.legacy-staging-marker"
$c="run-code-intel.ps1::$b"
# Frozen snapshot identity for the historical packet (pre-removal of the
# two-hunk legacy publication branch). After architecture convergence removed
# the live branch, this packet is content-anchored like E09 rather than
# re-derived from the live tree.
$frozenSnapshotIdentity="2314ccd84ecf78c128fabfa58cc156e228f8de4e191172e5cbb980ce710801ea"
foreach($artifact in @($t,$m,$g,$d)){
    if($artifact.snapshotIdentity -ne $frozenSnapshotIdentity){
        throw "E05 artifacts do not share the frozen snapshot identity"
    }
}
if($t.legacyBranch.branchId-ne$b-or$t.legacyBranch.callPath-ne$c-or$m.approvalSubject.legacyBranch.callPath-ne$c){throw "E05 branch/callPath mismatch"}
if(@($t.affectedFiles).Count-ne1-or$t.affectedFiles[0]-ne"run-code-intel.ps1"){throw "E05 escaped file boundary"}
if($t.replacement.capabilityId-ne"run.commit"-or$m.approvalSubject.replacement.capabilityId-ne"run.commit"){throw "E05 replacement mismatch"}
if($d.patch.algorithm-ne"replayable-delete-only-v1"-or@($d.patch.files).Count-ne1-or@($d.patch.files[0].hunks).Count-ne2){throw "E05 patch must contain exactly two replayable hunks"}
$base=$d.patch.files[0].baseText
$result=$d.patch.files[0].resultText
if($base-notmatch'\.staging-\$stagingNonce'-or$base-notmatch'run-complete\.json'-or$result-match'\.staging-\$stagingNonce'-or$result-match'run-complete\.json'){throw "E05 patch does not remove exactly legacy staging/marker ownership"}
if($base-match'update-code-intel-index\.ps1'-or$result-match'update-code-intel-index\.ps1'){throw "E05 patch includes index traversal"}
foreach($h in @($d.patch.files[0].hunks)){
    if($h.newLines-ne0-or@($h.addedLines).Count-ne0-or$h.oldLines-le0){throw "E05 patch is not deletion-only"}
}
$e=@(Get-ChildItem (Join-Path $PacketRoot evidence) -Filter *.json -File|ForEach-Object{Get-Content $_.FullName -Raw|ConvertFrom-Json})
if($e.Count-ne12){throw "E05 must have twelve evidence objects"}
if($g.decision-ne"blocked"-or$s.deletionExecuted-ne$false-or$s.retired-ne$false){throw "E05 cannot claim deletion/retirement"}
$stderr=Get-Content (Join-Path $PacketRoot "e01-stderr.txt") -Raw
if($stderr-notmatch'ticket requires an approved E00 decision'){throw "E01 rejection boundary mismatch"}
$rollback=$e|Where-Object evidenceClass -eq rollback_execution
if($rollback.details.exactReplay-ne$true-or$rollback.details.sourceSha256-ne$rollback.details.targetSha256){throw "E05 rollback evidence is not exact"}
$rehearsal=Join-Path $PacketRoot ([string]$rollback.details.target-replace'/',[IO.Path]::DirectorySeparatorChar)
if(-not(Test-Path $rehearsal -PathType Leaf)){throw "E05 rollback rehearsal file is missing"}
$rehearsalHash=(Get-FileHash $rehearsal -Algorithm SHA256).Hash.ToLowerInvariant()
if($rehearsalHash-ne$rollback.details.targetSha256){throw "E05 rollback rehearsal digest drifted from packet evidence"}
# Live facade is no longer required to match the historical source digest once
# the two-hunk branch has been removed (divergence-anchored historical packet).
$boundary=& pwsh -NoLogo -NoProfile -File (Join-Path $RepoRoot "tools\compatibility\Test-PublicationRetirementBoundary.ps1")|ConvertFrom-Json
if($boundary.a09ToA07Connected-ne$false-or$boundary.indexTraversalOwned-ne$false){throw "live boundary changed"}
if(-not (
    ($boundary.legacyHunks -eq 2 -and $boundary.branchAbsent -eq $false) -or
    ($boundary.markerRetired -eq $true)
)){
    throw "live publication boundary is neither the frozen two-hunk branch nor a marker-retired intermediate/final state"
}
[ordered]@{
    ok=$true
    retirementId=$s.retirementId
    decision=$s.decision
    deletionExecuted=$s.deletionExecuted
    retired=$s.retired
    evidenceCount=$e.Count
    legacyHunks=[int]$boundary.legacyHunks
    branchAbsent=[bool]$boundary.branchAbsent
    markerRetired=[bool]$boundary.markerRetired
    a09ToA07Connected=$false
    indexTraversalExcluded=$true
    historicalPacket=$true
}|ConvertTo-Json -Compress
