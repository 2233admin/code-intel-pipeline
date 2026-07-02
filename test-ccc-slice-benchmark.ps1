param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$benchmarkScript = Join-Path $root "Invoke-CccSliceBenchmark.ps1"

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

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("ccc-slice-benchmark-test-" + [guid]::NewGuid().ToString("N"))

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null

    & $benchmarkScript -OutputDir $temp -NoiseFileCount 12 | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "Invoke-CccSliceBenchmark.ps1 failed with exit code $LASTEXITCODE"
    }

    $scorecardPath = Join-Path $temp "ccc-slice-benchmark-scorecard.json"
    if (-not (Test-Path -LiteralPath $scorecardPath -PathType Leaf)) {
        throw "Missing CCC slice benchmark scorecard: $scorecardPath"
    }

    $scorecard = Read-JsonFile $scorecardPath
    if ([string]$scorecard.schema -ne "ccc-slice-benchmark.v1") {
        throw "Unexpected CCC slice benchmark schema."
    }
    if ($null -eq $scorecard.nativeEvidence -or [string]$scorecard.nativeEvidence.status -ne "ok") {
        throw "Native evidence prerequisite did not pass."
    }
    foreach ($variantName in @("fullRepo", "nativeSelected", "changedFiles", "structuralGrep")) {
        if ($null -eq $scorecard.variants.$variantName) {
            throw "Missing benchmark variant: $variantName"
        }
    }
    if ([string]$scorecard.ccc.command.status -eq "available") {
        foreach ($variantName in @("fullRepo", "nativeSelected", "changedFiles")) {
            if ([string]$scorecard.variants.$variantName.status -ne "ok") {
                throw "Expected semantic benchmark variant to pass: $variantName"
            }
            if ([int]$scorecard.variants.$variantName.fileCount -le 0) {
                throw "Variant has no files: $variantName"
            }
            if ([int]$scorecard.variants.$variantName.index.durationMs -le 0) {
                throw "Variant missing index duration: $variantName"
            }
        }
        if ([string]$scorecard.variants.structuralGrep.status -ne "ok") {
            throw "Expected no-index structural grep to pass."
        }
        if ([int]$scorecard.variants.nativeSelected.fileCount -ge [int]$scorecard.variants.fullRepo.fileCount) {
            throw "Native selected slice should be smaller than full repo."
        }
        if ([int]$scorecard.variants.changedFiles.fileCount -ge [int]$scorecard.variants.fullRepo.fileCount) {
            throw "Changed-files slice should be smaller than full repo."
        }
    }

    Write-Host "PASS CCC slice benchmark scorecard: $scorecardPath"
} finally {
    Remove-TreeWithRetry -Path $temp
}
