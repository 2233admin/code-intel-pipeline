#requires -Version 7.2

[CmdletBinding()]
param(
    [ValidateRange(100, 30000)]
    [int]$TimeoutMs = 3000,
    [string[]]$ClaudeCandidates = @(),
    [string[]]$CodexCandidates = @(),
    [string[]]$OpenCodeCandidates = @(),
    [string[]]$OllamaCandidates = @(),
    [string[]]$CcSwitchCandidates = @(),
    [ValidateSet("", "openai", "anthropic")]
    [string]$UserProvider = "",
    [string]$UserModel = "",
    [string]$UserBaseUrl = "",
    [string]$UserCredentialEnvName = "",
    [ValidateSet("local_compute", "subscription_cli", "free_or_internal_quota", "metered_api")]
    [string]$UserCostScope = "local_compute",
    [switch]$ProbeUserEndpoint,
    [string]$UserModelCatalogFixture = "",
    [switch]$UseOnlyProvidedCandidates,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Protect-InventoryText {
    param([AllowEmptyString()][string]$Text)

    $safe = [string]$Text
    $safe = $safe -replace '(?i)Bearer\s+[A-Za-z0-9._~+/-]+', 'Bearer [REDACTED]'
    $safe = $safe -replace '(?i)\b(sk|key|token|secret)-[A-Za-z0-9._~+/-]{4,}\b', '[REDACTED]'
    $safe = $safe -replace '(?i)(api[_-]?key|auth[_-]?token|access[_-]?token|secret)\s*[:=]\s*[^\s,;]+', '$1=[REDACTED]'
    $safe = $safe -replace '[\r\n]+', ' '
    if ($safe.Length -gt 160) { $safe = $safe.Substring(0, 160) }
    return $safe.Trim()
}

function Get-UniqueCandidatePaths {
    param(
        [string]$CommandName,
        [string[]]$Provided,
        [string[]]$KnownPaths = @()
    )

    $items = [System.Collections.Generic.List[string]]::new()
    foreach ($item in @($Provided)) {
        if (-not [string]::IsNullOrWhiteSpace($item)) { $items.Add($item) }
    }
    if (-not $UseOnlyProvidedCandidates) {
        foreach ($command in @(Get-Command $CommandName -All -ErrorAction SilentlyContinue)) {
            $path = ""
            if ($command.PSObject.Properties.Name -contains "Path" -and -not [string]::IsNullOrWhiteSpace([string]$command.Path)) {
                $path = [string]$command.Path
            }
            elseif ($command.PSObject.Properties.Name -contains "Source") {
                $path = [string]$command.Source
            }
            if (-not [string]::IsNullOrWhiteSpace($path)) { $items.Add($path) }
        }
        foreach ($path in $KnownPaths) {
            if (-not [string]::IsNullOrWhiteSpace($path)) { $items.Add($path) }
        }
    }

    return @($items | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -Unique)
}

function Invoke-BoundedExecutable {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string[]]$Arguments
    )

    $extension = [IO.Path]::GetExtension($Path).ToLowerInvariant()
    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true

    if ($extension -eq ".ps1") {
        $pwsh = (Get-Process -Id $PID).Path
        $start.FileName = $pwsh
        foreach ($prefix in @("-NoLogo", "-NoProfile", "-NonInteractive", "-File", $Path)) {
            [void]$start.ArgumentList.Add($prefix)
        }
    }
    elseif ($extension -in @(".cmd", ".bat")) {
        if ($Path.IndexOfAny([char[]]'"&|<>^') -ge 0) {
            return [pscustomobject][ordered]@{ ok = $false; timed_out = $false; exit_code = -1; stdout = ""; category = "local_tool_error" }
        }
        $start.FileName = $env:ComSpec
        foreach ($prefix in @("/d", "/s", "/c", ('"{0}"' -f $Path))) {
            [void]$start.ArgumentList.Add($prefix)
        }
    }
    else {
        $start.FileName = $Path
    }
    foreach ($argument in $Arguments) { [void]$start.ArgumentList.Add($argument) }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        [void]$process.Start()
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutMs)) {
            try { $process.Kill($true) } catch { }
            [void]$process.WaitForExit(1000)
            return [pscustomobject][ordered]@{ ok = $false; timed_out = $true; exit_code = -1; stdout = ""; category = "local_tool_error" }
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        [void]$stderrTask.GetAwaiter().GetResult()
        return [pscustomobject][ordered]@{
            ok = ($process.ExitCode -eq 0)
            timed_out = $false
            exit_code = $process.ExitCode
            stdout = [string]$stdout
            category = if ($process.ExitCode -eq 0) { "" } else { "local_tool_error" }
        }
    }
    catch {
        return [pscustomobject][ordered]@{ ok = $false; timed_out = $false; exit_code = -1; stdout = ""; category = "local_tool_error" }
    }
    finally {
        $process.Dispose()
    }
}

