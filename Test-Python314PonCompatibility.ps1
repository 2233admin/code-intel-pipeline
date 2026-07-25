#requires -Version 7.2

param(
    [ValidateSet("development", "pon-candidate")]
    [string]$Profile = "development",

    [string]$Policy = (Join-Path $PSScriptRoot "orchestration\python314-pon-development-policy.v1.json"),

    [string]$CPythonCommand = "",

    [string[]]$CPythonPrefixArgs = @(),

    [string]$PonCommand = "",

    [string[]]$PonPrefixArgs = @(),

    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = [System.IO.Path]::GetFullPath($PSScriptRoot)
$gates = [System.Collections.Generic.List[object]]::new()
$caseResults = [System.Collections.Generic.List[object]]::new()

function Add-Gate {
    param([string]$Id, [bool]$Passed, [string]$Detail)
    $gates.Add([pscustomobject]@{ id = $Id; passed = $Passed; detail = $Detail })
}

function Resolve-RepoFile {
    param([string]$RelativePath)
    if ([string]::IsNullOrWhiteSpace($RelativePath) -or [System.IO.Path]::IsPathRooted($RelativePath)) {
        throw "path must be repository-relative: $RelativePath"
    }
    $prefix = [System.IO.Path]::TrimEndingDirectorySeparator($root) + [System.IO.Path]::DirectorySeparatorChar
    $resolved = [System.IO.Path]::GetFullPath((Join-Path $root $RelativePath))
    if (-not $resolved.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "path escapes repository: $RelativePath"
    }
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "required file is missing: $RelativePath"
    }
    return $resolved
}

function Resolve-Executable {
    param([string]$Command)
    if ([string]::IsNullOrWhiteSpace($Command)) { return $null }
    if ([System.IO.Path]::IsPathRooted($Command)) {
        if (Test-Path -LiteralPath $Command -PathType Leaf) { return [System.IO.Path]::GetFullPath($Command) }
        return $null
    }
    $resolved = Get-Command $Command -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $resolved) { return $null }
    return [string]$resolved.Source
}

function Invoke-CapturedProcess {
    param([string]$Executable, [string[]]$Arguments)
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.WorkingDirectory = $root
    foreach ($argument in @($Arguments)) { $startInfo.ArgumentList.Add([string]$argument) }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) { throw "process did not start: $Executable" }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            $process.Kill($true)
            throw "process timed out after 30 seconds: $Executable"
        }
        return [pscustomobject]@{
            exitCode = $process.ExitCode
            stdout = $stdoutTask.GetAwaiter().GetResult()
            stderr = $stderrTask.GetAwaiter().GetResult()
        }
    } finally {
        $process.Dispose()
    }
}

function Normalize-Newlines {
    param([string]$Text)
    return $Text.Replace("`r`n", "`n").Replace("`r", "`n")
}

function Find-CPython314 {
    $candidates = [System.Collections.Generic.List[object]]::new()
    if (-not [string]::IsNullOrWhiteSpace($CPythonCommand)) {
        $candidates.Add([pscustomobject]@{ command = $CPythonCommand; prefix = @($CPythonPrefixArgs) })
    } else {
        $candidates.Add([pscustomobject]@{ command = "py"; prefix = @("-3.14") })
        $candidates.Add([pscustomobject]@{ command = "python3.14"; prefix = @() })
        $candidates.Add([pscustomobject]@{ command = "python"; prefix = @() })
    }
    foreach ($candidate in $candidates) {
        $executable = Resolve-Executable ([string]$candidate.command)
        if ($null -eq $executable) { continue }
        try {
            $probe = Invoke-CapturedProcess $executable (@($candidate.prefix) + @("-c", "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')"))
            if ($probe.exitCode -eq 0 -and $probe.stdout.Trim() -eq "3.14") {
                return [pscustomobject]@{ executable = $executable; prefix = @($candidate.prefix); version = $probe.stdout.Trim() }
            }
        } catch { continue }
    }
    return $null
}

function Complete-Result {
    param([int]$MalformedExit = 0, [string]$CPython = "", [string]$PonStatus = "not_checked")
    $failed = @($gates | Where-Object { -not $_.passed })
    $result = [ordered]@{
        schema = "code-intel-python314-pon-compatibility-result.v1"
        profile = $Profile
        verdict = if ($failed.Count -eq 0 -and $MalformedExit -eq 0) { "pass" } else { "fail" }
        cpython = $CPython
        ponStatus = $PonStatus
        gates = @($gates)
        failedGateIds = @($failed | ForEach-Object id)
        cases = @($caseResults)
    }
    if ($Json) { $result | ConvertTo-Json -Depth 12 }
    else {
        Write-Host "Python 3.14 / Pon compatibility: $($result.verdict) profile=$Profile pon=$PonStatus"
        foreach ($gate in $gates) { Write-Host "$(if ($gate.passed) { 'PASS' } else { 'FAIL' }) $($gate.id): $($gate.detail)" }
    }
    if ($MalformedExit -ne 0) { exit $MalformedExit }
    if ($failed.Count -gt 0) { exit 1 }
    exit 0
}

