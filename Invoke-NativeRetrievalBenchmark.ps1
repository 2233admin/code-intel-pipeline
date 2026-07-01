param(
    [string]$OutputDir = "",
    [int]$NoiseFileCount = 40,
    [string]$Query = "user routing session logic"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$runner = Join-Path $root "run-code-intel.ps1"

if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path ([System.IO.Path]::GetTempPath()) ("native-retrieval-benchmark-" + [guid]::NewGuid().ToString("N"))
}
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

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

export function deactivateUser(user) {
  return { ...user, active: false };
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
export function createSession(user) {
  return { userId: user.id, issuedAt: Date.now() };
}
'@ | Set-Content -LiteralPath (Join-Path $repo "src\session.js") -Encoding UTF8

    @'
import { addUser, findUser } from "../src/users.js";
import { routeUser } from "../src/router.js";

test("adds active user", () => {
  expect(addUser({ id: 1, name: "Ada" }).active).toBe(true);
});

test("finds user route by id", () => {
  const route = routeUser([{ id: 1, name: "Ada" }], 1);
  expect(route).toBe("/users/1");
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

    $repo
}

function Get-QueryTerms {
    param([string]$Text)

    @($Text.ToLowerInvariant() -split '[^a-z0-9_]+' | Where-Object { $_.Length -ge 3 } | Select-Object -Unique)
}

function Add-SelectedFile {
    param(
        [System.Collections.Generic.HashSet[string]]$Set,
        [string]$Path
    )

    if (-not [string]::IsNullOrWhiteSpace($Path)) {
        [void]$Set.Add(($Path -replace '\\', '/'))
    }
}

function Resolve-ImportTarget {
    param(
        [string]$Importer,
        [string]$Target
    )

    if ([string]::IsNullOrWhiteSpace($Target) -or -not $Target.StartsWith(".")) {
        return ""
    }

    $parent = Split-Path -Parent ($Importer -replace '/', '\')
    $candidate = Join-Path $parent $Target
    $candidate = [System.IO.Path]::GetFullPath((Join-Path "C:\native-retrieval-root" $candidate))
    $root = [System.IO.Path]::GetFullPath("C:\native-retrieval-root")
    if (-not $candidate.StartsWith($root)) {
        return ""
    }
    $relative = $candidate.Substring($root.Length).TrimStart('\') -replace '\\', '/'
    if ([System.IO.Path]::GetExtension($relative) -eq "") {
        $relative = "$relative.js"
    }
    $relative
}

function Select-NativeFiles {
    param(
        [object]$FilesJson,
        [object]$SymbolsJson,
        [object]$ImportsJson,
        [string]$QueryText
    )

    $started = Get-Date
    $terms = Get-QueryTerms -Text $QueryText
    $selected = [System.Collections.Generic.HashSet[string]]::new()

    foreach ($file in $FilesJson.files) {
        $path = [string]$file.path
        $haystack = $path.ToLowerInvariant()
        foreach ($term in $terms) {
            if ($haystack.Contains($term)) {
                Add-SelectedFile -Set $selected -Path $path
            }
        }
    }

    foreach ($symbol in $SymbolsJson.symbols) {
        $name = ([string]$symbol.name).ToLowerInvariant()
        foreach ($term in $terms) {
            if ($name.Contains($term)) {
                Add-SelectedFile -Set $selected -Path ([string]$symbol.file)
            }
        }
    }

    $importsByFile = @{}
    foreach ($import in $ImportsJson.imports) {
        $file = ([string]$import.file) -replace '\\', '/'
        if (-not $importsByFile.ContainsKey($file)) {
            $importsByFile[$file] = New-Object System.Collections.Generic.List[object]
        }
        $importsByFile[$file].Add($import)
    }

    $frontier = @($selected)
    foreach ($file in $frontier) {
        if ($importsByFile.ContainsKey($file)) {
            foreach ($import in $importsByFile[$file]) {
                $resolved = Resolve-ImportTarget -Importer $file -Target ([string]$import.target)
                Add-SelectedFile -Set $selected -Path $resolved
            }
        }
    }

    foreach ($import in $ImportsJson.imports) {
        $file = ([string]$import.file) -replace '\\', '/'
        $resolved = Resolve-ImportTarget -Importer $file -Target ([string]$import.target)
        if ($selected.Contains($resolved)) {
            Add-SelectedFile -Set $selected -Path $file
        }
    }

    [ordered]@{
        durationMs = [int]((Get-Date) - $started).TotalMilliseconds
        queryTerms = @($terms)
        selectedFiles = @($selected)
    }
}

$fixtureRoot = Join-Path $OutputDir "fixture"
$repoPath = New-BenchmarkRepo -Parent $fixtureRoot

$artifactRoot = Join-Path $OutputDir "native-artifacts"
$pipelineStarted = Get-Date
& $runner -RepoPath $repoPath -Mode lite -ArtifactRoot $artifactRoot -SkipRepowise -SkipSentrux -SkipGitHubResearch | Out-Null
$pipelineDurationMs = [int]((Get-Date) - $pipelineStarted).TotalMilliseconds
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

$selection = Select-NativeFiles -FilesJson $filesJson -SymbolsJson $symbolsJson -ImportsJson $importsJson -QueryText $Query
$expectedFiles = @("src/users.js", "src/router.js", "src/session.js", "tests/users.test.js")
$hits = @($expectedFiles | Where-Object { $selection.selectedFiles -contains $_ })
$recall = if ($expectedFiles.Count -eq 0) { 1.0 } else { [double]$hits.Count / [double]$expectedFiles.Count }

$scorecard = [ordered]@{
    schema = "native-retrieval-benchmark.v1"
    fixtureRepo = $repoPath
    query = $Query
    nativeEvidence = [ordered]@{
        status = [string]$report.codeEvidence.status
        artifactDir = $runDir.FullName
        durationMs = $pipelineDurationMs
        files = [int]$report.codeEvidence.files
        symbols = [int]$report.codeEvidence.symbols
        chunks = [int]$report.codeEvidence.chunks
        imports = [int]$report.codeEvidence.imports
    }
    retrieval = [ordered]@{
        selection = [ordered]@{
            durationMs = [int]$selection.durationMs
            queryTerms = @($selection.queryTerms)
        }
        selectedFileCount = @($selection.selectedFiles).Count
        selectedFiles = @($selection.selectedFiles)
        expectedFiles = $expectedFiles
        hitFiles = $hits
        recallAtSelected = $recall
    }
}

$scorecardPath = Join-Path $OutputDir "native-retrieval-scorecard.json"
$scorecard | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $scorecardPath -Encoding UTF8

$markdownPath = Join-Path $OutputDir "native-retrieval-scorecard.md"
@(
    "# Native Retrieval Scorecard",
    "",
    "- Native evidence files: $($scorecard.nativeEvidence.files)",
    "- Native evidence duration ms: $($scorecard.nativeEvidence.durationMs)",
    "- Selection duration ms: $($scorecard.retrieval.selection.durationMs)",
    "- Selected files: $($scorecard.retrieval.selectedFileCount)",
    "- Recall: $($scorecard.retrieval.recallAtSelected)",
    "",
    "JSON: ``$scorecardPath``"
) | Set-Content -LiteralPath $markdownPath -Encoding UTF8

Write-Host "Native retrieval scorecard: $scorecardPath"
exit 0
