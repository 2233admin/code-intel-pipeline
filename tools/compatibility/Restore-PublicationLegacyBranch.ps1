[CmdletBinding(DefaultParameterSetName="Rehearsal")]
param(
 [Parameter(Mandatory=$true,ParameterSetName="Apply")][string]$TargetPath,
 [Parameter(Mandatory=$true,ParameterSetName="Rehearsal")][string]$RehearsalRoot,
 [string]$RepoRoot=(Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),[string]$SourceRevision="working-tree-bounded-source"
)
Set-StrictMode -Version Latest;$ErrorActionPreference="Stop"
$live=Join-Path $RepoRoot "run-code-intel.ps1";$source=[IO.File]::ReadAllText($live);$liveHash=(Get-FileHash $live -Algorithm SHA256).Hash.ToLowerInvariant()
$initial='(?s)\$stagingNonce = \[guid\]::NewGuid\(\)\.ToString\("N"\)\.Substring\(0, 12\).*?New-Item -ItemType Directory -Force -Path \$runDir \| Out-Null\r?\n'
$final='(?s)# A run is authoritative only after every materialized view has been written,.*?Set-Content -LiteralPath \(Join-Path \$runDir "run-complete\.json"\) -Encoding UTF8\r?\n'
$sourceInitial=[regex]::Match($source,$initial);$sourceFinal=[regex]::Match($source,$final);if(-not$sourceInitial.Success-or-not$sourceFinal.Success){throw "current snapshot lacks bounded legacy publication branch"}
$deleted=[regex]::Replace([regex]::Replace($source,$initial,"",1),$final,"",1)
if($PSCmdlet.ParameterSetName-eq"Rehearsal"){
 if(Test-Path $RehearsalRoot){throw "rehearsal root must be exclusive"};New-Item -ItemType Directory $RehearsalRoot|Out-Null;$TargetPath=Join-Path $RehearsalRoot "run-code-intel.ps1"
 [IO.File]::WriteAllText($TargetPath,$deleted,[Text.UTF8Encoding]::new($false))
}
$target=[IO.Path]::GetFullPath($TargetPath);$text=[IO.File]::ReadAllText($target);if($text-ne$deleted){throw "rollback target is not the exact bounded publication deletion result"}
[IO.File]::WriteAllText($target,$source,[Text.UTF8Encoding]::new($false));$targetHash=(Get-FileHash $target -Algorithm SHA256).Hash.ToLowerInvariant();if($targetHash-ne$liveHash){throw "rollback did not restore the exact current facade"};if((Get-FileHash $live -Algorithm SHA256).Hash.ToLowerInvariant()-ne$liveHash){throw "rehearsal changed live facade"}
[ordered]@{schema="code-intel-compatibility-rollback-rehearsal.v1";branchId="run-code-intel.publication.legacy-staging-marker";target=$target;sourceRevision=$SourceRevision;rehearsal=($PSCmdlet.ParameterSetName-eq"Rehearsal");changedFiles=@($target);exactReplay=$true;sourceSha256=$liveHash;targetSha256=$targetHash;replacementChanged=$false;indexTraversalChanged=$false;liveFacadeChanged=$false}|ConvertTo-Json -Compress
