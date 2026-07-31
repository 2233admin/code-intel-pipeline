#requires -Version 7.2
param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,

    [string]$ArtifactDir = "",

    [string]$Workspace = "",

    [string[]]$Exclude = @(),

    [ValidateSet("auto", "docker", "podman")]
    [string]$ContainerRuntime = "auto",

    [switch]$Analyze,

    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-CodeIntelDirectory {
    param([string]$Path, [string]$Label)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "$Label is required."
    }

    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        throw "$Label must be a directory: $Path"
    }

    return $item.FullName
}

function ConvertTo-GreenfieldArgument {
    param([string]$Value)

    $escaped = $Value.Replace('"', '\"')
    return '"' + $escaped + '"'
}

function New-GreenfieldAnalyzePrompt {
    param(
        [string]$TargetPath,
        [string]$WorkspacePath,
        [string[]]$ExcludedSources,
        [string]$Runtime
    )

    $parts = @(
        "/analyze",
        (ConvertTo-GreenfieldArgument $TargetPath),
        "--workspace",
        (ConvertTo-GreenfieldArgument $WorkspacePath)
    )

    if ($ExcludedSources.Count -gt 0) {
        $parts += @("--exclude", ($ExcludedSources -join ","))
    }

    if ($Runtime -ne "auto") {
        $parts += @("--container-runtime", $Runtime)
    }

    return ($parts -join " ")
}

function Write-GreenfieldPlanMarkdown {
    param(
        [string]$Path,
        [hashtable]$State
    )

    $markdown = @"
# Greenfield Spec Extraction

Status: $($State.status)

Target repo:

```text
$($State.repoPath)
```

Greenfield workspace:

```text
$($State.workspace)
```

Claude prompt:

```text
$($State.prompt)
```

Expected Greenfield output:

- ``workspace/output/specs/``: sanitized behavioral specifications.
- ``workspace/output/test-vectors/``: implementation-independent test vectors.
- ``workspace/output/validation/``: acceptance criteria.
- ``workspace/provenance/``: citation trail for every behavioral claim.

Notes:

- Default Code Intel runs only prepare this plan; pass ``-Analyze`` to try invoking ``claude -p`` locally.
- Greenfield is a Claude Code plugin. If ``claude`` or the plugin is missing, run the prompt manually inside Claude Code after installing Greenfield.
- Output specs must describe observable behavior, not source modules, internal names, or implementation structure.
"@

    $markdown | Set-Content -LiteralPath $Path -Encoding UTF8
}

$resolvedRepo = Resolve-CodeIntelDirectory -Path $RepoPath -Label "RepoPath"

if ([string]::IsNullOrWhiteSpace($ArtifactDir)) {
    $ArtifactDir = Join-Path (Get-Location).Path "greenfield-artifacts"
}

New-Item -ItemType Directory -Force -Path $ArtifactDir | Out-Null
$resolvedArtifactDir = Resolve-CodeIntelDirectory -Path $ArtifactDir -Label "ArtifactDir"

if ([string]::IsNullOrWhiteSpace($Workspace)) {
    $Workspace = Join-Path $resolvedArtifactDir "greenfield-workspace"
}

New-Item -ItemType Directory -Force -Path $Workspace | Out-Null
$resolvedWorkspace = Resolve-CodeIntelDirectory -Path $Workspace -Label "Workspace"

$validExcludes = @(
    "source",
    "docs",
    "sdk",
    "community",
    "runtime",
    "binary",
    "git-history",
    "tests",
    "visual",
    "contracts"
)

$normalizedExclude = @()
foreach ($item in $Exclude) {
    if ([string]::IsNullOrWhiteSpace($item)) {
        continue
    }

    foreach ($part in ($item -split ",")) {
        $value = $part.Trim().ToLowerInvariant()
        if ([string]::IsNullOrWhiteSpace($value)) {
            continue
        }
        if ($validExcludes -notcontains $value) {
            throw "Invalid Greenfield source exclude '$value'. Valid values: $($validExcludes -join ', ')"
        }
        if ($normalizedExclude -notcontains $value) {
            $normalizedExclude += $value
        }
    }
}

$prompt = New-GreenfieldAnalyzePrompt `
    -TargetPath $resolvedRepo `
    -WorkspacePath $resolvedWorkspace `
    -ExcludedSources $normalizedExclude `
    -Runtime $ContainerRuntime

$manifestPath = Join-Path $resolvedArtifactDir "greenfield-manifest.json"
$planPath = Join-Path $resolvedArtifactDir "greenfield-plan.md"
$stdoutPath = Join-Path $resolvedArtifactDir "greenfield-stdout.txt"
$stderrPath = Join-Path $resolvedArtifactDir "greenfield-stderr.txt"

$state = [ordered]@{
    schema = "code-intel-greenfield-spec-extraction.v1"
    generatedAt = (Get-Date).ToString("o")
    status = "planned"
    requiredAction = "Run the generated Claude prompt manually or re-run this adapter with -Analyze."
    repoPath = $resolvedRepo
    artifactDir = $resolvedArtifactDir
    workspace = $resolvedWorkspace
    prompt = $prompt
    excludedSources = $normalizedExclude
    containerRuntime = $ContainerRuntime
    analyzeRequested = [bool]$Analyze
    claude = [ordered]@{
        available = $false
        path = ""
        command = @("claude", "-p", $prompt)
    }
    outputs = [ordered]@{
        manifest = $manifestPath
        plan = $planPath
        stdout = $stdoutPath
        stderr = $stderrPath
        greenfieldSpecs = Join-Path $resolvedWorkspace "output\specs"
        greenfieldTestVectors = Join-Path $resolvedWorkspace "output\test-vectors"
        greenfieldValidation = Join-Path $resolvedWorkspace "output\validation"
        greenfieldProvenance = Join-Path $resolvedWorkspace "provenance"
    }
    reason = ""
}

$claude = Get-Command claude -ErrorAction SilentlyContinue
if ($claude) {
    $state.claude.available = $true
    $state.claude.path = $claude.Source
}

if ($Analyze) {
    if (-not $claude) {
        $state.status = "manual_required"
        $state.reason = "Claude CLI not found on PATH. Install Claude Code and the Greenfield plugin, then run the prompt in greenfield-plan.md."
    } else {
        $state.status = "running"
        $state | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
        Write-GreenfieldPlanMarkdown -Path $planPath -State $state

        $stdout = ""
        $stderr = ""
        try {
            $output = & $claude.Source -p $prompt 2>&1
            $exitCode = $LASTEXITCODE
            $stdout = ($output | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
            $stdout | Set-Content -LiteralPath $stdoutPath -Encoding UTF8
            "" | Set-Content -LiteralPath $stderrPath -Encoding UTF8

            if ($exitCode -eq 0) {
                $state.status = "completed"
                $state.requiredAction = "Review Greenfield output under workspace/output and provenance."
            } else {
                $state.status = "failed"
                $state.reason = "claude -p exited with code $exitCode."
            }
        } catch {
            $stderr = $_.Exception.Message
            $stderr | Set-Content -LiteralPath $stderrPath -Encoding UTF8
            $state.status = "failed"
            $state.reason = $stderr
        }
    }
}

Write-GreenfieldPlanMarkdown -Path $planPath -State $state
$state | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding UTF8

if ($Json) {
    $state | ConvertTo-Json -Depth 8
} else {
    Write-Host "Greenfield spec extraction $($state.status): $manifestPath"
    if (-not [string]::IsNullOrWhiteSpace($state.reason)) {
        Write-Host $state.reason
    }
}
