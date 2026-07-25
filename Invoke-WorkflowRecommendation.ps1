param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,
    [switch]$Auto,
    [switch]$Quiet,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$atom = Join-Path $PSScriptRoot "OpenSpec-Detector.ps1"
if (-not (Test-Path -LiteralPath $atom -PathType Leaf)) {
    throw "Workflow recommendation atom not found: $atom"
}

if ($Quiet) {
    $result = & $atom -RepoPath $RepoPath -Auto:$Auto -Quiet 6>$null
}
else {
    $result = & $atom -RepoPath $RepoPath -Auto:$Auto
}
if ($Json) {
    return $result | ConvertTo-Json -Depth 30 -Compress
}
return $result
