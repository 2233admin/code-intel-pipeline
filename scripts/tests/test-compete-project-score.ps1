#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$adapter = Join-Path $root "Invoke-CompeteProjectScore.ps1"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-compete-test-" + [guid]::NewGuid().ToString("N"))
$repo = Join-Path $tempRoot "repo"
$artifacts = Join-Path $tempRoot "artifacts"
$compete = Join-Path $tempRoot "compete"
$scripts = Join-Path $compete "skills/compete/scripts"
$data = Join-Path $tempRoot "data"

New-Item -ItemType Directory -Force -Path $repo, $artifacts, $scripts, $data | Out-Null
try {
    & $adapter -Action prepare -RepoPath $repo -ArtifactRoot $artifacts -CompeteRoot $compete | Out-Null
    $request = Get-Content -LiteralPath (Join-Path $artifacts "competitive-intelligence-request.json") -Raw | ConvertFrom-Json
    if ($request.schema -ne "code-intel-competitive-intelligence-request.v1") { throw "unexpected request schema" }
    if ($request.status -ne "prepared" -or $request.authority -ne "advisory") { throw "prepared request must stay advisory" }
    if (-not (Test-Path -LiteralPath $request.prompt -PathType Leaf)) { throw "Agent prompt was not written" }

    @'
RADAR_AXES = ["Scale", "Pricing", "Marketing", "Social", "SEO", "Tech"]
def load_all(input_dir): return {}
def build_entities(data): return [{"ref": "self", "name": "Fixture", "is_self": True}]
def synth_report(data, entities, now_iso):
    return ({"executive_summary": {"key_findings": {"value": ["one"]}}}, {"self": {"scores": dict(zip(RADAR_AXES, [10, 20, 30, 40, 50, 60]))}})
def build_view_models(entities, enriched):
    return {"radar": {"axes": RADAR_AXES, "series": [{"ref": "self", "name": "Fixture", "scores": [10, 20, 30, 40, 50, 60], "is_self": True}]}}
'@ | Set-Content -LiteralPath (Join-Path $scripts "build_report.py") -Encoding utf8

    & $adapter -Action score -RepoPath $repo -ArtifactRoot $artifacts -CompeteRoot $compete -CompeteDataPath $data | Out-Null
    $score = Get-Content -LiteralPath (Join-Path $artifacts "competitive-score.json") -Raw | ConvertFrom-Json
    if ($score.schema -ne "code-intel-competitive-score.v1") { throw "unexpected score schema" }
    if ($score.status -ne "completed" -or $score.authority -ne "advisory") { throw "score must stay advisory" }
    if ([double]$score.overallScore -ne 35) { throw "expected mean score 35" }
    if (@($score.axes).Count -ne 6) { throw "expected six score axes" }
    if ($score.source.generator -ne "compete/build_report.py") { throw "score source must identify upstream implementation" }

    Write-Host "Compete project score test: OK"
}
finally {
    if (Test-Path -LiteralPath $tempRoot) { Remove-Item -LiteralPath $tempRoot -Recurse -Force }
}
