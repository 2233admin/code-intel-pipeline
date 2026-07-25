#requires -Version 7.2

param(
    [switch]$UpdateFloor,
    [string]$ReviewReason = "",
    [string]$FloorPath = "",
    [string]$FixtureRoot = "",
    [string]$BaselineScript = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Contract {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Get-SortedUniqueStrings {
    param([object[]]$Values, [string]$Label)

    $items = @($Values | ForEach-Object { [string]$_ })
    Assert-Contract ($items.Count -gt 0) "$Label must contain at least one case."
    Assert-Contract (@($items | Where-Object { [string]::IsNullOrWhiteSpace($_) }).Count -eq 0) "$Label contains an empty case name."
    $sorted = @($items | Sort-Object -CaseSensitive -Unique)
    Assert-Contract ($sorted.Count -eq $items.Count) "$Label contains duplicate case names."
    return $sorted
}

function Invoke-ParityCase {
    param([string]$ScriptPath, [string]$CaseName)

    try {
        $output = @(& $ScriptPath -Fixture $CaseName 2>&1 | ForEach-Object { $_.ToString() })
        return [pscustomobject]@{ case = $CaseName; passed = $true; output = ($output -join "`n") }
    }
    catch {
        return [pscustomobject]@{ case = $CaseName; passed = $false; output = ($_ | Out-String).Trim() }
    }
}

$root = Split-Path -Parent $PSCommandPath
if ([string]::IsNullOrWhiteSpace($FloorPath)) { $FloorPath = Join-Path $root "tests\fixtures\parity\parity-floor.json" }
if ([string]::IsNullOrWhiteSpace($FixtureRoot)) { $FixtureRoot = Join-Path $root "tests\fixtures\parity" }
if ([string]::IsNullOrWhiteSpace($BaselineScript)) { $BaselineScript = Join-Path $root "test-parity-baseline.ps1" }

Assert-Contract (Test-Path -LiteralPath $FloorPath -PathType Leaf) "Missing parity floor: $FloorPath"
Assert-Contract (Test-Path -LiteralPath $FixtureRoot -PathType Container) "Missing parity fixture root: $FixtureRoot"
Assert-Contract (Test-Path -LiteralPath $BaselineScript -PathType Leaf) "Missing parity oracle: $BaselineScript"

$floor = Get-Content -LiteralPath $FloorPath -Raw | ConvertFrom-Json -Depth 20
Assert-Contract ([string]$floor.schema -eq "code-intel-parity-floor.v1") "Parity floor schema is invalid."
Assert-Contract ([int]$floor.minimumPassCount -gt 0) "Parity floor minimumPassCount must be positive."
Assert-Contract (-not [string]::IsNullOrWhiteSpace([string]$floor.reviewReason)) "Parity floor lacks its review reason."
$floorCases = @(Get-SortedUniqueStrings @($floor.passingCases) "Parity floor passingCases")
Assert-Contract ($floorCases.Count -ge [int]$floor.minimumPassCount) "Parity floor set is smaller than minimumPassCount."
$missingFloorFixtures = @(
    $floorCases | Where-Object {
        -not (Test-Path -LiteralPath (Join-Path $FixtureRoot "$_\machine-artifacts.json") -PathType Leaf)
    }
)
Assert-Contract ($missingFloorFixtures.Count -eq 0) "Parity floor fixture(s) are missing: $($missingFloorFixtures -join ', ')"

$candidateCases = @(
    Get-ChildItem -LiteralPath $FixtureRoot -Directory |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName "machine-artifacts.json") -PathType Leaf } |
        Select-Object -ExpandProperty Name |
        Sort-Object -CaseSensitive -Unique
)
$allCases = @($floorCases + $candidateCases | Sort-Object -CaseSensitive -Unique)
$results = @($allCases | ForEach-Object { Invoke-ParityCase $BaselineScript $_ })
$passedCases = @($results | Where-Object passed | Select-Object -ExpandProperty case | Sort-Object -CaseSensitive -Unique)
$failedCases = @($results | Where-Object { -not $_.passed } | Select-Object -ExpandProperty case | Sort-Object -CaseSensitive -Unique)
$regressedCases = @($floorCases | Where-Object { $_ -notin $passedCases })
$progressedCases = @($passedCases | Where-Object { $_ -notin $floorCases })

if ($regressedCases.Count -gt 0) {
    foreach ($caseName in $regressedCases) {
        $detail = $results | Where-Object case -eq $caseName | Select-Object -First 1
        Write-Error "parity floor regressed: $caseName`n$($detail.output)"
    }
    throw "Parity floor rejected $($regressedCases.Count) regressed or missing case(s)."
}
Assert-Contract ($passedCases.Count -ge [int]$floor.minimumPassCount) "Passing parity count $($passedCases.Count) is below floor $($floor.minimumPassCount)."

if ($UpdateFloor) {
    Assert-Contract (-not [string]::IsNullOrWhiteSpace($ReviewReason)) "Updating the parity floor requires -ReviewReason."
    Assert-Contract ($failedCases.Count -eq 0) "Parity floor update rejected because candidate cases failed: $($failedCases -join ', ')"
    Assert-Contract ($passedCases.Count -ge $floorCases.Count) "Parity floor update cannot lower the passing count."
    Assert-Contract (@($floorCases | Where-Object { $_ -notin $passedCases }).Count -eq 0) "Parity floor update cannot remove passing cases."

    $updated = [ordered]@{
        schema = "code-intel-parity-floor.v1"
        minimumPassCount = $passedCases.Count
        passingCases = $passedCases
        updatePolicy = "monotonic_set_and_count_with_explicit_review_reason"
        source = $floor.source
        reviewReason = $ReviewReason.Trim()
    }
    [System.IO.File]::WriteAllText(
        [System.IO.Path]::GetFullPath($FloorPath),
        (($updated | ConvertTo-Json -Depth 20) + "`n"),
        [System.Text.UTF8Encoding]::new($false)
    )
}

Write-Host "Parity floor passed: $($floorCases.Count) floor case(s); $($progressedCases.Count) progressed candidate(s)."
if ($progressedCases.Count -gt 0) { Write-Host "Progressed: $($progressedCases -join ', ')" }
