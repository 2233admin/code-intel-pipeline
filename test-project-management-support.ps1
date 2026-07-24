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
        throw "Missing file: $Path"
    }

    $text = Get-Content -LiteralPath $Path -Raw
    if ($text -notmatch $Pattern) {
        throw $Message
    }
}

$root = (Resolve-Path -LiteralPath $RepoPath).Path

$projectManagementPath = Join-Path $root "docs/project-management-support.md"
$issueTrackerPath = Join-Path $root "docs/agents/issue-tracker.md"
$triageLabelsPath = Join-Path $root "docs/agents/triage-labels.md"
$domainPath = Join-Path $root "docs/agents/domain.md"
$adrPath = Join-Path $root "docs/adr/0006-project-management-support-as-agent-intake.md"
$contextPath = Join-Path $root "CONTEXT.md"
$readmePath = Join-Path $root "README.md"
$skillPath = Join-Path $root "skills/code-intel-pipeline/SKILL.md"

$checks = @(
    @{
        Path = $projectManagementPath
        Pattern = "mattpocock/skills"
        Message = "Project management support doc must name mattpocock/skills setup concepts."
    },
    @{
        Path = $projectManagementPath
        Pattern = "issue tracker"
        Message = "Project management support doc must cover issue tracker setup."
    },
    @{
        Path = $projectManagementPath
        Pattern = "triage label"
        Message = "Project management support doc must cover triage labels."
    },
    @{
        Path = $projectManagementPath
        Pattern = "domain doc"
        Message = "Project management support doc must cover domain docs."
    },
    @{
        Path = $projectManagementPath
        Pattern = "Linear"
        Message = "Project management support doc must include Linear."
    },
    @{
        Path = $projectManagementPath
        Pattern = "Obsidian/LLM wiki"
        Message = "Project management support doc must include Obsidian/LLM wiki."
    },
    @{
        Path = $projectManagementPath
        Pattern = "not scanner runtime"
        Message = "Project management support doc must preserve scanner runtime boundary."
    },
    @{
        Path = $issueTrackerPath
        Pattern = "Linear"
        Message = "Issue tracker doc must configure Linear."
    },
    @{
        Path = $issueTrackerPath
        Pattern = "no Linear runtime dependency"
        Message = "Issue tracker doc must preserve no-runtime-dependency boundary."
    },
    @{
        Path = $issueTrackerPath
        Pattern = "Do not store Linear API keys"
        Message = "Issue tracker doc must forbid stored Linear secrets."
    },
    @{
        Path = $triageLabelsPath
        Pattern = "needs-evaluation"
        Message = "Triage labels must include needs-evaluation."
    },
    @{
        Path = $triageLabelsPath
        Pattern = "needs-reporter-response"
        Message = "Triage labels must include needs-reporter-response."
    },
    @{
        Path = $triageLabelsPath
        Pattern = "ready-for-afk-agent"
        Message = "Triage labels must include ready-for-afk-agent."
    },
    @{
        Path = $triageLabelsPath
        Pattern = "ready-for-human"
        Message = "Triage labels must include ready-for-human."
    },
    @{
        Path = $triageLabelsPath
        Pattern = "wontfix"
        Message = "Triage labels must include wontfix."
    },
    @{
        Path = $domainPath
        Pattern = "single-context"
        Message = "Domain doc must record single-context layout."
    },
    @{
        Path = $domainPath
        Pattern = "Obsidian/LLM wiki"
        Message = "Domain doc must include wiki consumption rules."
    },
    @{
        Path = $adrPath
        Pattern = "agent intake, not scanner runtime dependency"
        Message = "ADR 0006 must record project-management boundary."
    },
    @{
        Path = $contextPath
        Pattern = "Project Management Support"
        Message = "CONTEXT.md must define Project Management Support."
    },
    @{
        Path = $readmePath
        Pattern = "docs/project-management-support.md"
        Message = "README must link project management support doc."
    },
    @{
        Path = $skillPath
        Pattern = "docs/project-management-support.md"
        Message = "skills/code-intel-pipeline/SKILL.md must link project management support doc."
    }
)

foreach ($check in $checks) {
    Assert-FileContains `
        -Path $check.Path `
        -Pattern $check.Pattern `
        -Message $check.Message
}

Write-Host "Project management support checks passed."
