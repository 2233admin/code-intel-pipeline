#requires -Version 7.2

# Thin forwarder. The tool/runtime health probe this script used to implement
# now lives in Rust (`code-intel doctor bootstrap`, crates/code-intel-cli/src/
# doctor_bootstrap.rs) — see issue #48 (T3). Kept as a compatibility entry
# point for installers and rollback paths; new call sites should invoke the
# binary directly.

param(
    [string]$Config = "",
    [string]$Repo = "",
    [string]$RepoPath = "",
    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto",
    [switch]$RequireRepowise,
    [switch]$RequireUnderstand,
    [switch]$Json
)

if (-not $PSBoundParameters.ContainsKey("RequireRepowise")) { $RequireRepowise = $true }

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$pipelineRoot = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$exe = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
# Repo-local builds win over an installed copy on PATH: this script probes
# *this* checkout, so a globally installed older binary must not answer for it.
$candidates = @(
    (Join-Path $pipelineRoot "target/release/$exe"),
    (Join-Path $pipelineRoot "target/debug/$exe"),
    (Get-Command "code-intel" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source)
)
$cli = @($candidates | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and (Test-Path -LiteralPath $_ -PathType Leaf) }) | Select-Object -First 1

if ($null -eq $cli) {
    # A missing binary is itself a doctor finding, not a crash: installers
    # parse this JSON and must keep reporting the rest of their own checks.
    $absent = [ordered]@{
        schema = "code-intel-doctor-bootstrap-observation.v1"
        authority = "observation_only"
        source = "forwarder"
        ok = $false
        missing = @("code-intel binary")
    }
    if ($Json) { $absent | ConvertTo-Json -Depth 4 } else { Write-Host "Code intel doctor: missing code-intel binary" }
    exit 1
}

$forward = @("doctor", "bootstrap", "--pipeline-root", $pipelineRoot, "--platform", $Platform)
if (-not [string]::IsNullOrWhiteSpace($Config)) { $forward += @("--config", $Config) }
if (-not [string]::IsNullOrWhiteSpace($Repo)) { $forward += @("--repo", $Repo) }
if (-not [string]::IsNullOrWhiteSpace($RepoPath)) { $forward += @("--repo-path", $RepoPath) }
$forward += if ($RequireRepowise) { "--require-repowise" } else { "--no-require-repowise" }
if ($RequireUnderstand) { $forward += "--require-understand" }
if ($Json) { $forward += "--json" }

& $cli @forward
exit $LASTEXITCODE