function Get-VersionCandidates {
    param(
        [string[]]$Paths,
        [string[]]$Arguments = @("--version")
    )

    $results = [System.Collections.Generic.List[object]]::new()
    foreach ($path in $Paths) {
        $probe = Invoke-BoundedExecutable -Path $path -Arguments $Arguments
        $version = ""
        if ($probe.ok) {
            $firstLine = @(([string]$probe.stdout -split '\r?\n') | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 1)
            if ($firstLine.Count -gt 0) { $version = Protect-InventoryText ([string]$firstLine[0]) }
        }
        $results.Add([pscustomobject][ordered]@{
            path = [IO.Path]::GetFullPath($path)
            executable_verified = [bool]$probe.ok
            timed_out = [bool]$probe.timed_out
            version = $version
            category = [string]$probe.category
        })
    }
    return @($results)
}

function Get-FirstVerifiedCandidate {
    param([object[]]$Candidates)
    return @($Candidates | Where-Object { $_.executable_verified } | Select-Object -First 1)
}

function Get-Presence {
    param([string[]]$Names)
    foreach ($name in $Names) {
        if (-not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name, "Process")) -or
            -not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name, "User"))) {
            return $true
        }
    }
    return $false
}

function Test-LocalEndpointUri {
    param([string]$Value)
    $uri = $null
    if (-not [Uri]::TryCreate($Value, [UriKind]::Absolute, [ref]$uri)) { return $false }
    if ($uri.Scheme -notin @("http", "https")) { return $false }
    return $uri.IsLoopback -or $uri.Host -ieq "localhost"
}

