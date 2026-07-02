param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,

    [Parameter(Mandatory = $true)]
    [string]$ArtifactDir,

    [object[]]$FailedSteps = @(),
    [object[]]$FailureClassifications = @(),

[ValidateSet("lite", "normal", "full")]
[string]$Mode = "normal",

[object]$SentruxFailures = $null,

[switch]$SkipGitHubResearch
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$OutputEncoding = [System.Text.UTF8Encoding]::new()

function Get-FirstNonEmptyLine {
    param([string]$Text)

    if ([string]::IsNullOrWhiteSpace($Text)) { return "" }
    $line = @($Text -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 1)
    if ($line.Count -eq 0) { return "" }
    return [string]$line[0]
}

function Get-QueryToken {
    param([string]$Text)

    if ([string]::IsNullOrWhiteSpace($Text)) { return @() }
    $tokens = [System.Collections.Generic.List[string]]::new()
    foreach ($match in [regex]::Matches($Text, "(?i)([a-z0-9_.-]+\.ps1|[a-z0-9_.-]+\.(exe|cmd|bat)|[a-z0-9_.-]+@[0-9][^\s,;)]*|error\s+code:\s*\d+|[a-z0-9_.-]+error|[a-z0-9_.-]+exception)")) {
        $value = $match.Value.Trim()
        if (-not [string]::IsNullOrWhiteSpace($value) -and -not $tokens.Contains($value)) {
            $tokens.Add($value)
        }
    }
    return @($tokens)
}

function New-ManualResult {
    param(
        [string]$Reason,
        [object[]]$Queries,
[object[]]$FailedSteps,
[object[]]$FailureClassifications,
[object]$SentruxFailures = $null,
[string[]]$EvidenceLinks = @()
    )

    return [ordered]@{
        schema = "github-solution-research.v1"
        generatedAt = (Get-Date).ToString("o")
        repo = $RepoPath
        mode = $Mode
        status = "manual_required"
        reason = $Reason
        queries = $Queries
        candidates = @()
        evidenceLinks = $EvidenceLinks
failedSteps = $FailedSteps
failureClassifications = $FailureClassifications
sentruxFailures = $SentruxFailures
nextStep = "Run github-solution-research with the failed step names, first error lines, tool/package names, and any version constraints."
        exitCriteria = @(
            "GitHub issue, PR, code, release, or repository evidence explains an applicable solution",
            "or GitHub evidence is explicitly recorded as insufficient and local-only diagnosis continues"
        )
    }
}

function Get-JsonProperty {
    param(
        [object]$Object,
        [string]$Name
    )

    if ($null -eq $Object) { return $null }
    $prop = $Object.PSObject.Properties[$Name]
    if ($null -eq $prop) { return $null }
    return $prop.Value
}

function Invoke-GhJson {
    param([string[]]$Arguments)

    $output = & gh @Arguments 2>&1
    $exitCode = $LASTEXITCODE
    return [ordered]@{
        exitCode = $exitCode
        text = ($output | Out-String).Trim()
    }
}

New-Item -ItemType Directory -Force -Path $ArtifactDir | Out-Null
$jsonPath = Join-Path $ArtifactDir "github-solution-research.json"
$markdownPath = Join-Path $ArtifactDir "github-solution-research.md"

