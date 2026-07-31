param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,
    [switch]$Auto,
    [switch]$Quiet,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($Json -and $IsWindows) {
    # -Json is a machine contract parsed as UTF-8 by the capability adapter.
    # Windows pwsh encodes redirected stdout with the system codepage (GBK on
    # zh-CN hosts), so pin it. Unix pwsh is already UTF-8, and touching
    # [Console]::OutputEncoding there can crash the runtime (macOS assembly
    # load failure), so the pin is Windows-only. BOM-less explicitly.
    [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
}

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
