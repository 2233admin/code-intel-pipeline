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
$skillBenchmarkPath = Join-Path $root "docs/skill-development-benchmark.md"
$skillBenchmarkAdrPath = Join-Path $root "docs/adr/0004-yao-meta-skill-as-skill-development-benchmark.md"
$implementationMinimalismPath = Join-Path $root "docs/implementation-minimalism-benchmark.md"
$implementationMinimalismAdrPath = Join-Path $root "docs/adr/0005-ponytail-as-implementation-minimalism-benchmark.md"
$ponytailImpactScoreboardPath = Join-Path $root "docs/ponytail-impact-scoreboard.md"
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
    "docs\skill-development-benchmark.md",
    "docs\implementation-minimalism-benchmark.md",
    "docs\ponytail-impact-scoreboard.md"
)

foreach ($link in $requiredSkillLinks) {
    if ($skillText -notmatch [regex]::Escape($link)) {
        throw "skill/SKILL.md missing canonical link: $link"
    }
}

$checks = @(
    @{
        Path = $contextPath
        Pattern = "Skill Development Benchmark"
        Message = "CONTEXT.md must define Skill Development Benchmark."
    },
    @{
        Path = $contextPath
        Pattern = "Implementation Minimalism Benchmark"
        Message = "CONTEXT.md must define Implementation Minimalism Benchmark."
    },
    @{
        Path = $skillBenchmarkPath
        Pattern = "yao-meta-skill"
        Message = "Skill benchmark doc must name yao-meta-skill as reference benchmark."
    },
    @{
        Path = $skillBenchmarkPath
        Pattern = "runtime dependency"
        Message = "Skill benchmark doc must preserve no-runtime-dependency boundary."
    },
    @{
        Path = $skillBenchmarkPath
        Pattern = "trigger"
        Message = "Skill benchmark doc must include trigger quality guidance."
    },
    @{
        Path = $skillBenchmarkPath
        Pattern = "failure"
        Message = "Skill benchmark doc must include failure-case guidance."
    },
    @{
        Path = $skillBenchmarkPath
        Pattern = "release gates"
        Message = "Skill benchmark doc must include release-gate guidance."
    },
    @{
        Path = $skillBenchmarkAdrPath
        Pattern = "development benchmark, not runtime dependency"
        Message = "ADR 0004 must record benchmark-not-runtime boundary."
    },
    @{
        Path = $artifactContractPath
        Pattern = "Agent Goal Intake"
        Message = "Artifact contract must mention upstream Agent Goal Intake boundary."
    },
    @{
        Path = $goalIntakePath
        Pattern = "Do not wire these concepts into the scanner runtime"
        Message = "Agent Goal Intake doc must protect scanner runtime boundary."
    },
    @{
        Path = $harnessReferencePath
        Pattern = "Do not make MetaHarness or any harness factory a runtime dependency of the scanner"
        Message = "Harness Factory Reference doc must protect runtime dependency boundary."
    }
)

$implementationMinimalismChecks = @(
    @("Ponytail", "name Ponytail as reference benchmark"),
    @("Do nothing", "include do-nothing rung"),
    @("Reuse this repository", "include repository-reuse rung"),
    @("standard library", "include standard-library rung"),
    @("platform native", "include platform-native rung"),
    @("already-installed dependency", "include installed-dependency rung"),
    @("one-liner", "include one-liner rung"),
    @("smallest local implementation", "include smallest-local-implementation rung"),
    @("verification", "preserve verification boundary"),
    @("security", "preserve security boundary"),
    @("accessibility", "preserve accessibility boundary"),
    @("data-loss", "preserve data-loss prevention boundary")
)

foreach ($check in $checks) {
    Assert-FileContains `
        -Path $check.Path `
        -Pattern $check.Pattern `
        -Message $check.Message
}

foreach ($check in $implementationMinimalismChecks) {
    Assert-FileContains `
        -Path $implementationMinimalismPath `
        -Pattern $check[0] `
        -Message "Implementation minimalism doc must $($check[1])."
}

Assert-FileContains `
    -Path $implementationMinimalismAdrPath `
    -Pattern "benchmark, not runtime dependency" `
    -Message "ADR 0005 must record Ponytail benchmark-not-runtime boundary."

$ponytailImpactScoreboardChecks = @(
    @("measured impact only", "forbid unmeasured savings claims"),
    @("code_removed_lines", "track code removal"),
    @("benchmark_before_seconds", "track timing baseline"),
    @("cost_before", "track cost baseline"),
    @("not_measured", "record unknown values explicitly")
)

foreach ($check in $ponytailImpactScoreboardChecks) {
    Assert-FileContains `
        -Path $ponytailImpactScoreboardPath `
        -Pattern $check[0] `
        -Message "Ponytail impact scoreboard must $($check[1])."
}

Write-Host "Skill development benchmark checks passed."
