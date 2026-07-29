param(
    [Parameter(Mandatory = $true)]
    [string]$Report,

    [string]$Policy = (Join-Path ([System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))) "orchestration\language-adapter-acceptance-policy.v1.json"),

    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$gates = [System.Collections.Generic.List[object]]::new()

function Add-Gate {
    param([string]$Id, [bool]$Passed, [string]$Detail)
    $gates.Add([pscustomobject]@{ id = $Id; passed = $Passed; detail = $Detail })
}

function Test-ContainsAll {
    param([object[]]$Actual, [object[]]$Required)
    foreach ($item in $Required) {
        if ($Actual -notcontains $item) { return $false }
    }
    return $true
}

function Resolve-RepoBoundFile {
    param([string]$RelativePath)
    if ([string]::IsNullOrWhiteSpace($RelativePath) -or [System.IO.Path]::IsPathRooted($RelativePath)) {
        throw "provenance path must be repository-relative: $RelativePath"
    }
    $root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
    $prefix = [System.IO.Path]::TrimEndingDirectorySeparator($root) + [System.IO.Path]::DirectorySeparatorChar
    $resolved = [System.IO.Path]::GetFullPath((Join-Path $root $RelativePath))
    if (-not $resolved.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "provenance path escapes repository: $RelativePath"
    }
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "provenance file is missing: $RelativePath"
    }
    return $resolved
}

function Write-ResultAndExit {
    param([string]$AdapterId, [string]$Stage, [int]$MalformedExit = 0)
    $failed = @($gates | Where-Object { -not $_.passed })
    $result = [ordered]@{
        schema = "code-intel-language-adapter-acceptance-result.v1"
        adapterId = $AdapterId
        requestedStage = $Stage
        verdict = if ($failed.Count -eq 0 -and $MalformedExit -eq 0) { "pass" } else { "fail" }
        gates = @($gates)
        failedGateIds = @($failed | ForEach-Object { $_.id })
    }
    if ($Json) {
        $result | ConvertTo-Json -Depth 8
    } else {
        Write-Host "Language adapter acceptance: $($result.verdict) adapter=$AdapterId stage=$Stage"
        foreach ($gate in $gates) {
            Write-Host "$(if ($gate.passed) { 'PASS' } else { 'FAIL' }) $($gate.id): $($gate.detail)"
        }
    }
    if ($MalformedExit -ne 0) { exit $MalformedExit }
    if ($failed.Count -gt 0) { exit 1 }
    exit 0
}

try {
    $reportDocument = Get-Content -Raw -LiteralPath $Report | ConvertFrom-Json
    $policyDocument = Get-Content -Raw -LiteralPath $Policy | ConvertFrom-Json
} catch {
    Add-Gate -Id "input-shape" -Passed $false -Detail $_.Exception.Message
    Write-ResultAndExit -AdapterId "" -Stage "" -MalformedExit 2
}

$adapterId = [string]$reportDocument.adapter.id
$stage = [string]$reportDocument.adapter.requestedStage
$claimLevel = [string]$reportDocument.adapter.claimLevel
$stageProperty = $policyDocument.stages.PSObject.Properties[$stage]

if ([string]$reportDocument.schema -ne "code-intel-language-adapter-acceptance.v1" -or
    [string]$policyDocument.schema -ne "code-intel-language-adapter-acceptance-policy.v1" -or
    $null -eq $stageProperty -or
    @($policyDocument.claimLevels) -notcontains $claimLevel) {
    Add-Gate -Id "input-shape" -Passed $false -Detail "unknown schema, stage, or claim level"
    Write-ResultAndExit -AdapterId $adapterId -Stage $stage -MalformedExit 2
}
Add-Gate -Id "input-shape" -Passed $true -Detail "schemas, stage, and claim level recognized"

