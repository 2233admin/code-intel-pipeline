Set-StrictMode -Version Latest

function Get-CodeIntelFollowUpProperty {
    param([object]$Value, [string]$Name)
    if ($null -eq $Value) { return $null }
    $property = $Value.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

function Resolve-CodeIntelFollowUpSettings {
    [CmdletBinding()]
    param(
        [object]$Config,
        [ValidateSet("auto", "enabled", "disabled")] [string]$ProactiveSkillSuggestions,
        [ValidateSet("auto", "ask", "enabled", "disabled")] [string]$AutomaticPullRequests,
        [string]$BugSkill
    )

    $suggestionConfig = Get-CodeIntelFollowUpProperty $Config "proactiveSkillSuggestions"
    $automaticPrConfig = Get-CodeIntelFollowUpProperty $Config "automaticPullRequests"
    $configuredEnabled = Get-CodeIntelFollowUpProperty $suggestionConfig "enabled"
    $configuredMode = [string](Get-CodeIntelFollowUpProperty $automaticPrConfig "mode")
    $configuredBugSkill = [string](Get-CodeIntelFollowUpProperty $suggestionConfig "bugSkill")
    $suggestionMode = if ($ProactiveSkillSuggestions -ne "auto") { $ProactiveSkillSuggestions } elseif ($configuredEnabled -is [bool] -and -not $configuredEnabled) { "disabled" } else { "enabled" }
    $prMode = if ($AutomaticPullRequests -ne "auto") { $AutomaticPullRequests } elseif ($configuredMode -in @("ask", "enabled", "disabled")) { $configuredMode } else { "ask" }
    $resolvedBugSkill = if (-not [string]::IsNullOrWhiteSpace($BugSkill)) { $BugSkill } elseif (-not [string]::IsNullOrWhiteSpace($configuredBugSkill)) { $configuredBugSkill } else { "/investigate" }
    return [ordered]@{
        proactiveSkillSuggestions = $suggestionMode
        automaticPullRequests = $prMode
        bugSkill = $resolvedBugSkill
    }
}

function Get-CodeIntelFollowUpSummaryLines {
    param([Parameter(Mandatory)] [object]$Automation)
    $suggestions = @($Automation.proactiveSkillSuggestions.suggestions)
    $suggestedSkill = if ($suggestions.Count -gt 0) { "$([string]$suggestions[0].skill) (proposal only; not executed)" } else { "(none)" }
    $decisionRequest = if ($Automation.automaticPullRequests.decisionRequest) { [string]$Automation.automaticPullRequests.decisionRequest } else { "(none)" }
    return @(
        "## Follow-up Automation",
        "",
        "- Proactive skill suggestions: $($Automation.proactiveSkillSuggestions.status)",
        "- Suggested skill: $suggestedSkill",
        "- Automatic PR proposal: $($Automation.automaticPullRequests.proposalStatus)",
        "- Automatic PR consent: $($Automation.automaticPullRequests.consentStatus)",
        "- User decision requested: $decisionRequest",
        "- Automatic PR execution: $($Automation.automaticPullRequests.executionStatus)"
    )
}

function Write-CodeIntelFollowUpPrompt {
    param([Parameter(Mandatory)] [object]$Automation, [Parameter(Mandatory)] [string]$RunDirectory)
    if ([string]$Automation.automaticPullRequests.consentStatus -ne "pending") { return }
    Write-Host "Decision required: enable one draft pull request for this evidence snapshot?"
    Write-Host "Options: keep_disabled | enable_once_for_snapshot"
    Write-Host "Request: $(Join-Path $RunDirectory 'automatic-pr-consent.request.json')"
}

function New-CodeIntelFollowUpAutomation {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [AllowEmptyCollection()] [object[]]$FailureClassifications,
        [Parameter(Mandatory)] [int]$BlockingSentruxDebt,
        [Parameter(Mandatory)] [string]$EvidencePath,
        [Parameter(Mandatory)] [string]$OutputDirectory,
        [ValidateSet("enabled", "disabled")] [string]$ProactiveSkillSuggestions = "enabled",
        [ValidateSet("ask", "enabled", "disabled")] [string]$AutomaticPullRequests = "ask",
        [string]$BugSkill = "/investigate",
        [long]$IssuedAt = 0,
        [long]$ExpiresAt = 0
    )

    if (-not (Test-Path -LiteralPath $EvidencePath -PathType Leaf)) {
        throw "Follow-up automation evidence does not exist: $EvidencePath"
    }
    if ([string]::IsNullOrWhiteSpace($BugSkill)) { throw "Bug skill must be non-empty" }
    if ($IssuedAt -le 0) { $IssuedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds() }
    if ($ExpiresAt -le 0) { $ExpiresAt = $IssuedAt + (7 * 24 * 60 * 60) }
    if ($ExpiresAt -le $IssuedAt) { throw "Follow-up automation expiry must be later than issue time" }

    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $evidenceSha256 = (Get-FileHash -LiteralPath $EvidencePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $excludedCategories = @("provider_quota", "provider_unavailable", "config_error", "graph_missing")
    $actionableFailures = @(
        $FailureClassifications | Where-Object {
            $category = [string]$_.category
            $status = [string]$_.status
            $status -eq "failed" -and $category -notin $excludedCategories
        }
    )
    $hasBugSignal = $actionableFailures.Count -gt 0 -or $BlockingSentruxDebt -gt 0

    $suggestions = @()
    if ($ProactiveSkillSuggestions -eq "enabled" -and $hasBugSignal) {
        $reasons = @($actionableFailures | ForEach-Object { "$([string]$_.step):$([string]$_.category)" })
        if ($BlockingSentruxDebt -gt 0) { $reasons += "blocking_sentrux_debt:$BlockingSentruxDebt" }
        $suggestions = @([ordered]@{
            id = "investigate-detected-problem"
            skill = $BugSkill
            trigger = "pipeline_bug_or_error_signal"
            reasons = @($reasons | Select-Object -Unique)
            kind = "proposal"
            effects = @()
        })
    }

    $proposalStatus = if ($hasBugSignal) { "proposed" } else { "not_applicable" }
    $consentStatus = switch ($AutomaticPullRequests) {
        "ask" { if ($hasBugSignal) { "pending" } else { "unasked" } }
        "enabled" { "explicit_mode_requested" }
        default { "disabled" }
    }
    $decisionRequestFile = $null
    if ($AutomaticPullRequests -eq "ask" -and $hasBugSignal) {
        $correlationSuffix = $evidenceSha256.Substring(0, 24)
        $decisionRequestFile = "automatic-pr-consent.request.json"
        $decisionRequest = [ordered]@{
            schema = "code-intel-decision-request.v1"
            correlationId = "auto-pr-$correlationSuffix"
            gapId = "automatic-pr-consent"
            question = "Code Intel detected an actionable problem. Enable one draft pull-request creation for this evidence snapshot?"
            recommendation = [ordered]@{
                optionId = "keep_disabled"
                rationale = "External repository mutation and network publication stay disabled unless the user grants one-snapshot authority."
            }
            evidenceRefs = @([ordered]@{
                refId = (Split-Path -Leaf $EvidencePath)
                sha256 = $evidenceSha256
                observedAt = $IssuedAt
                expiresAt = $ExpiresAt
            })
            options = @(
                [ordered]@{
                    id = "keep_disabled"
                    label = "Keep automatic PR disabled"
                    consequence = "The scan and skill suggestions continue, but no pull request may be created."
                },
                [ordered]@{
                    id = "enable_once_for_snapshot"
                    label = "Enable one draft PR"
                    consequence = "A separate executor may create one draft PR only after scoped authority, HEAD, snapshot, repository-mutation, and network checks pass."
                }
            )
            authorityNeeded = [ordered]@{
                kind = "repository_mutation_and_network_pr_create"
                actorIds = @("user")
            }
            issuedAt = $IssuedAt
            expiresAt = $ExpiresAt
            affectedBranches = @("automatic_pr_execution")
        }
        $decisionRequest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $OutputDirectory $decisionRequestFile) -Encoding utf8
    }

    $artifact = [ordered]@{
        schema = "code-intel-follow-up-automation.v1"
        generatedAt = [DateTimeOffset]::FromUnixTimeSeconds($IssuedAt).ToString("o")
        evidence = [ordered]@{
            path = $EvidencePath
            sha256 = $evidenceSha256
        }
        proactiveSkillSuggestions = [ordered]@{
            status = $ProactiveSkillSuggestions
            suggestions = $suggestions
            effects = @()
        }
        automaticPullRequests = [ordered]@{
            defaultEnabled = $false
            configuredMode = $AutomaticPullRequests
            proposalStatus = $proposalStatus
            consentStatus = $consentStatus
            decisionRequest = $decisionRequestFile
            executionStatus = "not_authorized"
            orchestrator = "Invoke-CodeIntelAutomaticPullRequestFlow.ps1"
            executor = "Invoke-CodeIntelAutomaticPullRequest.ps1"
            requiredEffects = @("repo_mutation", "network")
            effects = @()
        }
        effects = @()
    }
    $artifactPath = Join-Path $OutputDirectory "follow-up-automation.json"
    $artifact | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $artifactPath -Encoding utf8
    return $artifact
}

Export-ModuleMember -Function New-CodeIntelFollowUpAutomation, Resolve-CodeIntelFollowUpSettings, Get-CodeIntelFollowUpSummaryLines, Write-CodeIntelFollowUpPrompt
