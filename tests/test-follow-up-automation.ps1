#requires -Version 7.2
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Import-Module (Join-Path $root "tools/code-intel-follow-up-automation.psm1") -Force
$temp = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-follow-up-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $temp | Out-Null
try {
    $evidence = Join-Path $temp "sentrux-debt-register.json"
    '{"schema":"test-evidence.v1"}' | Set-Content -LiteralPath $evidence -Encoding utf8
    $failure = [pscustomobject]@{ category = "local_tool_error"; step = "unit test"; status = "failed" }
    $out = Join-Path $temp "actionable"
    $result = New-CodeIntelFollowUpAutomation -FailureClassifications @($failure) -BlockingSentruxDebt 0 -EvidencePath $evidence -OutputDirectory $out -IssuedAt 1000 -ExpiresAt 2000
    if (@($result.proactiveSkillSuggestions.suggestions).Count -ne 1) { throw "actionable failure must suggest one skill" }
    if ([string]$result.proactiveSkillSuggestions.suggestions[0].skill -ne "/investigate") { throw "bug suggestion must be /investigate" }
    if ([string]$result.automaticPullRequests.consentStatus -ne "pending") { throw "automatic PR must ask by default" }
    if (-not (Test-Path -LiteralPath (Join-Path $out "automatic-pr-consent.request.json"))) { throw "decision request was not emitted" }
    if (@($result.effects).Count -ne 0) { throw "proposal path must be zero-effect" }

    $cleanOut = Join-Path $temp "clean"
    $clean = New-CodeIntelFollowUpAutomation -FailureClassifications @() -BlockingSentruxDebt 0 -EvidencePath $evidence -OutputDirectory $cleanOut -IssuedAt 1000 -ExpiresAt 2000
    if (@($clean.proactiveSkillSuggestions.suggestions).Count -ne 0) { throw "clean scan must not suggest investigate" }
    if ([string]$clean.automaticPullRequests.consentStatus -ne "unasked") { throw "clean scan must not ask for automatic PR" }
    if (Test-Path -LiteralPath (Join-Path $cleanOut "automatic-pr-consent.request.json")) { throw "clean scan emitted a decision request" }

    $quotaOut = Join-Path $temp "quota"
    $quota = New-CodeIntelFollowUpAutomation -FailureClassifications @([pscustomobject]@{ category = "provider_quota"; step = "provider"; status = "failed" }) -BlockingSentruxDebt 0 -EvidencePath $evidence -OutputDirectory $quotaOut -IssuedAt 1000 -ExpiresAt 2000
    if (@($quota.proactiveSkillSuggestions.suggestions).Count -ne 0) { throw "provider quota must not be mislabeled as a code bug" }

    $disabledOut = Join-Path $temp "disabled"
    $disabled = New-CodeIntelFollowUpAutomation -FailureClassifications @($failure) -BlockingSentruxDebt 0 -EvidencePath $evidence -OutputDirectory $disabledOut -ProactiveSkillSuggestions disabled -AutomaticPullRequests disabled -IssuedAt 1000 -ExpiresAt 2000
    if (@($disabled.proactiveSkillSuggestions.suggestions).Count -ne 0) { throw "disabled suggestions emitted output" }
    if ([string]$disabled.automaticPullRequests.consentStatus -ne "disabled") { throw "automatic PR disable was ignored" }
    if (Test-Path -LiteralPath (Join-Path $disabledOut "automatic-pr-consent.request.json")) { throw "disabled automatic PR emitted a decision request" }

    foreach ($pair in @(
        @{ Document = (Join-Path $out "follow-up-automation.json"); Schema = (Join-Path $root "orchestration/schemas/code-intel-follow-up-automation.v1.schema.json") },
        @{ Document = (Join-Path $out "automatic-pr-consent.request.json"); Schema = (Join-Path $root "orchestration/schemas/code-intel-decision-request.v1.schema.json") }
    )) {
        if (-not (Get-Content -LiteralPath $pair.Document -Raw | Test-Json -SchemaFile $pair.Schema -ErrorAction Stop)) {
            throw "schema validation failed: $($pair.Document)"
        }
    }
    Write-Host "follow-up automation contract: passed"
}
finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
