param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$runner = Join-Path $root "run-code-intel.ps1"

function Read-JsonFile {
    param([string]$Path)
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function New-TestRepo {
    param([string]$Parent)

    $repo = Join-Path $Parent "repo"
    New-Item -ItemType Directory -Force -Path $repo | Out-Null
    @'
function greet(name) {
  return `hello ${name}`;
}

module.exports = { greet };
'@ | Set-Content -LiteralPath (Join-Path $repo "index.js") -Encoding UTF8
    @'
const { greet } = require("./index");

test("greet returns message", () => {
  expect(greet("Ada")).toContain("Ada");
});
'@ | Set-Content -LiteralPath (Join-Path $repo "index.test.js") -Encoding UTF8

    & git -C $repo init --quiet
    & git -C $repo add .
    & git -C $repo -c user.email=test@example.invalid -c user.name="Code Intel Test" commit --quiet -m "fixture"
    if ($LASTEXITCODE -ne 0) {
        throw "Failed git fixture repo."
    }

    $repo
}

function Write-TestConfig {
    param(
        [string]$Path,
        [bool]$CocoEnabled,
        [string]$CocoCommand = "ccc"
    )

    $config = [ordered]@{
        artifactRoot = ""
        repowiseWorkspaceRoot = ""
        codeEvidence = [ordered]@{
            enabled = $true
            nativeMinimal = $true
            adapters = [ordered]@{
                "cocoindex-code" = [ordered]@{
                    enabled = $CocoEnabled
                    mode = "evaluate"
                    required = $false
                    offlineOnly = $true
                    command = $CocoCommand
                }
            }
        }
        inventoryExclude = @()
        repos = [ordered]@{}
    }

    $config | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Path -Encoding UTF8
}

function Invoke-CodeEvidenceFixture {
    param(
        [string]$TempRoot,
        [string]$ScenarioName,
        [bool]$CocoEnabled,
        [string]$CocoCommand = "ccc"
    )

    $scenarioRoot = Join-Path $TempRoot $ScenarioName
    $artifactRoot = Join-Path $scenarioRoot "artifacts"
    New-Item -ItemType Directory -Force -Path $scenarioRoot | Out-Null
    $repo = New-TestRepo -Parent $scenarioRoot
    $configPath = Join-Path $scenarioRoot "pipeline.config.json"
    Write-TestConfig -Path $configPath -CocoEnabled $CocoEnabled -CocoCommand $CocoCommand

    & $runner `
        -RepoPath $repo `
        -Config $configPath `
        -Mode lite `
        -ArtifactRoot $artifactRoot `
        -SkipRepowise `
        -SkipSentrux `
        -SkipGitHubResearch | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "run-code-intel.ps1 failed with exit code $LASTEXITCODE for $ScenarioName"
    }

    $repoArtifactRoot = Join-Path $artifactRoot (Split-Path -Leaf $repo)
    $runDir = Get-ChildItem -LiteralPath $repoArtifactRoot -Directory | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($null -eq $runDir) {
        throw "No artifact run directory produced for $ScenarioName."
    }

    $runDir.FullName
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

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("code-evidence-layer-test-" + [guid]::NewGuid().ToString("N"))

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null

    $runPath = Invoke-CodeEvidenceFixture -TempRoot $temp -ScenarioName "disabled" -CocoEnabled $false
    $runDir = Get-Item -LiteralPath $runPath
    $report = Read-JsonFile (Join-Path $runDir.FullName "report.json")
    if ($null -eq $report.codeEvidence) {
        throw "report.json missing codeEvidence summary."
    }
    if ([string]$report.codeEvidence.status -ne "ok") {
        throw "Expected native codeEvidence status ok, got '$($report.codeEvidence.status)'."
    }

    $agentIndex = Join-Path $runDir.FullName "code-evidence\merged\agent\index.md"
    $agentRankingJson = Join-Path $runDir.FullName "code-evidence\merged\agent\ranking.json"
    $nativeRetrievalSlice = Join-Path $runDir.FullName "code-evidence\merged\agent\slices\native-retrieval.md"
    $filesJson = Join-Path $runDir.FullName "code-evidence\merged\full\files.json"
    $symbolsJson = Join-Path $runDir.FullName "code-evidence\merged\full\symbols.json"
    $chunksJson = Join-Path $runDir.FullName "code-evidence\merged\full\chunks.json"
    $symbolChunksJson = Join-Path $runDir.FullName "code-evidence\merged\full\symbol-chunks.json"
    $importsJson = Join-Path $runDir.FullName "code-evidence\merged\full\imports.json"
    $scorecardJson = Join-Path $runDir.FullName "code-evidence\merged\scorecard.json"
    $cocoOutcomeJson = Join-Path $runDir.FullName "code-evidence\adapters\cocoindex-code\outcome.json"
    foreach ($path in @($agentIndex, $agentRankingJson, $nativeRetrievalSlice, $filesJson, $symbolsJson, $chunksJson, $symbolChunksJson, $importsJson, $scorecardJson, $cocoOutcomeJson)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Missing Code Evidence artifact: $path"
        }
    }

    $files = Read-JsonFile $filesJson
    if (@($files.files).Count -lt 2) {
        throw "Expected at least two files in files.json."
    }
    $symbols = Read-JsonFile $symbolsJson
    if (-not (@($symbols.symbols) | Where-Object { $_.name -eq "greet" -and $_.kind -eq "function" })) {
        throw "Expected greet function symbol in symbols.json."
    }
    $imports = Read-JsonFile $importsJson
    if (-not (@($imports.imports) | Where-Object { $_.target -eq "./index" })) {
        throw "Expected imports.json to record require('./index')."
    }
    $cocoOutcome = Read-JsonFile $cocoOutcomeJson
    if ([string]$cocoOutcome.status -ne "skipped" -or [bool]$cocoOutcome.fatal) {
        throw "Expected disabled cocoindex-code adapter to skip non-fatally."
    }
    if ([bool]$cocoOutcome.enabled) {
        throw "Expected disabled cocoindex-code adapter outcome to record enabled=false."
    }
    if ([bool]$cocoOutcome.required) {
        throw "Expected disabled cocoindex-code adapter outcome to record required=false."
    }
    if ([string]$cocoOutcome.reasonCode -ne "disabled") {
        throw "Expected disabled cocoindex-code reasonCode=disabled."
    }

    $agentRanking = Read-JsonFile $agentRankingJson
    if ([string]$agentRanking.schema -ne "agent-code-slice-ranking.v1") {
        throw "Unexpected Agent Code Slice ranking schema."
    }
    if (@($agentRanking.files).Count -lt 2) {
        throw "Expected Agent Code Slice ranking to include fixture files."
    }
    if (-not (@($agentRanking.files) | Where-Object { $_.path -eq "index.js" -and $_.reasons -contains "entrypoint" })) {
        throw "Expected index.js to be ranked as an entrypoint."
    }
    if (-not (@($agentRanking.files) | Where-Object { $_.path -eq "index.test.js" -and $_.reasons -contains "test" })) {
        throw "Expected index.test.js to be ranked as a test."
    }

    $understanding = Get-Content -LiteralPath (Join-Path $runDir.FullName "understanding.md") -Raw
    if ($understanding -notmatch "code-evidence/merged/agent/index\.md") {
        throw "understanding.md does not link Agent Code Slice."
    }
    $agentIndexText = Get-Content -LiteralPath $agentIndex -Raw
    if ($agentIndexText -notmatch "ranking\.json" -or $agentIndexText -notmatch "native-retrieval\.md") {
        throw "Agent Code Map does not link ranking and native retrieval slice."
    }
    $summary = Get-Content -LiteralPath (Join-Path $runDir.FullName "summary.md") -Raw
    if ($summary -notmatch "Code Evidence") {
        throw "summary.md does not expose Code Evidence status."
    }

    $enabledRunPath = Invoke-CodeEvidenceFixture `
        -TempRoot $temp `
        -ScenarioName "enabled-missing-command" `
        -CocoEnabled $true `
        -CocoCommand "definitely-missing-ccc-for-code-intel-test"
    $enabledOutcomePath = Join-Path $enabledRunPath "code-evidence\adapters\cocoindex-code\outcome.json"
    $enabledOutcome = Read-JsonFile $enabledOutcomePath
    if ([string]$enabledOutcome.status -ne "skipped") {
        throw "Expected enabled missing cocoindex-code command to skip, got '$($enabledOutcome.status)'."
    }
    if ([bool]$enabledOutcome.fatal) {
        throw "Expected missing cocoindex-code command to stay non-fatal."
    }
    if (-not [bool]$enabledOutcome.enabled) {
        throw "Expected enabled cocoindex-code adapter outcome to record enabled=true."
    }
    if ([string]$enabledOutcome.command -ne "definitely-missing-ccc-for-code-intel-test") {
        throw "Expected cocoindex-code outcome to record configured command."
    }
    if ([bool]$enabledOutcome.required) {
        throw "Expected missing cocoindex-code command scenario to record required=false."
    }
    if ([string]$enabledOutcome.reasonCode -ne "command_unavailable") {
        throw "Expected missing cocoindex-code command reasonCode=command_unavailable."
    }
    if ([string]$enabledOutcome.reason -notmatch "not found|unavailable|missing") {
        throw "Expected missing cocoindex-code reason to mention unavailable command."
    }

    Write-Host "PASS code evidence layer connectivity: $($runDir.FullName)"
} finally {
    Remove-TreeWithRetry -Path $temp
}
