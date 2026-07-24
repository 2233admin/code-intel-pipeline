param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$gate = Join-Path $root "scripts/tests/Test-Python314PonCompatibility.ps1"
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("python314-pon-compat-" + [guid]::NewGuid().ToString("N"))

function Find-CPython314Executable {
    $candidates = @(
        @{ command = "py"; prefix = @("-3.14") },
        @{ command = "python3.14"; prefix = @() },
        @{ command = "python"; prefix = @() }
    )
    foreach ($candidate in $candidates) {
        if ($null -eq (Get-Command $candidate.command -ErrorAction SilentlyContinue)) { continue }
        try {
            $lines = @(& $candidate.command @($candidate.prefix) -c "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}'); print(sys.executable)" 2>$null)
            if ($LASTEXITCODE -eq 0 -and $lines.Count -ge 2 -and [string]$lines[0] -eq "3.14") {
                return [string]$lines[1]
            }
        } catch { continue }
    }
    throw "CPython 3.14 executable is required for this test."
}

function Invoke-Gate {
    param([string]$Profile, [string]$PonCommand = "", [string]$Shim = "")
    $arguments = @("-NoProfile", "-File", $gate, "-Profile", $Profile, "-Json")
    if (-not [string]::IsNullOrWhiteSpace($PonCommand)) {
        $arguments += @("-PonCommand", $PonCommand, "-PonPrefixArgs", $Shim)
    }
    $output = & pwsh @arguments 2>&1
    $exitCode = $LASTEXITCODE
    $text = ($output | ForEach-Object { $_.ToString() }) -join "`n"
    [pscustomobject]@{ exitCode = $exitCode; result = ($text | ConvertFrom-Json) }
}

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null
    $python = Find-CPython314Executable

    $development = Invoke-Gate "development"
    if ($development.exitCode -ne 0 -or [string]$development.result.verdict -ne "pass" -or [string]$development.result.ponStatus -ne "unavailable") {
        throw "Development profile must pass on CPython 3.14 while reporting missing Pon."
    }

    $missingPon = Invoke-Gate "pon-candidate"
    if ($missingPon.exitCode -ne 1 -or @($missingPon.result.failedGateIds) -notcontains "pon-availability") {
        throw "Pon candidate must fail closed when Pon is unavailable."
    }

    $shim = Join-Path $temp "pon_shim.py"
    @'
import subprocess
import sys

if len(sys.argv) < 3 or sys.argv[1] != "run":
    raise SystemExit(64)
result = subprocess.run([sys.executable, *sys.argv[2:]], capture_output=True)
sys.stdout.buffer.write(result.stdout)
sys.stderr.buffer.write(result.stderr)
raise SystemExit(result.returncode)
'@ | Set-Content -LiteralPath $shim -Encoding UTF8

    $matching = Invoke-Gate "pon-candidate" $python $shim
    if ($matching.exitCode -ne 0 -or [string]$matching.result.ponStatus -ne "pass") {
        throw "A behaviorally matching test backend must pass Pon parity."
    }

    $badShim = Join-Path $temp "bad_pon_shim.py"
    @'
import sys

print("deliberate divergence")
raise SystemExit(0)
'@ | Set-Content -LiteralPath $badShim -Encoding UTF8

    $diverged = Invoke-Gate "pon-candidate" $python $badShim
    if ($diverged.exitCode -ne 1 -or @($diverged.result.failedGateIds) -notcontains "pon-parity" -or [string]$diverged.result.ponStatus -ne "diverged") {
        throw "A divergent backend must fail the Pon parity gate."
    }

    Write-Host "PASS Python 3.14/Pon lane: development, missing backend, matching backend, and divergence paths verified"
} finally {
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
