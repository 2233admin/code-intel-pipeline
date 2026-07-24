#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$delegate = Join-Path $repoRoot "Invoke-ModelChannelDelegate.ps1"
$root = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-delegate-test-{0}" -f [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $root | Out-Null

function Assert-Equal($Actual, $Expected, [string]$Message) {
    if ($Actual -ne $Expected) { throw "$Message (expected=$Expected actual=$Actual)" }
}

function Invoke-DelegateChild {
    param([string]$Request, [string]$Artifacts)
    $output = & pwsh -NoProfile -File $delegate -Request $Request -ArtifactRoot $Artifacts -AllowLegacyRawExecutable 2>&1
    return [ordered]@{ exitCode = $LASTEXITCODE; output = ($output | Out-String) }
}

try {
    $prompt = Join-Path $root "prompt.txt"
    [IO.File]::WriteAllText($prompt, "TOP-SECRET-PROMPT", [Text.UTF8Encoding]::new($false))
    $fake = Join-Path $root "fake-claude.cmd"
    @(
        "@echo off",
        "set /p ignored=",
        'echo {"type":"result","result":"ok"}',
        "exit /b 0"
    ) | Set-Content -LiteralPath $fake -Encoding ascii

    $deniedArtifacts = Join-Path $root "denied"
    $deniedRequest = Join-Path $root "denied.json"
    [ordered]@{
        schema = "code-intel-model-adapter-request.v1"; adapter = "claude_cli"; executable = $fake; endpoint = $null; protocol = $null; credentialEnvName = $null; model = "fixture-model"
        promptFile = $prompt; timeoutSeconds = 10; costScope = "metered_api"; externalData = $true; responseFormat = "json"
        consent = [ordered]@{ consumption = "granted"; localCompute = "unanswered"; paidSpend = "denied"; externalData = "granted" }
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $deniedRequest -Encoding utf8
    $denied = Invoke-DelegateChild $deniedRequest $deniedArtifacts
    Assert-Equal $denied.exitCode 2 "paid route without paid consent must be blocked"
    $deniedResult = Get-Content -LiteralPath (Join-Path $deniedArtifacts "model-channel-result.json") -Raw | ConvertFrom-Json
    Assert-Equal $deniedResult.attempt.invoked $false "blocked route must not invoke executable"
    Assert-Equal $deniedResult.category "paid_usage_forbidden" "blocked metered route category"
    if ((Get-Content -LiteralPath (Join-Path $deniedArtifacts "model-channel-result.json") -Raw).Contains("TOP-SECRET-PROMPT")) {
        throw "attempt result leaked prompt text"
    }

    $successArtifacts = Join-Path $root "success"
    $successRequest = Join-Path $root "success.json"
    [ordered]@{
        schema = "code-intel-model-adapter-request.v1"; adapter = "claude_cli"; executable = $fake; endpoint = $null; protocol = $null; credentialEnvName = $null; model = "fixture-model"
        promptFile = $prompt; timeoutSeconds = 10; costScope = "subscription_cli"; externalData = $true; responseFormat = "json"
        consent = [ordered]@{ consumption = "granted"; localCompute = "unanswered"; paidSpend = "unanswered"; externalData = "granted" }
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $successRequest -Encoding utf8
    $success = Invoke-DelegateChild $successRequest $successArtifacts
    Assert-Equal $success.exitCode 0 "authorized fake Claude delegate should complete"
    $successResult = Get-Content -LiteralPath (Join-Path $successArtifacts "model-channel-result.json") -Raw | ConvertFrom-Json
    Assert-Equal $successResult.status "completed" "authorized delegate status"
    Assert-Equal $successResult.attempt.exitCode 0 "authorized delegate exit code"
    [void](Get-Content -LiteralPath $successResult.responseArtifact -Raw | ConvertFrom-Json -ErrorAction Stop)

    $cliEgressArtifacts = Join-Path $root "cli-egress-denied"
    $cliEgressRequest = Join-Path $root "cli-egress-denied.json"
    [ordered]@{
        schema = "code-intel-model-adapter-request.v1"; adapter = "claude_cli"; executable = $fake; endpoint = $null; protocol = $null; credentialEnvName = $null; model = "fixture-model"
        promptFile = $prompt; timeoutSeconds = 10; costScope = "subscription_cli"; externalData = $false; responseFormat = "json"
        consent = [ordered]@{ consumption = "granted"; localCompute = "unanswered"; paidSpend = "unanswered"; externalData = "denied" }
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $cliEgressRequest -Encoding utf8
    $cliEgressDenied = Invoke-DelegateChild $cliEgressRequest $cliEgressArtifacts
    Assert-Equal $cliEgressDenied.exitCode 2 "subscription CLI cannot weaken external-data gating"
    $cliEgressResult = Get-Content -LiteralPath (Join-Path $cliEgressArtifacts "model-channel-result.json") -Raw | ConvertFrom-Json
    Assert-Equal $cliEgressResult.attempt.invoked $false "denied CLI egress must make zero calls"
    Assert-Equal $cliEgressResult.category "external_data_forbidden" "denied CLI egress category"

    $localCliEgressArtifacts = Join-Path $root "local-cli-egress-denied"
    $localCliEgressRequest = Join-Path $root "local-cli-egress-denied.json"
    [ordered]@{
        schema = "code-intel-model-adapter-request.v1"; adapter = "claude_cli"; executable = $fake; endpoint = $null; protocol = $null; credentialEnvName = $null; model = "fixture-model"
        promptFile = $prompt; timeoutSeconds = 10; costScope = "local_compute"; externalData = $false; responseFormat = "json"
        consent = [ordered]@{ consumption = "granted"; localCompute = "granted"; paidSpend = "unanswered"; externalData = "denied" }
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $localCliEgressRequest -Encoding utf8
    $localCliEgressDenied = Invoke-DelegateChild $localCliEgressRequest $localCliEgressArtifacts
    Assert-Equal $localCliEgressDenied.exitCode 2 "local-compute label cannot weaken CLI external-data gating"
    $localCliEgressResult = Get-Content -LiteralPath (Join-Path $localCliEgressArtifacts "model-channel-result.json") -Raw | ConvertFrom-Json
    Assert-Equal $localCliEgressResult.attempt.invoked $false "denied local-label CLI egress must make zero calls"
    Assert-Equal $localCliEgressResult.category "external_data_forbidden" "denied local-label CLI egress category"

    $quotaFake = Join-Path $root "fake-quota.cmd"
    @("@echo off", "set /p ignored=", "echo HTTP 429 quota exhausted 1>&2", "exit /b 1") | Set-Content -LiteralPath $quotaFake -Encoding ascii
    $quotaArtifacts = Join-Path $root "quota"
    $quotaRequest = Join-Path $root "quota.json"
    [ordered]@{
        schema = "code-intel-model-adapter-request.v1"; adapter = "claude_cli"; executable = $quotaFake; endpoint = $null; protocol = $null; credentialEnvName = $null; model = "fixture-model"
        promptFile = $prompt; timeoutSeconds = 10; costScope = "subscription_cli"; externalData = $true; responseFormat = "json"
        consent = [ordered]@{ consumption = "granted"; localCompute = "unanswered"; paidSpend = "unanswered"; externalData = "granted" }
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $quotaRequest -Encoding utf8
    $quota = Invoke-DelegateChild $quotaRequest $quotaArtifacts
    Assert-Equal $quota.exitCode 75 "quota failure must be classified as transient upstream"
    $quotaResult = Get-Content -LiteralPath (Join-Path $quotaArtifacts "model-channel-result.json") -Raw | ConvertFrom-Json
    Assert-Equal $quotaResult.category "provider_quota" "quota failure category"
    if ((Get-Content -LiteralPath (Join-Path $quotaArtifacts "model-channel-result.json") -Raw).Contains("quota exhausted")) {
        throw "delegate result leaked provider stderr"
    }

    $localArtifacts = Join-Path $root "local-denied"
    $localRequest = Join-Path $root "local-denied.json"
    [ordered]@{
        schema = "code-intel-model-adapter-request.v1"; adapter = "ollama"; executable = $null
        endpoint = "http://127.0.0.1:1/api/generate"; protocol = "ollama"; credentialEnvName = $null; model = "fixture-model"
        promptFile = $prompt; timeoutSeconds = 1; costScope = "local_compute"; externalData = $false; responseFormat = "json"
        consent = [ordered]@{ consumption = "granted"; localCompute = "denied"; paidSpend = "unanswered"; externalData = "unanswered" }
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $localRequest -Encoding utf8
    $localDenied = Invoke-DelegateChild $localRequest $localArtifacts
    Assert-Equal $localDenied.exitCode 2 "local endpoint without compute consent must be blocked before connection"
    $localResult = Get-Content -LiteralPath (Join-Path $localArtifacts "model-channel-result.json") -Raw | ConvertFrom-Json
    Assert-Equal $localResult.attempt.invoked $false "denied local route must make zero calls"

    $badFake = Join-Path $root "fake-bad-json.cmd"
    @("@echo off", "set /p ignored=", "echo not-json", "exit /b 0") | Set-Content -LiteralPath $badFake -Encoding ascii
    $badArtifacts = Join-Path $root "bad-json"
    $badRequest = Join-Path $root "bad-json.json"
    [ordered]@{
        schema = "code-intel-model-adapter-request.v1"; adapter = "opencode_cli"; executable = $badFake
        endpoint = $null; protocol = $null; credentialEnvName = $null; model = "fixture-model"
        promptFile = $prompt; timeoutSeconds = 10; costScope = "subscription_cli"; externalData = $true; responseFormat = "json"
        consent = [ordered]@{ consumption = "granted"; localCompute = "unanswered"; paidSpend = "unanswered"; externalData = "granted" }
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $badRequest -Encoding utf8
    $bad = Invoke-DelegateChild $badRequest $badArtifacts
    Assert-Equal $bad.exitCode 65 "non-JSON delegate output must be a protocol violation"
    $badResult = Get-Content -LiteralPath (Join-Path $badArtifacts "model-channel-result.json") -Raw | ConvertFrom-Json
    Assert-Equal $badResult.category "adapter_protocol_error" "non-JSON category"

    $slowFake = Join-Path $root "fake-slow.cmd"
    @("@echo off", "set /p ignored=", "ping -n 5 127.0.0.1 >nul", 'echo {"ok":true}') | Set-Content -LiteralPath $slowFake -Encoding ascii
    $slowArtifacts = Join-Path $root "timeout"
    $slowRequest = Join-Path $root "timeout.json"
    [ordered]@{
        schema = "code-intel-model-adapter-request.v1"; adapter = "codex_cli"; executable = $slowFake
        endpoint = $null; protocol = $null; credentialEnvName = $null; model = "fixture-model"
        promptFile = $prompt; timeoutSeconds = 1; costScope = "subscription_cli"; externalData = $true; responseFormat = "json"
        consent = [ordered]@{ consumption = "granted"; localCompute = "unanswered"; paidSpend = "unanswered"; externalData = "granted" }
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $slowRequest -Encoding utf8
    $slow = Invoke-DelegateChild $slowRequest $slowArtifacts
    Assert-Equal $slow.exitCode 75 "delegate timeout must be transient upstream"
    $slowResult = Get-Content -LiteralPath (Join-Path $slowArtifacts "model-channel-result.json") -Raw | ConvertFrom-Json
    Assert-Equal $slowResult.attempt.timedOut $true "timeout evidence"

    $requestSchema = Join-Path $repoRoot "orchestration\schemas\code-intel-model-adapter-request.v1.schema.json"
    $resultSchema = Join-Path $repoRoot "orchestration\schemas\code-intel-model-adapter-result.v1.schema.json"
    $requestFiles = @($deniedRequest, $successRequest, $cliEgressRequest, $localCliEgressRequest, $quotaRequest, $localRequest, $badRequest, $slowRequest)
    $resultFiles = @($deniedArtifacts, $successArtifacts, $cliEgressArtifacts, $localCliEgressArtifacts, $quotaArtifacts, $localArtifacts, $badArtifacts, $slowArtifacts) | ForEach-Object { Join-Path $_ "model-channel-result.json" }
    & python -c "import json,sys,jsonschema; s=json.load(open(sys.argv[1],encoding='utf-8')); v=jsonschema.Draft202012Validator(s); [v.validate(json.load(open(p,encoding='utf-8-sig'))) for p in sys.argv[2:]]" $requestSchema @requestFiles
    if ($LASTEXITCODE -ne 0) { throw "adapter request fixture failed its closed schema" }
    & python -c "import json,sys,jsonschema; s=json.load(open(sys.argv[1],encoding='utf-8')); v=jsonschema.Draft202012Validator(s); [v.validate(json.load(open(p,encoding='utf-8-sig'))) for p in sys.argv[2:]]" $resultSchema @resultFiles
    if ($LASTEXITCODE -ne 0) { throw "adapter result fixture failed its closed schema" }

    $facadeArtifacts = Join-Path $root "pipeline-facade"
    & pwsh -NoProfile -File (Join-Path $repoRoot "run-code-intel.ps1") -ModelAdapterRequest $deniedRequest -ModelAdapterArtifactRoot $facadeArtifacts 2>&1 | Out-Null
    Assert-Equal $LASTEXITCODE 65 "pipeline model-adapter facade must reject legacy raw executable requests"
    if (Test-Path -LiteralPath (Join-Path $facadeArtifacts "model-channel-result.json")) { throw "rejected legacy pipeline request must not create an invocation result" }

    $baseNegative = Get-Content -LiteralPath $successRequest -Raw | ConvertFrom-Json
    $wrongSchema = Join-Path $root "wrong-schema.json"
    $baseNegative.schema = "wrong.v1"
    $baseNegative | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $wrongSchema -Encoding utf8
    Assert-Equal (Invoke-DelegateChild $wrongSchema (Join-Path $root "wrong-schema-out")).exitCode 65 "wrong schema must be a contract violation"

    $unknownConsent = Join-Path $root "unknown-consent.json"
    $baseNegative = Get-Content -LiteralPath $successRequest -Raw | ConvertFrom-Json
    $baseNegative.consent | Add-Member -NotePropertyName surprise -NotePropertyValue granted
    $baseNegative | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $unknownConsent -Encoding utf8
    Assert-Equal (Invoke-DelegateChild $unknownConsent (Join-Path $root "unknown-consent-out")).exitCode 65 "unknown nested consent field must be rejected"

    $missingRequired = Join-Path $root "missing-required.json"
    $baseNegative = Get-Content -LiteralPath $successRequest -Raw | ConvertFrom-Json
    $baseNegative.PSObject.Properties.Remove("responseFormat")
    $baseNegative | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $missingRequired -Encoding utf8
    Assert-Equal (Invoke-DelegateChild $missingRequired (Join-Path $root "missing-required-out")).exitCode 65 "missing required field must be rejected"

    $malformed = Join-Path $root "malformed.json"
    "{" | Set-Content -LiteralPath $malformed -Encoding utf8
    Assert-Equal (Invoke-DelegateChild $malformed (Join-Path $root "malformed-out")).exitCode 64 "malformed JSON must be a usage error"

    $duplicate = Join-Path $root "duplicate-consent.json"
    $duplicateText = [regex]::Replace(
        (Get-Content -LiteralPath $successRequest -Raw),
        '"externalData"\s*:\s*"granted"',
        '"externalData": "granted", "externalData": "denied"',
        1
    )
    if ($duplicateText -eq (Get-Content -LiteralPath $successRequest -Raw)) { throw "duplicate-key fixture construction failed" }
    [IO.File]::WriteAllText($duplicate, $duplicateText, [Text.UTF8Encoding]::new($false))
    Assert-Equal (Invoke-DelegateChild $duplicate (Join-Path $root "duplicate-out")).exitCode 65 "duplicate consent key must be rejected"

    $delegateSource = Get-Content -LiteralPath $delegate -Raw
    foreach ($requiredGuard in @('$arguments.Add("--pure")', '$arguments.Add("plan")', '$start.WorkingDirectory = $artifactDir')) {
        if (-not $delegateSource.Contains($requiredGuard)) { throw "OpenCode isolation guard missing: $requiredGuard" }
    }

    "PASS test-model-channel-delegate"
}
finally {
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
