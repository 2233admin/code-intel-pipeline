param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$sourceAdapter = Join-Path $root "Invoke-CodeIntelAcceptance.ps1"
$sourcePolicy = Join-Path $root "orchestration\code-intel-acceptance-policy.v1.json"
$sourceSchema = Join-Path $root "orchestration\schemas\code-intel-acceptance-result.v1.schema.json"
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("code intel acceptance fixture " + [guid]::NewGuid().ToString("N"))

function Test-ExactSet {
    param([object[]]$Actual, [object[]]$Expected)
    $actualItems = @($Actual | ForEach-Object { [string]$_ } | Sort-Object)
    $expectedItems = @($Expected | ForEach-Object { [string]$_ } | Sort-Object)
    return $actualItems.Count -eq $expectedItems.Count -and @(Compare-Object $actualItems $expectedItems).Count -eq 0
}

function New-TargetJson {
    param(
        [string]$Id,
        [string]$Kind,
        [string]$File = "",
        [string]$Executable = "",
        [string[]]$Arguments = @()
    )

    $document = [ordered]@{ id = $Id; kind = $Kind }
    if ($Kind -eq "pwsh") { $document.file = $File } else { $document.executable = $Executable }
    $document.args = @($Arguments)
    return ($document | ConvertTo-Json -Depth 8 -Compress)
}

function Invoke-Acceptance {
    param(
        [string]$Stage,
        [string[]]$Targets = @(),
        [string]$SkipReason = "",
        [string]$ProjectVerdict = "pass"
    )

    $env:ACCEPTANCE_STUB_VERDICT = $ProjectVerdict
    $arguments = @("-NoProfile", "-File", (Join-Path $temp "Invoke-CodeIntelAcceptance.ps1"), "-Stage", $Stage, "-Json")
    foreach ($target in @($Targets)) { $arguments += @("-TargetCheckJson", $target) }
    if (-not [string]::IsNullOrWhiteSpace($SkipReason)) { $arguments += @("-SkipTargetedChecksReason", $SkipReason) }
    $output = @(& pwsh @arguments 2>&1 | ForEach-Object { $_.ToString() })
    $exitCode = $LASTEXITCODE
    $text = $output -join "`n"
    try {
        $result = $text | ConvertFrom-Json -Depth 30
    } catch {
        throw "Acceptance adapter did not return JSON. output=$text"
    }
    return [pscustomobject]@{ ExitCode = $exitCode; Result = $result }
}

function Reset-Events {
    if (Test-Path -LiteralPath $env:ACCEPTANCE_EVENT_LOG) {
        Remove-Item -LiteralPath $env:ACCEPTANCE_EVENT_LOG -Force
    }
}

function Read-Events {
    if (-not (Test-Path -LiteralPath $env:ACCEPTANCE_EVENT_LOG)) { return @() }
    return @(Get-Content -LiteralPath $env:ACCEPTANCE_EVENT_LOG)
}

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null
    $targetDirectory = Join-Path $temp "target checks"
    New-Item -ItemType Directory -Force -Path $targetDirectory | Out-Null
    Copy-Item -LiteralPath $sourceAdapter -Destination (Join-Path $temp "Invoke-CodeIntelAcceptance.ps1")
    New-Item -ItemType Directory -Force -Path (Join-Path $temp "orchestration") | Out-Null
    Copy-Item -LiteralPath $sourcePolicy -Destination (Join-Path $temp "orchestration\code-intel-acceptance-policy.v1.json")

    @'
param(
    [ValidateSet("fast", "full")]
    [string]$Profile,
    [switch]$Json
)
$verdict = if ([string]::IsNullOrWhiteSpace($env:ACCEPTANCE_STUB_VERDICT)) { "pass" } else { $env:ACCEPTANCE_STUB_VERDICT }
Add-Content -LiteralPath $env:ACCEPTANCE_EVENT_LOG -Value "conformance:$Profile"
[ordered]@{ schema = "code-intel-project-conformance-result.v1"; profile = $Profile; verdict = $verdict } | ConvertTo-Json
if ($verdict -eq "pass") { exit 0 }
exit 1
'@ | Set-Content -LiteralPath (Join-Path $temp "scripts/tests/Test-CodeIntelProjectConformance.ps1") -Encoding UTF8

    @'
param([string]$Value = "ok")
Add-Content -LiteralPath $env:ACCEPTANCE_EVENT_LOG -Value "target:$Value"
exit 0
'@ | Set-Content -LiteralPath (Join-Path $targetDirectory "pass target.ps1") -Encoding UTF8

    @'
