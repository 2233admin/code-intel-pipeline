#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$inventory = Join-Path ([System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))) "Invoke-ProviderRuntimeInventory.ps1"
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-provider-inventory-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
$originalOpenAiBaseUrl = [Environment]::GetEnvironmentVariable("OPENAI_BASE_URL", "Process")
$originalAnthropicBaseUrl = [Environment]::GetEnvironmentVariable("ANTHROPIC_BASE_URL", "Process")

function Write-FakeCli {
    param([string]$Name, [string]$Body)
    $path = Join-Path $fixtureRoot ($Name + ".ps1")
    Set-Content -LiteralPath $path -Value $Body -Encoding utf8NoBOM
    return $path
}

try {
    [Environment]::SetEnvironmentVariable("OPENAI_BASE_URL", "https://inventory-hidden.example.test/v1", "Process")
    [Environment]::SetEnvironmentVariable("ANTHROPIC_BASE_URL", "https://inventory-hidden.example.test", "Process")
    $brokenOpenCode = Write-FakeCli "opencode-broken" @'
exit 9
'@
    $healthyOpenCode = Write-FakeCli "opencode-healthy" @'
param([Parameter(ValueFromRemainingArguments=$true)][string[]]$Rest)
if ($Rest -contains "--version") { "opencode 1.2.3"; exit 0 }
exit 0
'@
    $timeoutCli = Write-FakeCli "timeout" @'
Start-Sleep -Seconds 5
"late"
'@
    $leakyClaude = Write-FakeCli "claude" @'
param([Parameter(ValueFromRemainingArguments=$true)][string[]]$Rest)
if ($Rest -contains "--version") { "claude sk-test-supersecret"; exit 0 }
if ($Rest -contains "status") { '{"authenticated":true,"authMethod":"oauth","token":"sk-test-supersecret"}'; exit 0 }
exit 1
'@
    $codex = Write-FakeCli "codex" @'
param([Parameter(ValueFromRemainingArguments=$true)][string[]]$Rest)
if ($Rest -contains "--version") { "codex 0.1.0"; exit 0 }
if ($Rest -contains "status") { "Logged in using ChatGPT"; exit 0 }
exit 1
'@
    $ollama = Write-FakeCli "ollama" @'
param([Parameter(ValueFromRemainingArguments=$true)][string[]]$Rest)
if ($Rest -contains "--version") { "ollama 0.30.0"; exit 0 }
if ($Rest -contains "list") { "NAME ID SIZE"; "qwen3.5:9b abc 6GB"; "qwen2.5:7b def 5GB"; exit 0 }
exit 1
'@
    $catalogFixture = Join-Path $fixtureRoot "models.json"
    Set-Content -LiteralPath $catalogFixture -Value '{"data":[{"id":"local-model-a"}]}' -Encoding utf8NoBOM

    $raw = & $inventory -TimeoutMs 400 -UseOnlyProvidedCandidates -OpenCodeCandidates @($brokenOpenCode, $healthyOpenCode) -ClaudeCandidates @($leakyClaude) -CodexCandidates @($codex) -OllamaCandidates @($ollama, $timeoutCli) -Json
    $result = $raw | ConvertFrom-Json

    if ($result.schema -ne "code-intel-model-channel-inventory-result.v1") { throw "schema drift" }
    $topNames = @($result.PSObject.Properties.Name)
    if (@($topNames | Where-Object { $_ -notin @("schema", "candidates", "configurationBrokers") }).Count -ne 0) { throw "closed top-level contract violated" }

    $openCode = @($result.candidates | Where-Object id -eq "opencode_cli")[0]
    if (-not [bool]$openCode.executableVerified) { throw "healthy OpenCode fallback was not selected" }
    if (@($openCode.diagnostics) -notcontains "candidate_verification_failed") { throw "broken OpenCode candidate evidence missing" }
    if (@($openCode.diagnostics) -notcontains "fallback_candidate_selected") { throw "healthy OpenCode fallback evidence missing" }

    $ollamaResult = @($result.candidates | Where-Object model -eq "qwen3.5:9b")[0]
    if ($ollamaResult.modelAvailable -ne "available") { throw "Ollama model inventory missing" }
    if ([bool]$ollamaResult.externalEgress) { throw "local Ollama was misclassified as external egress" }
    if (@($ollamaResult.diagnostics) -notcontains "candidate_timed_out") { throw "timeout was not bounded/classified" }

    $claudeResult = @($result.candidates | Where-Object id -eq "claude_cli")[0]
    if ($claudeResult.authPresent -ne "present" -or @($claudeResult.diagnostics) -notcontains "auth_method_oauth") { throw "Claude auth boolean/method projection failed" }
    $serialized = $result | ConvertTo-Json -Depth 10 -Compress
    if ($serialized -match "supersecret") { throw "secret leaked from CLI output" }
    if ($serialized -match '"token"') { throw "credential field leaked from CLI output" }

    foreach ($hiddenEndpointId in @("local_openai_compatible", "local_anthropic_compatible")) {
        $hiddenEndpoint = @($result.candidates | Where-Object id -eq $hiddenEndpointId)[0]
        if (-not [bool]$hiddenEndpoint.externalEgress) { throw "hidden endpoint locality was incorrectly trusted: $hiddenEndpointId" }
        if (@($hiddenEndpoint.diagnostics) -notcontains "external_egress_assumed") { throw "hidden endpoint conservative-classification evidence missing: $hiddenEndpointId" }
    }

    $declaredRaw = & $inventory -UseOnlyProvidedCandidates -UserProvider openai -UserModel local-model-a -UserBaseUrl http://127.0.0.1:9999/v1/ -Json
    $declared = $declaredRaw | ConvertFrom-Json
    $declaredCandidate = @($declared.candidates | Where-Object id -eq "user_local_compatible")[0]
    if ([bool]$declaredCandidate.executableVerified -or $declaredCandidate.modelAvailable -ne "unknown") { throw "user declaration was incorrectly treated as verification" }

    $probedRaw = & $inventory -UseOnlyProvidedCandidates -UserProvider openai -UserModel local-model-a -UserBaseUrl http://127.0.0.1:9999/v1/ -ProbeUserEndpoint -UserModelCatalogFixture $catalogFixture -Json
    $probed = $probedRaw | ConvertFrom-Json
    $probedCandidate = @($probed.candidates | Where-Object id -eq "user_local_compatible")[0]
    if (-not [bool]$probedCandidate.executableVerified -or $probedCandidate.modelAvailable -ne "available") { throw "explicit bounded catalog probe did not verify candidate" }

    $remoteRaw = & $inventory -UseOnlyProvidedCandidates -UserProvider openai -UserModel local-model-a -UserBaseUrl https://models.example.test/v1/ -ProbeUserEndpoint -UserModelCatalogFixture $catalogFixture -Json
    $remote = $remoteRaw | ConvertFrom-Json
    $remoteCandidate = @($remote.candidates | Where-Object id -eq "user_local_compatible")[0]
    if (-not [bool]$remoteCandidate.externalEgress) { throw "fixture changed remote endpoint egress classification" }

    $allowedTop = @("schema", "candidates", "configurationBrokers")
    $unexpectedTop = @($result.PSObject.Properties.Name | Where-Object { $_ -notin $allowedTop })
    if ($unexpectedTop.Count -ne 0) { throw "unexpected top-level fields: $($unexpectedTop -join ', ')" }

    [pscustomobject][ordered]@{
        ok = $true
        passed = 12
        total = 12
    } | ConvertTo-Json -Compress
}
finally {
    [Environment]::SetEnvironmentVariable("OPENAI_BASE_URL", $originalOpenAiBaseUrl, "Process")
    [Environment]::SetEnvironmentVariable("ANTHROPIC_BASE_URL", $originalAnthropicBaseUrl, "Process")
    Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
}
