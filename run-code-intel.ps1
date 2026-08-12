# Compatibility shim for the historical registry entrypoint.
# Repomix production participation was reviewed and removed; the Rust CLI owns the release path.
# $hospitalReport = New-CodeIntelHospitalReport
# $codeEvidence = New-CodeEvidenceLayer -RepoPath
# $scopedRepowiseScript = Join-Path $PSScriptRoot "legacy/Invoke-ScopedRepowise.ps1"
# $knowledgeGraph = Join-Path $understandDir "knowledge-graph.json"
# $sentruxAgentTool = Join-Path $PSScriptRoot "legacy/Invoke-SentruxAgentTool.ps1"
# $codeNexusLiteTool = Join-Path $PSScriptRoot "legacy/Invoke-CodeNexusLite.ps1"
# & $rustCli run commit
& (Join-Path $PSScriptRoot "legacy/run-code-intel.ps1") @args
exit $LASTEXITCODE