function Get-UserEndpointProbe {
    param(
        [string]$Provider,
        [string]$BaseUrl,
        [string]$Model,
        [string]$CredentialEnvName,
        [string]$CatalogFixture
    )

    $authState = "not_applicable"
    $credential = ""
    if (-not [string]::IsNullOrWhiteSpace($CredentialEnvName)) {
        if ($CredentialEnvName -notmatch '^[A-Za-z_][A-Za-z0-9_]*$') {
            return [pscustomobject]@{ verified = $false; auth = "unknown"; model = "unknown"; diagnostic = "endpoint_probe_failed" }
        }
        $credential = [Environment]::GetEnvironmentVariable($CredentialEnvName, "Process")
        if ([string]::IsNullOrWhiteSpace($credential)) { $credential = [Environment]::GetEnvironmentVariable($CredentialEnvName, "User") }
        $authState = if ([string]::IsNullOrWhiteSpace($credential)) { "absent" } else { "present" }
    }

    try {
        $catalog = $null
        if (-not [string]::IsNullOrWhiteSpace($CatalogFixture)) {
            if (-not $UseOnlyProvidedCandidates) { throw "catalog fixtures are test-mode only" }
            $catalog = Get-Content -LiteralPath $CatalogFixture -Raw -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
        }
        else {
            if (-not (Test-LocalEndpointUri $BaseUrl)) {
                return [pscustomobject]@{ verified = $false; auth = $authState; model = "unknown"; diagnostic = "endpoint_probe_failed" }
            }
            $headers = @{}
            if (-not [string]::IsNullOrWhiteSpace($credential)) {
                if ($Provider -eq "anthropic") {
                    $headers["x-api-key"] = $credential
                    $headers["anthropic-version"] = "2023-06-01"
                }
                else {
                    $headers["Authorization"] = "Bearer $credential"
                }
            }
            $catalogUri = ([Uri]::new(([Uri]$BaseUrl), "models")).AbsoluteUri
            $catalog = Invoke-RestMethod -Method Get -Uri $catalogUri -Headers $headers -TimeoutSec ([Math]::Max(1, [Math]::Ceiling($TimeoutMs / 1000))) -ErrorAction Stop
        }

        $ids = @($catalog.data | ForEach-Object { [string]$_.id } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        $modelState = if ([string]::IsNullOrWhiteSpace($Model)) { "unknown" } elseif ($ids -contains $Model) { "available" } else { "unavailable" }
        return [pscustomobject]@{
            verified = $true
            auth = $authState
            model = $modelState
            diagnostic = if ($modelState -eq "unavailable") { "model_not_in_catalog" } else { "endpoint_probe_passed" }
        }
    }
    catch {
        return [pscustomobject]@{ verified = $false; auth = $authState; model = "unknown"; diagnostic = "endpoint_probe_failed" }
    }
    finally {
        $credential = ""
    }
}

function New-CandidateObservation {
    param(
        [string]$Id,
        [string]$ChannelKind,
        [AllowNull()][object]$Provider,
        [AllowNull()][object]$Model,
        [string]$CostScope,
        [bool]$EndpointConfigured,
        [bool]$Discovered,
        [bool]$ExecutableVerified,
        [string]$AuthPresent,
        [string]$ModelAvailable,
        [bool]$ExternalEgress,
        [string]$Source,
        [string[]]$Diagnostics
    )
    return [pscustomobject][ordered]@{
        id = $Id
        channelKind = $ChannelKind
        provider = $Provider
        model = $Model
        costScope = $CostScope
        endpointConfigured = $EndpointConfigured
        discovered = $Discovered
        executableVerified = $ExecutableVerified
        authPresent = $AuthPresent
        modelAvailable = $ModelAvailable
        externalEgress = $ExternalEgress
        source = $Source
        diagnostics = @($Diagnostics | Select-Object -Unique)
    }
}

function Get-CandidateDiagnostics {
    param([object[]]$Candidates)
    $diagnostics = [System.Collections.Generic.List[string]]::new()
    $sawFailure = $false
    for ($index = 0; $index -lt $Candidates.Count; $index++) {
        $candidate = $Candidates[$index]
        if ($candidate.executable_verified) {
            $diagnostics.Add("candidate_verified")
            $diagnostics.Add("version_probe_passed")
            if ($sawFailure) { $diagnostics.Add("fallback_candidate_selected") }
        }
        elseif ($candidate.timed_out) {
            $diagnostics.Add("candidate_timed_out")
            $sawFailure = $true
        }
        else {
            $diagnostics.Add("candidate_verification_failed")
            $sawFailure = $true
        }
    }
    return @($diagnostics | Select-Object -Unique)
}

$localBin = if (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) { Join-Path $env:USERPROFILE ".local\bin" } else { "" }
$localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$appData = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)

