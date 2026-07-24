#requires -Version 7.2

param(
    [ValidateSet("fast", "full")]
    [string]$Profile = "fast",

    [string]$Policy = (Join-Path ([System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))) "orchestration\code-intel-project-conformance-policy.v1.json"),

    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$gates = [System.Collections.Generic.List[object]]::new()
$suiteResults = [System.Collections.Generic.List[object]]::new()

function Add-Gate {
    param([string]$Id, [bool]$Passed, [string]$Detail)
    $gates.Add([pscustomobject]@{ id = $Id; passed = $Passed; detail = $Detail })
}

function Test-ExactSet {
    param([object[]]$Actual, [object[]]$Expected)
    $actualItems = @($Actual | ForEach-Object { [string]$_ } | Sort-Object)
    $expectedItems = @($Expected | ForEach-Object { [string]$_ } | Sort-Object)
    return $actualItems.Count -eq $expectedItems.Count -and @(Compare-Object $actualItems $expectedItems).Count -eq 0
}

function Resolve-RepoPath {
    param([string]$RelativePath, [switch]$Leaf)
    if ([string]::IsNullOrWhiteSpace($RelativePath) -or [System.IO.Path]::IsPathRooted($RelativePath)) {
        throw "path must be repository-relative: $RelativePath"
    }
    $prefix = [System.IO.Path]::TrimEndingDirectorySeparator($root) + [System.IO.Path]::DirectorySeparatorChar
    $resolved = [System.IO.Path]::GetFullPath((Join-Path $root $RelativePath))
    if (-not $resolved.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "path escapes repository: $RelativePath"
    }
    if (-not (Test-Path -LiteralPath $resolved)) {
        throw "required path is missing: $RelativePath"
    }
    if ($Leaf -and -not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "required file is missing: $RelativePath"
    }
    return $resolved
}

function Complete-Result {
    param([int]$MalformedExit = 0)
    $failed = @($gates | Where-Object { -not $_.passed })
    $result = [ordered]@{
        schema = "code-intel-project-conformance-result.v1"
        profile = $Profile
        verdict = if ($failed.Count -eq 0 -and $MalformedExit -eq 0) { "pass" } else { "fail" }
        gates = @($gates)
        failedGateIds = @($failed | ForEach-Object id)
        suites = @($suiteResults)
    }
    if ($Json) {
        $result | ConvertTo-Json -Depth 12
    } else {
        Write-Host "Code Intel project conformance: $($result.verdict) profile=$Profile"
        foreach ($gate in $gates) {
            Write-Host "$(if ($gate.passed) { 'PASS' } else { 'FAIL' }) $($gate.id): $($gate.detail)"
        }
        foreach ($suite in $suiteResults) {
            Write-Host "$(if ($suite.passed) { 'PASS' } else { 'FAIL' }) suite/$($suite.id): exit=$($suite.exitCode)"
        }
    }
    if ($MalformedExit -ne 0) { exit $MalformedExit }
    if ($failed.Count -gt 0) { exit 1 }
    exit 0
}

try {
    $policyDocument = Get-Content -Raw -LiteralPath $Policy | ConvertFrom-Json -Depth 30
} catch {
    Add-Gate "input-shape" $false $_.Exception.Message
    Complete-Result -MalformedExit 2
}

$profileProperty = $policyDocument.profiles.PSObject.Properties[$Profile]
$shapePass = [string]$policyDocument.schema -eq "code-intel-project-conformance-policy.v1" -and
    $null -ne $profileProperty -and
    -not [string]::IsNullOrWhiteSpace([string]$policyDocument.sourceMethod.uri) -and
    [string]$policyDocument.sourceMethod.revision -match '^[0-9a-f]{40}$'
Add-Gate "input-shape" $shapePass "policy schema, profile, source URI, and pinned revision are required"
if (-not $shapePass) { Complete-Result -MalformedExit 2 }

$requiredMapping = @(
    "reference-oracle",
    "conformance-corpus",
    "normalized-output-parity",
    "monotonic-floor",
    "expected-divergence-ledger",
    "mutation-robustness",
    "deterministic-stress",
    "performance-ratchet"
)
$mechanisms = @($policyDocument.mechanisms)
$mechanismIds = @($mechanisms | ForEach-Object { [string]$_.id })
$mappingPass = @($mechanismIds | Sort-Object -Unique).Count -eq $mechanismIds.Count -and
    @($requiredMapping | Where-Object { $_ -notin $mechanismIds }).Count -eq 0
