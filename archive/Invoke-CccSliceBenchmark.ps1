param(
    [string]$OutputDir = "",
    [int]$NoiseFileCount = 40
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$runner = Join-Path $root "run-code-intel.ps1"

if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path ([System.IO.Path]::GetTempPath()) ("ccc-slice-benchmark-" + [guid]::NewGuid().ToString("N"))
}
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

function Test-CommandAvailable {
    param([string]$Name)
    [bool](Get-Command $Name -ErrorAction SilentlyContinue)
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

    $result
}

function Read-JsonFile {
    param([string]$Path)
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function New-BenchmarkRepo {
    param([string]$Parent)

    $repo = Join-Path $Parent "fixture-repo"
    New-Item -ItemType Directory -Force -Path (Join-Path $repo "src") | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $repo "tests") | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $repo "noise") | Out-Null

    '{"type":"module"}' | Set-Content -LiteralPath (Join-Path $repo "package.json") -Encoding UTF8
    @'
export function addUser(user) {
  return { id: user.id, name: user.name, active: true };
}

export function findUser(users, id) {
  return users.find((user) => user.id === id);
}
'@ | Set-Content -LiteralPath (Join-Path $repo "src\users.js") -Encoding UTF8

    @'
import { findUser } from "./users.js";

export function routeUser(users, id) {
  const user = findUser(users, id);
  return user ? `/users/${user.id}` : "/users/missing";
}
'@ | Set-Content -LiteralPath (Join-Path $repo "src\router.js") -Encoding UTF8

    @'
import { addUser, findUser } from "../src/users.js";

test("adds active user", () => {
  expect(addUser({ id: 1, name: "Ada" }).active).toBe(true);
});

test("finds user by id", () => {
  expect(findUser([{ id: 1, name: "Ada" }], 1).name).toBe("Ada");
});
'@ | Set-Content -LiteralPath (Join-Path $repo "tests\users.test.js") -Encoding UTF8

    for ($i = 1; $i -le $NoiseFileCount; $i++) {
        @"
export function unrelated$i(value) {
  return value * $i;
}
"@ | Set-Content -LiteralPath (Join-Path $repo ("noise\unrelated{0}.js" -f $i)) -Encoding UTF8
    }

    & git -C $repo init --quiet
    & git -C $repo add .
    & git -C $repo -c user.email=test@example.invalid -c user.name="Code Intel Test" commit --quiet -m "fixture"
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to create fixture repo."
    }

    @'
export function createSession(user) {
  return { userId: user.id, issuedAt: Date.now() };
}
'@ | Set-Content -LiteralPath (Join-Path $repo "src\session.js") -Encoding UTF8

    Add-Content -LiteralPath (Join-Path $repo "src\users.js") -Value @'

export function deactivateUser(user) {
  return { ...user, active: false };
}
'@

    $repo
}

