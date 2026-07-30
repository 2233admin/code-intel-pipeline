[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$PacketRoot,
    [string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Read-Json([string]$RelativePath) {
    $path = Join-Path $PacketRoot $RelativePath
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "packet file is missing: $RelativePath" }
    return Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
}

$ticket = Read-Json "compatibility-retirement-ticket.json"
$manifest = Read-Json "compatibility-retirement-manifest.json"
$decision = Read-Json "gate-out/compatibility-retirement-decision.json"
$diff = Read-Json "compatibility-retirement-deletion-diff.json"
$status = Read-Json "status.json"
$expectedBranch = "run-code-intel.workflow-recommender.inline"
$expectedReplacement = "advisory.workflow-recommend"
$expectedCallPath = "run-code-intel.ps1::$expectedBranch"

if ($ticket.legacyBranch.capabilityId -ne "facade.workflow-recommender.inline" -or
    $ticket.legacyBranch.branchId -ne $expectedBranch -or
    $ticket.legacyBranch.callPath -ne $expectedCallPath) {
    throw "E02 ticket must identify exactly the inline recommender branch and call path"
}
if (@($ticket.affectedFiles).Count -ne 1 -or $ticket.affectedFiles[0] -ne "run-code-intel.ps1") {
    throw "E02 ticket cannot include provider-preflight or any file other than run-code-intel.ps1"
}
if (@($diff.affectedFiles).Count -ne 1 -or $diff.affectedFiles[0] -ne "run-code-intel.ps1" -or
    $diff.legacyBranchId -ne $expectedBranch -or $diff.deletionsOnly -ne $true) {
    throw "E02 deletion diff exceeds the single recommender branch boundary"
}
if ($ticket.replacement.capabilityId -ne $expectedReplacement -or
    $manifest.approvalSubject.replacement.capabilityId -ne $expectedReplacement) {
    throw "E02 replacement must remain advisory.workflow-recommend"
}
if ($manifest.approvalSubject.legacyBranch.branchId -ne $expectedBranch) {
    throw "E00 manifest branch differs from the E02 ticket"
}
if ($manifest.approvalSubject.legacyBranch.callPath -ne $expectedCallPath -or
    @($manifest.approvalSubject.legacyBranch.affectedFiles).Count -ne 1 -or
    $manifest.approvalSubject.legacyBranch.affectedFiles[0] -ne "run-code-intel.ps1") {
    throw "E00 approval subject does not bind the exact E02 call path and file set"
}
if ($diff.patch.algorithm -ne "replayable-delete-only-v1" -or
    @($diff.patch.files).Count -ne 1 -or $diff.patch.files[0].path -ne "run-code-intel.ps1" -or
    @($diff.patch.files[0].hunks).Count -ne 2) {
    throw "E02 deletion proof is not the bounded replayable two-hunk patch"
}
foreach ($hunk in @($diff.patch.files[0].hunks)) {
    if ($hunk.newLines -ne 0 -or @($hunk.addedLines).Count -ne 0 -or $hunk.oldLines -le 0) {
        throw "E02 deletion proof contains an addition/replacement or an empty deletion"
    }
}
$e01Rejection = Get-Content -LiteralPath (Join-Path $PacketRoot "e01-stderr.txt") -Raw
if ($e01Rejection -notmatch "ticket requires an approved E00 decision") {
    throw "E01 did not validate the patch before rejecting the blocked E00 decision"
}

$evidence = @(Get-ChildItem -LiteralPath (Join-Path $PacketRoot "evidence") -Filter "*.json" -File |
    ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json })
if ($evidence.Count -ne 12) { throw "E02 must contain exactly twelve closed E00 evidence artifacts" }
foreach ($item in $evidence) {
    if ($item.legacyBranchId -ne $expectedBranch -or $item.replacementCapabilityId -ne $expectedReplacement) {
        throw "E02 evidence crossed a branch or replacement boundary"
    }
}

$expectedBlockers = @(
    "dependency_approval_set_mismatch",
    "unproven_compatibility_window",
    "unproven_dependency_approval",
    "unproven_independent_approval",
    "unproven_usage_observation"
)
if ($decision.decision -ne "blocked" -or (Compare-Object @($decision.blockers) $expectedBlockers)) {
    throw "E02 decision must retain the current compatibility, usage, D02, and independent-approval blockers"
}
if ($status.decision -ne "blocked" -or $status.deletionExecuted -ne $false -or $status.retired -ne $false) {
    throw "blocked E02 packet cannot claim deletion or retirement"
}

$runText = Get-Content -LiteralPath (Join-Path $RepoRoot "run-code-intel.ps1") -Raw
if ($runText -notmatch 'Invoke-RepowiseProviderProbe\.ps1') {
    throw "provider-preflight marker is absent; E02 scope was violated"
}

[ordered]@{
    ok = $true
    retirementId = $status.retirementId
    decision = $status.decision
    deletionExecuted = $status.deletionExecuted
    retired = $status.retired
    evidenceCount = $evidence.Count
    affectedFiles = @($ticket.affectedFiles)
} | ConvertTo-Json -Depth 5 -Compress