Add-Content -LiteralPath $env:ACCEPTANCE_EVENT_LOG -Value "target:failed"
exit 7
'@ | Set-Content -LiteralPath (Join-Path $targetDirectory "fail target.ps1") -Encoding UTF8

    $env:ACCEPTANCE_EVENT_LOG = Join-Path $temp "events.log"
    $passTarget = New-TargetJson -Id "focused-test" -Kind "pwsh" -File "target checks\pass target.ps1" -Arguments @("hello world")
    $failTarget = New-TargetJson -Id "focused-failure" -Kind "pwsh" -File "target checks\fail target.ps1"

    Reset-Events
    $agent = Invoke-Acceptance -Stage "agent" -Targets @($passTarget)
    if ($agent.ExitCode -ne 0 -or [string]$agent.Result.status -ne "pass" -or [string]$agent.Result.profile -ne "fast") {
        throw "Agent stage must pass targeted checks plus the fast profile."
    }
    if (-not (Test-ExactSet -Actual (Read-Events) -Expected @("target:hello world", "conformance:fast"))) {
        throw "Agent stage did not execute exactly the targeted check and fast conformance in a path containing spaces."
    }

    Reset-Events
    $land = Invoke-Acceptance -Stage "land"
    if ($land.ExitCode -ne 0 -or [string]$land.Result.status -ne "pass" -or [string]$land.Result.profile -ne "fast" -or
        -not (Test-ExactSet -Actual (Read-Events) -Expected @("conformance:fast"))) {
        throw "Land stage must map only to fast project conformance."
    }

    Reset-Events
    $promote = Invoke-Acceptance -Stage "promote"
    if ($promote.ExitCode -ne 0 -or [string]$promote.Result.status -ne "pass" -or [string]$promote.Result.profile -ne "full" -or
        -not (Test-ExactSet -Actual (Read-Events) -Expected @("conformance:full"))) {
        throw "Promote stage must map only to full project conformance."
    }

    Reset-Events
    $targetFailure = Invoke-Acceptance -Stage "agent" -Targets @($failTarget)
    if ($targetFailure.ExitCode -ne 1 -or [string]$targetFailure.Result.status -ne "fail" -or
        @($targetFailure.Result.checks | Where-Object { $_.id -eq "focused-failure" -and $_.status -eq "fail" }).Count -ne 1) {
        throw "A targeted test failure must propagate as fail."
    }

    Reset-Events
    $projectFailure = Invoke-Acceptance -Stage "land" -ProjectVerdict "fail"
    if ($projectFailure.ExitCode -ne 1 -or [string]$projectFailure.Result.status -ne "fail" -or
        @($projectFailure.Result.checks | Where-Object { $_.kind -eq "project-conformance" -and $_.status -eq "fail" }).Count -ne 1) {
        throw "A project conformance rejection must propagate as fail."
    }

    Reset-Events
    $missingTarget = Invoke-Acceptance -Stage "agent"
    if ($missingTarget.ExitCode -ne 2 -or [string]$missingTarget.Result.status -ne "blocked" -or @(Read-Events).Count -ne 0) {
        throw "Agent acceptance without targeted evidence must block before execution."
    }

    Reset-Events
    $skippedTarget = Invoke-Acceptance -Stage "agent" -SkipReason "no focused test exists for documentation-only change"
    if ($skippedTarget.ExitCode -ne 3 -or [string]$skippedTarget.Result.status -ne "skipped-with-reason" -or
        -not (Test-ExactSet -Actual (Read-Events) -Expected @("conformance:fast"))) {
        throw "An explicit target skip must remain non-passing while still collecting fast conformance evidence."
    }

    Reset-Events
    $marker = Join-Path $temp "injection-marker.txt"
    $injectionTarget = New-TargetJson -Id "injection" -Kind "pwsh" -File "target checks\pass target.ps1" -Arguments @("safe; Set-Content -LiteralPath '$marker' injected")
    $injection = Invoke-Acceptance -Stage "agent" -Targets @($injectionTarget)
    if ($injection.ExitCode -ne 2 -or [string]$injection.Result.status -ne "blocked" -or
        (Test-Path -LiteralPath $marker) -or @(Read-Events).Count -ne 0) {
        throw "Shell syntax must block the complete request before any target or project command runs."
    }

    Reset-Events
    $inlineEval = New-TargetJson -Id "inline-eval" -Kind "process" -Executable "python" -Arguments @("-c", "print('unsafe')")
    $evalResult = Invoke-Acceptance -Stage "agent" -Targets @($inlineEval)
    if ($evalResult.ExitCode -ne 2 -or [string]$evalResult.Result.status -ne "blocked" -or @(Read-Events).Count -ne 0) {
        throw "Inline interpreter evaluation must be blocked before execution."
    }

    Reset-Events
    $landOverride = Invoke-Acceptance -Stage "land" -Targets @($passTarget)
    if ($landOverride.ExitCode -ne 2 -or [string]$landOverride.Result.status -ne "blocked" -or @(Read-Events).Count -ne 0) {
        throw "Land and promote stages must reject targeted-check overrides."
    }

    $schema = Get-Content -Raw -LiteralPath $sourceSchema | ConvertFrom-Json -Depth 30
    $statuses = @($schema.'$defs'.status.enum)
    if (-not (Test-ExactSet -Actual $statuses -Expected @("pass", "fail", "blocked", "skipped-with-reason"))) {
        throw "The result schema must expose exactly the four unified statuses."
    }

    Write-Host "PASS three-stage acceptance: agent/land/promote mapping, failure propagation, fail-closed skips, injection defenses, and spaced paths"
} finally {
    Remove-Item Env:ACCEPTANCE_STUB_VERDICT -ErrorAction SilentlyContinue
    Remove-Item Env:ACCEPTANCE_EVENT_LOG -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $temp) {
        Remove-Item -LiteralPath $temp -Recurse -Force
    }
}
