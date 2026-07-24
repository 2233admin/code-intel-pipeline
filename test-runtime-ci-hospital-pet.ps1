#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$temp = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-runtime-pet-" + [guid]::NewGuid().ToString("N"))
$repo = Join-Path $temp "repo"
$sourceRoot = Join-Path $temp "runtime"
$artifacts = Join-Path $temp "artifacts"
New-Item -ItemType Directory -Force -Path $repo,$sourceRoot,$artifacts | Out-Null
try {
    [IO.File]::WriteAllText((Join-Path $repo "main.ps1"), "'ok'", [Text.UTF8Encoding]::new($false))
    $snapshot = "a" * 64
    $source = [ordered]@{
        schema="code-intel-runtime-ci-observation.v1"
        provider=[ordered]@{id="fixture.local-json";runId="run-42";sourceRevision="abc123"}
        provenance=[ordered]@{collectorId="fixture-exporter";collectorVersion="1.0.0";collectionId="collection-42";collectedAt=1950}
        snapshotIdentity=$snapshot;observedAt=1950;completeness="complete"
        signals=[ordered]@{
            tests=[ordered]@{status="passed";observed=$true;summary="fixture passed"}
            build=[ordered]@{status="passed";observed=$true;summary="fixture passed"}
            runtime=[ordered]@{status="healthy";observed=$true;summary="fixture healthy"}
        }
    }
    $sourcePath = Join-Path $sourceRoot "runtime-ci.json"
    [IO.File]::WriteAllText($sourcePath, ($source | ConvertTo-Json -Depth 8 -Compress), [Text.UTF8Encoding]::new($false))
    $request = [ordered]@{
        schema="code-intel-runtime-ci-ingest-request.v1";expectedSnapshotIdentity=$snapshot
        artifact=[ordered]@{path="runtime-ci.json";sha256=(Get-FileHash $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()}
        policy=[ordered]@{evaluatedAt=2000;maxAgeSeconds=100}
    }
    $requestPath = Join-Path $temp "request.json"
    [IO.File]::WriteAllText($requestPath, ($request | ConvertTo-Json -Depth 6 -Compress), [Text.UTF8Encoding]::new($false))
    & (Join-Path $root "run-code-intel.ps1") -RepoPath $repo -ArtifactRoot $artifacts -Mode lite -SkipRepowise -SkipSentrux -SkipGitHubResearch -SkipOpenSpec -RuntimeCiEvidenceRequest $requestPath -RuntimeCiEvidenceArtifactRoot $sourceRoot | Out-Null
    $latest = Get-ChildItem -LiteralPath (Join-Path $artifacts "repo") -Directory | Sort-Object Name -Descending | Select-Object -First 1
    if ($null -eq $latest) { throw "pipeline did not publish an artifact run" }
    $hospital = Get-Content (Join-Path $latest.FullName "hospital-report.json") -Raw | ConvertFrom-Json
    $pet = @($hospital.modalities | Where-Object name -eq "pet") | Select-Object -First 1
    if ($null -eq $pet -or [string]$pet.artifact -notmatch 'runtime-ci-summary\.json$' -or [string]$pet.finding -notmatch 'health=green') { throw "Hospital/PET did not cite admitted runtime/CI summary" }
    "runtime/CI Hospital PET integration passed"
}
finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
