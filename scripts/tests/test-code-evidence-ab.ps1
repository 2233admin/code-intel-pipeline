param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$abScript = Join-Path $root "Invoke-CodeEvidenceABTest.ps1"

function Read-JsonFile {
    param([string]$Path)
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Remove-TreeWithRetry {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }

    for ($attempt = 1; $attempt -le 5; $attempt++) {
        try {
            if (Get-Command ccc -ErrorAction SilentlyContinue) {
                ccc daemon stop | Out-Null
            }
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
            return
        } catch {
            if ($attempt -eq 5) {
                throw
            }
            Start-Sleep -Milliseconds (250 * $attempt)
        }
    }
}

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("code-evidence-ab-test-" + [guid]::NewGuid().ToString("N"))

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null

    & $abScript -OutputDir $temp -Runs 2 | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "Invoke-CodeEvidenceABTest.ps1 failed with exit code $LASTEXITCODE"
    }

    $scorecardPath = Join-Path $temp "code-evidence-ab-scorecard.json"
    if (-not (Test-Path -LiteralPath $scorecardPath -PathType Leaf)) {
        throw "Missing A/B scorecard: $scorecardPath"
    }

    $scorecard = Read-JsonFile $scorecardPath
    if ([string]$scorecard.schema -ne "code-evidence-ab-scorecard.v1") {
        throw "Unexpected A/B scorecard schema."
    }
    if ([int]$scorecard.runs -ne 2) {
        throw "Expected scorecard runs=2."
    }
    if ($null -eq $scorecard.variants.A -or [string]$scorecard.variants.A.name -ne "native-minimal") {
        throw "Missing native-minimal A variant."
    }
    if ($null -eq $scorecard.variants.B -or [string]$scorecard.variants.B.name -ne "cocoindex-code") {
        throw "Missing cocoindex-code B variant."
    }
    if ([string]::IsNullOrWhiteSpace([string]$scorecard.variants.B.capabilities.semanticSearch.status)) {
        throw "B variant missing semantic search capability status."
    }
    if ([string]$scorecard.variants.B.capabilities.command.status -eq "available") {
        if ([string]$scorecard.variants.B.capabilities.semanticSearch.status -ne "available") {
            throw "B semantic search should be available after ccc init/index/search lifecycle."
        }
        if ($null -eq $scorecard.variants.B.capabilities.semanticSearch.lifecycle) {
            throw "B semantic search missing lifecycle probes."
        }
        foreach ($probeName in @("init", "index", "status", "search")) {
            if ($null -eq $scorecard.variants.B.capabilities.semanticSearch.lifecycle.$probeName) {
                throw "B semantic search missing lifecycle probe: $probeName"
            }
        }
    }
    if ([string]::IsNullOrWhiteSpace([string]$scorecard.variants.B.capabilities.structuralGrep.status)) {
        throw "B variant missing structural grep capability status."
    }
    if ($scorecard.stability.runs.Count -ne 2) {
        throw "Expected two stability run records."
    }
    if (-not [bool]$scorecard.stability.pipelineExitStable) {
        throw "Expected pipeline exit stability to be true."
    }

    Write-Host "PASS code evidence A/B scorecard: $scorecardPath"
} finally {
    Remove-TreeWithRetry -Path $temp
}
