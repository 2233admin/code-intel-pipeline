param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSCommandPath
$gate = Join-Path $root "Test-CodeIntelProjectConformance.ps1"
$policy = Join-Path $root "orchestration\code-intel-project-conformance-policy.v1.json"
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-project-conformance-" + [guid]::NewGuid().ToString("N"))

function Invoke-Gate {
    param([string]$Profile, [string]$PolicyPath = $policy)
    $output = & pwsh -NoProfile -File $gate -Profile $Profile -Policy $PolicyPath -Json 2>&1
    $exitCode = $LASTEXITCODE
    $text = ($output | ForEach-Object { $_.ToString() }) -join "`n"
    [pscustomobject]@{ ExitCode = $exitCode; Result = ($text | ConvertFrom-Json) }
}

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null

    $fast = Invoke-Gate "fast"
    if ($fast.ExitCode -ne 0 -or [string]$fast.Result.verdict -ne "pass" -or @($fast.Result.suites).Count -ne 5) {
        throw "Fast conformance must execute and pass its five suites."
    }

    $full = Invoke-Gate "full"
    if ($full.ExitCode -ne 1 -or @($full.Result.failedGateIds) -notcontains "mechanism-readiness") {
        throw "Full conformance must fail closed while mapped mechanisms are incomplete."
    }

    $missingMapping = Get-Content -Raw -LiteralPath $policy | ConvertFrom-Json -Depth 30
    $missingMapping.mechanisms = @($missingMapping.mechanisms | Where-Object id -ne "reference-oracle")
    $missingMappingPath = Join-Path $temp "missing-mapping.json"
    $missingMapping | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $missingMappingPath -Encoding UTF8
    $missing = Invoke-Gate "fast" $missingMappingPath
    if ($missing.ExitCode -ne 1 -or @($missing.Result.failedGateIds) -notcontains "policy-mapping") {
        throw "Missing Pon-to-Code-Intel mapping must fail policy-mapping."
    }

    $unpinned = Get-Content -Raw -LiteralPath $policy | ConvertFrom-Json -Depth 30
    $unpinned.sourceMethod.revision = "main"
    $unpinnedPath = Join-Path $temp "unpinned.json"
    $unpinned | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $unpinnedPath -Encoding UTF8
    $invalid = Invoke-Gate "fast" $unpinnedPath
    if ($invalid.ExitCode -ne 2 -or @($invalid.Result.failedGateIds) -notcontains "input-shape") {
        throw "Unpinned upstream evidence must be malformed input."
    }

    $weakened = Get-Content -Raw -LiteralPath $policy | ConvertFrom-Json -Depth 30
    $weakened.profiles.full.requiredMechanisms = @($weakened.profiles.full.requiredMechanisms | Where-Object { $_ -ne "performance-ratchet" })
    $weakenedPath = Join-Path $temp "weakened-full.json"
    $weakened | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $weakenedPath -Encoding UTF8
    $weak = Invoke-Gate "full" $weakenedPath
    if ($weak.ExitCode -ne 1 -or @($weak.Result.failedGateIds) -notcontains "profile-contract") {
        throw "Removing an unfinished full-profile mechanism must fail profile-contract."
    }

    Write-Host "PASS project conformance: fast suites pass; full and three invalid policies fail closed"
} finally {
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}

# Expected fail-closed child invocations leave a non-zero native exit code behind.
# Normalize the script's successful aggregate result for callers such as Actions.
$global:LASTEXITCODE = 0