$queries = [System.Collections.Generic.List[object]]::new()
if ($null -ne $SentruxFailures -and $null -ne $SentruxFailures.primary) {
    $primary = $SentruxFailures.primary
    $targetText = if ($null -ne $primary.target -and [string]$primary.target.status -eq "resolved") {
        "{0} {1}" -f [string]$primary.target.file, [string]$primary.target.symbol
    }
    else {
        [string]$primary.kind
    }
    $query = ("sentrux {0} {1}" -f [string]$primary.kind, $targetText).Trim()
    if (-not [string]::IsNullOrWhiteSpace($query)) {
        $queries.Add([ordered]@{
            step = "sentrux normalized failure"
            query = $query
            firstErrorLine = [string]$primary.stdout_excerpt
            tokens = @()
        })
    }
}
foreach ($step in @($FailedSteps)) {
    $name = [string]$step.name
    $detail = if (-not [string]::IsNullOrWhiteSpace([string]$step.error)) { [string]$step.error } else { [string]$step.output }
    $firstLine = Get-FirstNonEmptyLine $detail
    $tokens = @(Get-QueryToken ($name + "`n" + $firstLine))
    $queryParts = @($name)
    if (-not [string]::IsNullOrWhiteSpace($firstLine)) { $queryParts += $firstLine }
    foreach ($token in $tokens) { $queryParts += $token }
    $query = (($queryParts | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 4) -join " ").Trim()
    if (-not [string]::IsNullOrWhiteSpace($query)) {
        $queries.Add([ordered]@{
            step = $name
            query = $query
            firstErrorLine = $firstLine
            tokens = $tokens
        })
    }
}

if ($queries.Count -eq 0) {
    $queries.Add([ordered]@{
        step = "pipeline blocker"
        query = "code intel pipeline blocker"
        firstErrorLine = ""
        tokens = @()
    })
}

if ($SkipGitHubResearch) {
$result = New-ManualResult -Reason "GitHub research skipped by -SkipGitHubResearch." -Queries @($queries) -FailedSteps $FailedSteps -FailureClassifications $FailureClassifications -SentruxFailures $SentruxFailures
} elseif (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    $result = New-ManualResult -Reason "GitHub CLI 'gh' is not available on PATH." -Queries @($queries) -FailedSteps $FailedSteps -FailureClassifications $FailureClassifications -SentruxFailures $SentruxFailures
} else {
    $candidates = [System.Collections.Generic.List[object]]::new()
    $evidenceLinks = [System.Collections.Generic.List[string]]::new()
    $failureReason = ""

    foreach ($queryEntry in @($queries | Select-Object -First 3)) {
        $query = [string]$queryEntry.query
        foreach ($surface in @("issues", "prs", "repos", "code")) {
            $args = switch ($surface) {
                "issues" { @("search", "issues", $query, "--limit", "5", "--json", "title,url,state,updatedAt,repository") }
                "prs" { @("search", "prs", $query, "--limit", "5", "--json", "title,url,state,updatedAt,repository") }
                "repos" { @("search", "repos", $query, "--archived=false", "--sort", "stars", "--order", "desc", "--limit", "5", "--json", "fullName,url,description,stargazersCount,forksCount,language,license,pushedAt,isArchived") }
                "code" { @("search", "code", $query, "--limit", "5", "--json", "path,url,repository,sha") }
            }

            $search = Invoke-GhJson $args
            if ($search.exitCode -ne 0) {
                $failureReason = $search.text
                if ($failureReason -match "(?i)403|429|rate.?limit|api rate limit|authentication|not logged in") {
                    break
                }
                continue
            }

            try {
                $items = @($search.text | ConvertFrom-Json)
            } catch {
                $failureReason = "GitHub CLI returned non-JSON output for $surface search: $($_.Exception.Message)"
                continue
            }

            foreach ($item in ($items | Select-Object -First 3)) {
                $url = [string](Get-JsonProperty $item "url")
                if (-not [string]::IsNullOrWhiteSpace($url) -and -not $evidenceLinks.Contains($url)) {
                    $evidenceLinks.Add($url)
                }
                $repoName = ""
                $itemRepository = Get-JsonProperty $item "repository"
                $itemFullName = Get-JsonProperty $item "fullName"
                if ($null -ne $itemRepository) {
                    $repoFullName = Get-JsonProperty $itemRepository "fullName"
                    $repoNameWithOwner = Get-JsonProperty $itemRepository "nameWithOwner"
                    if ($null -ne $repoFullName) { $repoName = [string]$repoFullName }
                    elseif ($null -ne $repoNameWithOwner) { $repoName = [string]$repoNameWithOwner }
                } elseif ($null -ne $itemFullName) {
                    $repoName = [string]$itemFullName
                }
                $itemTitle = Get-JsonProperty $item "title"
                $itemPath = Get-JsonProperty $item "path"
                $itemState = Get-JsonProperty $item "state"
                $itemStars = Get-JsonProperty $item "stargazersCount"
                $itemLanguage = Get-JsonProperty $item "language"
                $itemUpdatedAt = Get-JsonProperty $item "updatedAt"
                $itemPushedAt = Get-JsonProperty $item "pushedAt"
                $candidates.Add([ordered]@{
                    surface = $surface
                    query = $query
                    title = if ($null -ne $itemTitle) { [string]$itemTitle } elseif ($null -ne $itemFullName) { [string]$itemFullName } else { [string]$itemPath }
                    repository = $repoName
                    url = $url
                    state = if ($null -ne $itemState) { [string]$itemState } else { "" }
                    stars = if ($null -ne $itemStars) { [int]$itemStars } else { $null }
                    language = if ($null -ne $itemLanguage) { [string]$itemLanguage } else { "" }
                    updatedAt = if ($null -ne $itemUpdatedAt) { [string]$itemUpdatedAt } elseif ($null -ne $itemPushedAt) { [string]$itemPushedAt } else { "" }
                })
}
        }

        if ($failureReason -match "(?i)403|429|rate.?limit|api rate limit|authentication|not logged in") {
            break
        }
    }

    if ($candidates.Count -eq 0) {
        $reason = if ([string]::IsNullOrWhiteSpace($failureReason)) { "No strong GitHub evidence returned for the generated blocker queries." } else { $failureReason }
        $result = New-ManualResult -Reason $reason -Queries @($queries) -FailedSteps $FailedSteps -FailureClassifications $FailureClassifications -SentruxFailures $SentruxFailures
    } else {
        $result = [ordered]@{
            schema = "github-solution-research.v1"
            generatedAt = (Get-Date).ToString("o")
            repo = $RepoPath
            mode = $Mode
            status = "auto_generated"
            reason = "GitHub evidence candidates were generated from pipeline blocker queries."
            queries = @($queries)
            candidates = @($candidates)
            evidenceLinks = @($evidenceLinks)
            failedSteps = $FailedSteps
            failureClassifications = $FailureClassifications
            sentruxFailures = $SentruxFailures
            nextStep = "Review candidate evidence, confirm applicability, then adapt the smallest local fix."
            exitCriteria = @(
                "GitHub evidence is linked and judged applicable",
                "or evidence is rejected with explicit reasons before local-only diagnosis continues"
            )
        }
    }
}

