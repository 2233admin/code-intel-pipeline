#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("prepare", "score")]
    [string]$Action,
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,
    [Parameter(Mandatory = $true)]
    [string]$ArtifactRoot,
    [string]$CompeteRoot = "",
    [string]$CompeteDataPath = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-CompeteRoot {
    param([string]$RequestedRoot)

    $candidates = @(
        $RequestedRoot,
        $env:COMPETE_HOME,
        (Join-Path $HOME ".claude/plugins/compete"),
        (Join-Path $HOME ".claude/skills/compete")
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }

    foreach ($candidate in $candidates) {
        $full = [IO.Path]::GetFullPath($candidate)
        if (Test-Path -LiteralPath (Join-Path $full "skills/compete/scripts/build_report.py") -PathType Leaf) { return $full }
        if (Test-Path -LiteralPath (Join-Path $full "scripts/build_report.py") -PathType Leaf) { return $full }
    }
    return $(if ([string]::IsNullOrWhiteSpace($RequestedRoot)) { "" } else { [IO.Path]::GetFullPath($RequestedRoot) })
}

function Get-CompeteScriptRoot {
    param([string]$Root)
    $pluginScripts = Join-Path $Root "skills/compete/scripts"
    if (Test-Path -LiteralPath (Join-Path $pluginScripts "build_report.py") -PathType Leaf) { return $pluginScripts }
    $skillScripts = Join-Path $Root "scripts"
    if (Test-Path -LiteralPath (Join-Path $skillScripts "build_report.py") -PathType Leaf) { return $skillScripts }
    throw "compete build_report.py is missing under: $Root"
}

$repo = (Resolve-Path -LiteralPath $RepoPath -ErrorAction Stop).Path
$artifactDirectory = [IO.Path]::GetFullPath($ArtifactRoot)
New-Item -ItemType Directory -Force -Path $artifactDirectory | Out-Null
$resolvedCompeteRoot = Resolve-CompeteRoot -RequestedRoot $CompeteRoot

if ($Action -eq "prepare") {
    $dataPath = if ([string]::IsNullOrWhiteSpace($CompeteDataPath)) { Join-Path $artifactDirectory "compete-data" } else { [IO.Path]::GetFullPath($CompeteDataPath) }
    $promptPath = Join-Path $artifactDirectory "competitive-intelligence-prompt.md"
    $requestPath = Join-Path $artifactDirectory "competitive-intelligence-request.json"
    $scorePath = Join-Path $artifactDirectory "competitive-score.json"
    $adapterPath = $PSCommandPath
    $competeLabel = if ([string]::IsNullOrWhiteSpace($resolvedCompeteRoot)) { "<install-or-clone-compete>" } else { $resolvedCompeteRoot }

    $prompt = @"
# Competitive project score task

Analyze $repo with the upstream compete skill at $competeLabel.

- Keep every generated dataset and report under $dataPath; do not write generated files into the repository.
- Run product analysis against $repo, then perform competitor discovery and intelligence collection with web research.
- Preserve compete confidence/provenance fields and prefer unknown over guessing.
- Produce the normal compete JSON datasets and InsightKit report.
- When complete, normalize the score with:

~~~powershell
& '$adapterPath' -Action score -RepoPath '$repo' -ArtifactRoot '$artifactDirectory' -CompeteRoot '$competeLabel' -CompeteDataPath '$dataPath'
~~~

The resulting score is advisory market/product intelligence. It must not change Code Intel structural gates or hospital discharge state.
"@
    [IO.File]::WriteAllText($promptPath, $prompt, [Text.UTF8Encoding]::new($false))

    $request = [ordered]@{
        schema = "code-intel-competitive-intelligence-request.v1"
        status = "prepared"
        authority = "advisory"
        repoPath = $repo
        artifactRoot = $artifactDirectory
        competeRoot = $(if ([string]::IsNullOrWhiteSpace($resolvedCompeteRoot)) { $null } else { $resolvedCompeteRoot })
        source = "https://github.com/lbj96347/compete"
        prompt = $promptPath
        expectedDataPath = $dataPath
        scoreArtifact = $scorePath
        dispatch = [ordered]@{
            coordinator = "orca"
            available = $null -ne (Get-Command orca -ErrorAction SilentlyContinue)
            instruction = "Open an Agent terminal for the target worktree and send the prompt file contents. Agent/web usage remains explicit and non-blocking."
        }
    }
    [IO.File]::WriteAllText($requestPath, ($request | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $request | ConvertTo-Json -Depth 8 -Compress
    exit 0
}

if ([string]::IsNullOrWhiteSpace($resolvedCompeteRoot)) { throw "-CompeteRoot or COMPETE_HOME is required for score" }
if ([string]::IsNullOrWhiteSpace($CompeteDataPath)) { throw "-CompeteDataPath is required for score" }
$dataDirectory = (Resolve-Path -LiteralPath $CompeteDataPath -ErrorAction Stop).Path
$competeScripts = Get-CompeteScriptRoot -Root $resolvedCompeteRoot
$normalizer = Join-Path $PSScriptRoot "tools/normalize_compete_score.py"
if (-not (Test-Path -LiteralPath $normalizer -PathType Leaf)) { throw "score normalizer is missing: $normalizer" }
$outputPath = Join-Path $artifactDirectory "competitive-score.json"

& python $normalizer --repo $repo --compete-scripts $competeScripts --data-dir $dataDirectory --output $outputPath
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Get-Content -LiteralPath $outputPath -Raw
