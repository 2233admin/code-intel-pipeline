#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSCommandPath
$script = Join-Path $root "Invoke-CompatibilityFacadeFinalize.ps1"
$policy = Join-Path $root "orchestration/facade-finalize-policy.v1.json"
$schema = Join-Path $root "orchestration/schemas/code-intel-compatibility-facade-finalize.v1.schema.json"
$scratch = Join-Path $env:TEMP ("code-intel-e06-{0}" -f [guid]::NewGuid().ToString("N"))

try {
    New-Item -ItemType Directory -Force -Path $scratch | Out-Null
    $out = Join-Path $scratch "final-manifest.json"
    & pwsh -NoProfile -File $script -RepoRoot $root -PolicyPath $policy -EvaluatedAt 1783987200 -OutFile $out -Json | Out-Null
    if ($LASTEXITCODE -ne 2) { throw "E06 current-state audit must exit 2, got $LASTEXITCODE" }
    $resultRaw = Get-Content -LiteralPath $out -Raw
    if (-not ($resultRaw | Test-Json -SchemaFile $schema)) { throw "E06 current-state manifest failed its closed schema" }
    $result = $resultRaw | ConvertFrom-Json
    if ($result.status -ne "blocked" -or [bool]$result.approvalEligible -or $null -ne $result.independentApproval) {
        throw "E06 must not fabricate final approval"
    }
    $policyValue = Get-Content -LiteralPath $policy -Raw | ConvertFrom-Json
    if ((@($result.dependencies.ticketId) -join ',') -ne (@($policyValue.dependencyTickets.ticketId) -join ',')) { throw "E06 dependency result drifted from policy source" }
    if ((@($result.retainedPowerShell.surfaceId) -join ',') -ne (@($policyValue.retainedPowerShell.surfaceId) -join ',')) { throw "E06 retained PowerShell result drifted from policy source" }
    if ((@($result.rollbackWindows) -join ',') -ne (@($policyValue.parity.rollbackTickets) -join ',')) { throw "E06 rollback result drifted from policy source" }
    if (@($result.modeMatrix.mode) -notcontains 'doctor' -or @($result.modeMatrix.mode) -notcontains 'lite' -or @($result.modeMatrix.mode) -notcontains 'normal' -or @($result.modeMatrix.mode) -notcontains 'full') {
        throw "E06 public mode matrix is incomplete"
    }
    $codes = @($result.unsupportedBranches.code)
    foreach ($required in @('retirement_not_completed', 'mode_smoke_unavailable', 'independent_approval_missing', 'retained_surface_expiry_missing_or_elapsed')) {
        if ($codes -notcontains $required) { throw "E06 did not expose blocker $required" }
    }
    if ($codes -contains 'retirement_packet_missing') { throw "E06 retained stale packet-missing evidence after E02-E10 materialized" }

    $schemaValue = Get-Content -LiteralPath $schema -Raw | ConvertFrom-Json
    if ($schemaValue.additionalProperties -ne $false) { throw "E06 result schema must be closed" }
    $requiredTop = @('schema', 'evaluatedAt', 'status', 'approvalEligible', 'independentApproval', 'dependencies', 'retainedPowerShell', 'modeMatrix', 'parity', 'rollbackWindows', 'unsupportedBranches')
    if ((@($schemaValue.required | Sort-Object) -join ',') -ne (@($requiredTop | Sort-Object) -join ',')) { throw "E06 schema required set drifted" }
    $actualTop = @($result.PSObject.Properties.Name | Sort-Object)
    if (($actualTop -join ',') -ne (@($requiredTop | Sort-Object) -join ',')) { throw "E06 output is not closed" }
    $forgedApproval = $resultRaw | ConvertFrom-Json
    $forgedApproval.independentApproval = [pscustomobject]@{}
    if (($forgedApproval | ConvertTo-Json -Depth 20) | Test-Json -SchemaFile $schema -ErrorAction SilentlyContinue) { throw "E06 schema accepted an empty independent approval object" }

    $attackCases = @(
        @{ name = "top-level"; mutate = { param($p) $p | Add-Member attackerField $true } },
        @{ name = "dependency"; mutate = { param($p) $p.dependencyTickets[0] | Add-Member attackerField $true } },
        @{ name = "retained"; mutate = { param($p) $p.retainedPowerShell[0] | Add-Member attackerField $true } },
        @{ name = "mode"; mutate = { param($p) $p.modeMatrix[0] | Add-Member attackerField $true } },
        @{ name = "parity"; mutate = { param($p) $p.parity | Add-Member attackerField $true } }
    )
    foreach ($attack in $attackCases) {
        $attackedPolicy = Get-Content -LiteralPath $policy -Raw | ConvertFrom-Json
        & $attack.mutate $attackedPolicy
        $attackedPolicyPath = Join-Path $scratch ("attacked-{0}.json" -f $attack.name)
        $attackedOut = Join-Path $scratch ("attacked-{0}-result.json" -f $attack.name)
        [System.IO.File]::WriteAllText($attackedPolicyPath, ($attackedPolicy | ConvertTo-Json -Depth 20), [System.Text.UTF8Encoding]::new($false))
        & pwsh -NoProfile -File $script -RepoRoot $root -PolicyPath $attackedPolicyPath -EvaluatedAt 1783987200 -OutFile $attackedOut -Json *> $null
        if ($LASTEXITCODE -eq 0 -or $LASTEXITCODE -eq 2) { throw "E06 accepted attackerField in $($attack.name) policy object" }
        if (Test-Path -LiteralPath $attackedOut) { throw "E06 published an audit result for invalid $($attack.name) policy" }
    }

    $fixtureRoot = Join-Path $scratch "passing-mode-fixtures"
    New-Item -ItemType Directory -Force -Path $fixtureRoot | Out-Null
    foreach ($mode in @($policyValue.modeMatrix)) {
        $nodes = @($mode.requiredCapabilities | ForEach-Object {
            [ordered]@{
                capabilityId = [string]$_
                registryBacked = $true
                enveloped = $true
                admission = "admitted"
                committed = ([string]$_ -eq "run.commit")
                indexed = ([string]$_ -eq "artifact.index-committed-only")
            }
        })
        $fixture = [ordered]@{ schema = "code-intel-facade-mode-audit-fixture.v1"; mode = [string]$mode.mode; available = $true; reason = "fixture"; nodes = $nodes }
        [System.IO.File]::WriteAllText((Join-Path $fixtureRoot ([string]$mode.fixture)), ($fixture | ConvertTo-Json -Depth 10), [System.Text.UTF8Encoding]::new($false))
    }
    $out2 = Join-Path $scratch "fixture-final-manifest.json"
    & pwsh -NoProfile -File $script -RepoRoot $root -PolicyPath $policy -FixtureRoot $fixtureRoot -EvaluatedAt 1783987200 -OutFile $out2 -Json | Out-Null
    if ($LASTEXITCODE -ne 2) { throw "passing mode fixtures must not override blocked dependency packets" }
    $fixtureResultRaw = Get-Content -LiteralPath $out2 -Raw
    if (-not ($fixtureResultRaw | Test-Json -SchemaFile $schema)) { throw "E06 fixture manifest failed its closed schema" }
    $fixtureResult = $fixtureResultRaw | ConvertFrom-Json
    if (@($fixtureResult.unsupportedBranches | Where-Object code -eq 'mode_smoke_unavailable').Count -ne 0) { throw "available fixtures were not executed" }
    if (@($fixtureResult.unsupportedBranches | Where-Object code -eq 'retirement_not_completed').Count -eq 0) { throw "mode fixtures improperly approved E06" }

    Write-Output "PASS compatibility facade finalize audit"
}
finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
