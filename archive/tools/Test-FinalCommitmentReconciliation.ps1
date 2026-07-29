[CmdletBinding()]
param(
    [string]$RepoRoot = (Split-Path $PSScriptRoot -Parent),
    [string]$ReconciliationPath = "orchestration/evidence/final-commitment-reconciliation.json",
    [string]$ProjectionPath = "docs/final-commitment-reconciliation.md"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-Sha256File([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Escape-Cell([string]$Value) {
    $Value.Replace("|", "\|").Replace("`r`n", "<br>").Replace("`n", "<br>")
}
function Render-Projection([object]$Document) {
    $counts = @($Document.items | Group-Object claimStatus | Sort-Object Name)
    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add("# Final 69-item commitment reconciliation")
    $lines.Add("")
    $lines.Add("This is the human-readable projection of ``orchestration/evidence/final-commitment-reconciliation.json``.")
    $lines.Add("The JSON is authoritative; this table must be regenerated or updated in the same change and is checked byte-for-byte after newline normalization.")
    $lines.Add("")
    $lines.Add("Source plan: ``$($Document.sourcePlan)``  ")
    $lines.Add("Source SHA-256: ``$($Document.sourcePlanSha256)``  ")
    $lines.Add("Tickets: **$($Document.ticketCount)**")
    $lines.Add("")
    $lines.Add("## Status totals")
    $lines.Add("")
    $lines.Add("| Claim status | Count |")
    $lines.Add("| --- | ---: |")
    foreach ($count in $counts) { $lines.Add("| $($count.Name) | $($count.Count) |") }
    $lines.Add("")
    $lines.Add("## Itemized reconciliation")
    $lines.Add("")
    $lines.Add("| Ticket | Claim status | Independent verdict | Blockers | Evidence artifacts |")
    $lines.Add("| --- | --- | --- | --- | --- |")
    foreach ($item in $Document.items) {
        $blockers = if (@($item.blockers).Count) { (@($item.blockers) | ForEach-Object { Escape-Cell ([string]$_) }) -join "<br>" } else { "—" }
        $artifacts = (@($item.artifacts) | ForEach-Object { "``$(Escape-Cell ([string]$_))``" }) -join "<br>"
        $lines.Add("| $($item.ticketId) — ``$(Escape-Cell ([string]$item.title))`` | $($item.claimStatus) | $($item.independentVerdict) | $blockers | $artifacts |")
    }
    ($lines -join "`n") + "`n"
}

$reconciliation = Join-Path $RepoRoot $ReconciliationPath
$projection = Join-Path $RepoRoot $ProjectionPath
$planPath = Join-Path $RepoRoot "docs/plans/adr-0010-execution-plan.md"
foreach ($path in @($reconciliation, $projection, $planPath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "required reconciliation input missing: $path" }
}
$document = Get-Content -LiteralPath $reconciliation -Raw | ConvertFrom-Json
if ($document.schema -ne "code-intel-final-commitment-reconciliation.v1") { throw "unexpected reconciliation schema" }
if ($document.sourcePlan -ne "docs/plans/adr-0010-execution-plan.md") { throw "reconciliation source plan changed" }
$actualPlanSha = Get-Sha256File $planPath
if ($document.sourcePlanSha256 -ne $actualPlanSha) { throw "reconciliation is stale relative to the frozen ADR" }

$planText = Get-Content -LiteralPath $planPath -Raw
$headings = @([regex]::Matches($planText, '(?m)^###\s+([A-Z]\d{2})\s+—\s+`([^`]+)`') | ForEach-Object {
    [pscustomobject]@{ ticketId = $_.Groups[1].Value; title = $_.Groups[2].Value }
})
$items = @($document.items)
if ($headings.Count -ne 69 -or $document.ticketCount -ne 69 -or $items.Count -ne 69) {
    throw "the frozen ADR and reconciliation must each contain exactly 69 tickets"
}
$planDuplicates = @($headings | Group-Object ticketId | Where-Object Count -ne 1)
$itemDuplicates = @($items | Group-Object ticketId | Where-Object Count -ne 1)
if ($planDuplicates.Count -or $itemDuplicates.Count) { throw "duplicate commitment ticket id detected" }
for ($index = 0; $index -lt 69; $index++) {
    if ($items[$index].ticketId -ne $headings[$index].ticketId -or $items[$index].title -ne $headings[$index].title) {
        throw "missing, extra, reordered, or renamed ticket at ADR position $($index + 1)"
    }
}

$allowedStatuses = @("implemented_verified", "implemented_pending_verification", "implemented_blocked", "retirement_blocked", "not_implemented")
if ((@($document.allowedClaimStatuses) -join "`n") -cne ($allowedStatuses -join "`n")) { throw "claim-status enum drifted" }
foreach ($item in $items) {
    $fields = @($item.psobject.Properties.Name)
    foreach ($required in @("ticketId", "title", "claimStatus", "evidenceCommands", "artifacts", "independentVerdict", "blockers")) {
        if ($fields -notcontains $required) { throw "$($item.ticketId) is missing field $required" }
    }
    if ($allowedStatuses -notcontains $item.claimStatus) { throw "$($item.ticketId) uses an unknown claim status" }
    if ([string]$item.claimStatus -match 'planned|draft|in.progress') { throw "$($item.ticketId) maps planning state into a claim status" }
    if (@($item.artifacts).Count -eq 0) { throw "$($item.ticketId) has no evidence artifact" }
    foreach ($artifact in @($item.artifacts)) {
        if ([IO.Path]::IsPathRooted([string]$artifact) -or [string]$artifact -match '(^|[\\/])\.\.([\\/]|$)') {
            throw "$($item.ticketId) has a non-portable evidence artifact path"
        }
        if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot ([string]$artifact)) -PathType Leaf)) {
            throw "$($item.ticketId) references missing evidence artifact: $artifact"
        }
    }
    foreach ($command in @($item.evidenceCommands)) {
        if ([string]::IsNullOrWhiteSpace([string]$command) -or [string]$command -match '<[^>]+>') {
            throw "$($item.ticketId) has an empty or placeholder evidence command"
        }
    }
    switch ([string]$item.claimStatus) {
        "implemented_verified" {
            if ($item.independentVerdict -ne "verified" -or @($item.evidenceCommands).Count -eq 0 -or @($item.blockers).Count -ne 0) {
                throw "$($item.ticketId) cannot claim implemented_verified without commands, a verified verdict, and zero blockers"
            }
        }
        "implemented_pending_verification" {
            if ($item.independentVerdict -ne "pending" -or @($item.blockers).Count -eq 0) { throw "$($item.ticketId) pending verification is incoherent" }
        }
        "implemented_blocked" {
            if ($item.independentVerdict -ne "blocked" -or @($item.blockers).Count -eq 0) { throw "$($item.ticketId) implemented blocker is incoherent" }
        }
        "retirement_blocked" {
            if ($item.independentVerdict -ne "blocked" -or @($item.blockers).Count -eq 0) { throw "$($item.ticketId) retirement blocker is incoherent" }
            $statusArtifact = @($item.artifacts | Where-Object { $_ -match '/status\.json$' })
            if ($statusArtifact.Count -ne 1) { throw "$($item.ticketId) must bind exactly one retirement status artifact" }
            $status = Get-Content -LiteralPath (Join-Path $RepoRoot $statusArtifact[0]) -Raw | ConvertFrom-Json
            if ($status.decision -ne "blocked" -or $status.deletionExecuted -ne $false -or $status.retired -ne $false) {
                throw "$($item.ticketId) retirement packet is not actually blocked and unexecuted"
            }
        }
        "not_implemented" {
            if ($item.independentVerdict -ne "pending" -or @($item.blockers).Count -eq 0) { throw "$($item.ticketId) not-implemented state is incoherent" }
        }
    }
}

$expectedProjection = Render-Projection $document
$actualProjection = [IO.File]::ReadAllText($projection).Replace("`r`n", "`n").Replace("`r", "`n")
if ($actualProjection -cne $expectedProjection) { throw "Markdown projection is stale or edited independently of the machine-readable reconciliation" }

$statusCounts = [ordered]@{}
foreach ($group in @($items | Group-Object claimStatus | Sort-Object Name)) { $statusCounts[$group.Name] = $group.Count }
[ordered]@{
    ok = $true
    ticketCount = $items.Count
    sourcePlanSha256 = $actualPlanSha
    statusCounts = $statusCounts
    plannedPromotedToImplemented = $false
    projectionExact = $true
} | ConvertTo-Json -Depth 8 -Compress
