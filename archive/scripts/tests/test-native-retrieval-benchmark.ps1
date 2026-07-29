param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$benchmarkScript = Join-Path $root "Invoke-NativeRetrievalBenchmark.ps1"

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
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
            return
        } catch {
            if ($attempt -eq 5) {
                throw
            }
            Start-Sleep -Milliseconds (200 * $attempt)
        }
    }
}

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("native-retrieval-benchmark-test-" + [guid]::NewGuid().ToString("N"))

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null

    & $benchmarkScript -OutputDir $temp -NoiseFileCount 20 -Query "user routing session logic" | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "Invoke-NativeRetrievalBenchmark.ps1 failed with exit code $LASTEXITCODE"
    }

    $scorecardPath = Join-Path $temp "native-retrieval-scorecard.json"
    if (-not (Test-Path -LiteralPath $scorecardPath -PathType Leaf)) {
        throw "Missing native retrieval scorecard: $scorecardPath"
    }

    $scorecard = Read-JsonFile $scorecardPath
    if ([string]$scorecard.schema -ne "native-retrieval-benchmark.v1") {
        throw "Unexpected native retrieval schema."
    }
    if ([string]$scorecard.nativeEvidence.status -ne "ok") {
        throw "Native evidence did not pass."
    }
    if ([int]$scorecard.retrieval.selection.durationMs -lt 0) {
        throw "Selection duration missing."
    }
    if ([int]$scorecard.retrieval.selectedFileCount -le 0) {
        throw "Native retrieval selected no files."
    }
    if ([int]$scorecard.retrieval.selectedFileCount -ge [int]$scorecard.nativeEvidence.files) {
        throw "Native retrieval did not crop the repo."
    }
    foreach ($expected in @("src/users.js", "src/router.js", "src/session.js", "tests/users.test.js")) {
        if ($scorecard.retrieval.selectedFiles -notcontains $expected) {
            throw "Native retrieval missed expected file: $expected"
        }
    }
    if ([double]$scorecard.retrieval.recallAtSelected -lt 1.0) {
        throw "Native retrieval recall should be complete for fixture."
    }

    Write-Host "PASS native retrieval benchmark scorecard: $scorecardPath"
} finally {
    Remove-TreeWithRetry -Path $temp
}
