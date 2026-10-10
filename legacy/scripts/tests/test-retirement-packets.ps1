#requires -Version 7.2

<#
.SYNOPSIS
Runs every compatibility retirement packet verifier plus the two audits that
consume their status files.

.DESCRIPTION
Nothing ran these before. That is how a path-resolution bug introduced by the
archive move, a packet frozen against a working-tree overlay that never existed
as a commit, and five stale packets all survived unnoticed. This suite is the
gate that makes those failures loud.

Historical packets remain frozen evidence, not current retirement approval.
For the three sources changed by #427, require both a complete replay against
their committed historical source and rejection against the current source.
Every other packet must still pass against the current tree.
#>

[CmdletBinding()]
param(
    [string]$LegacyRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),
    [string]$RepoRoot = (Split-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) -Parent)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$packets = @(
    @{ Ticket = "E02"; Verifier = "Test-RecommenderRetirementPacket.ps1";       Packet = "e02-recommender" }
    @{ Ticket = "E03"; Verifier = "Test-ProviderPreflightRetirementPacket.ps1"; Packet = "e03-provider-preflight" }
    @{ Ticket = "E04"; Verifier = "Test-CodeNexusDirectRetirementPacket.ps1";   Packet = "e04-codenexus-direct" }
    @{ Ticket = "E05"; Verifier = "Test-PublicationRetirementPacket.ps1";       Packet = "e05-publication" }
    @{ Ticket = "E07"; Verifier = "Test-NativeCodeRetirementPacket.ps1";        Packet = "e07-native-code" }
    @{ Ticket = "E08"; Verifier = "Test-HospitalRetirementPacket.ps1";          Packet = "e08-hospital" }
    @{ Ticket = "E09"; Verifier = "Test-DoctorWrapperRetirementPacket.ps1";     Packet = "e09-doctor-wrapper" }
    @{ Ticket = "E10"; Verifier = "Test-IndexRetirementPacket.ps1";             Packet = "e10-index" }
)

# #427 changes the replacement sources but does not regenerate old retirement
# attestations. Replay the unchanged packets against their actual committed
# source, and keep their freshness guards rejecting the current tree.
$HistoricalSourceCommit = "d131f3f04b5e7b925f200e609239de6854eb477e"
$KnownBlocked = @{
    E04 = "E04 packet is stale relative to its frozen source set"
    E07 = "E07 packet is stale relative to its frozen source set"
    E08 = "E08 snapshot drift"
}

function Test-HistoricalPacket {
    param([string]$Verifier, [string]$PacketRoot)
    $privateRoot = Join-Path ([IO.Path]::GetTempPath()) ("cip-retirement-historical-" + [Guid]::NewGuid().ToString("N"))
    [void](New-Item -ItemType Directory -Path $privateRoot)
    try {
        $archive = Join-Path $privateRoot "source.zip"
        & git -C $RepoRoot archive --format=zip "--output=$archive" $HistoricalSourceCommit
        if ($LASTEXITCODE -ne 0) { throw "cannot materialize committed historical retirement source" }
        $sourceRoot = Join-Path $privateRoot "source"
        Expand-Archive -LiteralPath $archive -DestinationPath $sourceRoot
        $historicalOutput = @(& pwsh -NoLogo -NoProfile -File $Verifier -PacketRoot $PacketRoot -RepoRoot (Join-Path $sourceRoot "legacy") 2>&1) -join "`n"
        if ($LASTEXITCODE -ne 0) { throw "historical retirement replay failed: $historicalOutput" }
        $evidence = $historicalOutput | ConvertFrom-Json
        if ($evidence.ok -ne $true -or $evidence.decision -ne "blocked" -or
            $evidence.deletionExecuted -ne $false -or $evidence.retired -ne $false) {
            throw "historical replay overstated retirement authority"
        }
        return $evidence
    }
    finally {
        Remove-Item -LiteralPath $privateRoot -Recurse -Force
    }
}

$failures = [Collections.Generic.List[string]]::new()

foreach ($packet in $packets) {
    $verifier = Join-Path $LegacyRoot "tools/compatibility/$($packet.Verifier)"
    $packetRoot = Join-Path $RepoRoot "orchestration/retirements/$($packet.Packet)"
    if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
        $failures.Add("$($packet.Ticket): verifier is missing: $verifier")
        continue
    }
    if (-not (Test-Path -LiteralPath $packetRoot -PathType Container)) {
        $failures.Add("$($packet.Ticket): packet directory is missing: $packetRoot")
        continue
    }

    $output = @(& pwsh -NoLogo -NoProfile -File $verifier -PacketRoot $packetRoot 2>&1) -join "`n"
    $succeeded = $LASTEXITCODE -eq 0

    if ($KnownBlocked.ContainsKey($packet.Ticket)) {
        $expected = $KnownBlocked[$packet.Ticket]
        if ($succeeded) {
            $failures.Add("$($packet.Ticket): frozen evidence unexpectedly authorizes the current changed source")
        }
        elseif ($output -notlike "*$expected*") {
            $failures.Add("$($packet.Ticket): failed for a new reason. Expected '$expected'. Got: $output")
        }
        else {
            try {
                $historicalEvidence = Test-HistoricalPacket -Verifier $verifier -PacketRoot $packetRoot
                Write-Output "HISTORICAL-PASS / CURRENT-STALE-REJECTED $($packet.Ticket) $($packet.Packet): source=$HistoricalSourceCommit decision=$($historicalEvidence.decision)"
            }
            catch {
                $failures.Add("$($packet.Ticket): $($_.Exception.Message)")
            }
        }
        continue
    }

    if (-not $succeeded) {
        $failures.Add("$($packet.Ticket): $output")
    }
    else {
        Write-Output "PASS $($packet.Ticket) $($packet.Packet)"
    }
}

# These two consume the packet status files, so they belong in the same gate:
# an out-of-band retirement or a regenerated packet must keep them coherent.
$audits = @(
    @{ Name = "final commitment reconciliation"; Script = Join-Path $LegacyRoot "tools/Test-FinalCommitmentReconciliation.ps1" }
    @{ Name = "compatibility facade finalize";   Script = Join-Path $LegacyRoot "scripts/tests/test-compatibility-facade-finalize.ps1" }
)
foreach ($audit in $audits) {
    $output = @(& pwsh -NoLogo -NoProfile -File $audit.Script 2>&1) -join "`n"
    if ($LASTEXITCODE -ne 0) { $failures.Add("$($audit.Name): $output") }
    else { Write-Output "PASS $($audit.Name)" }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) { [Console]::Error.WriteLine("FAIL $failure") }
    throw "retirement packet suite failed: $($failures.Count) of $($packets.Count + $audits.Count) checks"
}

Write-Output "Retirement packet suite passed: $($packets.Count) packets, $($audits.Count) audits, $($KnownBlocked.Count) known-blocked"
