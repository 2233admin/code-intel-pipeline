#requires -Version 7.2

param(
    [string]$Provider = "",
    [string]$Model = "",
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$probe = Join-Path $PSScriptRoot "Invoke-RepowiseProviderProbe.ps1"
if (-not (Test-Path -LiteralPath $probe -PathType Leaf)) {
    throw "Repowise provider probe is missing: $probe"
}
& $probe -Provider $Provider -Model $Model -Json:$Json
exit $LASTEXITCODE
