[CmdletBinding()]param([Parameter(Mandatory=$true)][string]$RehearsalRoot,[string]$RepoRoot=(Split-Path (Split-Path $PSScriptRoot -Parent) -Parent))
Set-StrictMode -Version Latest;$ErrorActionPreference="Stop";if(Test-Path -LiteralPath $RehearsalRoot){throw "rehearsal root must be exclusive"}
$source=[IO.File]::ReadAllText((Join-Path $RepoRoot "invoke-code-intel.ps1")).Replace("`r`n","`n").Replace("`r","`n")
$patterns=@(
 '(?m)^\$doctor = Join-Path \$root "check-code-intel-tools\.ps1"$',
 '(?sm)^    Write-Host "Code intel invoke: doctor \$label".*?^    \}(?=\n\n    Write-Host "Code intel invoke: pipeline \$label")',
 '(?sm)^if \(-not \(Test-Path -LiteralPath \$doctor -PathType Leaf\)\) \{\n    throw "Doctor script missing: \$doctor"\n\}'
)
$matches=@($patterns|ForEach-Object{$all=[regex]::Matches($source,$_);if($all.Count-ne1){throw "doctor wrapper marker absent or ambiguous: $_"};$all[0]}|Sort-Object Index)
$deleted=$source;foreach($m in @($matches|Sort-Object Index -Descending)){$deleted=$deleted.Remove($m.Index,$m.Length)}
$restored=$deleted;foreach($m in $matches){$restored=$restored.Insert($m.Index,$m.Value)};if($restored-cne$source){throw "doctor wrapper rollback did not exactly replay normalized bytes"}
New-Item -ItemType Directory -Path $RehearsalRoot|Out-Null;$target=Join-Path $RehearsalRoot "invoke-code-intel.ps1";[IO.File]::WriteAllText($target,$restored,[Text.UTF8Encoding]::new($false))
[ordered]@{schema="code-intel-doctor-wrapper-rollback-rehearsal.v1";branchId="invoke-code-intel.doctor.direct-production";exactReplay=$true;segmentCount=3;target=$target;bootstrapChanged=$false;otherWrapperBranchesChanged=$false}|ConvertTo-Json -Compress
