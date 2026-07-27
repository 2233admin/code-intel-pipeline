#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Request,
    [Parameter(Mandatory = $true)]
    [string]$ArtifactRoot,
    [switch]$AllowLegacyRawExecutable
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Byte budget for a single local-process delegate's stdout (see
# Read-BoundedProcessOutput below). A runaway or looping model stream must be
# caught here, mid-read, well under the artifact-publish size ceiling
# enforced later in the pipeline (docs/artifact-data-contract.md) — that
# later check cannot help because it only runs after the bytes are already
# buffered in full and written to disk.
$maxDelegateOutputBytes = 4MB

function Get-RequiredProperty {
    param([object]$Object, [string]$Name)
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { throw "delegate request is missing '$Name'" }
    return $property.Value
}

function Test-ConsentGranted {
    param([object]$Consent, [string]$Name)
    if ($null -eq $Consent) { return $false }
    $property = $Consent.PSObject.Properties[$Name]
    return ($null -ne $property -and [string]$property.Value -eq "granted")
}

function Write-Result {
    param([hashtable]$Value, [string]$Path)
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
    $Value | ConvertTo-Json -Depth 12 -Compress
}

function Get-FailureCategory {
    param([string]$Text)
    if ($Text -match '(?i)quota|rate.?limit|too many requests|\b429\b') { return "provider_quota" }
    if ($Text -match '(?i)model.?not.?found|provider.?unavailable|service.?unavailable|\b404\b|\b503\b') { return "provider_unavailable" }
    if ($Text -match '(?i)unauthori[sz]ed|forbidden|invalid.?api.?key|authentication|\b401\b|\b403\b') { return "config_error" }
    return "local_tool_error"
}

# The HTTP status code is authoritative when it is present: it comes from the
# transport, not from text a provider chose to put in a response body. A 500
# whose body happens to mention "quota" must not be reclassified as
# provider_quota by the regex-over-body path below — that would treat a hard
# failure as retry-eligible transient one. Every non-success HTTP status maps
# to exactly one category here, so the HTTP call site never needs to fall
# through to Get-FailureCategory (that regex path stays reserved for the
# local-process exit-code path, which has no status code to consult).
function Get-HttpFailureCategory {
    param([int]$StatusCode)
    if ($StatusCode -eq 429) { return "provider_quota" }
    if ($StatusCode -eq 401 -or $StatusCode -eq 403) { return "config_error" }
    if ($StatusCode -eq 404 -or $StatusCode -eq 503) { return "provider_unavailable" }
    return "local_tool_error"
}

function Read-BoundedProcessOutput {
    # Reads $Reader in chunks instead of ReadToEndAsync, so a looping or
    # runaway model process cannot force this delegate to buffer an unbounded
    # amount of output in memory (and later on disk) before anything notices.
    # Stops at the first of: end of stream, $MaxBytes exceeded, or $Deadline
    # reached. The byte budget is approximated via the reader's own
    # CurrentEncoding so this introduces no decoding-behavior change versus
    # the ReadToEndAsync it replaces. Never kills the process itself — the
    # caller owns that decision so it can also fold in the local-process
    # ExitCode/WaitForExit sequence the rest of this script already uses.
    param(
        [IO.StreamReader]$Reader,
        [long]$MaxBytes,
        [datetime]$Deadline
    )
    $buffer = [char[]]::new(16384)
    $text = [Text.StringBuilder]::new()
    $byteCount = 0L
    $truncated = $false
    $timedOut = $false
    while ($true) {
        $remaining = $Deadline - [DateTime]::UtcNow
        if ($remaining -le [TimeSpan]::Zero) { $timedOut = $true; break }
        $readTask = $Reader.ReadAsync($buffer, 0, $buffer.Length)
        if (-not $readTask.Wait($remaining)) { $timedOut = $true; break }
        $charsRead = $readTask.GetAwaiter().GetResult()
        if ($charsRead -le 0) { break }
        [void]$text.Append($buffer, 0, $charsRead)
        $byteCount += $Reader.CurrentEncoding.GetByteCount($buffer, 0, $charsRead)
        if ($byteCount -gt $MaxBytes) { $truncated = $true; break }
    }
    [PSCustomObject]@{
        Text      = $text.ToString()
        Truncated = $truncated
        TimedOut  = $timedOut
    }
}

