[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$PacketRoot,[string]$RepoRoot=(Split-Path (Split-Path $PSScriptRoot -Parent) -Parent))
Set-StrictMode -Version Latest
$ErrorActionPreference="Stop"
function Read-Json([string]$Relative) { $p=Join-Path $PacketRoot $Relative; if(-not(Test-Path $p -PathType Leaf)){throw "packet file missing: $Relative"}; Get-Content $p -Raw|ConvertFrom-Json }
$ticket=Read-Json "compatibility-retirement-ticket.json"; $manifest=Read-Json "compatibility-retirement-manifest.json"; $decision=Read-Json "gate-out/compatibility-retirement-decision.json"; $diff=Read-Json "compatibility-retirement-deletion-diff.json"; $status=Read-Json "status.json"
$branch="run-code-intel.provider-preflight.test-wrapper"; $callPath="run-code-intel.ps1::$branch"; $replacement="provider.repowise-adapt"
if($ticket.legacyBranch.branchId-ne$branch-or$ticket.legacyBranch.callPath-ne$callPath-or$ticket.legacyBranch.capabilityId-ne"facade.provider-preflight.test-wrapper"){throw "E03 ticket branch binding mismatch"}
if(@($ticket.affectedFiles).Count-ne1-or$ticket.affectedFiles[0]-ne"run-code-intel.ps1"){throw "E03 ticket file scope escaped"}
if($manifest.approvalSubject.legacyBranch.callPath-ne$callPath-or@($manifest.approvalSubject.legacyBranch.affectedFiles).Count-ne1-or$manifest.approvalSubject.legacyBranch.affectedFiles[0]-ne"run-code-intel.ps1"){throw "E00 subject does not bind exact E03 call path/file"}
if($ticket.replacement.capabilityId-ne$replacement-or$manifest.approvalSubject.replacement.capabilityId-ne$replacement){throw "E03 replacement mismatch"}
if($diff.legacyBranchId-ne$branch-or$diff.patch.algorithm-ne"replayable-delete-only-v1"-or@($diff.patch.files).Count-ne1-or@($diff.patch.files[0].hunks).Count-ne1){throw "E03 historical replayable patch shape mismatch"}
$file=$diff.patch.files[0]; $hunk=$file.hunks[0]
if($file.path-ne"run-code-intel.ps1"-or$file.baseText-notmatch'test-code-intel-provider\.ps1'-or$file.baseText-match'Invoke-RepowiseProviderProbe\.ps1'-or$file.resultText-ne""-or$hunk.newLines-ne0-or@($hunk.addedLines).Count-ne0){throw "E03 diff is not a pure deletion of the historical direct wrapper block"}
$e01=Get-Content (Join-Path $PacketRoot "e01-stderr.txt") -Raw; if($e01-notmatch"ticket requires an approved E00 decision"){throw "E01 did not validate patch before blocked decision"}
$evidence=@(Get-ChildItem (Join-Path $PacketRoot "evidence") -Filter *.json -File|ForEach-Object{Get-Content $_.FullName -Raw|ConvertFrom-Json}); if($evidence.Count-ne12){throw "E03 must contain exactly twelve E00 evidence artifacts"}
foreach($item in $evidence){if($item.legacyBranchId-ne$branch-or$item.replacementCapabilityId-ne$replacement){throw "E03 evidence crossed branch/replacement boundary"}}
if($decision.decision-ne"blocked"-or$status.decision-ne"blocked"-or$status.deletionExecuted-ne$false-or$status.retired-ne$false){throw "E03 cannot claim approved deletion or retirement"}
$live=Get-Content (Join-Path $RepoRoot "run-code-intel.ps1") -Raw
if(@([regex]::Matches($live,'test-code-intel-provider\.ps1')).Count-ne0-or@([regex]::Matches($live,'Invoke-RepowiseProviderProbe\.ps1')).Count-ne1-or$live-notmatch'Index-only repowise will still run'){throw "current correct production probe changed"}
[ordered]@{ok=$true;retirementId=$status.retirementId;decision=$status.decision;deletionExecuted=$status.deletionExecuted;retired=$status.retired;evidenceCount=$evidence.Count;historicalBaseOnly=$true;liveProbeCalls=1}|ConvertTo-Json -Compress
