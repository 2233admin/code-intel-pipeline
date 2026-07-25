param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSCommandPath
$gate = Join-Path $root "Test-LanguageAdapterAcceptance.ps1"
$sourceReport = Join-Path $root "orchestration\acceptance\native-code-evidence-candidate.json"
$sourcePolicy = Join-Path $root "orchestration\language-adapter-acceptance-policy.v1.json"
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("language-adapter-acceptance-" + [guid]::NewGuid().ToString("N"))

function Invoke-Gate {
    param([string]$ReportPath, [string]$PolicyPath = $sourcePolicy)
    $output = & pwsh -NoProfile -File $gate -Report $ReportPath -Policy $PolicyPath -Json 2>&1
    $exitCode = $LASTEXITCODE
    $text = ($output | ForEach-Object { $_.ToString() }) -join "`n"
    [pscustomobject]@{ ExitCode = $exitCode; Result = ($text | ConvertFrom-Json) }
}

function Write-Mutation {
    param([string]$Name, [scriptblock]$Mutate)
    $document = Get-Content -Raw -LiteralPath $sourceReport | ConvertFrom-Json
    & $Mutate $document
    $path = Join-Path $temp "$Name.json"
    $document | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $path -Encoding UTF8
    $path
}

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null

    $passing = Invoke-Gate -ReportPath $sourceReport
    if ($passing.ExitCode -ne 0 -or [string]$passing.Result.verdict -ne "pass") {
        throw "Expected native candidate report to pass."
    }

    $lowPrecision = Write-Mutation -Name "low-precision" -Mutate { param($doc) $doc.corpus.precision = 0.74 }
    $lowPrecisionResult = Invoke-Gate -ReportPath $lowPrecision
    if ($lowPrecisionResult.ExitCode -ne 1 -or @($lowPrecisionResult.Result.failedGateIds) -notcontains "measured-quality") {
        throw "Low precision must fail measured-quality."
    }

    $implicitUnsupported = Write-Mutation -Name "implicit-unsupported" -Mutate { param($doc) $doc.corpus.unsupportedExplicit = $false }
    $unsupportedResult = Invoke-Gate -ReportPath $implicitUnsupported
    if ($unsupportedResult.ExitCode -ne 1 -or @($unsupportedResult.Result.failedGateIds) -notcontains "unsupported-behavior") {
        throw "Implicit unsupported behavior must fail."
    }

    $network = Write-Mutation -Name "hidden-network" -Mutate {
        param($doc)
        $doc.effects.observed = @("repo_read", "local_write", "network")
        $doc.effects.networkUsed = $true
    }
    $networkResult = Invoke-Gate -ReportPath $network
    if ($networkResult.ExitCode -ne 1 -or @($networkResult.Result.failedGateIds) -notcontains "effect-boundary") {
        throw "Hidden network use must fail effect-boundary."
    }

    $staleDigest = Write-Mutation -Name "stale-digest" -Mutate { param($doc) $doc.provenance.sourceDigest = ("0" * 64) }
    $staleDigestResult = Invoke-Gate -ReportPath $staleDigest
    if ($staleDigestResult.ExitCode -ne 1 -or @($staleDigestResult.Result.failedGateIds) -notcontains "provenance") {
        throw "Stale source digest must fail provenance."
    }

    $semantic = Write-Mutation -Name "semantic-overclaim" -Mutate { param($doc) $doc.adapter.claimLevel = "semantic" }
    $semanticResult = Invoke-Gate -ReportPath $semantic
    if ($semanticResult.ExitCode -ne 1 -or @($semanticResult.Result.failedGateIds) -notcontains "claim-boundary") {
        throw "Semantic overclaim must fail claim-boundary."
    }

    $production = Write-Mutation -Name "premature-production" -Mutate { param($doc) $doc.adapter.requestedStage = "production" }
    $productionResult = Invoke-Gate -ReportPath $production
    if ($productionResult.ExitCode -ne 1 -or @($productionResult.Result.failedGateIds) -notcontains "independent-verification") {
        throw "Premature production promotion must fail independent verification."
    }

    $weakPolicy = Get-Content -Raw -LiteralPath $sourcePolicy | ConvertFrom-Json
    $weakPolicy.stages.production.minimums.precision = 0.7
    $weakPolicyPath = Join-Path $temp "weakened-production-policy.json"
    $weakPolicy | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $weakPolicyPath -Encoding UTF8
    $weakPolicyResult = Invoke-Gate -ReportPath $sourceReport -PolicyPath $weakPolicyPath
    if ($weakPolicyResult.ExitCode -ne 1 -or @($weakPolicyResult.Result.failedGateIds) -notcontains "policy-monotonicity") {
        throw "A weakened production threshold must fail policy-monotonicity."
    }

    Write-Host "PASS language adapter acceptance: candidate pass and seven fail-closed mutations"
} finally {
    if (Test-Path -LiteralPath $temp) {
        Remove-Item -LiteralPath $temp -Recurse -Force
    }
}
