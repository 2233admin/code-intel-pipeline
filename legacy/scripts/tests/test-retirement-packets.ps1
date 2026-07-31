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

Every packet is expected to pass. The one exception is declared in
$KnownBlocked with the exact message it must fail with, so that neither a new
failure mode nor an accidental fix can pass silently.
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

# Nothing is known-blocked right now. E05 was, because dag-coordinate exited 10
# when diagnosis.hospital reported domain_failed on any repository; 805ef28
# ("an ungoverned repository is not an architecture gate failure") fixed that,
# so E05 regenerates and verifies again and was removed from this table.
#
# To park a packet here, map its ticket id to the exact error it must fail with.
# The suite then fails if that packet passes, or fails differently — a parked
# lane cannot rot silently, and an accidental fix cannot go unnoticed.
$KnownBlocked = @{}

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
            $failures.Add("$($packet.Ticket): is recorded as known-blocked but now passes; remove it from `$KnownBlocked and drop the follow-up")
        }
        elseif ($output -notlike "*$expected*") {
            $failures.Add("$($packet.Ticket): failed for a new reason. Expected '$expected'. Got: $output")
        }
        else {
            Write-Output "KNOWN-BLOCKED $($packet.Ticket) $($packet.Packet): $expected"
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