function Test-StructuredOutput {
    param([string]$Text, [string]$Format)
    if ([string]::IsNullOrWhiteSpace($Text)) { return $false }
    if ($Format -eq "json") {
        try { [void]($Text | ConvertFrom-Json -ErrorAction Stop); return $true } catch { return $false }
    }
    foreach ($line in @($Text -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })) {
        try { [void]($line | ConvertFrom-Json -ErrorAction Stop) } catch { return $false }
    }
    return $true
}

function Assert-NoDuplicateJsonKeys {
    param([System.Text.Json.JsonElement]$Element, [string]$Path = '$')
    if ($Element.ValueKind -eq [System.Text.Json.JsonValueKind]::Object) {
        $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($property in $Element.EnumerateObject()) {
            if (-not $names.Add($property.Name)) { throw "duplicate JSON key at $Path.$($property.Name)" }
            Assert-NoDuplicateJsonKeys -Element $property.Value -Path "$Path.$($property.Name)"
        }
    }
    elseif ($Element.ValueKind -eq [System.Text.Json.JsonValueKind]::Array) {
        $index = 0
        foreach ($item in $Element.EnumerateArray()) {
            Assert-NoDuplicateJsonKeys -Element $item -Path "$Path[$index]"
            $index++
        }
    }
}

function Resolve-VerifiedExecutableHandle {
    param([string]$HandlePath, [string]$ExpectedAdapter)
    try { $resolvedHandle = (Resolve-Path -LiteralPath $HandlePath -ErrorAction Stop).Path }
    catch { throw "cannot read executable handle" }
    $raw = Get-Content -LiteralPath $resolvedHandle -Raw
    try { $document = [System.Text.Json.JsonDocument]::Parse($raw) }
    catch { throw "executable handle is not valid JSON" }
    try {
        Assert-NoDuplicateJsonKeys -Element $document.RootElement
        $observedText = $document.RootElement.GetProperty("observedAt").GetString()
        $expiresText = $document.RootElement.GetProperty("expiresAt").GetString()
    }
    finally { $document.Dispose() }
    $handle = $raw | ConvertFrom-Json -ErrorAction Stop
    $fields = @("schema", "adapter", "executablePath", "sha256", "length", "lastWriteTimeUtcTicks", "observedAt", "expiresAt", "identity")
    if ((@($handle.PSObject.Properties.Name | Sort-Object) -join "`n") -ne (($fields | Sort-Object) -join "`n")) { throw "executable handle has an unsupported shape" }
    if ([string]$handle.schema -ne "code-intel-model-executable-handle.v1") { throw "executable handle schema is unsupported" }
    if ([string]$handle.adapter -ne $ExpectedAdapter) { throw "executable handle adapter mismatch" }
    $observedAt = [DateTimeOffset]::MinValue
    $expiresAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParseExact($observedText, "O", [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::None, [ref]$observedAt) -or
        -not [DateTimeOffset]::TryParseExact($expiresText, "O", [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::None, [ref]$expiresAt)) { throw "executable handle time is invalid" }
    if ($expiresAt -le $observedAt -or [DateTimeOffset]::UtcNow -gt $expiresAt) { throw "executable handle is expired" }
    $path = (Resolve-Path -LiteralPath ([string]$handle.executablePath) -ErrorAction Stop).Path
    $item = Get-Item -LiteralPath $path -ErrorAction Stop
    if ($item.PSIsContainer) { throw "executable handle does not name a file" }
    $sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($sha256 -ne [string]$handle.sha256 -or [long]$item.Length -ne [long]$handle.length -or [long]$item.LastWriteTimeUtc.Ticks -ne [long]$handle.lastWriteTimeUtcTicks) { throw "executable handle content binding is stale" }
    $material = @(
        [string]$handle.schema, [string]$handle.adapter, $path, $sha256,
        [string][long]$item.Length, [string][long]$item.LastWriteTimeUtc.Ticks,
        $observedText, $expiresText
    ) -join "`n"
    $computed = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($material))).ToLowerInvariant()
    if ($computed -ne [string]$handle.identity) { throw "executable handle identity mismatch" }
    return $path
}