foreach ($mechanism in $mechanisms) {
    if ([string]$mechanism.status -notin @("implemented", "partial", "designed", "deferred")) { $mappingPass = $false }
    if ([string]::IsNullOrWhiteSpace([string]$mechanism.ponForm) -or [string]::IsNullOrWhiteSpace([string]$mechanism.codeIntelForm)) { $mappingPass = $false }
    foreach ($evidencePath in @($mechanism.evidence)) {
        try { $null = Resolve-RepoPath ([string]$evidencePath) } catch { $mappingPass = $false }
    }
}
Add-Gate "policy-mapping" $mappingPass "all eight Pon mechanisms have explicit Code Intel mappings and valid statuses"

$profilePolicy = $profileProperty.Value
$acceptedStatuses = @($profilePolicy.acceptedMechanismStatuses | ForEach-Object { [string]$_ })
$expectedProfileMechanisms = if ($Profile -eq "fast") {
    @("reference-oracle", "conformance-corpus", "normalized-output-parity", "monotonic-floor", "mutation-robustness", "deterministic-stress")
} else {
    $requiredMapping
}
$expectedProfileSuites = if ($Profile -eq "fast") {
    @("parity-floor", "adapter-contract", "multilanguage-corpus", "python314-development", "merge-queue-contract")
} else {
    @("parity-floor", "adapter-contract", "multilanguage-corpus", "python314-development", "merge-queue-contract", "pipeline-smoke")
}
$expectedStatuses = if ($Profile -eq "fast") { @("implemented", "partial") } else { @("implemented") }
$declaredProfileMechanisms = @($profilePolicy.requiredMechanisms | ForEach-Object { [string]$_ })
$declaredProfileSuites = @($profilePolicy.requiredSuites | ForEach-Object { [string]$_ })
$profileContractPass = (Test-ExactSet $declaredProfileMechanisms $expectedProfileMechanisms) -and
    (Test-ExactSet $declaredProfileSuites $expectedProfileSuites) -and
    (Test-ExactSet $acceptedStatuses $expectedStatuses)
Add-Gate "profile-contract" $profileContractPass "required mechanisms, suites, and accepted statuses cannot be weakened"

$unready = [System.Collections.Generic.List[string]]::new()
foreach ($requiredId in @($profilePolicy.requiredMechanisms)) {
    $mechanism = $mechanisms | Where-Object id -eq $requiredId | Select-Object -First 1
    if ($null -eq $mechanism -or [string]$mechanism.status -notin $acceptedStatuses) {
        $status = if ($null -eq $mechanism) { "missing" } else { [string]$mechanism.status }
        $unready.Add("$requiredId=$status")
    }
}
$readinessPass = $unready.Count -eq 0
Add-Gate "mechanism-readiness" $readinessPass "unready=$($unready -join ', '); accepted=$($acceptedStatuses -join ',')"

$suiteIds = @($policyDocument.suites | ForEach-Object { [string]$_.id })
$requiredSuiteIds = @($profilePolicy.requiredSuites | ForEach-Object { [string]$_ })
$suiteContractPass = @($suiteIds | Sort-Object -Unique).Count -eq $suiteIds.Count -and
    @($requiredSuiteIds | Where-Object { $_ -notin $suiteIds }).Count -eq 0
Add-Gate "suite-contract" $suiteContractPass "required suites exist and suite ids are unique"

if ($mappingPass -and $profileContractPass -and $readinessPass -and $suiteContractPass) {
    foreach ($suiteId in $requiredSuiteIds) {
        $suite = $policyDocument.suites | Where-Object id -eq $suiteId | Select-Object -First 1
        $command = $suite.command
        $output = @()
        $exitCode = 1
        try {
            if ([string]$command.kind -eq "pwsh") {
                $scriptPath = Resolve-RepoPath ([string]$command.file) -Leaf
                $output = @(& pwsh -NoProfile -File $scriptPath @($command.args) 2>&1 | ForEach-Object { $_.ToString() })
                $exitCode = $LASTEXITCODE
            } elseif ([string]$command.kind -eq "process") {
                $output = @(& ([string]$command.executable) @($command.args) 2>&1 | ForEach-Object { $_.ToString() })
                $exitCode = $LASTEXITCODE
            } else {
                throw "unsupported suite command kind: $($command.kind)"
            }
        } catch {
            $output = @($_.Exception.Message)
            $exitCode = 1
        }
        $text = ($output -join "`n")
        if ($text.Length -gt 4000) { $text = $text.Substring($text.Length - 4000) }
        $suiteResults.Add([pscustomobject]@{
            id = $suiteId
            passed = $exitCode -eq 0
            exitCode = $exitCode
            mechanisms = @($suite.mechanisms)
            outputTail = $text
        })
    }
}

$suitePass = $readinessPass -and $suiteContractPass -and
    $suiteResults.Count -eq $requiredSuiteIds.Count -and
    @($suiteResults | Where-Object { -not $_.passed }).Count -eq 0
Add-Gate "executable-suites" $suitePass "passed=$(@($suiteResults | Where-Object passed).Count)/$($requiredSuiteIds.Count)"

Complete-Result
