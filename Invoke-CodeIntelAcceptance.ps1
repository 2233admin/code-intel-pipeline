#requires -Version 7.2

param(
    [Parameter(Mandatory)]
    [ValidateSet("agent", "land", "promote")]
    [string]$Stage,

    [string[]]$TargetCheckJson = @(),

    [string]$SkipTargetedChecksReason = "",

    [string]$Policy = (Join-Path $PSScriptRoot "orchestration\code-intel-acceptance-policy.v1.json"),

    [string]$ProjectConformanceScript = (Join-Path $PSScriptRoot "scripts/tests/Test-CodeIntelProjectConformance.ps1"),

    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = [System.IO.Path]::GetFullPath($PSScriptRoot)
$profile = if ($Stage -eq "promote") { "full" } else { "fast" }
$checks = [System.Collections.Generic.List[object]]::new()
$validatedTargets = [System.Collections.Generic.List[object]]::new()

function Test-ExactSet {
    param([object[]]$Actual, [object[]]$Expected)
    $actualItems = @($Actual | ForEach-Object { [string]$_ } | Sort-Object)
    $expectedItems = @($Expected | ForEach-Object { [string]$_ } | Sort-Object)
    return $actualItems.Count -eq $expectedItems.Count -and @(Compare-Object $actualItems $expectedItems).Count -eq 0
}

function Resolve-RepoBoundFile {
    param([string]$Path, [string]$Purpose)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "$Purpose path is required"
    }
    $candidate = if ([System.IO.Path]::IsPathRooted($Path)) { $Path } else { Join-Path $root $Path }
    $resolved = [System.IO.Path]::GetFullPath($candidate)
    $prefix = [System.IO.Path]::TrimEndingDirectorySeparator($root) + [System.IO.Path]::DirectorySeparatorChar
    if (-not $resolved.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$Purpose path escapes repository: $Path"
    }
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "$Purpose file is missing: $Path"
    }
    return $resolved
}

function Add-Check {
    param(
        [string]$Id,
        [ValidateSet("targeted", "project-conformance")]
        [string]$Kind,
        [ValidateSet("pass", "fail", "blocked", "skipped-with-reason")]
        [string]$Status,
        [AllowNull()][Nullable[int]]$ExitCode,
        [string]$Reason,
        [string]$OutputTail = ""
    )

    if ($OutputTail.Length -gt 4000) {
        $OutputTail = $OutputTail.Substring($OutputTail.Length - 4000)
    }
    $checks.Add([pscustomobject][ordered]@{
        id = $Id
        kind = $Kind
        status = $Status
        exitCode = $ExitCode
        reason = $Reason
        outputTail = $OutputTail
    })
}

function Complete-Result {
    $status = if (@($checks | Where-Object status -eq "blocked").Count -gt 0) {
        "blocked"
    } elseif (@($checks | Where-Object status -eq "fail").Count -gt 0) {
        "fail"
    } elseif (@($checks | Where-Object status -eq "skipped-with-reason").Count -gt 0) {
        "skipped-with-reason"
    } else {
        "pass"
    }
    $nonPassing = @($checks | Where-Object status -ne "pass")
    $reason = if ($nonPassing.Count -eq 0) {
        "all required acceptance checks passed"
    } else {
        ($nonPassing | ForEach-Object { "$($_.id): $($_.reason)" }) -join "; "
    }
    $result = [ordered]@{
        schema = "code-intel-acceptance-result.v1"
        stage = $Stage
        profile = $profile
        status = $status
        reason = $reason
        checks = @($checks)
    }

    if ($Json) {
        $result | ConvertTo-Json -Depth 12
    } else {
        Write-Host "Code Intel acceptance: $status stage=$Stage profile=$profile"
        foreach ($check in $checks) {
            Write-Host "$($check.status.ToUpperInvariant()) $($check.kind)/$($check.id): $($check.reason)"
        }
    }

    switch ($status) {
        "pass" { exit 0 }
        "fail" { exit 1 }
        "blocked" { exit 2 }
        "skipped-with-reason" { exit 3 }
    }
}

