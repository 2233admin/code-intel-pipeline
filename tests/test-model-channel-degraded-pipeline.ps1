#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$pipeline = Join-Path $repoRoot "run-code-intel.ps1"
$root = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-model-degraded-{0}" -f [guid]::NewGuid().ToString("N"))

try {
    $fixtureRepo = Join-Path $root "repo"
    $artifacts = Join-Path $root "artifacts"
    New-Item -ItemType Directory -Force -Path $fixtureRepo | Out-Null
    "fixture" | Set-Content -LiteralPath (Join-Path $fixtureRepo "README.md") -Encoding utf8
    & git -C $fixtureRepo init -q
    & git -C $fixtureRepo add README.md
    & git -C $fixtureRepo -c user.name=fixture -c user.email=fixture@example.invalid commit -qm init
    $routePath = Join-Path $root "routing-result.json"
    [ordered]@{
        schema = "code-intel-model-routing-result.v1"
        status = "deterministic_degraded"
        selected = $null
        authorization = [ordered]@{
            consumptionAuthorization = [ordered]@{ status = "unanswered"; scopes = @() }
            externalData = [ordered]@{ status = "unanswered" }
            paidSpend = [ordered]@{ status = "unanswered" }
        }
        attempts = @([ordered]@{
            candidateId = "fixture"; readinessState = "model_available"; eligible = $false
            failureCategory = "model_unavailable"; reason = "fixture_has_no_ready_model_route"
        })
        manualAction = "provide_or_enable_model_channel"
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $routePath -Encoding utf8

    & pwsh -NoProfile -File $pipeline `
        -RepoPath $fixtureRepo -ArtifactRoot $artifacts -Mode lite `
        -RepowiseDocs -ModelRoutingResult $routePath `
        -SkipRepowise -SkipSentrux -SkipOpenSpec -SkipRepomix | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "model-unavailable degraded pipeline exited $LASTEXITCODE" }
    $reportPath = (Get-ChildItem -LiteralPath $artifacts -Recurse -Filter report.json | Select-Object -First 1).FullName
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    if ($report.modelChannel.status -ne "deterministic_degraded") { throw "model channel status was not preserved" }
    if (-not (Test-Path -LiteralPath $report.modelChannel.assistanceDossier -PathType Leaf)) { throw "assistance dossier was not emitted" }
    $dossier = Get-Content -LiteralPath $report.modelChannel.assistanceDossier -Raw | ConvertFrom-Json
    if ($dossier.status -ne "manual_required" -or [string]::IsNullOrWhiteSpace([string]$dossier.copyablePrompt)) {
        throw "assistance dossier is incomplete"
    }
    $schemaPath = Join-Path $repoRoot "orchestration\schemas\code-intel-model-assistance-dossier.v1.schema.json"
    & python -c "import json,sys,jsonschema; jsonschema.Draft202012Validator(json.load(open(sys.argv[1],encoding='utf-8'))).validate(json.load(open(sys.argv[2],encoding='utf-8-sig')))" $schemaPath $report.modelChannel.assistanceDossier
    if ($LASTEXITCODE -ne 0) { throw "assistance dossier failed its closed JSON schema" }
    if (@($report.steps | Where-Object status -eq "failed").Count -ne 0) { throw "model absence created a failed deterministic step" }
    "PASS test-model-channel-degraded-pipeline"
}
finally {
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
