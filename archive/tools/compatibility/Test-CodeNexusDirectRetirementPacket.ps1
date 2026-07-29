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

$branch = "run-code-intel.codenexus-lite.direct"
$replacement = "provider.codenexus-adapt"
$callPath = "run-code-intel.ps1::$branch"
$ticket = Read-Packet "compatibility-retirement-ticket.json"
$manifest = Read-Packet "compatibility-retirement-manifest.json"
$decision = Read-Packet "gate-out/compatibility-retirement-decision.json"
$diff = Read-Packet "compatibility-retirement-deletion-diff.json"
$status = Read-Packet "status.json"
$snapshotInputs = @(
    "run-code-intel.ps1", "Invoke-CodeNexusLite.ps1", "crates/code-intel-cli/src/codenexus_adapter.rs",
    "crates/code-intel-cli/src/survival_scan.rs", "orchestration/integrations.json"
)
$currentSnapshotIdentity = Get-Sha256Text (($snapshotInputs | ForEach-Object {
    (Get-FileHash -LiteralPath (Join-Path $RepoRoot $_) -Algorithm SHA256).Hash.ToLowerInvariant()
}) -join "`n")
if ($manifest.snapshotIdentity -ne $currentSnapshotIdentity -or $ticket.snapshotIdentity -ne $currentSnapshotIdentity -or
    $decision.snapshotIdentity -ne $currentSnapshotIdentity -or $diff.snapshotIdentity -ne $currentSnapshotIdentity) {
    throw "E04 packet is stale relative to its frozen source set"
}
$rollbackText = [IO.File]::ReadAllText((Join-Path $PacketRoot "rollback-rehearsal/run-code-intel.ps1"))
$currentRunText = [IO.File]::ReadAllText((Join-Path $RepoRoot "run-code-intel.ps1")).Replace("`r`n", "`n").Replace("`r", "`n")
if ($rollbackText -cne $currentRunText) { throw "E04 rollback rehearsal no longer exactly replays the current normalized facade" }

if ($ticket.legacyBranch.branchId -ne $branch -or $ticket.legacyBranch.callPath -ne $callPath -or
    $ticket.legacyBranch.capabilityId -ne "localization.codenexus-lite") { throw "E04 ticket branch is not exact" }
if (@($ticket.affectedFiles).Count -ne 1 -or $ticket.affectedFiles[0] -ne "run-code-intel.ps1") { throw "E04 ticket includes another branch or file" }
if ($ticket.replacement.capabilityId -ne $replacement -or $manifest.approvalSubject.replacement.capabilityId -ne $replacement) { throw "E04 replacement differs from B04" }
if ($manifest.approvalSubject.legacyBranch.callPath -ne $callPath -or
    @($manifest.approvalSubject.legacyBranch.affectedFiles).Count -ne 1 -or
    $manifest.approvalSubject.legacyBranch.affectedFiles[0] -ne "run-code-intel.ps1") { throw "E00 subject does not bind the exact E04 branch" }

if ($diff.legacyBranchId -ne $branch -or @($diff.affectedFiles).Count -ne 1 -or
    $diff.affectedFiles[0] -ne "run-code-intel.ps1" -or $diff.deletionsOnly -ne $true -or
    $diff.patch.algorithm -ne "replayable-delete-only-v1" -or @($diff.patch.files).Count -ne 1 -or
    @($diff.patch.files[0].hunks).Count -ne 1) { throw "E04 deletion proof is not one bounded replayable branch" }
$hunk = $diff.patch.files[0].hunks[0]
if ($hunk.newLines -ne 0 -or @($hunk.addedLines).Count -ne 0 -or $hunk.oldLines -le 0) { throw "E04 diff contains additions or an empty deletion" }
if (($hunk.deletedLines -join "`n") -notmatch 'Invoke-CodeNexusLite\.ps1' -or
    ($hunk.deletedLines -join "`n") -match 'Invoke-RepowiseProviderProbe|repository survival-scan|provider codenexus-adapt') {
    throw "E04 diff crossed its direct CodeNexus-lite branch boundary"
}

$evidence = @(Get-ChildItem -LiteralPath (Join-Path $PacketRoot "evidence") -Filter "*.json" -File | ForEach-Object {
    Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
})
if ($evidence.Count -ne 11) { throw "E04 requires exactly eleven E00 evidence artifacts" }
foreach ($item in $evidence) {
    if ($item.legacyBranchId -ne $branch -or $item.replacementCapabilityId -ne $replacement) { throw "E04 evidence crossed a branch boundary" }
}
$contract = $evidence | Where-Object evidenceClass -eq "contract_parity"
$effects = $evidence | Where-Object evidenceClass -eq "effect_parity"
if ($contract.details.PSObject.Properties.Name -contains "fixtures") { throw "fixture identities belong in golden evidence, not contract prose" }
if ($contract.details.unavailableRoute -ne "repository.survival-scan" -or
    $contract.details.structuralVerdictWhenUnavailable -ne "unknown") { throw "B05 fallback or unknown structural boundary is missing" }
if ($effects.details.processOwnership -ne "provider" -or $effects.details.storageOwnership -ne "provider" -or
    $effects.details.transport -ne "artifact_ref" -or $effects.details.effectSetsRemainModeSpecific -ne $true) { throw "B04 process/storage/effect boundary is not frozen" }

if ($decision.decision -ne "blocked" -or $status.decision -ne "blocked" -or
    $status.liveDirectBranch -ne $true -or $status.facadeRoutedThroughB04 -ne $false -or
    $status.deletionExecuted -ne $false -or $status.retired -ne $false) { throw "E04 blocked state was overstated" }
$requiredBlockers = @("unproven_compatibility_window", "unproven_independent_approval", "unproven_replacement_atom", "unproven_usage_observation")
foreach ($blocker in $requiredBlockers) { if ($blocker -notin @($decision.blockers)) { throw "E04 blocker missing: $blocker" } }
$e01 = Get-Content -LiteralPath (Join-Path $PacketRoot "e01-stderr.txt") -Raw
if ($e01 -notmatch "ticket requires an approved E00 decision") { throw "E01 did not reject the blocked E00 decision after validating the packet" }

$run = Get-Content -LiteralPath (Join-Path $RepoRoot "run-code-intel.ps1") -Raw
if ([regex]::Matches($run, 'Join-Path \$PSScriptRoot "Invoke-CodeNexusLite\.ps1"').Count -ne 1) { throw "live direct branch changed despite blocked E04" }
if ([regex]::Matches($run, 'provider codenexus-adapt').Count -ne 1 -or [regex]::Matches($run, 'repository survival-scan').Count -ne 1) { throw "B04/B05 facades are absent or ambiguous" }

[ordered]@{
    ok = $true; retirementId = $status.retirementId; decision = $status.decision
    liveDirectBranch = $status.liveDirectBranch; facadeRoutedThroughB04 = $status.facadeRoutedThroughB04
    deletionExecuted = $status.deletionExecuted; retired = $status.retired; evidenceCount = $evidence.Count
} | ConvertTo-Json -Depth 5 -Compress
