[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$RehearsalRoot,
    [string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (Test-Path -LiteralPath $RehearsalRoot) {
    throw "rollback rehearsal root must be exclusive: $RehearsalRoot"
}

$sourcePath = Join-Path $RepoRoot "run-code-intel.ps1"
$source = [IO.File]::ReadAllText($sourcePath).Replace("`r`n", "`n").Replace("`r", "`n")
$pattern = '(?s)\n\$codeNexusLiteTool = Join-Path \$PSScriptRoot "Invoke-CodeNexusLite\.ps1".*?(?=\n\$reportPath = Join-Path \$runDir "report\.json")'
$match = [regex]::Match($source, $pattern)
if (-not $match.Success) { throw "live CodeNexus direct branch marker is absent" }
if ([regex]::Matches($source, $pattern).Count -ne 1) { throw "CodeNexus direct branch is ambiguous" }

$withoutLegacy = $source.Remove($match.Index, $match.Length)
$restored = $withoutLegacy.Insert($match.Index, $match.Value)
if ($restored -cne $source) { throw "rollback rehearsal did not reproduce the exact facade bytes" }

$null = New-Item -ItemType Directory -Path $RehearsalRoot
$target = Join-Path $RehearsalRoot "run-code-intel.ps1"
[IO.File]::WriteAllText($target, $restored, [Text.UTF8Encoding]::new($false))

[ordered]@{
    ok = $true
    schema = "code-intel-codenexus-direct-rollback-rehearsal.v1"
    branchId = "run-code-intel.codenexus-lite.direct"
    source = "run-code-intel.ps1"
    target = $target
    exactReplay = $true
    unrelatedBranchesChanged = $false
} | ConvertTo-Json -Depth 5 -Compress