$expectedClaimLevels = @("inventory", "structural", "semantic", "behavioral")
$policyMonotonic = @($policyDocument.claimLevels).Count -eq $expectedClaimLevels.Count
for ($index = 0; $index -lt $expectedClaimLevels.Count -and $policyMonotonic; $index++) {
    if ([string]$policyDocument.claimLevels[$index] -ne $expectedClaimLevels[$index]) { $policyMonotonic = $false }
}
$minimumFields = @("languages", "labeledSamples", "precision", "recall", "declaredCoverage", "deterministicReplays", "parityArtifacts", "semanticOracleCases", "behavioralOracleCases")
$requirementFields = @("knownLicense", "rollbackTested", "independentVerification")
$stageOrder = @("research", "candidate", "production")
for ($index = 1; $index -lt $stageOrder.Count -and $policyMonotonic; $index++) {
    $previous = $policyDocument.stages.PSObject.Properties[$stageOrder[$index - 1]].Value
    $current = $policyDocument.stages.PSObject.Properties[$stageOrder[$index]].Value
    foreach ($field in $minimumFields) {
        if ([double]$current.minimums.$field -lt [double]$previous.minimums.$field) { $policyMonotonic = $false }
    }
    foreach ($field in $requirementFields) {
        if ([bool]$previous.requirements.$field -and -not [bool]$current.requirements.$field) { $policyMonotonic = $false }
    }
}
Add-Gate -Id "policy-monotonicity" -Passed $policyMonotonic -Detail "claim order fixed; research <= candidate <= production for all thresholds and requirements"

$stagePolicy = $stageProperty.Value
$minimums = $stagePolicy.minimums
$requirements = $stagePolicy.requirements
$languages = @($reportDocument.adapter.languages)
$uniqueLanguages = @($languages | Sort-Object -Unique)
$languagePass = $languages.Count -ge [int]$minimums.languages -and $uniqueLanguages.Count -eq $languages.Count
Add-Gate -Id "language-set" -Passed $languagePass -Detail "languages=$($languages.Count), required=$($minimums.languages), unique=$($uniqueLanguages.Count)"

$artifactSchemas = @($reportDocument.contract.artifactSchemas)
$contractPass = [bool]$reportDocument.contract.schemaValidated -and
    [bool]$reportDocument.contract.backwardCompatible -and
    (Test-ContainsAll -Actual $artifactSchemas -Required @($policyDocument.requiredArtifactSchemas))
Add-Gate -Id "contract" -Passed $contractPass -Detail "schemaValidated=$($reportDocument.contract.schemaValidated), backwardCompatible=$($reportDocument.contract.backwardCompatible)"

$claimOrder = @($policyDocument.claimLevels)
$claimIndex = [array]::IndexOf($claimOrder, $claimLevel)
$claimPass = $true
for ($index = 0; $index -lt $claimOrder.Count; $index++) {
    $name = [string]$claimOrder[$index]
    $actual = [bool]$reportDocument.claims.$name
    $expected = $index -le $claimIndex
    if ($actual -ne $expected) { $claimPass = $false }
}
Add-Gate -Id "claim-boundary" -Passed $claimPass -Detail "declared=$claimLevel with lower levels required and higher levels forbidden"

$qualityPass = [int]$reportDocument.corpus.labeledSamples -ge [int]$minimums.labeledSamples
if ($claimIndex -ge 1) {
    $qualityPass = $qualityPass -and
        [double]$reportDocument.corpus.precision -ge [double]$minimums.precision -and
        [double]$reportDocument.corpus.recall -ge [double]$minimums.recall -and
        [double]$reportDocument.corpus.declaredCoverage -ge [double]$minimums.declaredCoverage
}
Add-Gate -Id "measured-quality" -Passed $qualityPass -Detail "samples=$($reportDocument.corpus.labeledSamples), precision=$($reportDocument.corpus.precision), recall=$($reportDocument.corpus.recall), coverage=$($reportDocument.corpus.declaredCoverage)"

$unsupportedPass = [bool]$reportDocument.corpus.unsupportedExplicit -and [int]$reportDocument.corpus.fabricatedFactsForUnsupported -eq 0
Add-Gate -Id "unsupported-behavior" -Passed $unsupportedPass -Detail "explicit=$($reportDocument.corpus.unsupportedExplicit), fabricated=$($reportDocument.corpus.fabricatedFactsForUnsupported)"

$determinismPass = [bool]$reportDocument.determinism.stable -and [int]$reportDocument.determinism.replays -ge [int]$minimums.deterministicReplays
Add-Gate -Id "determinism" -Passed $determinismPass -Detail "stable=$($reportDocument.determinism.stable), replays=$($reportDocument.determinism.replays), required=$($minimums.deterministicReplays)"