try { $requestPath = (Resolve-Path -LiteralPath $Request -ErrorAction Stop).Path }
catch { [Console]::Error.WriteLine("cannot read model adapter request"); exit 74 }
try { $requestRaw = Get-Content -LiteralPath $requestPath -Raw }
catch { [Console]::Error.WriteLine("cannot read model adapter request"); exit 74 }
try { $document = [System.Text.Json.JsonDocument]::Parse($requestRaw) }
catch { [Console]::Error.WriteLine("model adapter request is not valid JSON"); exit 64 }
try { Assert-NoDuplicateJsonKeys -Element $document.RootElement }
catch { [Console]::Error.WriteLine("model adapter request contains duplicate JSON keys"); exit 65 }
finally { $document.Dispose() }
try { $requestObject = $requestRaw | ConvertFrom-Json -ErrorAction Stop }
catch { [Console]::Error.WriteLine("model adapter request is not valid JSON"); exit 64 }
trap {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 65
}
$isV2 = [string]$requestObject.schema -eq "code-intel-model-adapter-request.v2"
$allowedTopLevel = if ($isV2) {
    @("schema", "adapter", "executableHandle", "endpoint", "protocol", "credentialEnvName", "model", "promptFile", "timeoutSeconds", "consent", "costScope", "externalData", "responseFormat")
} else {
    @("schema", "adapter", "executable", "endpoint", "protocol", "credentialEnvName", "model", "promptFile", "timeoutSeconds", "consent", "costScope", "externalData", "responseFormat")
}
$actualTopLevel = @($requestObject.PSObject.Properties.Name | Sort-Object)
if (($actualTopLevel -join "`n") -ne (($allowedTopLevel | Sort-Object) -join "`n")) { throw "delegate request must contain exactly: $($allowedTopLevel -join ', ')" }
if ([string]$requestObject.schema -notin @("code-intel-model-adapter-request.v1", "code-intel-model-adapter-request.v2")) { throw "delegate request schema is unsupported" }
if ($null -eq $requestObject.consent) { throw "delegate request consent must be an object" }
$consentFields = @("consumption", "localCompute", "paidSpend", "externalData")
if ((@($requestObject.consent.PSObject.Properties.Name | Sort-Object) -join "`n") -ne (($consentFields | Sort-Object) -join "`n")) { throw "delegate consent must contain exactly: $($consentFields -join ', ')" }
foreach ($field in $consentFields) {
    if ([string]$requestObject.consent.$field -notin @("unanswered", "granted", "denied")) { throw "delegate consent '$field' is outside the closed vocabulary" }
}