$claudePaths = @(Get-UniqueCandidatePaths -CommandName "claude" -Provided $ClaudeCandidates -KnownPaths @(
    $(if ($localBin) { Join-Path $localBin "claude.cmd" })
))
$codexPaths = @(Get-UniqueCandidatePaths -CommandName "codex" -Provided $CodexCandidates)
$openCodePaths = @(Get-UniqueCandidatePaths -CommandName "opencode" -Provided $OpenCodeCandidates -KnownPaths @(
    $(if ($localAppData) { Join-Path $localAppData "OpenCode\opencode-cli.exe" })
))
$ollamaPaths = @(Get-UniqueCandidatePaths -CommandName "ollama" -Provided $OllamaCandidates)
$ccSwitchPaths = @(Get-UniqueCandidatePaths -CommandName "cc-switch" -Provided $CcSwitchCandidates -KnownPaths @(
    $(if ($localAppData) { Join-Path $localAppData "Programs\CC Switch\cc-switch.exe" })
))

$claudeCandidateResults = @(Get-VersionCandidates -Paths $claudePaths)
$codexCandidateResults = @(Get-VersionCandidates -Paths $codexPaths)
$openCodeCandidateResults = @(Get-VersionCandidates -Paths $openCodePaths)
$ollamaCandidateResults = @(Get-VersionCandidates -Paths $ollamaPaths)

$claudeSelected = @(Get-FirstVerifiedCandidate $claudeCandidateResults)
$codexSelected = @(Get-FirstVerifiedCandidate $codexCandidateResults)
$openCodeSelected = @(Get-FirstVerifiedCandidate $openCodeCandidateResults)
$ollamaSelected = @(Get-FirstVerifiedCandidate $ollamaCandidateResults)

$claudeAuth = $false
$claudeAuthMethod = "unknown"
if ($claudeSelected.Count -eq 1) {
    $auth = Invoke-BoundedExecutable -Path $claudeSelected[0].path -Arguments @("auth", "status", "--json")
    if ($auth.ok) {
        try {
            $authJson = ([string]$auth.stdout | ConvertFrom-Json -ErrorAction Stop)
            $claudeAuth = [bool]$authJson.authenticated
            if ($claudeAuth -and $authJson.PSObject.Properties.Name -contains "authMethod") {
                $allowedMethods = @("api_key", "oauth", "subscription", "unknown")
                $candidateMethod = ([string]$authJson.authMethod).ToLowerInvariant()
                if ($candidateMethod -in $allowedMethods) { $claudeAuthMethod = $candidateMethod }
            }
        }
        catch { $claudeAuth = $false }
    }
}

$codexAuth = $false
if ($codexSelected.Count -eq 1) {
    $login = Invoke-BoundedExecutable -Path $codexSelected[0].path -Arguments @("login", "status")
    $codexAuth = [bool]$login.ok
}

$ollamaModels = @()
if ($ollamaSelected.Count -eq 1) {
    $list = Invoke-BoundedExecutable -Path $ollamaSelected[0].path -Arguments @("list")
    if ($list.ok) {
        $ollamaModels = @(([string]$list.stdout -split '\r?\n') |
            Select-Object -Skip 1 |
            ForEach-Object { @($_ -split '\s+')[0] } |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
            ForEach-Object { Protect-InventoryText $_ } |
            Select-Object -Unique)
    }
}

$ccConfigPresent = $false
if (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) {
    $ccRoot = Join-Path $env:USERPROFILE ".cc-switch"
    $ccConfigPresent = (Test-Path -LiteralPath (Join-Path $ccRoot "settings.json") -PathType Leaf) -or
        (Test-Path -LiteralPath (Join-Path $ccRoot "cc-switch.db") -PathType Leaf)
}

$claudeConfigPresent = (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) -and
    (Test-Path -LiteralPath (Join-Path $env:USERPROFILE ".claude\settings.json") -PathType Leaf)
$codexConfigPresent = (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) -and
    (Test-Path -LiteralPath (Join-Path $env:USERPROFILE ".codex\config.toml") -PathType Leaf)
$openCodeConfigPresent = (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) -and
    (Test-Path -LiteralPath (Join-Path $env:USERPROFILE ".config\opencode\opencode.json") -PathType Leaf)

