#requires -Version 7.2

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$facade = Join-Path $root "run-code-intel.ps1"
$snapshot = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
$implementationDigest = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-b04-ps-" + [guid]::NewGuid().ToString("N"))

function Write-JsonNoBom {
    param([string]$Path, [object]$Value)
    [System.IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 20 -Compress), [System.Text.UTF8Encoding]::new($false))
}

function Invoke-CodeNexusCase {
    param(
        [string]$Name,
        [ValidateSet("full", "lite")][string]$Mode,
        [ValidateSet("current", "unavailable")][string]$Status,
        [string]$ExpectedVerdict,
        [bool]$ExpectedUsable
    )

    $caseRoot = Join-Path $tempRoot $Name
    [System.IO.Directory]::CreateDirectory($caseRoot) | Out-Null
    $providerId = if ($Mode -eq "full") { "codenexus.full" } else { "codenexus.lite-compat" }
    $implementationId = if ($Mode -eq "full") { "codenexus.service.v1" } else { "invoke-codenexus-lite.ps1" }
    $activation = if ($Mode -eq "full") { "primary" } else { "explicit_fallback" }
    $effects = if ($Mode -eq "full") { @("network_provider", "read_provider_artifact") } else { @("read_repository", "read_git_history", "read_sentrux_artifacts", "write_compatibility_artifact") }
    $completeness = if ($Status -eq "current") { "complete" } else { "partial" }
    $availability = if ($Status -eq "current") { "available" } else { "provider_unavailable" }
    $providerData = if ($Status -eq "current") { [ordered]@{ opaque = [ordered]@{ providerOwned = $true } } } else { $null }
    $payload = [ordered]@{
        schema = "code-intel-evidence-payload.v1"
        data = [ordered]@{
            codenexus = [ordered]@{
                schema = "code-intel-codenexus-evidence.v1"
                snapshotIdentity = $snapshot
                provider = [ordered]@{ mode = $Mode; providerId = $providerId; implementationId = $implementationId; activation = $activation }
                provenance = [ordered]@{ sourceRevision = "$Name-revision"; observedAt = 1950 }
                completeness = $completeness
                availability = $availability
                providerData = $providerData
            }
        }
    }
    $payloadPath = Join-Path $caseRoot "payload.json"
    Write-JsonNoBom $payloadPath $payload
    $payloadDigest = (Get-FileHash -LiteralPath $payloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $native = [ordered]@{
        schema = "code-intel-codenexus-native-result.v1"
        providerMode = $Mode
        status = $Status
        providerId = $providerId
        implementation = [ordered]@{ id = $implementationId; version = "1.0.0"; digest = $implementationDigest }
        sourceRevision = "$Name-revision"
        expectedSnapshotIdentity = $snapshot
        sourceSnapshotIdentity = $snapshot
        collectedAt = 1949
        observedAt = 1950
        payload = [ordered]@{
            schema = "code-intel-artifact-ref.v1"
            artifactSchema = "code-intel-evidence-payload.v1"
            type = "observed.evidence.payload"
            path = "payload.json"
            sha256 = $payloadDigest
            consumedSnapshotIdentity = $snapshot
        }
        activation = $activation
        effects = $effects
    }
    $requestPath = Join-Path $caseRoot "native.json"
    Write-JsonNoBom $requestPath $native
    $raw = & $facade `
        -CodeNexusAdapterRequest $requestPath `
        -CodeNexusAdapterArtifactRoot $caseRoot `
        -CodeNexusAdapterEvaluatedAt 2000 `
        -CodeNexusAdapterMaxAgeSeconds 100
    if ($LASTEXITCODE -ne 0) { throw "$Name facade route failed with exit $LASTEXITCODE" }
    $result = $raw | ConvertFrom-Json
    if ($result.schema -ne "code-intel-codenexus-route-result.v1" -or $result.status -ne "completed") {
        throw "$Name did not return the B04 production route envelope"
    }
    if ($result.admission.domainVerdict -ne $ExpectedVerdict) { throw "$Name verdict drifted" }
    if ([bool]$result.adapter.port.perceptionUsable -ne $ExpectedUsable) { throw "$Name usability drifted" }
    if (@($result.engineeringFacts).Count -ne 0) { throw "$Name fabricated Engineering Facts" }
}

function Invoke-LiteScriptEndToEnd {
    $caseRoot = Join-Path $tempRoot "lite-script"
    $repoRoot = Join-Path $caseRoot "repo"
    [System.IO.Directory]::CreateDirectory($repoRoot) | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $repoRoot "README.md"), "fixture", [System.Text.UTF8Encoding]::new($false))
    $requestPath = Join-Path $caseRoot "native.json"
    & (Join-Path $root "Invoke-CodeNexusLite.ps1") `
        -RepoPath $repoRoot `
        -RunDir $caseRoot `
        -AdapterRequestPath $requestPath `
        -ExpectedSnapshotIdentity $snapshot `
        -SourceSnapshotIdentity $snapshot `
        -SourceRevision "lite-script-revision" `
        -ObservedAt 1950 `
        -AdapterActivation "explicit_fallback" `
        -Quiet
    if ($LASTEXITCODE -ne 0) { throw "CodeNexus lite adapter-output mode failed" }
    $raw = & $facade `
        -CodeNexusAdapterRequest $requestPath `
        -CodeNexusAdapterArtifactRoot $caseRoot `
        -CodeNexusAdapterEvaluatedAt 2000 `
        -CodeNexusAdapterMaxAgeSeconds 100
    if ($LASTEXITCODE -ne 0) { throw "CodeNexus lite script facade route failed" }
    $result = $raw | ConvertFrom-Json
    if ($result.admission.domainVerdict -ne "observed" -or -not [bool]$result.adapter.port.perceptionUsable) {
        throw "CodeNexus lite script did not pass the B04/A04 production route"
    }
    if ($result.adapter.port.provider.activation -ne "explicit_fallback") {
        throw "CodeNexus lite script lost explicit fallback identity"
    }
    if (@($result.engineeringFacts).Count -ne 0) { throw "CodeNexus lite script fabricated Engineering Facts" }
}

try {
    Push-Location $root
    cargo build -p code-intel --quiet
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    Invoke-CodeNexusCase "full" "full" "current" "observed" $true
    Invoke-CodeNexusCase "lite" "lite" "current" "observed" $true
    Invoke-CodeNexusCase "unavailable" "full" "unavailable" "unknown" $false
    Invoke-LiteScriptEndToEnd
    Write-Host "CodeNexus adapter PowerShell facade contract passed."
}
finally {
    Pop-Location
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