$adapter = [string](Get-RequiredProperty $requestObject "adapter")
if ($adapter -notin @("claude_cli", "opencode_cli", "codex_cli", "ollama", "local_compatible")) { throw "unsupported delegate adapter '$adapter'" }
$executable = if ($isV2) { "" } elseif ($null -eq $requestObject.executable) { "" } else { [string]$requestObject.executable }
$endpoint = if ($null -eq $requestObject.endpoint) { "" } else { [string]$requestObject.endpoint }
$protocol = if ($null -eq $requestObject.protocol) { "" } else { [string]$requestObject.protocol }
$credentialEnvName = if ($null -eq $requestObject.credentialEnvName) { "" } else { [string]$requestObject.credentialEnvName }
$isHttpAdapter = $adapter -in @("ollama", "local_compatible")
if (-not $isV2 -and -not $isHttpAdapter -and -not $AllowLegacyRawExecutable) {
    throw "legacy raw executable requests are disabled; synthesize a v2 request with a verified executable handle"
}
$model = [string](Get-RequiredProperty $requestObject "model")
if ([string]::IsNullOrWhiteSpace($model)) { throw "delegate request model must be non-empty" }
if ($isHttpAdapter -and [string]::IsNullOrWhiteSpace($endpoint)) { throw "HTTP model adapters require endpoint" }
if ($adapter -eq "ollama" -and $protocol -ne "ollama") { throw "ollama adapter requires protocol=ollama" }
if ($adapter -eq "local_compatible" -and $protocol -notin @("openai", "anthropic")) { throw "local_compatible requires protocol=openai or anthropic" }
if (-not $isV2 -and -not $isHttpAdapter -and [string]::IsNullOrWhiteSpace($executable)) { throw "CLI model adapters require executable" }
if ($isHttpAdapter -and -not [string]::IsNullOrWhiteSpace($executable)) { throw "HTTP model adapters require executable=null" }
if (-not $isHttpAdapter -and (-not [string]::IsNullOrWhiteSpace($endpoint) -or -not [string]::IsNullOrWhiteSpace($protocol) -or -not [string]::IsNullOrWhiteSpace($credentialEnvName))) { throw "CLI model adapters require endpoint, protocol, and credentialEnvName to be null" }
if ($adapter -eq "ollama" -and -not [string]::IsNullOrWhiteSpace($credentialEnvName)) { throw "ollama adapter does not accept a credential environment variable" }
if (-not [string]::IsNullOrWhiteSpace($credentialEnvName) -and $credentialEnvName -notmatch '^[A-Za-z_][A-Za-z0-9_]{0,127}$') { throw "credentialEnvName is invalid" }
$promptFile = [string](Get-RequiredProperty $requestObject "promptFile")
$promptPath = (Resolve-Path -LiteralPath $promptFile -ErrorAction Stop).Path
$timeoutSeconds = if ($requestObject.PSObject.Properties["timeoutSeconds"]) { [int]$requestObject.timeoutSeconds } else { 300 }
if ($timeoutSeconds -lt 1 -or $timeoutSeconds -gt 3600) { throw "timeoutSeconds must be between 1 and 3600" }
$costScope = [string](Get-RequiredProperty $requestObject "costScope")
if ($costScope -notin @("local_compute", "subscription_cli", "free_or_internal_quota", "metered_api")) { throw "unsupported costScope '$costScope'" }
$responseFormat = if ($requestObject.PSObject.Properties["responseFormat"]) { [string]$requestObject.responseFormat } else { "json" }
if ($responseFormat -notin @("json", "jsonl")) { throw "responseFormat must be json or jsonl" }
if ($isHttpAdapter -and $responseFormat -ne "json") { throw "HTTP model adapters require responseFormat=json" }
if ($isV2) {
    $handlePath = if ($null -eq $requestObject.executableHandle) { "" } else { [string]$requestObject.executableHandle }
    if ($isHttpAdapter -and -not [string]::IsNullOrWhiteSpace($handlePath)) { throw "HTTP model adapters require executableHandle=null" }
    if (-not $isHttpAdapter -and [string]::IsNullOrWhiteSpace($handlePath)) { throw "CLI model adapters require executableHandle" }
    if (-not $isHttpAdapter) { $executable = Resolve-VerifiedExecutableHandle -HandlePath $handlePath -ExpectedAdapter $adapter }
}

$artifactDir = [IO.Path]::GetFullPath($ArtifactRoot)
New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null
$resultPath = Join-Path $artifactDir "model-channel-result.json"
$responsePath = Join-Path $artifactDir "model-channel-response.jsonl"

$consent = $requestObject.consent
$denials = [Collections.Generic.List[string]]::new()
$paidDenied = $false
$externalDenied = $false
if (-not (Test-ConsentGranted $consent "consumption")) { $denials.Add("consumption consent is not granted") }
if ($costScope -eq "local_compute" -and -not (Test-ConsentGranted $consent "localCompute")) { $denials.Add("local compute consent is not granted") }
if ($costScope -eq "metered_api" -and -not (Test-ConsentGranted $consent "paidSpend")) { $paidDenied = $true; $denials.Add("paid spend consent is not granted") }
$externalData = [bool]$requestObject.externalData
if ($isHttpAdapter) {
    $endpointUri = $null
    if (-not [Uri]::TryCreate($endpoint, [UriKind]::Absolute, [ref]$endpointUri) -or $endpointUri.Scheme -notin @("http", "https")) { throw "endpoint must be an absolute HTTP(S) URI" }
    if (-not $endpointUri.IsLoopback) { $externalData = $true }
}
elseif ($adapter -in @("claude_cli", "opencode_cli", "codex_cli")) {
    # A CLI cost label does not prove that repository data stays local. Even an
    # OSS/local-compute route can have telemetry or provider resolution outside
    # this adapter's control, so require the external-data grant fail-closed.
    $externalData = $true
}
if ($externalData -and -not (Test-ConsentGranted $consent "externalData")) { $externalDenied = $true; $denials.Add("external data consent is not granted") }

