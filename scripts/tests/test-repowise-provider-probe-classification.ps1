#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$probe = Join-Path ([System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))) "Invoke-RepowiseProviderProbe.ps1"
$cases = @(
    @{ fixture = "model_not_found"; expected = "provider_unavailable" },
    @{ fixture = "authentication_error"; expected = "config_error" },
    @{ fixture = "rate_limit"; expected = "provider_quota" },
    @{ fixture = "local_import_error"; expected = "local_tool_error" }
)

$passed = 0
foreach ($case in $cases) {
    $raw = if ($case.fixture -eq "model_not_found") {
        & $probe -Provider anthropic -FailureFixture $case.fixture -Json
    }
    else {
        & $probe -Provider anthropic -Model MiniMax-M2.7 -FailureFixture $case.fixture -Json
    }
    $exitCode = $LASTEXITCODE
    $result = $raw | ConvertFrom-Json
    if ($exitCode -ne 1) { throw "$($case.fixture): expected exit 1, got $exitCode" }
    if ([bool]$result.ok) { throw "$($case.fixture): expected ok=false" }
    if ([string]$result.category -ne $case.expected) {
        throw "$($case.fixture): expected $($case.expected), got $($result.category)"
    }
    if ([string]$result.model -ne "MiniMax-M2.7") {
        throw "$($case.fixture): model identity drifted"
    }
    $passed++
}

$pipeline = Join-Path ([System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))) "run-code-intel.ps1"
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($pipeline, [ref]$tokens, [ref]$parseErrors)
if (@($parseErrors).Count -ne 0) {
    throw "run-code-intel.ps1 parse failed: $(@($parseErrors.Message) -join '; ')"
}
$classifier = @($ast.FindAll({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -eq "Get-StepFailureCategory"
}, $true))
if ($classifier.Count -ne 1) { throw "expected one Get-StepFailureCategory function" }
Invoke-Expression $classifier[0].Extent.Text

$pipelineCases = @(
    @{ text = '{"category":"provider_unavailable","message":"Error code: 404 model_not_found"}'; expected = "provider_unavailable" },
    @{ text = '{"category":"config_error","message":"invalid API key"}'; expected = "config_error" },
    @{ text = '{"category":"provider_quota","message":"Error code: 429"}'; expected = "provider_quota" },
    @{ text = 'python executable missing'; expected = "local_tool_error" }
)
foreach ($case in $pipelineCases) {
    $actual = Get-StepFailureCategory ([pscustomobject]@{
        name = "repowise provider health"
        status = "failed"
        output = $case.text
        error = "Command exited with code 1"
    })
    if ($actual -ne $case.expected) {
        throw "pipeline classification: expected $($case.expected), got $actual"
    }
    $passed++
}

[pscustomobject][ordered]@{
    ok = $true
    passed = $passed
    total = $cases.Count + $pipelineCases.Count
} | ConvertTo-Json -Compress
