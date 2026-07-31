#requires -Version 7.2

[CmdletBinding()]
param(
    [string]$RepoRoot = $PSScriptRoot,
    [string]$PolicyPath = "orchestration/facade-finalize-policy.v1.json",
    [string]$FixtureRoot = "orchestration/facade-finalize-fixtures",
    [long]$EvaluatedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds(),
    [string]$OutFile = "",
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-RepoFile {
    param([string]$Path)
    if ([System.IO.Path]::IsPathRooted($Path)) { return $Path }
    return Join-Path $RepoRoot ($Path -replace '/', [System.IO.Path]::DirectorySeparatorChar)
}

function Read-JsonFile {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Assert-ExactProperties {
    param([object]$Value, [string[]]$Names, [string]$Context)
    if ($null -eq $Value) { throw "$Context is missing" }
    $actual = @($Value.PSObject.Properties.Name | Sort-Object)
    $expected = @($Names | Sort-Object)
    if (($actual -join "`n") -ne ($expected -join "`n")) {
        throw "$Context must have exactly [$($expected -join ', ')], got [$($actual -join ', ')]"
    }
}

function Add-Unsupported {
    param(
        [System.Collections.Generic.List[object]]$List,
        [string]$Code,
        [string]$Subject,
        [string]$Dependency,
        [string]$Detail
    )
    $List.Add([pscustomobject][ordered]@{
        code = $Code
        subject = $Subject
        dependency = $Dependency
        detail = $Detail
    })
}

if ($EvaluatedAt -le 0) { throw "EvaluatedAt must be a positive Unix timestamp" }
$RepoRoot = (Get-Item -LiteralPath $RepoRoot -ErrorAction Stop).FullName
$policy = Read-JsonFile (Resolve-RepoFile $PolicyPath)
Assert-ExactProperties $policy @("schema", "ticketId", "dependencyTickets", "retainedPowerShell", "modeMatrix", "parity") "policy"
if ([string]$policy.schema -ne "code-intel-compatibility-facade-finalize-policy.v1" -or [string]$policy.ticketId -ne "E06") {
    throw "policy identity mismatch"
}
Assert-ExactProperties $policy.parity @("a00Evidence", "rollbackTickets") "policy parity"

$unsupported = [System.Collections.Generic.List[object]]::new()
$dependencies = [System.Collections.Generic.List[object]]::new()
$ticketIds = @($policy.dependencyTickets | ForEach-Object { [string]$_.ticketId })
if ($ticketIds.Count -ne 8 -or @($ticketIds | Sort-Object -Unique).Count -ne $ticketIds.Count) {
    throw "E06 policy must contain exactly eight unique dependency tickets"
}

foreach ($ticket in @($policy.dependencyTickets)) {
    Assert-ExactProperties $ticket @("ticketId", "directory", "branchId", "path") "dependency ticket"
    $ticketId = [string]$ticket.ticketId
    $packetRoot = Resolve-RepoFile ([string]$ticket.directory)
    $statusPath = Join-Path $packetRoot "status.json"
    $manifestPath = Join-Path $packetRoot "compatibility-retirement-manifest.json"
    $status = Read-JsonFile $statusPath
    $manifest = Read-JsonFile $manifestPath
    $packetBlockers = @()
    $observedDecision = "missing"
    $retired = $false
    $deletionExecuted = $false
    $owner = ""

    if ($null -eq $status -or $null -eq $manifest) {
        Add-Unsupported $unsupported "retirement_packet_missing" ([string]$ticket.branchId) $ticketId "status.json or compatibility-retirement-manifest.json is missing"
    }
    else {
        $observedDecision = [string]$status.decision
        $retired = [bool]$status.retired
        $deletionExecuted = [bool]$status.deletionExecuted
        $packetBlockers = @($status.blockers | ForEach-Object { [string]$_ })
        $owner = [string]$manifest.approvalSubject.legacyBranch.owner
        if ([string]$manifest.approvalSubject.legacyBranch.branchId -ne [string]$ticket.branchId) {
            Add-Unsupported $unsupported "retirement_subject_mismatch" ([string]$ticket.branchId) $ticketId "packet branchId does not match the frozen E06 subject"
        }
        if ($observedDecision -ne "approved" -or -not $retired -or -not $deletionExecuted) {
            $detail = if ($packetBlockers.Count -gt 0) { $packetBlockers -join ',' } else { "decision=$observedDecision;retired=$retired;deletionExecuted=$deletionExecuted" }
            Add-Unsupported $unsupported "retirement_not_completed" ([string]$ticket.branchId) $ticketId $detail
        }
    }

    $parityOutcomes = [ordered]@{}
    foreach ($name in @("golden-parity", "contract-parity", "effect-parity", "rollback-execution", "compatibility-window")) {
        $evidence = Read-JsonFile (Join-Path $packetRoot "evidence/$name.json")
        $outcome = if ($null -eq $evidence) { "missing" } else { [string]$evidence.details.outcome }
        $parityOutcomes[$name] = $outcome
        if ($outcome -ne "passed") {
            $code = if ($name -eq "rollback-execution") { "rollback_window_unproven" } elseif ($name -eq "compatibility-window") { "compatibility_window_unproven" } else { "a00_parity_unproven" }
            Add-Unsupported $unsupported $code ([string]$ticket.branchId) $ticketId "$name outcome=$outcome"
        }
    }

    $dependencies.Add([pscustomobject][ordered]@{
        ticketId = $ticketId
        branchId = [string]$ticket.branchId
        path = [string]$ticket.path
        packetRoot = ([string]$ticket.directory -replace '\\', '/')
        owner = $owner
        decision = $observedDecision
        retired = $retired
        deletionExecuted = $deletionExecuted
        blockers = @($packetBlockers)
        evidenceOutcomes = [pscustomobject]$parityOutcomes
    })
}

$registry = Read-JsonFile (Resolve-RepoFile "orchestration/integrations.json")
if ($null -eq $registry) { throw "orchestration/integrations.json is missing" }
$registered = @($registry.integrations | ForEach-Object { [string]$_.id })
$retained = [System.Collections.Generic.List[object]]::new()
$surfaceIds = @($policy.retainedPowerShell | ForEach-Object { [string]$_.surfaceId })
if ($surfaceIds.Count -ne 11 -or @($surfaceIds | Sort-Object -Unique).Count -ne $surfaceIds.Count) {
    throw "E06 policy must contain exactly eleven unique retained PowerShell surfaces"
}
foreach ($surface in @($policy.retainedPowerShell)) {
    Assert-ExactProperties $surface @("surfaceId", "path", "owner", "registryParticipantId", "expiresAt", "classification") "retained PowerShell surface"
    $pathExists = Test-Path -LiteralPath (Resolve-RepoFile ([string]$surface.path)) -PathType Leaf
    $registryBacked = $registered -contains [string]$surface.registryParticipantId
    $ownerPresent = -not [string]::IsNullOrWhiteSpace([string]$surface.owner)
    $expiry = if ($null -eq $surface.expiresAt) { $null } else { [long]$surface.expiresAt }
    $expiryCurrent = $null -ne $expiry -and $expiry -gt $EvaluatedAt
    if (-not $pathExists) { Add-Unsupported $unsupported "retained_surface_missing" ([string]$surface.surfaceId) "E06" ([string]$surface.path) }
    if (-not $registryBacked) { Add-Unsupported $unsupported "retained_surface_unregistered" ([string]$surface.surfaceId) "B07" ([string]$surface.registryParticipantId) }
    if (-not $ownerPresent) { Add-Unsupported $unsupported "retained_surface_owner_missing" ([string]$surface.surfaceId) "E06" ([string]$surface.path) }
    if (-not $expiryCurrent) { Add-Unsupported $unsupported "retained_surface_expiry_missing_or_elapsed" ([string]$surface.surfaceId) "E06" "expiresAt must be greater than EvaluatedAt" }
    $retained.Add([pscustomobject][ordered]@{
        surfaceId = [string]$surface.surfaceId
        path = [string]$surface.path
        classification = [string]$surface.classification
        owner = [string]$surface.owner
        registryParticipantId = [string]$surface.registryParticipantId
        registryBacked = $registryBacked
        expiresAt = $expiry
        expiryCurrent = $expiryCurrent
    })
}

$modes = [System.Collections.Generic.List[object]]::new()
foreach ($modePolicy in @($policy.modeMatrix)) {
    Assert-ExactProperties $modePolicy @("mode", "fixture", "requiredCapabilities") "mode policy"
    $mode = [string]$modePolicy.mode
    $fixture = Read-JsonFile (Resolve-RepoFile (Join-Path $FixtureRoot ([string]$modePolicy.fixture)))
    if ($null -ne $fixture) {
        Assert-ExactProperties $fixture @("schema", "mode", "available", "reason", "nodes") "mode fixture"
        if ([string]$fixture.schema -ne "code-intel-facade-mode-audit-fixture.v1" -or [string]$fixture.mode -ne $mode) { throw "mode fixture identity mismatch: $mode" }
    }
    $available = $null -ne $fixture -and [bool]$fixture.available
    $reason = if ($null -eq $fixture) { "fixture missing" } else { [string]$fixture.reason }
    $nodeResults = [System.Collections.Generic.List[object]]::new()
    if (-not $available) {
        Add-Unsupported $unsupported "mode_smoke_unavailable" $mode "E06" $reason
    }
    else {
        foreach ($capability in @($modePolicy.requiredCapabilities)) {
            $matches = @($fixture.nodes | Where-Object { [string]$_.capabilityId -eq [string]$capability })
            if ($matches.Count -ne 1) {
                Add-Unsupported $unsupported "mode_node_missing" $mode ([string]$capability) "required node must appear exactly once"
                continue
            }
            $node = $matches[0]
            Assert-ExactProperties $node @("capabilityId", "registryBacked", "enveloped", "admission", "committed", "indexed") "mode node"
            $nodeOk = [bool]$node.registryBacked -and [bool]$node.enveloped -and @("admitted", "not_applicable") -contains [string]$node.admission
            if ($mode -ne "doctor" -and [string]$capability -eq "run.commit") { $nodeOk = $nodeOk -and [bool]$node.committed }
            if ($mode -ne "doctor" -and [string]$capability -eq "artifact.index-committed-only") { $nodeOk = $nodeOk -and [bool]$node.indexed }
            if (-not $nodeOk) { Add-Unsupported $unsupported "mode_node_contract_failed" $mode ([string]$capability) "registry/envelope/admission/commit/index contract failed" }
            $nodeResults.Add($node)
        }
    }
    $modes.Add([pscustomobject][ordered]@{ mode = $mode; available = $available; reason = $reason; nodes = @($nodeResults) })
}

$a00Path = Resolve-RepoFile ([string]$policy.parity.a00Evidence)
$a00Present = Test-Path -LiteralPath $a00Path -PathType Leaf
if (-not $a00Present) { Add-Unsupported $unsupported "a00_final_parity_missing" "public-mode-matrix" "A00" ([string]$policy.parity.a00Evidence) }
foreach ($ticketId in @($policy.parity.rollbackTickets)) {
    if ($ticketIds -notcontains [string]$ticketId) { throw "unknown rollback ticket in policy: $ticketId" }
}
if ((@($policy.parity.rollbackTickets | ForEach-Object { [string]$_ }) -join ',') -ne ($ticketIds -join ',')) {
    throw "E06 rollback ticket set/order must match dependency tickets"
}

Add-Unsupported $unsupported "independent_approval_missing" "E06-final-manifest" "independent-verifier" "E06 implementer cannot sign final approval"
$sortedUnsupported = @($unsupported | Sort-Object code, subject, dependency, detail -Unique)
$result = [pscustomobject][ordered]@{
    schema = "code-intel-compatibility-facade-finalize.v1"
    evaluatedAt = $EvaluatedAt
    status = "blocked"
    approvalEligible = $false
    independentApproval = $null
    dependencies = @($dependencies)
    retainedPowerShell = @($retained)
    modeMatrix = @($modes)
    parity = [pscustomobject][ordered]@{ a00Evidence = [string]$policy.parity.a00Evidence; present = $a00Present }
    rollbackWindows = @($policy.parity.rollbackTickets | ForEach-Object { [string]$_ })
    unsupportedBranches = $sortedUnsupported
}

$serialized = $result | ConvertTo-Json -Depth 20
if (-not [string]::IsNullOrWhiteSpace($OutFile)) {
    $resolvedOut = if ([System.IO.Path]::IsPathRooted($OutFile)) { $OutFile } else { Join-Path $RepoRoot $OutFile }
    $parent = Split-Path -Parent $resolvedOut
    if (-not [string]::IsNullOrWhiteSpace($parent)) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    [System.IO.File]::WriteAllText($resolvedOut, $serialized + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
}
if ($Json) { $serialized } else { $result }
if ($sortedUnsupported.Count -gt 0) { exit 2 }
