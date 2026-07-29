param(
    [string]$RepoPath = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Contract {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function ConvertTo-CanonicalFixtureJson {
    param([Parameter(Mandatory = $true)]$Value)
    return ($Value | ConvertTo-Json -Depth 20 -Compress)
}

function Assert-SchemaValid {
    param([string]$Json, [string]$SchemaPath, [string]$Name)
    $valid = Test-Json -Json $Json -SchemaFile $SchemaPath -ErrorAction SilentlyContinue
    Assert-Contract $valid "$Name must satisfy the capability envelope schema."
}

function Assert-SchemaInvalid {
    param([string]$Json, [string]$SchemaPath, [string]$Name)
    $valid = Test-Json -Json $Json -SchemaFile $SchemaPath -ErrorAction SilentlyContinue
    Assert-Contract (-not $valid) "$Name must be rejected by the capability envelope schema."
}

function Assert-ObservedEffectsDeclared {
    param([string[]]$DeclaredEffects, [string[]]$ObservedEffects, [string]$Name)
    $undeclared = @($ObservedEffects | Where-Object { $_ -notin $DeclaredEffects })
    Assert-Contract ($undeclared.Count -eq 0) "$Name observed undeclared effects: $($undeclared -join ', ')"
}

function Assert-SetEqual {
    param([string[]]$Expected, [string[]]$Actual, [string]$Name)
    $missing = @($Expected | Where-Object { $_ -notin $Actual })
    $unexpected = @($Actual | Where-Object { $_ -notin $Expected })
    Assert-Contract ($missing.Count -eq 0 -and $unexpected.Count -eq 0) "$Name differs: missing=[$($missing -join ', ')], unexpected=[$($unexpected -join ', ')]"
}

function Assert-EnvelopesCoherent {
    param($Declaration, $Request, $Result, [string]$Name)
    Assert-Contract ($Request.capability -eq $Declaration.id) "$Name request capability differs from declaration id."
    Assert-Contract ((ConvertTo-CanonicalFixtureJson $Request.implementation) -eq (ConvertTo-CanonicalFixtureJson $Declaration.implementation)) "$Name request implementation differs from declaration."
    $disallowed = @($Request.effectPolicy.allowedEffects | Where-Object { $_ -notin $Declaration.allowedEffects })
    Assert-Contract ($disallowed.Count -eq 0) "$Name request asks for effects outside declaration: $($disallowed -join ', ')"
    Assert-Contract ($Result.capability -eq $Request.capability) "$Name result capability differs from request."
    Assert-Contract ((ConvertTo-CanonicalFixtureJson $Result.implementation) -eq (ConvertTo-CanonicalFixtureJson $Request.implementation)) "$Name result implementation differs from request."
    Assert-Contract ($Result.determinism -eq $Declaration.determinism) "$Name result determinism differs from declaration."
    Assert-SetEqual $Request.effectPolicy.allowedEffects $Result.declaredEffects "$Name result declared effects"
    Assert-ObservedEffectsDeclared $Result.declaredEffects $Result.observedEffects $Name
    Assert-Contract ($Result.snapshotIdentity -eq $Request.snapshot.identity) "$Name result snapshot identity differs from request."
    foreach ($artifactRef in @($Result.artifacts)) {
        if ($null -ne $artifactRef.consumedSnapshotIdentity) {
            Assert-Contract ($artifactRef.consumedSnapshotIdentity -eq $Result.snapshotIdentity) "$Name output artifact consumed snapshot differs from result."
        }
    }
}

$root = (Resolve-Path -LiteralPath $RepoPath).Path
$registryPath = Join-Path $root "orchestration\integrations.json"
$contractPath = Join-Path $root "orchestration\capability-contract.v1.json"
$schemaPath = Join-Path $root "orchestration\schemas\code-intel-capability-envelope.v1.schema.json"
$documentationPaths = @(
    (Join-Path $root "CONTEXT.md"),
    (Join-Path $root "docs\atomic-development-model.md"),
    (Join-Path $root "docs\adr\0009-atomic-capability-execution-model.md"),
    (Join-Path $root "docs\code-intel-architecture.md")
)

foreach ($path in @($registryPath, $contractPath, $schemaPath) + $documentationPaths) {
    Assert-Contract (Test-Path -LiteralPath $path -PathType Leaf) "Required atomic capability contract file is missing: $path"
}

$registry = Get-Content -LiteralPath $registryPath -Raw | ConvertFrom-Json -ErrorAction Stop
$contract = Get-Content -LiteralPath $contractPath -Raw | ConvertFrom-Json -ErrorAction Stop
$null = Get-Content -LiteralPath $schemaPath -Raw | ConvertFrom-Json -ErrorAction Stop

Assert-Contract ($contract.schema -eq "code-intel-capability-contract.v1") "Unexpected capability contract schema."
Assert-Contract ($contract.contractVersion -eq 1) "Capability contract version must be 1."
Assert-Contract ($contract.envelopeSchema -eq "orchestration/schemas/code-intel-capability-envelope.v1.schema.json") "Contract must bind the canonical JSON Schema."
Assert-Contract ($registry.policy.capabilityContract -eq "orchestration/capability-contract.v1.json") "Integration registry must bind the canonical capability contract."

$toolchainEvidenceCapabilities = 0
$rootPrefix = [System.IO.Path]::TrimEndingDirectorySeparator($root) + [System.IO.Path]::DirectorySeparatorChar
foreach ($integration in @($registry.integrations)) {
    $evidenceProperty = $integration.PSObject.Properties["toolchainDigestEvidence"]
    if ($null -eq $evidenceProperty) { continue }

    $implementationProperty = $integration.capabilityDeclaration.PSObject.Properties["implementation"]
    Assert-Contract ($null -ne $implementationProperty) "$($integration.id) toolchain evidence requires an implementation declaration."
    Assert-Contract ([string]$evidenceProperty.Value.algorithm -eq "sha256") "$($integration.id) toolchain evidence must use SHA-256."

    $inputs = @($evidenceProperty.Value.inputs)
    $declaredDigests = @($implementationProperty.Value.toolchainDigests)
    Assert-Contract ($inputs.Count -gt 0) "$($integration.id) toolchain evidence must declare at least one input."
    Assert-Contract ($inputs.Count -eq $declaredDigests.Count) "$($integration.id) toolchain evidence input/digest counts differ."

    for ($index = 0; $index -lt $inputs.Count; $index++) {
        $relativePath = [string]$inputs[$index]
        Assert-Contract (-not [string]::IsNullOrWhiteSpace($relativePath)) "$($integration.id) toolchain evidence contains an empty input path."
        Assert-Contract (-not [System.IO.Path]::IsPathFullyQualified($relativePath)) "$($integration.id) toolchain evidence input must be repository-relative: $relativePath"
        $sourcePath = [System.IO.Path]::GetFullPath((Join-Path $root $relativePath))
        Assert-Contract ($sourcePath.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) "$($integration.id) toolchain evidence escapes the repository: $relativePath"
        Assert-Contract (Test-Path -LiteralPath $sourcePath -PathType Leaf) "$($integration.id) toolchain evidence input is missing: $relativePath"
        $actualDigest = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
        $declaredDigest = ([string]$declaredDigests[$index]).ToLowerInvariant()
        Assert-Contract ($declaredDigest -eq $actualDigest) "$($integration.id) toolchain digest is stale for $relativePath."
    }
    $toolchainEvidenceCapabilities++
}

$expectedVocabulary = @("Capability Atom", "Snapshot Identity", "Artifact Ref", "Effect Boundary", "Domain Verdict", "Run Commit", "Materialized View")
$expectedVerdicts = @("pass", "fail", "unknown", "not_applicable")
$expectedEffects = @("repo_read", "local_write", "network", "repo_mutation")
$expectedExitCodes = @(0, 10, 20, 64, 65, 69, 70, 74)

Assert-Contract ((@($contract.vocabulary) -join "|") -eq ($expectedVocabulary -join "|")) "Atomic vocabulary changed without a contract version bump."
Assert-Contract ((@($contract.result.verdicts) -join "|") -eq ($expectedVerdicts -join "|")) "Verdict lattice changed without a contract version bump."
Assert-Contract ((@($contract.effectBoundary.effects) -join "|") -eq ($expectedEffects -join "|")) "Effect allowlist changed without a contract version bump."
Assert-Contract ((@($contract.exitCodes.code) -join "|") -eq ($expectedExitCodes -join "|")) "Exit-code contract changed without a version bump."
Assert-Contract (-not (@($contract.effectBoundary.effects) -contains "pure")) "Purity/determinism must not be encoded as a permission effect."

$exitCodeDuplicates = @($contract.exitCodes.code | Group-Object | Where-Object Count -gt 1)
Assert-Contract ($exitCodeDuplicates.Count -eq 0) "Exit codes must be unique."
Assert-Contract ($contract.cacheKey.algorithm -eq "sha256") "Capability cache keys must use SHA-256."
Assert-Contract (@($contract.cacheKey.orderedComponents) -contains "snapshotIdentity") "Cache key must bind the repository snapshot."
Assert-Contract (@($contract.cacheKey.orderedComponents) -contains "orderedInputArtifactDigests") "Cache key must bind input artifact digests."
Assert-Contract (@($contract.cacheKey.forbiddenComponents) -contains "generatedAt") "Wall-clock time must not enter the deterministic cache key."
Assert-Contract ($contract.publication.completionMarker -eq "run-complete.json") "Transactional publication requires the canonical completion marker."

foreach ($path in $documentationPaths) {
    $text = Get-Content -LiteralPath $path -Raw
    foreach ($term in $expectedVocabulary) {
        Assert-Contract ($text.Contains($term)) "$path is missing canonical atomic vocabulary: $term"
    }
}

$digestA = "a" * 64
$digestB = "b" * 64
$implementation = [ordered]@{
    id = "inventory.rg"
    version = "1.0.0"
    toolchainDigests = @($digestA)
}
$artifact = [ordered]@{
    schema = "code-intel-artifact-ref.v1"
    artifactSchema = "code-intel-file-inventory.v1"
    type = "inventory.files"
    path = "artifacts/inventory.json"
    sha256 = $digestB
    consumedSnapshotIdentity = $digestA
}
$declaration = [ordered]@{
    schema = "code-intel-capability-declaration.v1"
    id = "inventory.rg"
    contractVersion = 1
    implementation = $implementation
    determinism = "deterministic"
    allowedEffects = @("repo_read", "local_write")
    dependencies = @()
}
$request = [ordered]@{
    schema = "code-intel-capability-request.v1"
    capability = "inventory.rg"
    contractVersion = 1
    implementation = $implementation
    snapshot = [ordered]@{
        identity = $digestA
        repoIdentity = "github.com/2233admin/code-intel-pipeline"
        head = "0123456789abcdef0123456789abcdef01234567"
        workingTreePolicy = "head_only"
        scope = @(".")
        inputDigest = $digestA
    }
    options = [ordered]@{}
    inputs = @($artifact)
    effectPolicy = [ordered]@{ allowedEffects = @("repo_read", "local_write") }
}
$result = [ordered]@{
    schema = "code-intel-capability-result.v1"
    capability = "inventory.rg"
    implementation = $implementation
    snapshotIdentity = $digestA
    status = "completed"
    verdict = "pass"
    domainVerdict = "pass"
    exitCode = 0
    determinism = "deterministic"
    declaredEffects = @("repo_read", "local_write")
    observedEffects = @("repo_read", "local_write")
    cache = [ordered]@{ key = $digestB; hit = $false }
    artifacts = @($artifact)
    diagnostics = @()
    provenance = [ordered]@{
        attemptId = "fixture-1"
        generatedAt = "2026-07-13T00:00:00Z"
    }
}

Assert-SchemaValid (ConvertTo-CanonicalFixtureJson $artifact) $schemaPath "valid artifact ref"
Assert-SchemaValid (ConvertTo-CanonicalFixtureJson $declaration) $schemaPath "valid capability declaration"
Assert-SchemaValid (ConvertTo-CanonicalFixtureJson $request) $schemaPath "valid capability request"
Assert-SchemaValid (ConvertTo-CanonicalFixtureJson $result) $schemaPath "valid completed result"
Assert-EnvelopesCoherent $declaration $request $result "valid envelope chain"

$domainFail = ConvertTo-CanonicalFixtureJson $result | ConvertFrom-Json
$domainFail.status = "completed"
$domainFail.verdict = "fail"
$domainFail.domainVerdict = "fail"
$domainFail.exitCode = 10
Assert-SchemaValid (ConvertTo-CanonicalFixtureJson $domainFail) $schemaPath "valid domain-fail result"

$blocked = ConvertTo-CanonicalFixtureJson $result | ConvertFrom-Json
$blocked.status = "blocked"
$blocked.verdict = "unknown"
$blocked.domainVerdict = "unknown"
$blocked.exitCode = 20
Assert-SchemaValid (ConvertTo-CanonicalFixtureJson $blocked) $schemaPath "valid blocked result"

$outcomeMatrixCases = 0
foreach ($status in @($contract.result.statuses)) {
    foreach ($verdict in @($contract.result.verdicts)) {
        foreach ($exitCode in @($contract.exitCodes.code)) {
            $candidate = ConvertTo-CanonicalFixtureJson $result | ConvertFrom-Json
            $candidate.status = $status
            $candidate.verdict = $verdict
            $candidate.domainVerdict = $verdict
            $candidate.exitCode = $exitCode
            $actualValid = Test-Json -Json (ConvertTo-CanonicalFixtureJson $candidate) -SchemaFile $schemaPath -ErrorAction SilentlyContinue
            $mapping = @($contract.exitCodes | Where-Object { $_.code -eq $exitCode })[0]
            $expectedValid = ($status -eq $mapping.status -and $verdict -in @($mapping.verdicts))
            Assert-Contract ($actualValid -eq $expectedValid) "Outcome matrix drift for status=$status verdict=$verdict exitCode=$exitCode."
            $outcomeMatrixCases++
        }
    }
}

$invalidOutcome = ConvertTo-CanonicalFixtureJson $result | ConvertFrom-Json
$invalidOutcome.status = "failed"
$invalidOutcome.verdict = "pass"
$invalidOutcome.exitCode = 70
Assert-SchemaInvalid (ConvertTo-CanonicalFixtureJson $invalidOutcome) $schemaPath "failed/pass outcome"

$invalidEffectType = ConvertTo-CanonicalFixtureJson $result | ConvertFrom-Json
$invalidEffectType.observedEffects = "repo_read"
Assert-SchemaInvalid (ConvertTo-CanonicalFixtureJson $invalidEffectType) $schemaPath "string effect set"

$unknownField = ConvertTo-CanonicalFixtureJson $request | ConvertFrom-Json
$unknownField | Add-Member -NotePropertyName surprise -NotePropertyValue $true
Assert-SchemaInvalid (ConvertTo-CanonicalFixtureJson $unknownField) $schemaPath "request with unknown field"

$badDigest = ConvertTo-CanonicalFixtureJson $artifact | ConvertFrom-Json
$badDigest.sha256 = "not-a-digest"
Assert-SchemaInvalid (ConvertTo-CanonicalFixtureJson $badDigest) $schemaPath "artifact with invalid digest"

$missingPayloadSchema = ConvertTo-CanonicalFixtureJson $artifact | ConvertFrom-Json
$missingPayloadSchema.PSObject.Properties.Remove("artifactSchema")
Assert-SchemaInvalid (ConvertTo-CanonicalFixtureJson $missingPayloadSchema) $schemaPath "artifact without payload schema"

$missingConsumedSnapshot = ConvertTo-CanonicalFixtureJson $artifact | ConvertFrom-Json
$missingConsumedSnapshot.PSObject.Properties.Remove("consumedSnapshotIdentity")
Assert-SchemaInvalid (ConvertTo-CanonicalFixtureJson $missingConsumedSnapshot) $schemaPath "artifact without consumed snapshot identity"

$coherenceMutators = @(
    @{ name = "request capability mismatch"; apply = { param($d, $q, $r) $q.capability = "other.capability" } },
    @{ name = "request implementation mismatch"; apply = { param($d, $q, $r) $q.implementation.version = "2.0.0" } },
    @{ name = "request effect outside declaration"; apply = { param($d, $q, $r) $q.effectPolicy.allowedEffects = @("repo_read", "network") } },
    @{ name = "result capability mismatch"; apply = { param($d, $q, $r) $r.capability = "other.capability" } },
    @{ name = "result implementation mismatch"; apply = { param($d, $q, $r) $r.implementation.version = "2.0.0" } },
    @{ name = "result determinism mismatch"; apply = { param($d, $q, $r) $r.determinism = "external_nondeterministic" } },
    @{ name = "result declared effect drift"; apply = { param($d, $q, $r) $r.declaredEffects = @("repo_read") } },
    @{ name = "observed undeclared effect"; apply = { param($d, $q, $r) $r.observedEffects = @("repo_read", "network") } },
    @{ name = "result snapshot mismatch"; apply = { param($d, $q, $r) $r.snapshotIdentity = "c" * 64 } }
    @{ name = "output artifact snapshot mismatch"; apply = { param($d, $q, $r) $r.artifacts[0].consumedSnapshotIdentity = "c" * 64 } }
)

foreach ($case in $coherenceMutators) {
    $badDeclaration = ConvertTo-CanonicalFixtureJson $declaration | ConvertFrom-Json
    $badRequest = ConvertTo-CanonicalFixtureJson $request | ConvertFrom-Json
    $badResult = ConvertTo-CanonicalFixtureJson $result | ConvertFrom-Json
    & $case.apply $badDeclaration $badRequest $badResult
    $rejected = $false
    try { Assert-EnvelopesCoherent $badDeclaration $badRequest $badResult $case.name }
    catch { $rejected = $true }
    Assert-Contract $rejected "$($case.name) must be rejected by cross-envelope coherence rules."
}

[ordered]@{
    ok = $true
    schema = $contract.schema
    envelopeSchema = $contract.envelopeSchema
    outcomeMatrixCases = $outcomeMatrixCases
    rejectedSchemaFixtures = 6
    rejectedCoherenceFixtures = $coherenceMutators.Count
    vocabularyTerms = $expectedVocabulary.Count
    toolchainEvidenceCapabilities = $toolchainEvidenceCapabilities
    completionMarker = $contract.publication.completionMarker
} | ConvertTo-Json -Depth 4
