#requires -Version 7.2

$ErrorActionPreference = "Stop"
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$repoRoot = Split-Path -Parent $root
$facade = Join-Path $root "run-code-intel.ps1"
$rustExecutableName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
$cargoTargetRoot = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    Join-Path $repoRoot "target"
}
else {
    [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
}
$rustCli = Join-Path (Join-Path $cargoTargetRoot "debug") $rustExecutableName
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
    $unreferencedPath = Join-Path $repoRoot "orphan.ps1"
    [System.IO.File]::WriteAllText($unreferencedPath, "function unrelated() {}`n", [System.Text.UTF8Encoding]::new($false))
    $hotspotsPath = Join-Path $caseRoot "hotspots.json"
    Write-JsonNoBom $hotspotsPath @{ files = @(@{ path = "orphan.ps1"; maxComplexity = $null; functionCount = $null }) }
    & (Join-Path $root "Invoke-CodeNexusLite.ps1") `
        -RepoPath $repoRoot `
        -TargetPath $repoRoot `
        -RunDir $caseRoot `
        -HotspotsPath $hotspotsPath `
        -OutputPath (Join-Path $caseRoot "unreferenced-context.json") `
        -MaxFiles 1 `
        -MaxReferencesPerFile 3 `
        -MaxCommitsPerFile 0 `
        -Quiet
    if ($LASTEXITCODE -ne 0) { throw "CodeNexus lite no-reference fallback failed with exit $LASTEXITCODE" }
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

function ConvertTo-NormalizedCodeNexusContext {
    param(
        [object]$Document,
        [string]$RepoPath
    )

    $repoPrefix = $RepoPath.Replace('\', '/').TrimEnd('/') + '/'

    return [ordered]@{
        tool = [string]$Document.tool
        generatedAt = "<normalized>"
        repo = [string]$Document.repo
        target = [string]$Document.target
        output = "<normalized>"
        sources = [ordered]@{
            dsm = [string]$Document.sources.dsm
            hotspots = [string]$Document.sources.hotspots
        }
        summary = [ordered]@{
            files = [int]$Document.summary.files
            references = [int]$Document.summary.references
            recentCommits = [int]$Document.summary.recentCommits
        }
        files = @($Document.files | ForEach-Object {
            [ordered]@{
                path = ([string]$_.path).Replace('\', '/')
                reason = [string]$_.reason
                maxComplexity = $_.maxComplexity
                functionCount = $_.functionCount
                riskScore = $_.riskScore
                digest = [ordered]@{
                    exists = [bool]$_.digest.exists
                    loc = [int]$_.digest.loc
                    firstLines = @($_.digest.firstLines | ForEach-Object { [string]$_ })
                }
                recentCommits = @($_.recentCommits | ForEach-Object { [string]$_ })
                references = @($_.references | ForEach-Object {
                    $reference = ([string]$_).Replace('\', '/')
                    if ($reference.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                        $reference = $reference.Substring($repoPrefix.Length)
                    }
                    while ($reference.StartsWith('./')) {
                        $reference = $reference.Substring(2)
                    }
                    $reference
                } | Sort-Object)
            }
        })
        nextQueries = @($Document.nextQueries | ForEach-Object { [string]$_ })
        limitations = @($Document.limitations | ForEach-Object { [string]$_ })
    }
}

function Invoke-RustGeneratorParity {
    $caseRoot = Join-Path $tempRoot "generator-parity"
    $fixtureRepo = Join-Path $caseRoot "repo"
    [System.IO.Directory]::CreateDirectory((Join-Path $fixtureRepo "src")) | Out-Null
    [System.IO.Directory]::CreateDirectory((Join-Path $fixtureRepo "docs")) | Out-Null
    [System.IO.Directory]::CreateDirectory((Join-Path $fixtureRepo "target")) | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $fixtureRepo "src/largest.rs"), "fn largest() {}`n// largest`n", [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText((Join-Path $fixtureRepo "src/small.rs"), "fn small() {}`n", [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText((Join-Path $fixtureRepo "docs/references.md"), "largest calls remain text references`n", [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText((Join-Path $fixtureRepo "target/ignored.rs"), ("ignored`n" * 100), [System.Text.UTF8Encoding]::new($false))
    $legacyOutput = Join-Path $caseRoot "legacy-context.json"
    $rustOutput = Join-Path $caseRoot "rust-context.json"

    & (Join-Path $root "Invoke-CodeNexusLite.ps1") `
        -RepoPath $fixtureRepo `
        -TargetPath $fixtureRepo `
        -RunDir $caseRoot `
        -OutputPath $legacyOutput `
        -MaxFiles 2 `
        -MaxReferencesPerFile 3 `
        -MaxCommitsPerFile 0 `
        -Quiet
    if ($LASTEXITCODE -ne 0) { throw "historical CodeNexus generator failed" }

    $null = & $rustCli codenexus generate `
        --repo $fixtureRepo `
        --target $fixtureRepo `
        --out $rustOutput `
        --observed-at 1950 `
        --max-files 2 `
        --max-references-per-file 3
    if ($LASTEXITCODE -ne 0) { throw "compiled CodeNexus generator failed" }

    $legacy = ConvertTo-NormalizedCodeNexusContext (Get-Content -Raw $legacyOutput | ConvertFrom-Json) $fixtureRepo
    $rust = ConvertTo-NormalizedCodeNexusContext (Get-Content -Raw $rustOutput | ConvertFrom-Json) $fixtureRepo
    $legacyJson = $legacy | ConvertTo-Json -Depth 12 -Compress
    $rustJson = $rust | ConvertTo-Json -Depth 12 -Compress
    if ($legacyJson -ne $rustJson) {
        throw "compiled CodeNexus active-path output drifted from the historical contract"
    }
    if ([int]$rust.summary.recentCommits -ne 0 -or -not [string]::IsNullOrWhiteSpace([string]$rust.sources.dsm) -or -not [string]::IsNullOrWhiteSpace([string]$rust.sources.hotspots)) {
        throw "compiled CodeNexus route admitted an unreachable DSM/hotspot/history branch"
    }

    $facadeText = Get-Content -Raw $facade
    $rustInvocationIndex = $facadeText.IndexOf('$null = & $codeNexusRustCli @codeNexusArgs', [StringComparison]::Ordinal)
    $fallbackCallIndex = if ($rustInvocationIndex -ge 0) {
        $facadeText.IndexOf('Invoke-CodeNexusLiteCompatibilityFallback `', $rustInvocationIndex, [StringComparison]::Ordinal)
    }
    else {
        -1
    }
    if ($facadeText -notmatch '"codenexus", "generate"' -or $rustInvocationIndex -lt 0 -or $fallbackCallIndex -le $rustInvocationIndex) {
        throw "production compatibility facade does not expose Rust-primary generation with an explicit lite fallback"
    }
}

try {
    Push-Location $root
    cargo build -p code-intel --quiet
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    Invoke-RustGeneratorParity
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