function Copy-RepoSlice {
    param(
        [string]$SourceRepo,
        [string]$DestinationRepo,
        [string[]]$RelativePaths
    )

    New-Item -ItemType Directory -Force -Path $DestinationRepo | Out-Null
    $paths = New-Object System.Collections.Generic.HashSet[string]
    [void]$paths.Add("package.json")
    foreach ($path in $RelativePaths) {
        if (-not [string]::IsNullOrWhiteSpace($path)) {
            [void]$paths.Add(($path -replace '/', '\'))
        }
    }

    foreach ($relativePath in $paths) {
        $source = Join-Path $SourceRepo $relativePath
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            continue
        }
        $destination = Join-Path $DestinationRepo $relativePath
        $destinationParent = Split-Path -Parent $destination
        New-Item -ItemType Directory -Force -Path $destinationParent | Out-Null
        Copy-Item -LiteralPath $source -Destination $destination -Force
    }
}

function Get-RepoFileCount {
    param([string]$Path)
    @(Get-ChildItem -LiteralPath $Path -File -Recurse | Where-Object { $_.FullName -notmatch '\\\.git\\|\\\.cocoindex_code\\' }).Count
}

function Invoke-CccSemanticLifecycle {
    param(
        [string]$Name,
        [string]$RepoPath,
        [string]$Query
    )

    if (-not (Test-CommandAvailable "ccc")) {
        return [ordered]@{
            name = $Name
            status = "unavailable"
            fileCount = Get-RepoFileCount -Path $RepoPath
            init = [ordered]@{ name = "ccc init"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" }
            index = [ordered]@{ name = "ccc index"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" }
            search = [ordered]@{ name = "ccc search"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" }
        }
    }

    Push-Location $RepoPath
    try {
        $initProbe = Invoke-Probe "$Name ccc init" { ccc init --force }
        $indexProbe = if ($initProbe.status -eq "ok") {
            Invoke-Probe "$Name ccc index" { ccc index }
        } else {
            [ordered]@{ name = "$Name ccc index"; status = "skipped"; exitCode = 1; durationMs = 0; output = ""; error = "ccc init failed" }
        }
        $searchProbe = if ($indexProbe.status -eq "ok") {
            Invoke-Probe "$Name ccc search" { ccc search --limit 3 $Query }
        } else {
            [ordered]@{ name = "$Name ccc search"; status = "skipped"; exitCode = 1; durationMs = 0; output = ""; error = "ccc index failed" }
        }

        [ordered]@{
            name = $Name
            status = if ($initProbe.status -eq "ok" -and $indexProbe.status -eq "ok" -and $searchProbe.status -eq "ok") { "ok" } else { "unavailable" }
            fileCount = Get-RepoFileCount -Path $RepoPath
            init = $initProbe
            index = $indexProbe
            search = $searchProbe
        }
    } finally {
        Pop-Location
    }
}

$fixtureRoot = Join-Path $OutputDir "fixture"
$repoPath = New-BenchmarkRepo -Parent $fixtureRoot

$artifactRoot = Join-Path $OutputDir "native-artifacts"
& $runner -RepoPath $repoPath -Mode lite -ArtifactRoot $artifactRoot -SkipRepowise -SkipSentrux -SkipGitHubResearch | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw "Native code evidence run failed."
}

$repoArtifactRoot = Join-Path $artifactRoot (Split-Path -Leaf $repoPath)
$runDir = Get-ChildItem -LiteralPath $repoArtifactRoot -Directory | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($null -eq $runDir) {
    throw "Native code evidence artifact directory was not created."
}

$report = Read-JsonFile (Join-Path $runDir.FullName "report.json")
$filesJson = Read-JsonFile (Join-Path $runDir.FullName "code-evidence\merged\full\files.json")
$symbolsJson = Read-JsonFile (Join-Path $runDir.FullName "code-evidence\merged\full\symbols.json")
$importsJson = Read-JsonFile (Join-Path $runDir.FullName "code-evidence\merged\full\imports.json")

$queryTerms = @("user", "session", "route")
$selected = New-Object System.Collections.Generic.HashSet[string]
foreach ($file in $filesJson.files) {
    $path = [string]$file.path
    if ($queryTerms | Where-Object { $path -match $_ }) {
        [void]$selected.Add($path)
    }
}
foreach ($symbol in $symbolsJson.symbols) {
    if ($queryTerms | Where-Object { ([string]$symbol.name) -match $_ }) {
        [void]$selected.Add([string]$symbol.file)
    }
}
foreach ($import in $importsJson.imports) {
    $importFile = [string]$import.file
    $target = [string]$import.target
    if ($selected.Contains($importFile) -or ($queryTerms | Where-Object { $target -match $_ })) {
        [void]$selected.Add($importFile)
    }
}

$changedFiles = @(& git -C $repoPath diff --name-only HEAD | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })

$fullRepo = Join-Path $OutputDir "ccc-full-repo"
$nativeSelectedRepo = Join-Path $OutputDir "ccc-native-selected"
$changedFilesRepo = Join-Path $OutputDir "ccc-changed-files"

Copy-Item -LiteralPath $repoPath -Destination $fullRepo -Recurse -Force
Copy-RepoSlice -SourceRepo $repoPath -DestinationRepo $nativeSelectedRepo -RelativePaths @($selected)
Copy-RepoSlice -SourceRepo $repoPath -DestinationRepo $changedFilesRepo -RelativePaths $changedFiles

$cccAvailable = Test-CommandAvailable "ccc"
$cccHelp = if ($cccAvailable) { Invoke-Probe "ccc help" { ccc --help } } else { [ordered]@{ name = "ccc help"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" } }
$doctorProbe = if ($cccAvailable) { Invoke-Probe "ccc doctor warmup" { ccc doctor } } else { [ordered]@{ name = "ccc doctor warmup"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" } }

$fullResult = Invoke-CccSemanticLifecycle -Name "fullRepo" -RepoPath $fullRepo -Query "user routing session logic"
$nativeSelectedResult = Invoke-CccSemanticLifecycle -Name "nativeSelected" -RepoPath $nativeSelectedRepo -Query "user routing session logic"
$changedFilesResult = Invoke-CccSemanticLifecycle -Name "changedFiles" -RepoPath $changedFilesRepo -Query "user session changes"

$grepProbe = if ($cccAvailable) {
    Invoke-Probe "ccc grep no-index" { ccc grep 'function \NAME(\(ARGS*\))' $repoPath --lang javascript --no-color }
} else {
    [ordered]@{ name = "ccc grep no-index"; status = "unavailable"; exitCode = 127; durationMs = 0; output = ""; error = "ccc not found" }
}

$scorecard = [ordered]@{
    schema = "ccc-slice-benchmark.v1"
    fixtureRepo = $repoPath
    nativeEvidence = [ordered]@{
        status = [string]$report.codeEvidence.status
        artifactDir = $runDir.FullName
        files = [int]$report.codeEvidence.files
        symbols = [int]$report.codeEvidence.symbols
        chunks = [int]$report.codeEvidence.chunks
        selectedFiles = @($selected)
        changedFiles = @($changedFiles)
    }
    ccc = [ordered]@{
        command = [ordered]@{
            status = if ($cccAvailable) { "available" } else { "unavailable" }
            probe = $cccHelp
        }
        warmup = $doctorProbe
    }
    variants = [ordered]@{
        fullRepo = $fullResult
        nativeSelected = $nativeSelectedResult
        changedFiles = $changedFilesResult
        structuralGrep = [ordered]@{
            name = "structuralGrep"
            status = if ($grepProbe.status -eq "ok") { "ok" } else { "unavailable" }
            fileCount = Get-RepoFileCount -Path $repoPath
            grep = $grepProbe
        }
    }
}

$scorecardPath = Join-Path $OutputDir "ccc-slice-benchmark-scorecard.json"
$scorecard | ConvertTo-Json -Depth 14 | Set-Content -LiteralPath $scorecardPath -Encoding UTF8

$markdownPath = Join-Path $OutputDir "ccc-slice-benchmark-scorecard.md"
@(
    "# CCC Slice Benchmark Scorecard",
    "",
    "- Native evidence: $($scorecard.nativeEvidence.status)",
    "- Full repo files: $($scorecard.variants.fullRepo.fileCount), index ms: $($scorecard.variants.fullRepo.index.durationMs)",
    "- Native selected files: $($scorecard.variants.nativeSelected.fileCount), index ms: $($scorecard.variants.nativeSelected.index.durationMs)",
    "- Changed files: $($scorecard.variants.changedFiles.fileCount), index ms: $($scorecard.variants.changedFiles.index.durationMs)",
    "- Structural grep ms: $($scorecard.variants.structuralGrep.grep.durationMs)",
    "",
    "JSON: ``$scorecardPath``"
) | Set-Content -LiteralPath $markdownPath -Encoding UTF8

Write-Host "CCC slice benchmark scorecard: $scorecardPath"
exit 0
