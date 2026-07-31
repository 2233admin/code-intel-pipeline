param(
    [string]$OutputDir = "",
    [int]$Runs = 3
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$runner = Join-Path $root "run-code-intel.ps1"

if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path ([System.IO.Path]::GetTempPath()) ("code-evidence-ab-" + [guid]::NewGuid().ToString("N"))
}
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

function Test-CommandAvailable {
    param([string]$Name)
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function Invoke-Probe {
    param(
        [string]$Name,
        [scriptblock]$Script
    )

    $started = Get-Date
    $result = [ordered]@{
        name = $Name
        status = "ok"
        exitCode = 0
        durationMs = 0
        output = ""
        error = ""
    }
    try {
        $global:LASTEXITCODE = 0
        $output = & $Script 2>&1
        $result.output = (($output | ForEach-Object { $_.ToString() }) -join "`n")
        $result.exitCode = $global:LASTEXITCODE
        if ($global:LASTEXITCODE -ne 0) {
            $result.status = "unavailable"
        }
    } catch {
        $result.status = "error"
        $result.exitCode = 1
        $result.error = $_.Exception.Message
    } finally {
        $result.durationMs = [int]((Get-Date) - $started).TotalMilliseconds
    }
    return $result
}

function New-FixtureRepo {
    param([string]$Parent)

    $repo = Join-Path $Parent "fixture-repo"
    New-Item -ItemType Directory -Force -Path $repo | Out-Null
    @'
export function greet(name) {
  return `hello ${name}`;
}

export function routeUser(id) {
  return greet(id);
}
'@ | Set-Content -LiteralPath (Join-Path $repo "index.js") -Encoding UTF8
    @'
import { greet } from "./index.js";

test("greet returns message", () => {
  expect(greet("Ada")).toContain("Ada");
});
'@ | Set-Content -LiteralPath (Join-Path $repo "index.test.js") -Encoding UTF8
    & git -C $repo init --quiet
    & git -C $repo add .
    & git -C $repo -c user.email=test@example.invalid -c user.name="Code Intel Test" commit --quiet -m "fixture"
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to create fixture repo."
    }
    return $repo
}