$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $jsonPath -Encoding UTF8

$candidateLines = @()
foreach ($candidate in @($result.candidates | Select-Object -First 10)) {
    $label = if (-not [string]::IsNullOrWhiteSpace([string]$candidate.title)) { [string]$candidate.title } else { [string]$candidate.repository }
    $candidateLines += "- [$($candidate.surface)] $label - $($candidate.url)"
}
if ($candidateLines.Count -eq 0) { $candidateLines = @("- No GitHub evidence candidates were generated automatically.") }

$markdown = @(
    "# GitHub Solution Research",
    "",
    "- Status: $($result.status)",
    "- Reason: $($result.reason)",
    "- Repo: $RepoPath",
    "- Mode: $Mode",
    "",
    "## Generated Queries",
    ""
)
foreach ($queryEntry in @($result.queries)) {
    $markdown += "- $($queryEntry.step): ``$($queryEntry.query)``"
}
$markdown += @(
    "",
    "## Evidence Candidates",
    ""
) + $candidateLines + @(
    "",
    "## Required Follow-Up",
    "",
    $result.nextStep,
    "",
    "## Exit Criteria",
    ""
)
foreach ($criterion in @($result.exitCriteria)) {
    $markdown += "- $criterion"
}
$markdown | Set-Content -LiteralPath $markdownPath -Encoding UTF8

return [ordered]@{
    status = $result.status
    required = $true
    reason = $result.reason
    path = $jsonPath
    markdown = $markdownPath
    evidenceLinks = @($result.evidenceLinks)
    candidates = @($result.candidates).Count
    queries = @($result.queries).Count
    exitCriteria = @($result.exitCriteria)
}