function Test-PolicyContract {
    param([object]$Document)

    try {
        $stageNames = @($Document.stages.PSObject.Properties.Name)
        $allowedExecutables = @($Document.targetedChecks.allowedProcessExecutables | ForEach-Object { [string]$_ })
        $intrinsicAllowed = @("cargo", "dotnet", "go", "node", "npm", "pnpm", "python", "python3", "pytest", "yarn")
        $prohibitedShells = @("bash", "cmd", "cmd.exe", "powershell", ("power" + "shell.exe"), "pwsh", "pwsh.exe", "sh", "wsl", "zsh")
        return [string]$Document.schema -eq "code-intel-acceptance-policy.v1" -and
            (Test-ExactSet @($Document.statuses) @("pass", "fail", "blocked", "skipped-with-reason")) -and
            (Test-ExactSet $stageNames @("agent", "land", "promote")) -and
            [string]$Document.stages.agent.projectProfile -eq "fast" -and
            [string]$Document.stages.agent.targetedChecks -eq "required" -and
            [string]$Document.stages.land.projectProfile -eq "fast" -and
            [string]$Document.stages.land.targetedChecks -eq "forbidden" -and
            [string]$Document.stages.promote.projectProfile -eq "full" -and
            [string]$Document.stages.promote.targetedChecks -eq "forbidden" -and
            [int]$Document.targetedChecks.maximumCount -ge 1 -and
            [int]$Document.targetedChecks.maximumCount -le 16 -and
            (Test-ExactSet @($Document.targetedChecks.allowedKinds) @("pwsh", "process")) -and
            @($allowedExecutables | Where-Object { $_ -notin $intrinsicAllowed }).Count -eq 0 -and
            @($allowedExecutables | Where-Object { $_ -in $prohibitedShells }).Count -eq 0 -and
            [string]$Document.targetedChecks.forbiddenShellTokenPattern -eq '[;&|<>`]|\$\(|[\r\n]'
    } catch {
        return $false
    }
}

function ConvertTo-ValidatedTarget {
    param([string]$Raw, [object]$PolicyDocument)

    try {
        $document = $Raw | ConvertFrom-Json -Depth 12
    } catch {
        throw "targeted check JSON is malformed: $($_.Exception.Message)"
    }
    if ($null -eq $document -or $document -is [array]) {
        throw "each targeted check must be one JSON object"
    }

    $id = [string]$document.id
    $kind = [string]$document.kind
    if ($id -notmatch '^[a-z0-9][a-z0-9._-]{0,63}$') {
        throw "targeted check id is invalid: $id"
    }
    if ($kind -notin @($PolicyDocument.targetedChecks.allowedKinds)) {
        throw "targeted check kind is not allowed: $kind"
    }

    $allowedProperties = if ($kind -eq "pwsh") { @("id", "kind", "file", "args") } else { @("id", "kind", "executable", "args") }
    $actualProperties = @($document.PSObject.Properties.Name)
    if (@($actualProperties | Where-Object { $_ -notin $allowedProperties }).Count -gt 0) {
        throw "targeted check $id contains unsupported properties"
    }

    $arguments = @($document.args | ForEach-Object { [string]$_ })
    $shellPattern = [string]$PolicyDocument.targetedChecks.forbiddenShellTokenPattern
    foreach ($argument in $arguments) {
        if ($argument -match $shellPattern -or $argument.IndexOf([char]0) -ge 0) {
            throw "targeted check $id contains forbidden shell syntax"
        }
    }

    if ($kind -eq "pwsh") {
        $file = Resolve-RepoBoundFile -Path ([string]$document.file) -Purpose "targeted PowerShell check"
        return [pscustomobject]@{ id = $id; kind = $kind; file = $file; executable = ""; args = $arguments }
    }

    $executable = [string]$document.executable
    if ([string]::IsNullOrWhiteSpace($executable) -or $executable -match '[/\\:]') {
        throw "targeted check $id process executable must be an allowed command name"
    }
    if ($executable -notin @($PolicyDocument.targetedChecks.allowedProcessExecutables)) {
        throw "targeted check $id process executable is not allowed: $executable"
    }

    $intrinsicForbiddenLeading = @{
        node = @("-e", "--eval", "-p", "--print")
        npm = @("exec", "x")
        pnpm = @("dlx", "exec")
        python = @("-c")
        python3 = @("-c")
        yarn = @("dlx", "exec")
    }
    if ($arguments.Count -gt 0 -and $intrinsicForbiddenLeading.ContainsKey($executable) -and
        $arguments[0] -in @($intrinsicForbiddenLeading[$executable])) {
        throw "targeted check $id uses a forbidden inline-execution argument"
    }
    return [pscustomobject]@{ id = $id; kind = $kind; file = ""; executable = $executable; args = $arguments }
}

try {
    $policyPath = Resolve-RepoBoundFile -Path $Policy -Purpose "acceptance policy"
    $policyDocument = Get-Content -Raw -LiteralPath $policyPath | ConvertFrom-Json -Depth 30
} catch {
    Add-Check -Id "request" -Kind "project-conformance" -Status "blocked" -ExitCode $null -Reason $_.Exception.Message
    Complete-Result
}

if (-not (Test-PolicyContract -Document $policyDocument)) {
    Add-Check -Id "policy-contract" -Kind "project-conformance" -Status "blocked" -ExitCode $null -Reason "acceptance policy is malformed or weakens the fixed stage contract"
    Complete-Result
}
$profile = [string]$policyDocument.stages.PSObject.Properties[$Stage].Value.projectProfile

