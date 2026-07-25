#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Inventory,
    [Parameter(Mandatory = $true)][string]$Routing,
    [Parameter(Mandatory = $true)][string]$PromptFile,
    [Parameter(Mandatory = $true)][string]$OutputPath,
    [string]$ExecutableHandle,
    [string]$Endpoint,
    [ValidateSet("openai", "anthropic", "ollama")][string]$Protocol,
    [string]$CredentialEnvName,
    [ValidateRange(1, 3600)][int]$TimeoutSeconds = 300,
    [ValidateSet("json", "jsonl")][string]$ResponseFormat = "json"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Read-ClosedJson([string]$Path, [string]$Label) {
    try { $raw = Get-Content -LiteralPath (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path -Raw }
    catch { throw "cannot read $Label" }
    try { $doc = [System.Text.Json.JsonDocument]::Parse($raw) } catch { throw "$Label is not valid JSON" }
    try {
        function Assert-Unique([System.Text.Json.JsonElement]$Element, [string]$At = '$') {
            if ($Element.ValueKind -eq [System.Text.Json.JsonValueKind]::Object) {
                $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
                foreach ($p in $Element.EnumerateObject()) {
                    if (-not $names.Add($p.Name)) { throw "duplicate JSON key at $At.$($p.Name)" }
                    Assert-Unique $p.Value "$At.$($p.Name)"
                }
            } elseif ($Element.ValueKind -eq [System.Text.Json.JsonValueKind]::Array) {
                $i = 0; foreach ($v in $Element.EnumerateArray()) { Assert-Unique $v "$At[$i]"; $i++ }
            }
        }
        Assert-Unique $doc.RootElement
    } finally { $doc.Dispose() }
    return ($raw | ConvertFrom-Json -ErrorAction Stop)
}

function Assert-ExactFields([object]$Object, [string[]]$Expected, [string]$Label) {
    $actual = @($Object.PSObject.Properties.Name | Sort-Object)
    if (($actual -join "`n") -ne (($Expected | Sort-Object) -join "`n")) { throw "$Label has an unsupported shape" }
}

$inventoryObject = Read-ClosedJson $Inventory "model inventory"
$routingObject = Read-ClosedJson $Routing "model routing"
Assert-ExactFields $inventoryObject @("schema", "candidates", "configurationBrokers") "model inventory"
Assert-ExactFields $routingObject @("schema", "status", "selected", "authorization", "attempts", "manualAction") "model routing"
if ([string]$inventoryObject.schema -ne "code-intel-model-channel-inventory-result.v1") { throw "model inventory schema is unsupported" }
if ([string]$routingObject.schema -ne "code-intel-model-routing-result.v1" -or [string]$routingObject.status -ne "ready" -or $null -eq $routingObject.selected) { throw "model routing is not ready" }
$selected = $routingObject.selected
$matches = @($inventoryObject.candidates | Where-Object { [string]$_.id -eq [string]$selected.candidateId })
if ($matches.Count -ne 1) { throw "selected candidate does not resolve uniquely in inventory" }
$candidate = $matches[0]
foreach ($field in @("channelKind", "provider", "model", "costScope")) {
    if ([string]$candidate.$field -ne [string]$selected.$field) { throw "selected candidate does not match inventory field '$field'" }
}
if (-not [bool]$candidate.discovered -or -not [bool]$candidate.executableVerified -or [string]$candidate.modelAvailable -ne "available") { throw "selected candidate is not execution-ready in inventory" }
$adapter = [string]$candidate.channelKind
$isHttp = $adapter -in @("ollama", "local_compatible")
$model = [string]$selected.model
if ([string]::IsNullOrWhiteSpace($model)) { throw "selected route has no concrete model" }
if (-not $isHttp -and [string]::IsNullOrWhiteSpace($ExecutableHandle)) { throw "CLI request synthesis requires an executable handle" }
if ($isHttp -and [string]::IsNullOrWhiteSpace($Endpoint)) { throw "HTTP request synthesis requires an explicit endpoint" }
if ($adapter -eq "ollama" -and $Protocol -ne "ollama") { throw "ollama synthesis requires protocol=ollama" }
if ($adapter -eq "local_compatible" -and $Protocol -notin @("openai", "anthropic")) { throw "local compatible synthesis requires protocol=openai or anthropic" }
if (-not $isHttp -and (-not [string]::IsNullOrWhiteSpace($Endpoint) -or -not [string]::IsNullOrWhiteSpace($Protocol) -or -not [string]::IsNullOrWhiteSpace($CredentialEnvName))) { throw "CLI synthesis does not accept HTTP configuration" }

$consumption = [string]$routingObject.authorization.consumptionAuthorization.status
$scopes = @($routingObject.authorization.consumptionAuthorization.scopes)
$external = [string]$routingObject.authorization.externalData.status
$paid = [string]$routingObject.authorization.paidSpend.status
$local = if ($consumption -eq "granted" -and $scopes -contains "local_compute") { "granted" } else { "unanswered" }
$derivedExternalData = [bool]$candidate.externalEgress
if (-not $isHttp) { $derivedExternalData = $true }
if ($isHttp) {
    $uri = $null
    if (-not [Uri]::TryCreate($Endpoint, [UriKind]::Absolute, [ref]$uri) -or $uri.Scheme -notin @("http", "https")) { throw "endpoint must be an absolute HTTP(S) URI" }
    if (-not $uri.IsLoopback) { $derivedExternalData = $true }
}
$request = [ordered]@{
    schema = "code-intel-model-adapter-request.v2"
    adapter = $adapter
    executableHandle = if ($isHttp) { $null } else { (Resolve-Path -LiteralPath $ExecutableHandle -ErrorAction Stop).Path }
    endpoint = if ($isHttp) { $Endpoint } else { $null }
    protocol = if ($isHttp) { $Protocol } else { $null }
    credentialEnvName = if ($isHttp -and -not [string]::IsNullOrWhiteSpace($CredentialEnvName)) { $CredentialEnvName } else { $null }
    model = $model
    promptFile = (Resolve-Path -LiteralPath $PromptFile -ErrorAction Stop).Path
    timeoutSeconds = $TimeoutSeconds
    consent = [ordered]@{ consumption = $consumption; localCompute = $local; paidSpend = $paid; externalData = $external }
    costScope = [string]$candidate.costScope
    externalData = $derivedExternalData
    responseFormat = $ResponseFormat
}
$parent = Split-Path -Parent ([IO.Path]::GetFullPath($OutputPath))
if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
[IO.File]::WriteAllText([IO.Path]::GetFullPath($OutputPath), ($request | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
$request | ConvertTo-Json -Depth 8 -Compress
