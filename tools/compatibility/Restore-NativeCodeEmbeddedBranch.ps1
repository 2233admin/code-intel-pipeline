[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string]$RehearsalRoot,
    [string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Test-Path -LiteralPath $RehearsalRoot) { throw "rollback rehearsal root must be exclusive: $RehearsalRoot" }

$sourcePath = Join-Path $RepoRoot "run-code-intel.ps1"
$source = [IO.File]::ReadAllText($sourcePath).Replace("`r`n", "`n").Replace("`r", "`n")
$patterns = @(
    '(?sm)^function Get-CodeEvidenceLanguage \{.*?(?=\nfunction ConvertTo-NullableDouble \{)',
    '(?m)^\$codeEvidenceConfig = Get-JsonProperty \$configData "codeEvidence"\n\$codeEvidence = New-CodeEvidenceLayer -RepoPath \$repoPath -RunDir \$runDir -Files \$inventoryFiles -CodeEvidenceConfig \$codeEvidenceConfig'
)
$matches = @($patterns | ForEach-Object {
    $all = [regex]::Matches($source, $_)
    if ($all.Count -ne 1) { throw "embedded Native Code Evidence marker is absent or ambiguous: $_" }
    $all[0]
} | Sort-Object Index)

$withoutLegacy = $source
foreach ($match in @($matches | Sort-Object Index -Descending)) {
    $withoutLegacy = $withoutLegacy.Remove($match.Index, $match.Length)
}
$restored = $withoutLegacy
foreach ($match in $matches) {
    $restored = $restored.Insert($match.Index, $match.Value)
}
if ($restored -cne $source) { throw "rollback rehearsal did not reproduce the exact normalized facade bytes" }

$null = New-Item -ItemType Directory -Path $RehearsalRoot
$target = Join-Path $RehearsalRoot "run-code-intel.ps1"
[IO.File]::WriteAllText($target, $restored, [Text.UTF8Encoding]::new($false))
[ordered]@{
    ok = $true; schema = "code-intel-native-code-embedded-rollback-rehearsal.v1"
    branchId = "run-code-intel.native-code.embedded"; source = "run-code-intel.ps1"; target = $target
    exactReplay = $true; unrelatedBranchesChanged = $false; segmentCount = 2
} | ConvertTo-Json -Depth 5 -Compress