if ($denials.Count -gt 0) {
    Write-Result ([ordered]@{
        schema = "code-intel-model-adapter-result.v1"
        status = "consent_required"
        adapter = $adapter
        category = if ($externalDenied) { "external_data_forbidden" } elseif ($paidDenied) { "paid_usage_forbidden" } else { "consent_required" }
        responseArtifact = $null
        reasons = @($denials)
        attempt = [ordered]@{ invoked = $false; timedOut = $false; exitCode = $null }
    }) $resultPath
    exit 2
}

if (-not $isHttpAdapter -and -not (Test-Path -LiteralPath $executable -PathType Leaf) -and $null -eq (Get-Command $executable -ErrorAction SilentlyContinue)) {
    Write-Result ([ordered]@{
        schema = "code-intel-model-adapter-result.v1"; status = "unavailable"; adapter = $adapter
        category = "provider_unavailable"; responseArtifact = $null; reasons = @("configured executable is unavailable")
        attempt = [ordered]@{ invoked = $false; timedOut = $false; exitCode = $null }
    }) $resultPath
    exit 69
}

if ($isHttpAdapter) {
    $credential = ""
    if (-not [string]::IsNullOrWhiteSpace($credentialEnvName)) {
        $credential = [Environment]::GetEnvironmentVariable($credentialEnvName, "Process")
        if ([string]::IsNullOrWhiteSpace($credential)) { $credential = [Environment]::GetEnvironmentVariable($credentialEnvName, "User") }
        if ([string]::IsNullOrWhiteSpace($credential)) {
            Write-Result ([ordered]@{
                schema = "code-intel-model-adapter-result.v1"; status = "failed"; adapter = $adapter
                category = "config_error"; responseArtifact = $null; reasons = @("configured credential environment variable is absent")
                attempt = [ordered]@{ invoked = $false; timedOut = $false; exitCode = $null }
            }) $resultPath
            exit 69
        }
    }
    $prompt = [IO.File]::ReadAllText($promptPath)
    # Every protocol sends an explicit output-token bound so a looping or
    # runaway completion cannot grow unbounded on the provider side before it
    # ever reaches this process's own stdout/response byte budget below.
    $body = switch ($protocol) {
        "ollama" { [ordered]@{ model = $model; prompt = $prompt; stream = $false; options = [ordered]@{ num_predict = 4096 } } }
        "openai" { [ordered]@{ model = $model; messages = @([ordered]@{ role = "user"; content = $prompt }); stream = $false; max_tokens = 4096 } }
        "anthropic" { [ordered]@{ model = $model; max_tokens = 4096; messages = @([ordered]@{ role = "user"; content = $prompt }) } }
    }
    $handler = [Net.Http.HttpClientHandler]::new()
    $client = [Net.Http.HttpClient]::new($handler)
    try {
        $client.Timeout = [TimeSpan]::FromSeconds($timeoutSeconds)
        $message = [Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::Post, $endpointUri)
        $message.Content = [Net.Http.StringContent]::new(($body | ConvertTo-Json -Depth 8 -Compress), [Text.Encoding]::UTF8, "application/json")
        if (-not [string]::IsNullOrWhiteSpace($credential)) {
            if ($protocol -eq "anthropic") { [void]$message.Headers.TryAddWithoutValidation("x-api-key", $credential); [void]$message.Headers.TryAddWithoutValidation("anthropic-version", "2023-06-01") }
            else { $message.Headers.Authorization = [Net.Http.Headers.AuthenticationHeaderValue]::new("Bearer", $credential) }
        }
        try { $response = $client.Send($message) }
        catch [Threading.Tasks.TaskCanceledException] {
            Write-Result ([ordered]@{ schema="code-intel-model-adapter-result.v1";status="failed";adapter=$adapter;category="provider_unavailable";responseArtifact=$null;reasons=@("delegate timed out");attempt=[ordered]@{invoked=$true;timedOut=$true;exitCode=$null} }) $resultPath
            exit 75
        }
        catch [Net.Http.HttpRequestException] {
            Write-Result ([ordered]@{ schema="code-intel-model-adapter-result.v1";status="failed";adapter=$adapter;category="provider_unavailable";responseArtifact=$null;reasons=@("model endpoint connection failed");attempt=[ordered]@{invoked=$true;timedOut=$false;exitCode=$null} }) $resultPath
            exit 75
        }
        $responseText = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        if (-not $response.IsSuccessStatusCode) {
            # Status-code-first: the body text is never part of the classifier
            # input here, so it can never outvote an authoritative status code.
            $failureCategory = Get-HttpFailureCategory ([int]$response.StatusCode)
            Write-Result ([ordered]@{ schema="code-intel-model-adapter-result.v1";status="failed";adapter=$adapter;category=$failureCategory;responseArtifact=$null;reasons=@("model endpoint returned a non-success status");attempt=[ordered]@{invoked=$true;timedOut=$false;exitCode=[int]$response.StatusCode} }) $resultPath
            exit $(if ($failureCategory -in @("provider_quota", "provider_unavailable")) { 75 } else { 69 })
        }
        if (-not (Test-StructuredOutput -Text $responseText -Format "json")) {
            Write-Result ([ordered]@{ schema="code-intel-model-adapter-result.v1";status="failed";adapter=$adapter;category="adapter_protocol_error";responseArtifact=$null;reasons=@("model endpoint returned non-JSON output");attempt=[ordered]@{invoked=$true;timedOut=$false;exitCode=0} }) $resultPath
            exit 65
        }
        [IO.File]::WriteAllText($responsePath, $responseText, [Text.UTF8Encoding]::new($false))
        Write-Result ([ordered]@{ schema="code-intel-model-adapter-result.v1";status="completed";adapter=$adapter;category=$null;responseArtifact=$responsePath;reasons=@();attempt=[ordered]@{invoked=$true;timedOut=$false;exitCode=0} }) $resultPath
        exit 0
    }
    finally { $client.Dispose(); $handler.Dispose() }
}

