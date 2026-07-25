param(
    [string]$RepoPath = $PSScriptRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Get-MapValue {
    param(
        [object]$Value,
        [string]$Name
    )

    if ($Value -is [System.Collections.IDictionary]) {
        return $Value[$Name]
    }

    if ($null -eq $Value) {
        return $null
    }

    $property = ([psobject]$Value).Properties[$Name]
    if ($null -ne $property) {
        return $property.Value
    }

    return $null
}

$root = (Resolve-Path -LiteralPath $RepoPath).Path
$detectorPath = Join-Path $root "OpenSpec-Detector.ps1"
Assert-True (Test-Path -LiteralPath $detectorPath -PathType Leaf) "OpenSpec-Detector.ps1 must exist."
$facadePath = Join-Path $root "Invoke-WorkflowRecommendation.ps1"
Assert-True (Test-Path -LiteralPath $facadePath -PathType Leaf) "Workflow recommendation facade must exist."
$schemaPath = Join-Path $root "orchestration/schemas/code-intel-advisory-workflow-recommendation.v1.schema.json"
$schema = Get-Content -LiteralPath $schemaPath -Raw | ConvertFrom-Json
Assert-True ($schema.additionalProperties -eq $false) "Advisory proposal schema must be closed."
$registry = Get-Content -LiteralPath (Join-Path $root "orchestration/integrations.json") -Raw | ConvertFrom-Json
$registration = @($registry.integrations | Where-Object { $_.id -eq "advisory.workflow-recommend" })
Assert-True ($registration.Count -eq 1) "A01 registry must contain exactly one advisory.workflow-recommend capability."
Assert-True (@($registration[0].effects).Count -eq 0) "Registered advisory capability must declare zero effects."
Assert-True ($registration[0].capabilityDeclaration.id -eq "advisory.workflow-recommend") "Registration must expose a real A01 capability declaration."
Assert-True (@($registration[0].capabilityDeclaration.allowedEffects).Count -eq 0) "A01 declaration must allow no advisory effects."
Assert-True ($registration[0].runtimeAdapter -eq "advisory.workflow-recommend.compat") "Registration must bind the executable runtime adapter."
Assert-True (Test-Path -LiteralPath (Join-Path $root "docs/advisory-workflow-recommendation.md") -PathType Leaf) "Advisory boundary documentation must exist."
$runnerSource = Get-Content -LiteralPath (Join-Path $root "run-code-intel.ps1") -Raw
Assert-True ($runnerSource -notmatch "function Get-CodeMetrics") "Main runner must not retain duplicated recommender implementation."
Assert-True ($runnerSource -match "capability exec advisory.workflow-recommend") "Main runner must invoke the recommendation through the A01 envelope."

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-brief-test-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null

try {
    New-Item -ItemType Directory -Path (Join-Path $tempRoot "openspec") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $tempRoot "src") -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $tempRoot "src/main.ps1") -Value "function Invoke-Demo { 'ok' }" -Encoding UTF8

    $result = & $detectorPath -RepoPath $tempRoot -Auto -Quiet 6>$null
    $facadeResult = & $facadePath -RepoPath $tempRoot -Auto -Quiet
    Assert-True ($null -ne $result) "Detector must return a result object."
    Assert-True (($result | ConvertTo-Json -Depth 20 -Compress) -eq ($facadeResult | ConvertTo-Json -Depth 20 -Compress)) "Standalone atom and facade must have normalized parity."
    Assert-True ((Get-MapValue $result "schema") -eq "code-intel-advisory-workflow-recommendation.v1") "Result must use the advisory proposal schema."
    Assert-True ((Get-MapValue $result "kind") -eq "proposal") "Workflow recommendation must remain a proposal."
    $keys = @($result.Keys | Sort-Object)
    $expectedKeys = @("alternatives", "confidence", "effects", "evidence", "kind", "provenance", "recommendation", "schema") | Sort-Object
    Assert-True (($keys -join "|") -eq ($expectedKeys -join "|")) "Proposal must have exact top-level fields."
    Assert-True (@(Get-MapValue $result "effects").Count -eq 0) "Advisory atom must declare zero effects."
    Assert-True ($null -eq (Get-MapValue $result "adoptionDecision")) "Proposal must not contain an Adoption Decision."

    $recommendation = Get-MapValue $result "recommendation"
    $brief = Get-MapValue $recommendation "brief"
    Assert-True ($null -ne $brief) "Recommendation must include its evidence brief."
    Assert-True ((Get-MapValue $brief "recommended") -eq "openspec-opsx") "Brief must recommend openspec-opsx for openspec/ repos."
    Assert-True ((Get-MapValue $result "confidence") -eq "high") "Already-adopted proposal must be high confidence."

    $guardrails = @(Get-MapValue $brief "doNotDoYet")
    Assert-True (($guardrails -join "`n") -match "Do not auto-run init") "Brief must preserve no-auto-init guardrail."

    $acceptance = @(Get-MapValue $brief "acceptance")
    Assert-True (($acceptance -join "`n") -match "Completion conditions") "Brief must include completion conditions."

    $sourceMethod = [string](Get-MapValue $brief "sourceMethod")
    Assert-True ($sourceMethod -match "improving-ai-agent-openspec") "Brief must cite the absorbed OpenSpec methodology."

    $alternatives = @(Get-MapValue $result "alternatives")
    Assert-True ($alternatives.Count -eq 3) "Proposal must expose exactly the three candidate workflow stacks."
    $candidateText = @($alternatives | ForEach-Object { Get-MapValue $_ "candidate" }) -join "|"
    Assert-True ($candidateText -match "gstack") "gstack must remain a candidate, not a dependency."
    Assert-True ($candidateText -match "openspec-opsx") "OpenSpec must remain a candidate, not an adopted dependency."

    $before = @(Get-ChildItem -LiteralPath $tempRoot -Force | Select-Object -ExpandProperty Name | Sort-Object)
    $null = & $facadePath -RepoPath $tempRoot -Auto -Quiet 6>$null
    $after = @(Get-ChildItem -LiteralPath $tempRoot -Force | Select-Object -ExpandProperty Name | Sort-Object)
    Assert-True (($before -join "|") -eq ($after -join "|")) "Recommendation must not initialize or write into the repository."
}
finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}

Write-Host "Workflow recommendation brief checks passed."
