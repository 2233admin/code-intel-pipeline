[CmdletBinding()]
param([string]$RepoRoot=(Split-Path (Split-Path $PSScriptRoot -Parent) -Parent))
Set-StrictMode -Version Latest
$ErrorActionPreference="Stop"
$run=Get-Content (Join-Path $RepoRoot "run-code-intel.ps1") -Raw
$initial='(?s)\$stagingNonce = \[guid\]::NewGuid\(\)\.ToString\("N"\)\.Substring\(0, 12\).*?New-Item -ItemType Directory -Force -Path \$runDir \| Out-Null'
$final='(?s)# A run is authoritative only after every materialized view has been written,.*?Set-Content -LiteralPath \(Join-Path \$runDir "run-complete\.json"\) -Encoding UTF8'
$dag='(?s)if \(\$DagCoordinate\) \{.*?run dag-coordinate.*?return\s*\}'
$legacyInitial=@([regex]::Matches($run,$initial)).Count
$legacyFinal=@([regex]::Matches($run,$final)).Count
$a09Only=@([regex]::Matches($run,$dag)).Count
$a07Facade=@([regex]::Matches($run,'& \$rustCli run commit')).Count
$routeConnected=($run -match '(?s)if \(\$DagCoordinate\).*?run dag-coordinate.*?run commit')
# Architecture convergence retires the authority-claiming final marker first
# (canonical A07 is Rust-only) while the staging allocation hunk may remain as
# a compatibility path. E05 therefore admits three live shapes:
#   (1,1) frozen two-hunk branch still present
#   (1,0) marker removed, staging still present (current architecture state)
#   (0,0) both hunks gone (full historical absence)
if($legacyInitial -eq 1 -and $legacyFinal -eq 1){
    $branchAbsent = $false
    $legacyHunks = 2
    $markerRetired = $false
} elseif($legacyInitial -eq 1 -and $legacyFinal -eq 0){
    $branchAbsent = $false
    $legacyHunks = 1
    $markerRetired = $true
} elseif($legacyInitial -eq 0 -and $legacyFinal -eq 0){
    $branchAbsent = $true
    $legacyHunks = 0
    $markerRetired = $true
} else {
    throw "E05 publication boundary is in an unexpected partial state (initial=$legacyInitial final=$legacyFinal)"
}
if($a09Only-ne1-or$a07Facade-ne1){throw "A09/A07 public facade declarations changed unexpectedly"}
if($run -match 'update-code-intel-index\.ps1'){throw "run-code-intel publication branch unexpectedly owns index traversal"}
[ordered]@{
 schema="code-intel-publication-retirement-boundary.v1";ok=$true;branchId="run-code-intel.publication.legacy-staging-marker"
 affectedFiles=@("run-code-intel.ps1");legacyHunks=$legacyHunks;branchAbsent=$branchAbsent;markerRetired=$markerRetired
 a09FacadeRoutes=$a09Only;a07FacadeRoutes=$a07Facade
 a09ToA07Connected=$routeConnected;facadeFailureInjectionAvailable=$false;indexTraversalOwned=$false
 deletionExecuted=$false;retired=$false
}|ConvertTo-Json -Compress