function Read-JsonFile {
    param([string]$Path)
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

$fixtureRoot = Join-Path $OutputDir "fixture"
$repoPath = New-FixtureRepo -Parent $fixtureRoot
$stabilityRuns = New-Object System.Collections.Generic.List[object]
$pipelineExitCodes = New-Object System.Collections.Generic.List[int]
$nativeMetrics = $null

for ($i = 1; $i -le $Runs; $i++) {
    $artifactRoot = Join-Path $OutputDir "artifacts-run-$i"
    $started = Get-Date
    & $runner `
        -RepoPath $repoPath `
        -Mode lite `
        -ArtifactRoot $artifactRoot `
        -SkipRepowise `
        -SkipSentrux `
        -SkipGitHubResearch | Out-Null
    $exitCode = $LASTEXITCODE
    $pipelineExitCodes.Add([int]$exitCode)
    $repoArtifactRoot = Join-Path $artifactRoot (Split-Path -Leaf $repoPath)
    $runDir = Get-ChildItem -LiteralPath $repoArtifactRoot -Directory | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    $report = if ($null -ne $runDir) { Read-JsonFile (Join-Path $runDir.FullName "report.json") } else { $null }
    if ($null -ne $report -and $null -ne $report.codeEvidence) {
        $nativeMetrics = [ordered]@{
            files = [int]$report.codeEvidence.files
            symbols = [int]$report.codeEvidence.symbols
            chunks = [int]$report.codeEvidence.chunks
            imports = [int]$report.codeEvidence.imports
        }
    }
    $stabilityRuns.Add([ordered]@{
        run = $i
        exitCode = $exitCode
        durationMs = [int]((Get-Date) - $started).TotalMilliseconds
        artifactDir = if ($null -ne $runDir) { $runDir.FullName } else { "" }
        codeEvidenceStatus = if ($null -ne $report -and $null -ne $report.codeEvidence) { [string]$report.codeEvidence.status } else { "missing" }
    })
}

$cccAvailable = Test-CommandAvailable "ccc"
$cccHelp = if ($cccAvailable) { Invoke-Probe "ccc help" { ccc --help } } else { [ordered]@{ name = "ccc help"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" } }
$grepProbe = if ($cccAvailable) { Invoke-Probe "ccc grep help" { ccc grep --help } } else { [ordered]@{ name = "ccc grep help"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" } }
$statusProbe = if ($cccAvailable) {
    Push-Location $repoPath
    try { Invoke-Probe "ccc status" { ccc status } } finally { Pop-Location }
} else {
    [ordered]@{ name = "ccc status"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" }
}
$semanticLifecycle = if ($cccAvailable) {
    Push-Location $repoPath
    try {
        $initProbe = Invoke-Probe "ccc init" { ccc init --force }
        $indexProbe = if ($initProbe.status -eq "ok") {
            Invoke-Probe "ccc index" { ccc index }
        } else {
            [ordered]@{ name = "ccc index"; status = "skipped"; exitCode = 1; durationMs = 0; output = ""; error = "ccc init failed" }
        }
        $statusAfterIndexProbe = if ($indexProbe.status -eq "ok") {
            Invoke-Probe "ccc status after index" { ccc status }
        } else {
            [ordered]@{ name = "ccc status after index"; status = "skipped"; exitCode = 1; durationMs = 0; output = ""; error = "ccc index failed" }
        }
        $searchProbe = if ($indexProbe.status -eq "ok") {
            Invoke-Probe "ccc search" { ccc search --limit 3 user routing logic }
        } else {
            [ordered]@{ name = "ccc search"; status = "skipped"; exitCode = 1; durationMs = 0; output = ""; error = "ccc index failed" }
        }
        [ordered]@{
            init = $initProbe
            index = $indexProbe
            status = $statusAfterIndexProbe
            search = $searchProbe
        }
    } finally {
        Pop-Location
    }
} else {
    [ordered]@{
        init = [ordered]@{ name = "ccc init"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" }
        index = [ordered]@{ name = "ccc index"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" }
        status = $statusProbe
        search = [ordered]@{ name = "ccc search"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" }
    }
}

$structuralStatus = if ($grepProbe.status -eq "ok") { "available" } else { "unavailable" }
$semanticStatus = if (
    $semanticLifecycle.init.status -eq "ok" -and
    $semanticLifecycle.index.status -eq "ok" -and
    $semanticLifecycle.status.status -eq "ok" -and
    $semanticLifecycle.search.status -eq "ok"
) { "available" } elseif ($cccAvailable) { "unavailable" } else { "unavailable" }
$exitCodeSet = @($pipelineExitCodes.ToArray() | Select-Object -Unique)
$scorecard = [ordered]@{
    schema = "code-evidence-ab-scorecard.v1"
    runs = $Runs
    fixtureRepo = $repoPath
    variants = [ordered]@{
        A = [ordered]@{
            name = "native-minimal"
            status = "ok"
            metrics = $nativeMetrics
        }
        B = [ordered]@{
            name = "cocoindex-code"
            status = if ($cccAvailable) { "probed" } else { "unavailable" }
            command = "ccc"
            capabilities = [ordered]@{
                command = [ordered]@{
                    status = if ($cccAvailable) { "available" } else { "unavailable" }
                    probe = $cccHelp
                }
                structuralGrep = [ordered]@{
                    status = $structuralStatus
                    probe = $grepProbe
                }
                semanticSearch = [ordered]@{
                    status = $semanticStatus
                    probe = $statusProbe
                    lifecycle = $semanticLifecycle
                }
            }
        }
    }
    stability = [ordered]@{
        pipelineExitStable = ($exitCodeSet.Count -eq 1 -and $exitCodeSet[0] -eq 0)
        runs = @($stabilityRuns.ToArray())
    }
}

$scorecardPath = Join-Path $OutputDir "code-evidence-ab-scorecard.json"
$scorecard | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $scorecardPath -Encoding UTF8

$markdownPath = Join-Path $OutputDir "code-evidence-ab-scorecard.md"
@(
    "# Code Evidence A/B Scorecard",
    "",
    "- A: native-minimal",
    "- B: cocoindex-code",
    "- Runs: $Runs",
    "- Pipeline exit stable: $($scorecard.stability.pipelineExitStable)",
    "- B command: $($scorecard.variants.B.capabilities.command.status)",
    "- B structural grep: $($scorecard.variants.B.capabilities.structuralGrep.status)",
    "- B semantic search: $($scorecard.variants.B.capabilities.semanticSearch.status)",
    "",
    "JSON: $scorecardPath"
) | Set-Content -LiteralPath $markdownPath -Encoding UTF8

Write-Host "Code Evidence A/B scorecard: $scorecardPath"
exit 0