$hasTargets = @($TargetCheckJson).Count -gt 0
$hasSkipReason = -not [string]::IsNullOrWhiteSpace($SkipTargetedChecksReason)
if ($Stage -eq "agent") {
    if ($hasTargets -and $hasSkipReason) {
        Add-Check -Id "targeted-checks" -Kind "targeted" -Status "blocked" -ExitCode $null -Reason "targeted checks and an explicit skip reason are mutually exclusive"
        Complete-Result
    }
    if (-not $hasTargets -and -not $hasSkipReason) {
        Add-Check -Id "targeted-checks" -Kind "targeted" -Status "blocked" -ExitCode $null -Reason "agent acceptance requires explicit targeted checks or an explicit skip reason"
        Complete-Result
    }
} elseif ($hasTargets -or $hasSkipReason) {
    Add-Check -Id "targeted-checks" -Kind "targeted" -Status "blocked" -ExitCode $null -Reason "$Stage acceptance forbids targeted-check overrides"
    Complete-Result
}

if (@($TargetCheckJson).Count -gt [int]$policyDocument.targetedChecks.maximumCount) {
    Add-Check -Id "targeted-checks" -Kind "targeted" -Status "blocked" -ExitCode $null -Reason "targeted check count exceeds policy maximum"
    Complete-Result
}

try {
    foreach ($raw in @($TargetCheckJson)) {
        $validatedTargets.Add((ConvertTo-ValidatedTarget -Raw $raw -PolicyDocument $policyDocument))
    }
    $ids = @($validatedTargets | ForEach-Object id)
    if (@($ids | Sort-Object -Unique).Count -ne $ids.Count) {
        throw "targeted check ids must be unique"
    }
    $conformancePath = Resolve-RepoBoundFile -Path $ProjectConformanceScript -Purpose "project conformance"
} catch {
    Add-Check -Id "targeted-checks" -Kind "targeted" -Status "blocked" -ExitCode $null -Reason $_.Exception.Message
    Complete-Result
}

if ($hasSkipReason) {
    Add-Check -Id "targeted-checks" -Kind "targeted" -Status "skipped-with-reason" -ExitCode $null -Reason $SkipTargetedChecksReason
} else {
    foreach ($target in $validatedTargets) {
        try {
            $output = if ($target.kind -eq "pwsh") {
                @(& pwsh -NoProfile -File $target.file @($target.args) 2>&1 | ForEach-Object { $_.ToString() })
            } else {
                @(& $target.executable @($target.args) 2>&1 | ForEach-Object { $_.ToString() })
            }
            $exitCode = $LASTEXITCODE
            $targetStatus = if ($exitCode -eq 0) { "pass" } else { "fail" }
            $targetReason = if ($exitCode -eq 0) { "targeted check passed" } else { "targeted check exited nonzero" }
            Add-Check -Id $target.id -Kind "targeted" -Status $targetStatus -ExitCode $exitCode -Reason $targetReason -OutputTail ($output -join "`n")
        } catch {
            Add-Check -Id $target.id -Kind "targeted" -Status "blocked" -ExitCode $null -Reason $_.Exception.Message
        }
    }
}

try {
    $projectOutput = @(& pwsh -NoProfile -File $conformancePath -Profile $profile -Json 2>&1 | ForEach-Object { $_.ToString() })
    $projectExitCode = $LASTEXITCODE
    $projectText = $projectOutput -join "`n"
    $projectResult = $projectText | ConvertFrom-Json -Depth 30
    $projectVerdict = [string]$projectResult.verdict
    $projectProfile = [string]$projectResult.profile
    if ($projectProfile -ne $profile -or $projectVerdict -notin @("pass", "fail")) {
        throw "project conformance returned a mismatched or malformed result"
    }
    if ($projectExitCode -eq 0 -and $projectVerdict -eq "pass") {
        Add-Check -Id "project-$profile" -Kind "project-conformance" -Status "pass" -ExitCode $projectExitCode -Reason "$profile project conformance passed" -OutputTail $projectText
    } elseif ($projectExitCode -eq 1 -and $projectVerdict -eq "fail") {
        Add-Check -Id "project-$profile" -Kind "project-conformance" -Status "fail" -ExitCode $projectExitCode -Reason "$profile project conformance failed" -OutputTail $projectText
    } else {
        Add-Check -Id "project-$profile" -Kind "project-conformance" -Status "blocked" -ExitCode $projectExitCode -Reason "project conformance exit code and verdict disagree" -OutputTail $projectText
    }
} catch {
    Add-Check -Id "project-$profile" -Kind "project-conformance" -Status "blocked" -ExitCode $null -Reason $_.Exception.Message
}

Complete-Result