$openAiEndpoint = Get-Presence @("OPENAI_BASE_URL", "CODE_INTEL_BASE_URL")
$anthropicEndpoint = Get-Presence @("ANTHROPIC_BASE_URL", "CODE_INTEL_ANTHROPIC_BASE_URL")

$candidates = [System.Collections.Generic.List[object]]::new()

if (-not [string]::IsNullOrWhiteSpace($UserProvider) -or -not [string]::IsNullOrWhiteSpace($UserModel) -or -not [string]::IsNullOrWhiteSpace($UserBaseUrl)) {
    $providerValue = if ([string]::IsNullOrWhiteSpace($UserProvider)) { $null } else { $UserProvider }
    $modelValue = if ([string]::IsNullOrWhiteSpace($UserModel)) { $null } else { Protect-InventoryText $UserModel }
    $userUri = $null
    $userUriValid = [Uri]::TryCreate($UserBaseUrl, [UriKind]::Absolute, [ref]$userUri)
    $userExternalEgress = $userUriValid -and -not ($userUri.IsLoopback -or $userUri.Host -ieq "localhost")
    $userProbe = [pscustomobject]@{ verified = $false; auth = "unknown"; model = "unknown"; diagnostic = "endpoint_not_probed" }
    if ($ProbeUserEndpoint) {
        $userProbe = Get-UserEndpointProbe -Provider $UserProvider -BaseUrl $UserBaseUrl -Model $UserModel -CredentialEnvName $UserCredentialEnvName -CatalogFixture $UserModelCatalogFixture
    }
    $candidates.Add((New-CandidateObservation -Id "user_local_compatible" -ChannelKind "local_compatible" -Provider $providerValue -Model $modelValue -CostScope $UserCostScope -EndpointConfigured (-not [string]::IsNullOrWhiteSpace($UserBaseUrl)) -Discovered $true -ExecutableVerified ([bool]$userProbe.verified) -AuthPresent ([string]$userProbe.auth) -ModelAvailable ([string]$userProbe.model) -ExternalEgress $userExternalEgress -Source "user_input" -Diagnostics @(
        $(if (-not [string]::IsNullOrWhiteSpace($UserBaseUrl)) { "endpoint_configured" } else { "endpoint_not_configured" }),
        $(if ($null -ne $modelValue) { "model_declared_by_user" } else { "model_not_declared" }),
        [string]$userProbe.diagnostic
    )))
}
else {
    foreach ($endpoint in @(
        @{ id = "local_openai_compatible"; provider = "openai"; present = $openAiEndpoint },
        @{ id = "local_anthropic_compatible"; provider = "anthropic"; present = $anthropicEndpoint }
    )) {
        if ($endpoint.present) {
            # The endpoint value is intentionally not collected. Since locality therefore
            # cannot be proven, classify the channel conservatively as external egress.
            $candidates.Add((New-CandidateObservation -Id $endpoint.id -ChannelKind "local_compatible" -Provider $endpoint.provider -Model $null -CostScope "local_compute" -EndpointConfigured $true -Discovered $true -ExecutableVerified $false -AuthPresent "unknown" -ModelAvailable "unknown" -ExternalEgress $true -Source "local_discovery" -Diagnostics @("endpoint_configured", "endpoint_value_not_collected", "external_egress_assumed")))
        }
    }
}

