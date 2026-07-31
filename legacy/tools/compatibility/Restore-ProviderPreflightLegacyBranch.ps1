[CmdletBinding(DefaultParameterSetName = "Rehearsal")]
param(
    [Parameter(Mandatory = $true, ParameterSetName = "Apply")][string]$TargetPath,
    [Parameter(Mandatory = $true, ParameterSetName = "Rehearsal")][string]$RehearsalRoot,
    [string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),
    [string]$SourceRevision = "ca9334aa8eb8df3be7e10c5547069f03645cabe2"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$livePath = Join-Path $RepoRoot "run-code-intel.ps1"
if (-not (Test-Path -LiteralPath $livePath -PathType Leaf)) { throw "run-code-intel.ps1 is missing" }
$liveHashBefore = (Get-FileHash -LiteralPath $livePath -Algorithm SHA256).Hash

if ($PSCmdlet.ParameterSetName -eq "Rehearsal") {
    if (Test-Path -LiteralPath $RehearsalRoot) { throw "rollback rehearsal root must be exclusive: $RehearsalRoot" }
    $null = New-Item -ItemType Directory -Path $RehearsalRoot
    $TargetPath = Join-Path $RehearsalRoot "run-code-intel.ps1"
    Copy-Item -LiteralPath $livePath -Destination $TargetPath
}
$resolvedTarget = [IO.Path]::GetFullPath($TargetPath)
if (-not (Test-Path -LiteralPath $resolvedTarget -PathType Leaf)) { throw "rollback target is missing: $resolvedTarget" }
if ($PSCmdlet.ParameterSetName -eq "Rehearsal" -and $resolvedTarget -eq [IO.Path]::GetFullPath($livePath)) {
    throw "rollback rehearsal cannot target the live facade"
}

$historical = @(& git -C $RepoRoot show "$SourceRevision`:run-code-intel.ps1") -join "`n"
if ($LASTEXITCODE -ne 0) { throw "cannot load historical provider-preflight source" }
$legacyPattern = '(?s)if \(\$RepowiseDocs -and -not \$SkipRepowise\) \{\n    \$providerPreflightScript = Join-Path \$PSScriptRoot "test-code-intel-provider\.ps1".*?\n\}'
$currentPattern = '(?s)if \(\$RepowiseDocs -and -not \$SkipRepowise\) \{\r?\n    \$providerProbeScript = Join-Path \$PSScriptRoot "Invoke-RepowiseProviderProbe\.ps1".*?\r?\n\}'
$legacyBlock = [regex]::Match($historical, $legacyPattern)
$current = [IO.File]::ReadAllText($resolvedTarget)
if (-not $legacyBlock.Success) { throw "bounded historical test-wrapper branch is absent" }
if (-not [regex]::IsMatch($current, $currentPattern)) { throw "target does not contain the current production probe block" }
$restored = [regex]::Replace($current, $currentPattern, [Text.RegularExpressions.MatchEvaluator]{ param($m) $legacyBlock.Value }, 1)
if ($restored -notmatch 'test-code-intel-provider\.ps1' -or $restored -match 'Invoke-RepowiseProviderProbe\.ps1') {
    throw "rollback copy did not restore exactly the historical direct test-wrapper branch"
}
[IO.File]::WriteAllText($resolvedTarget, $restored, [Text.UTF8Encoding]::new($false))
if ($PSCmdlet.ParameterSetName -eq "Rehearsal" -and (Get-FileHash -LiteralPath $livePath -Algorithm SHA256).Hash -ne $liveHashBefore) {
    throw "rollback rehearsal changed the live facade"
}
[ordered]@{
    schema = "code-intel-compatibility-rollback-rehearsal.v1"
    branchId = "run-code-intel.provider-preflight.test-wrapper"
    target = $resolvedTarget
    sourceRevision = $SourceRevision
    rehearsal = ($PSCmdlet.ParameterSetName -eq "Rehearsal")
    changedFiles = @($resolvedTarget)
    replacementChanged = $false
    liveFacadeChanged = $false
} | ConvertTo-Json -Depth 5 -Compress
