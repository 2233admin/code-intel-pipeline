[CmdletBinding(DefaultParameterSetName = "Rehearsal")]
param(
    [Parameter(Mandatory = $true, ParameterSetName = "Apply")]
    [string]$TargetPath,

    [Parameter(Mandatory = $true, ParameterSetName = "Rehearsal")]
    [string]$RehearsalRoot,

    [string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),
    # The legacy inline recommender was removed by 42de063, the same commit that
    # created this packet, so HEAD has never contained the markers this script
    # restores while New-RecommenderRetirementPacket.ps1 simultaneously requires
    # them absent from the live facade. Pin the last revision that has them, the
    # way Restore-ProviderPreflightLegacyBranch.ps1 already does.
    [string]$SourceRevision = "e6e73e4f720ab2ae2bca531a07ed638f55fecd1d"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Text.UTF8Encoding]::new($false)


$runPath = Join-Path $RepoRoot "run-code-intel.ps1"
if (-not (Test-Path -LiteralPath $runPath -PathType Leaf)) {
    throw "run-code-intel.ps1 is missing from repository root: $RepoRoot"
}

if ($PSCmdlet.ParameterSetName -eq "Rehearsal") {
    if (Test-Path -LiteralPath $RehearsalRoot) {
        throw "rollback rehearsal root must not already exist: $RehearsalRoot"
    }
    $null = New-Item -ItemType Directory -Path $RehearsalRoot
    $TargetPath = Join-Path $RehearsalRoot "run-code-intel.ps1"
    Copy-Item -LiteralPath $runPath -Destination $TargetPath
}

$resolvedTarget = [IO.Path]::GetFullPath($TargetPath)
if (-not (Test-Path -LiteralPath $resolvedTarget -PathType Leaf)) {
    throw "rollback target is missing: $resolvedTarget"
}

# `git show <rev>:<path>` is always repository-root-relative, and the archive
# move relocated run-code-intel.ps1. Revisions at or after that move carry it at
# legacy/, earlier ones at the root, so try both rather than pinning either:
# $SourceRevision defaults to HEAD but is caller-overridable to any revision.
$legacyCandidates = @("legacy/run-code-intel.ps1", "run-code-intel.ps1")
$legacyLines = @()
foreach ($candidate in $legacyCandidates) {
    $attempt = @(& git -C $RepoRoot show "$SourceRevision`:$candidate" 2>$null)
    if ($LASTEXITCODE -eq 0 -and $attempt.Count -gt 0) { $legacyLines = $attempt; break }
}
if ($legacyLines.Count -eq 0) {
    throw "cannot load legacy recommender source from $SourceRevision at any of: $($legacyCandidates -join ', ')"
}

$legacy = $legacyLines -join "`n"
$current = [IO.File]::ReadAllText($resolvedTarget)

$legacyFunctionsPattern = '(?s)# ============ 三栈工作流推荐器 \(Workflow Stack Recommender\) ============.*?(?=\r?\nfunction Get-JsonProperty)'
$legacyInvocationPattern = '(?s)# Three-stack workflow recommender \(matt-flow / gstack / spec-driven\)\..*?(?=\r?\nif \(-not \$toolState\.rg\))'
$currentFunctionsPattern = '(?s)# Workflow recommendations are owned by the standalone advisory atom in OpenSpec-Detector\.ps1\..*?(?=\r?\nfunction Get-JsonProperty)'
$currentInvocationPattern = '(?s)# Historical options now map to the standalone advisory atom: Skip disables it and.*?(?=\r?\nif \(-not \$toolState\.rg\))'

$legacyFunctions = [regex]::Match($legacy, $legacyFunctionsPattern)
$legacyInvocation = [regex]::Match($legacy, $legacyInvocationPattern)
if (-not $legacyFunctions.Success -or -not $legacyInvocation.Success) {
    throw "legacy recommender markers are absent from $SourceRevision`:run-code-intel.ps1"
}
if (-not [regex]::IsMatch($current, $currentFunctionsPattern) -or
    -not [regex]::IsMatch($current, $currentInvocationPattern)) {
    throw "target does not contain the retired recommender adapter markers"
}

$restored = [regex]::Replace($current, $currentFunctionsPattern, [System.Text.RegularExpressions.MatchEvaluator]{ param($match) $legacyFunctions.Value }, 1)
$restored = [regex]::Replace($restored, $currentInvocationPattern, [System.Text.RegularExpressions.MatchEvaluator]{ param($match) $legacyInvocation.Value }, 1)
if ($restored -notmatch 'function Invoke-WorkflowStackDetector' -or
    $restored -notmatch 'Invoke-WorkflowStackDetector -RepoPath \$repoPath -AutoMode \$AutoOpenSpec') {
    throw "restored target does not contain the bounded legacy recommender branch"
}

[IO.File]::WriteAllText($resolvedTarget, $restored, [Text.UTF8Encoding]::new($false))
[ordered]@{
    schema = "code-intel-compatibility-rollback-rehearsal.v1"
    branchId = "run-code-intel.workflow-recommender.inline"
    target = $resolvedTarget
    sourceRevision = $SourceRevision
    rehearsal = ($PSCmdlet.ParameterSetName -eq "Rehearsal")
    changedFiles = @($resolvedTarget)
    replacementChanged = $false
} | ConvertTo-Json -Depth 5 -Compress