if ($ollamaModels.Count -gt 0) {
    foreach ($ollamaModel in $ollamaModels) {
        $idSuffix = ([string]$ollamaModel -replace '[^A-Za-z0-9._-]', '_').ToLowerInvariant()
        $ollamaDiagnostics = @((Get-CandidateDiagnostics $ollamaCandidateResults)) + @("model_catalog_observed")
        $candidates.Add((New-CandidateObservation -Id "ollama_$idSuffix" -ChannelKind "ollama" -Provider "ollama" -Model ([string]$ollamaModel) -CostScope "local_compute" -EndpointConfigured $true -Discovered $true -ExecutableVerified $true -AuthPresent "not_applicable" -ModelAvailable "available" -ExternalEgress $false -Source "local_discovery" -Diagnostics $ollamaDiagnostics))
    }
}
else {
    $candidates.Add((New-CandidateObservation -Id "ollama_runtime" -ChannelKind "ollama" -Provider "ollama" -Model $null -CostScope "local_compute" -EndpointConfigured ($ollamaSelected.Count -eq 1) -Discovered ($ollamaPaths.Count -gt 0) -ExecutableVerified ($ollamaSelected.Count -eq 1) -AuthPresent "not_applicable" -ModelAvailable "unavailable" -ExternalEgress $false -Source "local_discovery" -Diagnostics (Get-CandidateDiagnostics $ollamaCandidateResults)))
}

$claudeDiagnostics = @((Get-CandidateDiagnostics $claudeCandidateResults)) + @("auth_method_$claudeAuthMethod")
$candidates.Add((New-CandidateObservation -Id "claude_cli" -ChannelKind "claude_cli" -Provider "anthropic" -Model $null -CostScope "subscription_cli" -EndpointConfigured $claudeConfigPresent -Discovered ($claudePaths.Count -gt 0) -ExecutableVerified ($claudeSelected.Count -eq 1) -AuthPresent $(if ($claudeAuth) { "present" } elseif ($claudeSelected.Count -eq 1) { "absent" } else { "unknown" }) -ModelAvailable "unknown" -ExternalEgress $true -Source "cli_config" -Diagnostics $claudeDiagnostics))
$candidates.Add((New-CandidateObservation -Id "opencode_cli" -ChannelKind "opencode_cli" -Provider $null -Model $null -CostScope "subscription_cli" -EndpointConfigured $openCodeConfigPresent -Discovered ($openCodePaths.Count -gt 0) -ExecutableVerified ($openCodeSelected.Count -eq 1) -AuthPresent "unknown" -ModelAvailable "unknown" -ExternalEgress $true -Source "cli_config" -Diagnostics (Get-CandidateDiagnostics $openCodeCandidateResults)))
$candidates.Add((New-CandidateObservation -Id "codex_cli" -ChannelKind "codex_cli" -Provider "openai" -Model $null -CostScope "subscription_cli" -EndpointConfigured $codexConfigPresent -Discovered ($codexPaths.Count -gt 0) -ExecutableVerified ($codexSelected.Count -eq 1) -AuthPresent $(if ($codexAuth) { "present" } elseif ($codexSelected.Count -eq 1) { "absent" } else { "unknown" }) -ModelAvailable "unknown" -ExternalEgress $true -Source "cli_config" -Diagnostics (Get-CandidateDiagnostics $codexCandidateResults)))

$result = [pscustomobject][ordered]@{
    schema = "code-intel-model-channel-inventory-result.v1"
    candidates = @($candidates)
    configurationBrokers = @(
        [pscustomobject][ordered]@{
            id = "cc_switch"
            kind = "cc_switch"
            discovered = (($ccSwitchPaths.Count -gt 0) -or $ccConfigPresent)
            configPresent = $ccConfigPresent
            diagnostics = @(
                $(if ($ccSwitchPaths.Count -gt 0) { "installation_present" } else { "installation_not_found" }),
                $(if ($ccConfigPresent) { "config_present_content_not_read" } else { "config_not_found" })
            )
        },
        [pscustomobject][ordered]@{
            id = "manual_config"
            kind = "manual_config"
            discovered = ($openAiEndpoint -or $anthropicEndpoint -or -not [string]::IsNullOrWhiteSpace($UserBaseUrl))
            configPresent = ($openAiEndpoint -or $anthropicEndpoint -or -not [string]::IsNullOrWhiteSpace($UserBaseUrl))
            diagnostics = @("presence_only", "credential_values_not_collected", "endpoint_values_not_collected")
        }
    )
}

if ($Json) { $result | ConvertTo-Json -Depth 8 }
else { $result }
