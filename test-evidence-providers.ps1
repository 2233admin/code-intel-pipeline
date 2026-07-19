param(
    [string]$RepoPath = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

$root = Split-Path -Parent $PSCommandPath
$exe = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
$cli = Join-Path $root "target/debug/$exe"
if (-not (Test-Path -LiteralPath $cli -PathType Leaf)) {
    Push-Location $root
    try { & cargo build -p code-intel | Out-Host }
    finally { Pop-Location }
}
Assert-True (Test-Path -LiteralPath $cli -PathType Leaf) "Missing code-intel CLI"

$scratch = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-provider-smoke-" + [guid]::NewGuid().ToString("N"))
$repo = if ([string]::IsNullOrWhiteSpace($RepoPath)) { Join-Path $scratch "repo" } else { (Resolve-Path -LiteralPath $RepoPath).Path }
$artifacts = Join-Path $scratch "artifacts"

try {
    if ([string]::IsNullOrWhiteSpace($RepoPath)) {
        New-Item -ItemType Directory -Force -Path $repo | Out-Null
        & git -C $repo init --quiet
        [IO.File]::WriteAllText((Join-Path $repo "README.md"), "fixture", [Text.UTF8Encoding]::new($false))
        & git -C $repo add README.md
        & git -C $repo -c user.name=code-intel -c user.email=code-intel@example.invalid commit --quiet -m fixture
    }
    $before = @(& git -C $repo status --porcelain=v1 --untracked-files=all)
    $prepared = & (Join-Path $root "Invoke-EvidenceProvider.ps1") -Provider compete -Operation prepare -RepoPath $repo -ArtifactDir $artifacts | ConvertFrom-Json
    $after = @(& git -C $repo status --porcelain=v1 --untracked-files=all)
    Assert-True ($prepared.status -eq "prepared") "Compete prepare did not report prepared"
    Assert-True (($before -join "`n") -eq ($after -join "`n")) "Compete prepare modified the target repository"

    $competeSchema = Join-Path $root "orchestration/schemas/code-intel-compete-native-result.v1.schema.json"
    $reactSchema = Join-Path $root "orchestration/schemas/code-intel-react-doctor-native-result.v1.schema.json"
    $routeSchema = Join-Path $root "orchestration/schemas/code-intel-evidence-route-result.v1.schema.json"
    $featureSchema = Join-Path $root "orchestration/schemas/code-intel-beta-feature-report.v1.schema.json"
    $competeNative = Get-Content -LiteralPath $prepared.nativeResult -Raw
    Assert-True (Test-Json -Json $competeNative -SchemaFile $competeSchema) "Prepared Compete native result is schema-invalid"

    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $reactNativePath = Join-Path $artifacts "react-doctor-native-result.json"
    $reactNative = [ordered]@{
        schema = "code-intel-react-doctor-native-result.v1"
        snapshotIdentity = ($competeNative | ConvertFrom-Json).snapshotIdentity
        status = "provider_unavailable"
        observedAt = $now
        tool = [ordered]@{
            version = "0.7.8"
            integrity = "sha512-G3spmtZJE/gWWPRJ3rpgUWTPRDJpEmdRja7iNZ7RAXlfpEO+NWVzPTca/cPI9hLwPo2Aq5/BZggo5JDBrwGrlA=="
            command = @("npx", "--yes", "react-doctor@0.7.8", "--json", "--no-telemetry")
        }
        report = $null
        error = "npm unavailable"
    }
    $reactJson = $reactNative | ConvertTo-Json -Depth 10
    [IO.File]::WriteAllText($reactNativePath, $reactJson, [Text.UTF8Encoding]::new($false))
    Assert-True (Test-Json -Json $reactJson -SchemaFile $reactSchema) "React Doctor native fixture is schema-invalid"

    $routeRaw = & $cli provider react-doctor-adapt --request $reactNativePath --artifact-root $artifacts --evaluated-at $now --max-age-seconds 60
    Assert-True ($LASTEXITCODE -eq 0) "React Doctor public adapt command failed"
    $routeText = $routeRaw -join "`n"
    $route = $routeText | ConvertFrom-Json
    Assert-True (Test-Json -Json $routeText -SchemaFile $routeSchema) "React Doctor route result is schema-invalid"
    Assert-True ($route.status -eq "unknown" -and $route.failureCategory -eq "provider_unavailable") "Provider unavailable semantics drifted"
    $routePath = Join-Path $artifacts "react-doctor-route-fixture.json"
    [IO.File]::WriteAllText($routePath, $routeText, [Text.UTF8Encoding]::new($false))

    $featureBuild = & (Join-Path $root "Invoke-CodeIntelBetaFeature.ps1") `
        -Feature react-diagnostics `
        -Operation build `
        -ArtifactDir $artifacts `
        -RouteResult $routePath | ConvertFrom-Json
    $featureReportPath = $featureBuild.artifacts.json
    $featureReportText = Get-Content -LiteralPath $featureReportPath -Raw
    $featureReport = $featureReportText | ConvertFrom-Json
    Assert-True (Test-Json -Json $featureReportText -SchemaFile $featureSchema) "Beta feature report is schema-invalid"
    Assert-True ($featureReport.feature -eq "react-diagnostics" -and $featureReport.beta) "Beta feature identity drifted"
    Assert-True ($featureReport.status -eq "unknown" -and @($featureReport.issues).Count -eq 0) "Unknown provider result claimed healthy diagnostics"
    Assert-True ($null -eq $featureReport.PSObject.Properties["score"]) "Beta feature reports must not expose a score"

    $features = (& $cli feature --action List --json | ConvertFrom-Json).features
    Assert-True (@($features).Count -eq 2) "Expected two first-party Beta features"
    foreach ($featureName in @("competitive-intelligence", "react-diagnostics")) {
        $feature = @($features | Where-Object { $_.id -eq $featureName })
        Assert-True ($feature.Count -eq 1) "Missing Beta feature $featureName"
        Assert-True (-not [bool]$feature[0].required) "$featureName must remain explicit and optional"
    }

    $stdinRouteRaw = $reactJson | & $cli provider react-doctor-adapt --request - --artifact-root $artifacts --evaluated-at $now --max-age-seconds 60
    Assert-True ($LASTEXITCODE -eq 0) "React Doctor stdin adapt command failed"
    $stdinRoute = ($stdinRouteRaw -join "`n") | ConvertFrom-Json
    Assert-True ($stdinRoute.failureCategory -eq "provider_unavailable") "Stdin native result changed failure semantics"

    $providers = (& $cli provider --action List --json | ConvertFrom-Json).operations
    foreach ($pair in @(
        @("compete", "prepare"),
        @("compete", "status"),
        @("compete", "adapt"),
        @("react-doctor", "scan"),
        @("react-doctor", "adapt")
    )) {
        $operation = @($providers | Where-Object { $_.provider -eq $pair[0] -and $_.operation -eq $pair[1] })
        Assert-True ($operation.Count -eq 1) "Missing provider operation $($pair -join '/')"
        Assert-True (-not [bool]$operation[0].required) "$($pair -join '/') must remain optional"
    }

    $manifest = & $cli orchestrate --action Plan --capability advisory_evidence --mode normal --json | ConvertFrom-Json
    Assert-True (@($manifest.plan).Count -eq 4) "Expected two providers and two first-party Beta feature integrations"
    Assert-True (-not (@($manifest.plan) | Where-Object { [bool]$_.required })) "Advisory evidence integrations must remain optional"
    $featurePlan = & $cli orchestrate --action Plan --capability beta_features --mode normal --json | ConvertFrom-Json
    Assert-True (@($featurePlan.plan).Count -eq 2) "Expected two first-party Beta feature plans"

    $runner = Get-Content -LiteralPath (Join-Path $root "run-code-intel.ps1") -Raw
    Assert-True ($runner -notmatch "Invoke-EvidenceProvider|react-doctor|compete-adapt") "Default pipeline must not auto-run advisory providers"
    $adapter = Get-Content -LiteralPath (Join-Path $root "Invoke-EvidenceProvider.ps1") -Raw
    Assert-True ($adapter -match "react-doctor@0\.7\.8 --json --no-telemetry") "Pinned React Doctor invocation flags drifted"
    Assert-True ($adapter -match "ProjectNotFoundError") "Non-React repositories must remain not_applicable"
    Assert-True ($adapter -notmatch "react-doctor@[^`r`n]+(?:ci\s+install|\sinstall)") "React Doctor installer must not be invoked"

    [ordered]@{
        ok = $true
        providers = 2
        operations = 5
        betaFeatures = 2
        schemas = 4
        targetRepoWrites = 0
        defaultPipelineInvocations = 0
    } | ConvertTo-Json
}
finally {
    if (Test-Path -LiteralPath $scratch) {
        Remove-Item -LiteralPath $scratch -Recurse -Force
    }
}