try {
    $policyDocument = Get-Content -Raw -LiteralPath $Policy | ConvertFrom-Json -Depth 30
    $profileProperty = $policyDocument.profiles.PSObject.Properties[$Profile]
    $shapePass = [string]$policyDocument.schema -eq "code-intel-python314-pon-development-policy.v1" -and
        $null -ne $profileProperty -and
        [int]$policyDocument.authority.requiredMajor -eq 3 -and
        [int]$policyDocument.authority.requiredMinor -eq 14 -and
        [string]$policyDocument.sourceMethod.revision -match '^[0-9a-f]{40}$'
    Add-Gate "input-shape" $shapePass "pinned policy requires CPython 3.14 and a known profile"
    if (-not $shapePass) { Complete-Result -MalformedExit 2 }
    $manifestPath = Resolve-RepoFile ([string]$policyDocument.corpus)
    $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json -Depth 20
    if ([string]$manifest.schema -ne "code-intel-python314-compat-corpus.v1" -or @($manifest.cases).Count -eq 0) {
        throw "invalid or empty Python 3.14 corpus"
    }
} catch {
    if (@($gates | Where-Object id -eq "input-shape").Count -eq 0) { Add-Gate "input-shape" $false $_.Exception.Message }
    Complete-Result -MalformedExit 2
}

$cpython = Find-CPython314
$cpythonPass = $null -ne $cpython
Add-Gate "cpython314-availability" $cpythonPass "resolved=$(if ($cpythonPass) { $cpython.executable } else { 'none' })"
if (-not $cpythonPass) { Complete-Result -CPython "" -PonStatus "not_checked" }
$cpythonIdentity = "$($cpython.executable) $($cpython.prefix -join ' ')".Trim()

$compileFiles = @($policyDocument.projectPythonFiles | ForEach-Object { Resolve-RepoFile ([string]$_) })
$compileProgram = "import pathlib, sys; [compile(pathlib.Path(path).read_text(encoding='utf-8'), path, 'exec') for path in sys.argv[1:]]"
$compile = Invoke-CapturedProcess $cpython.executable (@($cpython.prefix) + @("-c", $compileProgram) + $compileFiles)
$compilePass = -not [bool]$profileProperty.Value.requireProjectCompile -or $compile.exitCode -eq 0
Add-Gate "project-python-compile" $compilePass "files=$($compileFiles.Count), exit=$($compile.exitCode)"

$corpusPass = $true
foreach ($case in @($manifest.cases)) {
    $casePath = Resolve-RepoFile ([string]$case.path)
    $actual = Invoke-CapturedProcess $cpython.executable (@($cpython.prefix) + @($casePath))
    $passed = $actual.exitCode -eq [int]$case.expectedExitCode -and
        (Normalize-Newlines $actual.stdout) -ceq [string]$case.expectedStdout -and
        (Normalize-Newlines $actual.stderr) -ceq [string]$case.expectedStderr
    if (-not $passed) { $corpusPass = $false }
    $caseResults.Add([pscustomobject]@{
        id = [string]$case.id
        ponRequired = [bool]$case.ponRequired
        cpythonPassed = $passed
        ponPassed = $null
    })
}
Add-Gate "cpython314-corpus" $corpusPass "cases=$(@($manifest.cases).Count), passed=$(@($caseResults | Where-Object cpythonPassed).Count)"

$ponName = if ([string]::IsNullOrWhiteSpace($PonCommand)) { [string]$policyDocument.pon.defaultCommand } else { $PonCommand }
$ponExecutable = Resolve-Executable $ponName
$requirePon = [bool]$profileProperty.Value.requirePon
$ponAvailable = $null -ne $ponExecutable
$availabilityPass = -not $requirePon -or $ponAvailable
Add-Gate "pon-availability" $availabilityPass "required=$requirePon, resolved=$(if ($ponAvailable) { $ponExecutable } else { 'none' })"

$ponStatus = if ($ponAvailable) { "available" } else { "unavailable" }
$ponParityPass = $true
if ($ponAvailable -and [bool]$profileProperty.Value.runPonWhenAvailable) {
    foreach ($case in @($manifest.cases | Where-Object ponRequired)) {
        $casePath = Resolve-RepoFile ([string]$case.path)
        $cpythonResult = Invoke-CapturedProcess $cpython.executable (@($cpython.prefix) + @($casePath))
        $ponArgs = @($PonPrefixArgs) + @($policyDocument.pon.scriptArgsBeforePath) + @($casePath)
        $ponResult = Invoke-CapturedProcess $ponExecutable $ponArgs
        $passed = $ponResult.exitCode -eq $cpythonResult.exitCode -and
            $ponResult.stdout -ceq $cpythonResult.stdout -and
            $ponResult.stderr -ceq $cpythonResult.stderr
        if (-not $passed) { $ponParityPass = $false }
        $existing = $caseResults | Where-Object id -eq ([string]$case.id) | Select-Object -First 1
        $existing.ponPassed = $passed
    }
    $ponStatus = if ($ponParityPass) { "pass" } else { "diverged" }
}
if (-not $ponAvailable -and -not $requirePon) { $ponParityPass = $true }
if (-not $ponAvailable -and $requirePon) { $ponParityPass = $false }
Add-Gate "pon-parity" $ponParityPass "status=$ponStatus, requiredCases=$(@($manifest.cases | Where-Object ponRequired).Count)"

Complete-Result -CPython $cpythonIdentity -PonStatus $ponStatus
