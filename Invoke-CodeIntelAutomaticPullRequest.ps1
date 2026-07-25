#requires -Version 7.2

$implementation = Join-Path (Join-Path $PSScriptRoot "tools") "Invoke-CodeIntelAutomaticPullRequest.ps1"
if (-not (Test-Path -LiteralPath $implementation -PathType Leaf)) {
    throw "Automatic PR implementation is missing: $implementation"
}
& $implementation @args
exit $LASTEXITCODE
