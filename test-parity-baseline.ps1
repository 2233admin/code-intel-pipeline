#requires -Version 7.2

param(
    [switch]$UpdateFixtures,
    [string]$ReviewReason = "",
    [ValidateSet("", "clean", "dirty", "provider-unavailable", "domain-fail", "partial-evidence")]
    [string]$Fixture = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Contract {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Assert-ReviewReason {
    param([string]$Reason)
    if ([string]::IsNullOrWhiteSpace($Reason)) {
        throw "Updating parity fixtures requires -ReviewReason with an explicit reviewer-facing explanation."
    }
}

function Expand-FixtureValue {
    param($Value, [string]$TempRoot, [string]$Timestamp)

    if ($null -eq $Value) { return $null }
    if ($Value -is [string]) {
        return $Value.Replace("{{TEMP_ROOT}}", $TempRoot).Replace("{{TIME}}", $Timestamp)
    }
    if ($Value -is [System.Collections.IDictionary]) {
        $expanded = [ordered]@{}
        foreach ($key in $Value.Keys) {
            $expanded[[string]$key] = Expand-FixtureValue $Value[$key] $TempRoot $Timestamp
        }
        return $expanded
    }
    if ($Value -is [System.Management.Automation.PSCustomObject]) {
        $expanded = [ordered]@{}
        foreach ($property in $Value.PSObject.Properties) {
            $expanded[$property.Name] = Expand-FixtureValue $property.Value $TempRoot $Timestamp
        }
        return $expanded
    }
    if ($Value -is [System.Collections.IEnumerable]) {
        $items = @($Value | ForEach-Object { Expand-FixtureValue $_ $TempRoot $Timestamp })
        return ,$items
    }
    return $Value
}

function ConvertTo-NormalizedCanonicalValue {
    param($Value, [string]$PropertyName, [string]$TempRoot)

    if ($null -eq $Value) { return $null }
    if ($Value -is [string]) {
        $timeFields = @("generatedAt", "startedAt", "finishedAt", "completedAt", "observedAt", "publishedAt")
        if ($PropertyName -in $timeFields) { return "<TIME>" }

        $normalized = $Value.Replace("\", "/")
        $normalizedRoot = $TempRoot.Replace("\", "/").TrimEnd("/")
        if ($normalized.Equals($normalizedRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
            return "<TEMP_ROOT>"
        }
        if ($normalized.StartsWith($normalizedRoot + "/", [System.StringComparison]::OrdinalIgnoreCase)) {
            return "<TEMP_ROOT>" + $normalized.Substring($normalizedRoot.Length)
        }
        return $Value
    }
    if ($Value -is [System.Collections.IDictionary]) {
        $canonical = [ordered]@{}
        foreach ($key in @($Value.Keys | ForEach-Object { [string]$_ } | Sort-Object -CaseSensitive)) {
            $canonical[$key] = ConvertTo-NormalizedCanonicalValue $Value[$key] $key $TempRoot
        }
        return $canonical
    }
    if ($Value -is [System.Management.Automation.PSCustomObject]) {
        $canonical = [ordered]@{}
        foreach ($property in @($Value.PSObject.Properties | Sort-Object Name -CaseSensitive)) {
            $canonical[$property.Name] = ConvertTo-NormalizedCanonicalValue $property.Value $property.Name $TempRoot
        }
        return $canonical
    }
    if ($Value -is [System.Collections.IEnumerable]) {
        $items = @($Value | ForEach-Object { ConvertTo-NormalizedCanonicalValue $_ "" $TempRoot })
        return ,$items
    }
    return $Value
}

function Invoke-FixtureNormalization {
    param([string]$InputPath, [string]$TempRoot, [string]$Timestamp)

    $source = Get-Content -LiteralPath $InputPath -Raw | ConvertFrom-Json -Depth 100
    $expanded = Expand-FixtureValue $source $TempRoot $Timestamp
    $normalized = ConvertTo-NormalizedCanonicalValue $expanded "" $TempRoot
    return (($normalized | ConvertTo-Json -Depth 100) + "`n")
}

function Assert-GoldenMatch {
    param([string]$Expected, [string]$Actual, [string]$Name)
    if ($Expected -cne $Actual) {
        throw "Parity mismatch rejected for fixture '$Name'. Run with -UpdateFixtures -ReviewReason <reason> only after reviewing the semantic diff."
    }
}

function Assert-MismatchRejected {
    param([string]$Expected, [string]$Actual, [string]$Name)
    $rejected = $false
    try {
        Assert-GoldenMatch $Expected $Actual $Name
    }
    catch {
        $rejected = $true
    }
    Assert-Contract $rejected "Deliberate $Name mismatch was hidden by parity normalization."
}

$root = Split-Path -Parent $PSCommandPath
$fixtureRoot = Join-Path $root "tests\fixtures\parity"
$caseNames = @("clean", "dirty", "provider-unavailable", "domain-fail", "partial-evidence")
if (-not [string]::IsNullOrWhiteSpace($Fixture)) { $caseNames = @($Fixture) }

if ($UpdateFixtures) { Assert-ReviewReason $ReviewReason }
else {
    $missingReasonRejected = $false
    try { Assert-ReviewReason "" } catch { $missingReasonRejected = $true }
    Assert-Contract $missingReasonRejected "Fixture update guard accepted an empty review reason."
}

$tempBase = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-parity-" + [guid]::NewGuid().ToString("N"))
try {
    foreach ($caseName in $caseNames) {
        $caseRoot = Join-Path $fixtureRoot $caseName
        $inputPath = Join-Path $caseRoot "machine-artifacts.json"
        $goldenPath = Join-Path $caseRoot "normalized.json"
        $reviewPath = Join-Path $caseRoot "fixture-review.json"
        Assert-Contract (Test-Path -LiteralPath $inputPath -PathType Leaf) "Missing parity input: $inputPath"

        $runOneRoot = Join-Path $tempBase ("run-one\" + $caseName)
        $runTwoRoot = Join-Path $tempBase ("run-two\" + $caseName)
        $first = Invoke-FixtureNormalization $inputPath $runOneRoot "2026-07-13T01:02:03.0000000Z"
        $second = Invoke-FixtureNormalization $inputPath $runTwoRoot "2031-12-31T23:59:59.0000000Z"
        Assert-Contract ($first -ceq $second) "Fixture '$caseName' was not byte-identical across fresh roots and timestamps."

        if ($UpdateFixtures) {
            New-Item -ItemType Directory -Force -Path $caseRoot | Out-Null
            [System.IO.File]::WriteAllText($goldenPath, $first, [System.Text.UTF8Encoding]::new($false))
            $review = [ordered]@{
                schema = "code-intel-parity-fixture-review.v1"
                fixture = $caseName
                reviewReason = $ReviewReason.Trim()
            }
            [System.IO.File]::WriteAllText(
                $reviewPath,
                (($review | ConvertTo-Json) + "`n"),
                [System.Text.UTF8Encoding]::new($false)
            )
        }
        else {
            Assert-Contract (Test-Path -LiteralPath $goldenPath -PathType Leaf) "Missing parity golden: $goldenPath"
            Assert-Contract (Test-Path -LiteralPath $reviewPath -PathType Leaf) "Missing fixture review record: $reviewPath"
            $review = Get-Content -LiteralPath $reviewPath -Raw | ConvertFrom-Json
            Assert-Contract ([string]$review.schema -eq "code-intel-parity-fixture-review.v1") "Fixture '$caseName' has an invalid review schema."
            Assert-Contract (-not [string]::IsNullOrWhiteSpace([string]$review.reviewReason)) "Fixture '$caseName' lacks an explicit review reason."
            $golden = [System.IO.File]::ReadAllText($goldenPath)
            Assert-GoldenMatch $golden $first $caseName
        }
    }

    if (-not $UpdateFixtures) {
        $cleanInputPath = Join-Path $fixtureRoot "clean\machine-artifacts.json"
        $cleanSource = Get-Content -LiteralPath $cleanInputPath -Raw | ConvertFrom-Json -Depth 100
        $semanticRoot = Join-Path $tempBase "semantic-guard"
        $cleanExpanded = Expand-FixtureValue $cleanSource $semanticRoot "2028-01-01T00:00:00Z"
        $cleanCanonical = ConvertTo-NormalizedCanonicalValue $cleanExpanded "" $semanticRoot
        $cleanGolden = (($cleanCanonical | ConvertTo-Json -Depth 100) + "`n")

        Assert-Contract ([int]$cleanCanonical.artifacts.report.summary.failed -eq 0) "Verdict disappeared during normalization."
        Assert-Contract ([string]$cleanCanonical.provenance.producer -eq "run-code-intel.ps1") "Producer provenance disappeared during normalization."
        Assert-Contract ([string]$cleanCanonical.provenance.snapshotIdentity -eq "git:clean-fixture-head") "Snapshot provenance disappeared during normalization."
        Assert-Contract (@($cleanCanonical.missingEvidence).Count -eq 0) "Missing-evidence state disappeared during normalization."
        Assert-Contract ([string]$cleanCanonical.semanticBackslashSamples[0] -ceq "deny\allow") "Semantic deny/allow string was rewritten as a path."
        Assert-Contract ([string]$cleanCanonical.semanticBackslashSamples[1] -ceq "tool\name") "Semantic tool name was rewritten as a path."
        Assert-Contract ([string]$cleanCanonical.semanticBackslashSamples[2] -ceq "a\b") "Generic semantic string was rewritten as a path."

        $verdictMutation = $cleanSource | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $verdictMutation.artifacts.report.summary.failed = 1
        $verdictExpanded = Expand-FixtureValue $verdictMutation $semanticRoot "2029-02-02T00:00:00Z"
        $verdictNormalized = ConvertTo-NormalizedCanonicalValue $verdictExpanded "" $semanticRoot
        Assert-MismatchRejected $cleanGolden (($verdictNormalized | ConvertTo-Json -Depth 100) + "`n") "verdict"

        $provenanceMutation = $cleanSource | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $provenanceMutation.provenance.producer = "invented-tool"
        $provenanceExpanded = Expand-FixtureValue $provenanceMutation $semanticRoot "2029-03-03T00:00:00Z"
        $provenanceNormalized = ConvertTo-NormalizedCanonicalValue $provenanceExpanded "" $semanticRoot
        Assert-MismatchRejected $cleanGolden (($provenanceNormalized | ConvertTo-Json -Depth 100) + "`n") "provenance"

        $missingMutation = $cleanSource | ConvertTo-Json -Depth 100 | ConvertFrom-Json -Depth 100
        $missingMutation.missingEvidence = @("understand.graph")
        $missingExpanded = Expand-FixtureValue $missingMutation $semanticRoot "2029-04-04T00:00:00Z"
        $missingNormalized = ConvertTo-NormalizedCanonicalValue $missingExpanded "" $semanticRoot
        Assert-MismatchRejected $cleanGolden (($missingNormalized | ConvertTo-Json -Depth 100) + "`n") "missing-evidence"
    }
}
finally {
    $resolvedTempBase = [System.IO.Path]::GetFullPath($tempBase)
    $resolvedSystemTemp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
    if ($resolvedTempBase.StartsWith($resolvedSystemTemp, [System.StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $resolvedTempBase)) {
        Remove-Item -LiteralPath $resolvedTempBase -Recurse -Force
    }
}

Write-Host "Parity baseline fixtures passed: $($caseNames -join ', ')"
