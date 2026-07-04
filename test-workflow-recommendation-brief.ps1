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

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-brief-test-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null

try {
    New-Item -ItemType Directory -Path (Join-Path $tempRoot "openspec") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $tempRoot "src") -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $tempRoot "src/main.ps1") -Value "function Invoke-Demo { 'ok' }" -Encoding UTF8

    $result = & $detectorPath -RepoPath $tempRoot -Auto
    Assert-True ($null -ne $result) "Detector must return a result object."

    $specDriven = Get-MapValue $result "specDriven"
    Assert-True ($null -ne $specDriven) "Result must include specDriven."

    $brief = Get-MapValue $specDriven "recommendationBrief"
    Assert-True ($null -ne $brief) "specDriven must include recommendationBrief."
    Assert-True ((Get-MapValue $brief "recommended") -eq "openspec-opsx") "Brief must recommend openspec-opsx for openspec/ repos."
    Assert-True ((Get-MapValue $brief "confidence") -eq "high") "Already-adopted brief must be high confidence."

    $guardrails = @(Get-MapValue $brief "doNotDoYet")
    Assert-True (($guardrails -join "`n") -match "Do not auto-run init") "Brief must preserve no-auto-init guardrail."

    $acceptance = @(Get-MapValue $brief "acceptance")
    Assert-True (($acceptance -join "`n") -match "Completion conditions") "Brief must include completion conditions."

    $sourceMethod = [string](Get-MapValue $brief "sourceMethod")
    Assert-True ($sourceMethod -match "improving-ai-agent-openspec") "Brief must cite the absorbed OpenSpec methodology."

    $workflowBriefs = @((Get-MapValue $result "workflows") | ForEach-Object { Get-MapValue $_ "recommendationBrief" } | Where-Object { $null -ne $_ })
    Assert-True ($workflowBriefs.Count -ge 1) "workflows[] must carry recommendationBrief for spec-driven."

    $legacyBrief = Get-MapValue $result "recommendationBrief"
    Assert-True ($null -ne $legacyBrief) "standalone legacy top-level aliases must carry recommendationBrief."
}
finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}

Write-Host "Workflow recommendation brief checks passed."
