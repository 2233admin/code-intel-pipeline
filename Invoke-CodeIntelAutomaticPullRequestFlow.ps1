#requires -Version 7.2

$implementation = Join-Path (Join-Path $PSScriptRoot "tools") "Invoke-CodeIntelAutomaticPullRequestFlow.ps1"
if (-not (Test-Path -LiteralPath $implementation -PathType Leaf)) {
    throw "Automatic PR flow implementation is missing: $implementation"
}
& $implementation @args
exit $LASTEXITCODE
