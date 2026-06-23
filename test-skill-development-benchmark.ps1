param(
    [string]$RepoPath = $PSScriptRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-FileContains {
    param(
        [string]$Path,
        [string]$Pattern,
        [string]$Message
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Missing required file: $Path"
    }

    $text = Get-Content -LiteralPath $Path -Raw
    if ($text -notmatch $Pattern) {
        throw $Message
    }
}

function Assert-FrontMatterField {
    param(
        [string]$Text,
        [string]$Field
    )

    if ($Text -notmatch "(?m)^$([regex]::Escape($Field)):\s*\S") {
        throw "skill/SKILL.md missing frontmatter field: $Field"
    }
}

$root = (Resolve-Path -LiteralPath $RepoPath).Path
$skillPath = Join-Path $root "skill/SKILL.md"
$contextPath = Join-Path $root "CONTEXT.md"
$benchmarkPath = Join-Path $root "docs/skill-development-benchmark.md"
$benchmarkAdrPath = Join-Path $root "docs/adr/0004-yao-meta-skill-as-skill-development-benchmark.md"
$artifactContractPath = Join-Path $root "docs/artifact-data-contract.md"
$goalIntakePath = Join-Path $root "docs/agent-goal-intake.md"
$harnessReferencePath = Join-Path $root "docs/harness-factory-reference.md"

if (-not (Test-Path -LiteralPath $skillPath -PathType Leaf)) {
    throw "Missing skill/SKILL.md"
}

$skillText = Get-Content -LiteralPath $skillPath -Raw
if (-not $skillText.StartsWith("---")) {
    throw "skill/SKILL.md must start with YAML frontmatter."
}
Assert-FrontMatterField $skillText "name"
Assert-FrontMatterField $skillText "description"

$requiredSkillLinks = @(
    "docs\artifact-data-contract.md",
    "docs\agent-goal-intake.md",
    "docs\harness-factory-reference.md",
    "docs\skill-development-benchmark.md"
)
foreach ($link in $requiredSkillLinks) {
    if ($skillText -notmatch [regex]::Escape($link)) {
        throw "skill/SKILL.md missing canonical link: $link"
    }
}

Assert-FileContains `
    -Path $contextPath `
    -Pattern "Skill Development Benchmark" `
    -Message "CONTEXT.md must define Skill Development Benchmark."

Assert-FileContains `
    -Path $benchmarkPath `
    -Pattern "yao-meta-skill" `
    -Message "Skill benchmark doc must name yao-meta-skill as the reference benchmark."

Assert-FileContains `
    -Path $benchmarkPath `
    -Pattern "runtime dependency" `
    -Message "Skill benchmark doc must preserve the no-runtime-dependency boundary."

Assert-FileContains `
    -Path $benchmarkPath `
    -Pattern "trigger" `
    -Message "Skill benchmark doc must include trigger quality guidance."

Assert-FileContains `
    -Path $benchmarkPath `
    -Pattern "failure" `
    -Message "Skill benchmark doc must include failure-case guidance."

Assert-FileContains `
    -Path $benchmarkPath `
    -Pattern "release gates" `
    -Message "Skill benchmark doc must include release-gate guidance."

Assert-FileContains `
    -Path $benchmarkAdrPath `
    -Pattern "development benchmark, not runtime dependency" `
    -Message "ADR 0004 must record benchmark-not-runtime boundary."

Assert-FileContains `
    -Path $artifactContractPath `
    -Pattern "Agent Goal Intake" `
    -Message "Artifact contract must mention upstream Agent Goal Intake boundary."

Assert-FileContains `
    -Path $goalIntakePath `
    -Pattern "Do not wire these concepts into the scanner runtime" `
    -Message "Agent Goal Intake doc must protect scanner runtime boundary."

Assert-FileContains `
    -Path $harnessReferencePath `
    -Pattern "Do not make MetaHarness or any harness factory a runtime dependency" `
    -Message "Harness Factory Reference doc must protect runtime dependency boundary."

Write-Host "Skill development benchmark checks passed."