$arguments = [Collections.Generic.List[string]]::new()
switch ($adapter) {
    "claude_cli" {
        $arguments.Add("-p"); $arguments.Add("--output-format"); $arguments.Add($(if ($responseFormat -eq "jsonl") { "stream-json" } else { "json" }))
        $arguments.Add("--tools"); $arguments.Add("")
        if ($model) { $arguments.Add("--model"); $arguments.Add($model) }
    }
    "opencode_cli" {
        $arguments.Add("run"); $arguments.Add("--format"); $arguments.Add("json")
        $arguments.Add("--pure"); $arguments.Add("--agent"); $arguments.Add("plan")
        if ($model) { $arguments.Add("-m"); $arguments.Add($model) }
    }
    "codex_cli" {
        $arguments.Add("exec"); $arguments.Add("--json"); $arguments.Add("--skip-git-repo-check")
        $arguments.Add("--sandbox"); $arguments.Add("read-only")
        if ($costScope -eq "local_compute") { $arguments.Add("--oss") }
        if ($model) { $arguments.Add("-m"); $arguments.Add($model) }
        $arguments.Add("-")
    }
}

$stdout = Join-Path $artifactDir ("delegate-stdout-{0}.tmp" -f [guid]::NewGuid().ToString("N"))
$stderr = Join-Path $artifactDir ("delegate-stderr-{0}.tmp" -f [guid]::NewGuid().ToString("N"))
$process = $null
try {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $executable
    $start.UseShellExecute = $false
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.CreateNoWindow = $true
    $start.WorkingDirectory = $artifactDir
    foreach ($argument in $arguments) { [void]$start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) { throw "delegate process did not start" }
    $stderrTask = $process.StandardError.ReadToEndAsync()
    # Every adapter invoked below (claude_cli -p, codex_cli exec -, opencode_cli
    # run --pure) is a one-shot batch CLI that consumes the full prompt before
    # emitting output, so writing/closing stdin fully before this delegate
    # starts draining stdout does not risk a pipe deadlock for these adapters.
    $prompt = [IO.File]::ReadAllText($promptPath)
    $process.StandardInput.Write($prompt)
    $process.StandardInput.Close()
    $deadline = [DateTime]::UtcNow.AddSeconds($timeoutSeconds)
    $bounded = Read-BoundedProcessOutput -Reader $process.StandardOutput -MaxBytes $maxDelegateOutputBytes -Deadline $deadline
    $timedOut = $bounded.TimedOut
    $outputTruncated = $bounded.Truncated
    if ($timedOut -or $outputTruncated) {
        $process.Kill($true)
        $process.WaitForExit()
    }
    elseif (-not $process.WaitForExit([Math]::Max(0, [int][Math]::Ceiling(($deadline - [DateTime]::UtcNow).TotalMilliseconds)))) {
        $timedOut = $true
        $process.Kill($true)
        $process.WaitForExit()
    }
    $outText = $bounded.Text
    $errText = $stderrTask.GetAwaiter().GetResult()
    [IO.File]::WriteAllText($stdout, $outText, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($stderr, $errText, [Text.UTF8Encoding]::new($false))
    if ($timedOut) {
        Write-Result ([ordered]@{
            schema = "code-intel-model-adapter-result.v1"; status = "failed"; adapter = $adapter
            category = "provider_unavailable"; responseArtifact = $null; reasons = @("delegate timed out")
            attempt = [ordered]@{ invoked = $true; timedOut = $true; exitCode = $null }
        }) $resultPath
        exit 75
    }
    if ($outputTruncated) {
        Write-Result ([ordered]@{
            schema = "code-intel-model-adapter-result.v1"; status = "failed"; adapter = $adapter
            category = "adapter_protocol_error"; responseArtifact = $null; reasons = @("delegate stdout exceeded the output byte budget")
            attempt = [ordered]@{ invoked = $true; timedOut = $false; exitCode = $null }
        }) $resultPath
        exit 65
    }
    if ($process.ExitCode -ne 0) {
        $failureCategory = Get-FailureCategory $errText
        Write-Result ([ordered]@{
            schema = "code-intel-model-adapter-result.v1"; status = "failed"; adapter = $adapter
            category = $failureCategory; responseArtifact = $null; reasons = @("delegate process returned a non-zero exit code")
            attempt = [ordered]@{ invoked = $true; timedOut = $false; exitCode = [int]$process.ExitCode }
        }) $resultPath
        exit $(if ($failureCategory -in @("provider_quota", "provider_unavailable")) { 75 } else { 69 })
    }
    if (-not (Test-StructuredOutput -Text $outText -Format $responseFormat)) {
        Write-Result ([ordered]@{
            schema = "code-intel-model-adapter-result.v1"; status = "failed"; adapter = $adapter
            category = "adapter_protocol_error"; responseArtifact = $null; reasons = @("delegate output did not match the requested structured response format")
            attempt = [ordered]@{ invoked = $true; timedOut = $false; exitCode = 0 }
        }) $resultPath
        exit 65
    }
    Move-Item -LiteralPath $stdout -Destination $responsePath -Force
    Write-Result ([ordered]@{
        schema = "code-intel-model-adapter-result.v1"; status = "completed"; adapter = $adapter
        category = $null; responseArtifact = $responsePath; reasons = @()
        attempt = [ordered]@{ invoked = $true; timedOut = $false; exitCode = 0 }
    }) $resultPath
    exit 0
}
finally {
    if ($null -ne $process) { $process.Dispose() }
    Remove-Item -LiteralPath $stdout -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stderr -Force -ErrorAction SilentlyContinue
}