$compatibilityPass = [bool]$reportDocument.compatibility.passed -and [int]$reportDocument.compatibility.parityArtifacts -ge [int]$minimums.parityArtifacts
Add-Gate -Id "compatibility" -Passed $compatibilityPass -Detail "passed=$($reportDocument.compatibility.passed), artifacts=$($reportDocument.compatibility.parityArtifacts), required=$($minimums.parityArtifacts)"

$declaredEffects = @($reportDocument.effects.declared)
$observedEffects = @($reportDocument.effects.observed)
$effectPass = (Test-ContainsAll -Actual $declaredEffects -Required $observedEffects) -and
    (Test-ContainsAll -Actual @($policyDocument.allowedEffects) -Required $declaredEffects) -and
    -not [bool]$reportDocument.effects.networkUsed -and
    -not [bool]$reportDocument.effects.repoMutationUsed
Add-Gate -Id "effect-boundary" -Passed $effectPass -Detail "declared=$($declaredEffects -join ','), observed=$($observedEffects -join ','), network=$($reportDocument.effects.networkUsed), mutation=$($reportDocument.effects.repoMutationUsed)"

$license = [string]$reportDocument.provenance.implementationLicense
$knownLicense = -not [string]::IsNullOrWhiteSpace($license) -and $license -notmatch '^UNKNOWN'
$digestPattern = '^[0-9a-f]{64}$'
$digestBound = $false
$digestDetail = "not checked"
try {
    $sourcePath = Resolve-RepoBoundFile -RelativePath ([string]$reportDocument.provenance.sourcePath)
    $conformancePath = Resolve-RepoBoundFile -RelativePath ([string]$reportDocument.provenance.conformancePath)
    $actualSourceDigest = (Get-FileHash -Algorithm SHA256 -LiteralPath $sourcePath).Hash.ToLowerInvariant()
    $actualConformanceDigest = (Get-FileHash -Algorithm SHA256 -LiteralPath $conformancePath).Hash.ToLowerInvariant()
    $digestBound = $actualSourceDigest -eq [string]$reportDocument.provenance.sourceDigest -and
        $actualConformanceDigest -eq [string]$reportDocument.provenance.conformanceDigest
    $digestDetail = "sourceBound=$($actualSourceDigest -eq [string]$reportDocument.provenance.sourceDigest), conformanceBound=$($actualConformanceDigest -eq [string]$reportDocument.provenance.conformanceDigest)"
} catch {
    $digestDetail = $_.Exception.Message
}
$provenancePass = [bool]$reportDocument.provenance.sourceRevisionPinned -and
    [string]$reportDocument.provenance.sourceDigest -match $digestPattern -and
    [string]$reportDocument.provenance.conformanceDigest -match $digestPattern -and
    $digestBound -and
    (-not [bool]$requirements.knownLicense -or $knownLicense)
Add-Gate -Id "provenance" -Passed $provenancePass -Detail "revisionPinned=$($reportDocument.provenance.sourceRevisionPinned), license=$license, knownRequired=$($requirements.knownLicense), $digestDetail"

$rollbackPass = [bool]$reportDocument.rollback.documented -and (-not [bool]$requirements.rollbackTested -or [bool]$reportDocument.rollback.tested)
Add-Gate -Id "rollback" -Passed $rollbackPass -Detail "documented=$($reportDocument.rollback.documented), tested=$($reportDocument.rollback.tested), testedRequired=$($requirements.rollbackTested)"

$verificationPass = -not [bool]$requirements.independentVerification -or [bool]$reportDocument.verification.independent
Add-Gate -Id "independent-verification" -Passed $verificationPass -Detail "independent=$($reportDocument.verification.independent), required=$($requirements.independentVerification)"

$oraclePass = $true
if ($claimIndex -ge 2) {
    $oraclePass = [int]$reportDocument.oracles.semanticCases -ge [int]$minimums.semanticOracleCases
}
if ($claimIndex -ge 3) {
    $oraclePass = $oraclePass -and [int]$reportDocument.oracles.behavioralCases -ge [int]$minimums.behavioralOracleCases
}
Add-Gate -Id "oracle-depth" -Passed $oraclePass -Detail "semantic=$($reportDocument.oracles.semanticCases), behavioral=$($reportDocument.oracles.behavioralCases)"

$evidencePass = @($reportDocument.evidence).Count -gt 0
Add-Gate -Id "evidence" -Passed $evidencePass -Detail "evidenceRefs=$(@($reportDocument.evidence).Count)"

Write-ResultAndExit -AdapterId $adapterId -Stage $stage
