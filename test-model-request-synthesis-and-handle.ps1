#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$temp = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-handle-test-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $temp | Out-Null
try {
    $fake = Join-Path $temp "fake-model.exe"
    $source = Join-Path $temp "fake-model.cs"
    [IO.File]::WriteAllText($source, 'public static class Program { public static void Main(string[] args) { System.Console.In.ReadToEnd(); System.Console.WriteLine("{\"ok\":true}"); } }', [Text.UTF8Encoding]::new($false))
    & "$env:WINDIR\Microsoft.NET\Framework\v4.0.30319\csc.exe" /nologo /target:exe "/out:$fake" $source
    if ($LASTEXITCODE -ne 0) { throw "fake model compilation failed" }
    $prompt = Join-Path $temp "prompt.txt"
    [IO.File]::WriteAllText($prompt, "private prompt sentinel", [Text.UTF8Encoding]::new($false))
    $inventory = Join-Path $temp "inventory.json"
    $routing = Join-Path $temp "routing.json"
    $handle = Join-Path $temp "handle.json"
    $request = Join-Path $temp "request.json"
    $candidate = [ordered]@{
        id="fixture.claude"; channelKind="claude_cli"; provider="fixture"; model="fixture-model"; costScope="subscription_cli"
        endpointConfigured=$false; discovered=$true; executableVerified=$true; authPresent="present"; modelAvailable="available"
        externalEgress=$true; source="local_discovery"; diagnostics=@("candidate_verified")
    }
    [IO.File]::WriteAllText($inventory, ([ordered]@{schema="code-intel-model-channel-inventory-result.v1";candidates=@($candidate);configurationBrokers=@()} | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $route = [ordered]@{
        schema="code-intel-model-routing-result.v1";status="ready"
        selected=[ordered]@{candidateId="fixture.claude";channelKind="claude_cli";provider="fixture";model="fixture-model";costScope="subscription_cli";readinessState="ready"}
        authorization=[ordered]@{consumptionAuthorization=[ordered]@{status="granted";scopes=@("subscription_cli")};externalData=[ordered]@{status="granted"};paidSpend=[ordered]@{status="unanswered"}}
        attempts=@();manualAction=$null
    }
    [IO.File]::WriteAllText($routing, ($route | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))

    & (Join-Path $root "New-ModelExecutableHandle.ps1") -Adapter claude_cli -Executable $fake -OutputPath $handle -LifetimeSeconds 120 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "handle issuance failed" }
    & (Join-Path $root "New-ModelAdapterRequest.ps1") -Inventory $inventory -Routing $routing -PromptFile $prompt -ExecutableHandle $handle -OutputPath $request | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "request synthesis failed" }
    $synthesized = Get-Content $request -Raw | ConvertFrom-Json
    if ($synthesized.schema -ne "code-intel-model-adapter-request.v2" -or $synthesized.costScope -ne "subscription_cli" -or -not [bool]$synthesized.externalData) { throw "request did not preserve derived routing policy" }
    $artifact = Join-Path $temp "happy"
    & (Join-Path $root "Invoke-ModelChannelDelegate.ps1") -Request $request -ArtifactRoot $artifact | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "verified handle did not execute successfully" }
    $result = Get-Content (Join-Path $artifact "model-channel-result.json") -Raw | ConvertFrom-Json
    if ($result.status -ne "completed" -or -not [bool]$result.attempt.invoked) { throw "delegate did not report a completed invocation" }

    $pipelineArtifact = Join-Path $temp "pipeline"
    & (Join-Path $root "run-code-intel.ps1") -ModelInventoryResult $inventory -ModelRoutingResult $routing -ModelPromptFile $prompt -ModelExecutableHandle $handle -ModelAdapterArtifactRoot $pipelineArtifact | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "pipeline synthesis-and-invoke facade failed" }
    $pipelineResult = Get-Content (Join-Path $pipelineArtifact "model-channel-result.json") -Raw | ConvertFrom-Json
    if ($pipelineResult.status -ne "completed" -or -not (Test-Path (Join-Path $pipelineArtifact "model-adapter-request.v2.json"))) { throw "pipeline facade did not synthesize and complete v2 request" }

    [IO.File]::AppendAllText($fake, "mutation")
    $mutatedArtifact = Join-Path $temp "mutated"
    & pwsh -NoProfile -File (Join-Path $root "Invoke-ModelChannelDelegate.ps1") -Request $request -ArtifactRoot $mutatedArtifact 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0 -or (Test-Path (Join-Path $mutatedArtifact "model-channel-result.json"))) { throw "mutated executable was not rejected before invocation" }

    $route.selected.model = "different-model"
    [IO.File]::WriteAllText($routing, ($route | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    & pwsh -NoProfile -File (Join-Path $root "New-ModelAdapterRequest.ps1") -Inventory $inventory -Routing $routing -PromptFile $prompt -ExecutableHandle $handle -OutputPath (Join-Path $temp "bad.json") 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { throw "inventory/routing mismatch was accepted" }
    "model request synthesis and executable handle tests passed"
}
finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
