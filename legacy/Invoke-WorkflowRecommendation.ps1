param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,
    [switch]$Auto,
    [switch]$Quiet,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($Json -and $IsWindows) {
    [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
}

function Resolve-CodeIntelBinary {
    $binaryName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
    $candidates = [System.Collections.Generic.List[string]]::new()
    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_BIN)) {
        if (Test-Path -LiteralPath $env:CODE_INTEL_BIN -PathType Leaf) {
            $candidates.Add($env:CODE_INTEL_BIN)
        }
        else {
            $candidates.Add((Join-Path $env:CODE_INTEL_BIN $binaryName))
        }
    }
    $pipelineRoot = Split-Path -Parent $PSScriptRoot
    $candidates.Add((Join-Path $pipelineRoot "bin/$binaryName"))
    $candidates.Add((Join-Path $pipelineRoot "target/release/$binaryName"))
    $candidates.Add((Join-Path $pipelineRoot "target/debug/$binaryName"))
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    $command = Get-Command code-intel -CommandType Application -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }
    throw "Compiled code-intel binary not found."
}

function Resolve-IntegrationsManifest {
    param([Parameter(Mandatory = $true)][string]$Binary)

    $candidates = [System.Collections.Generic.List[string]]::new()
    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_INTEGRATIONS_MANIFEST)) {
        $candidates.Add($env:CODE_INTEL_INTEGRATIONS_MANIFEST)
    }
    $candidates.Add((Join-Path (Split-Path -Parent $PSScriptRoot) "orchestration/integrations.json"))
    $candidates.Add((Join-Path (Split-Path -Parent $Binary) "orchestration/integrations.json"))
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    throw "Capability registry for advisory.workflow-recommend not found."
}

$resolvedRepo = (Resolve-Path -LiteralPath $RepoPath).Path
$rustCli = Resolve-CodeIntelBinary
$manifestPath = Resolve-IntegrationsManifest -Binary $rustCli
$registry = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$registration = @($registry.integrations | Where-Object { $_.id -eq "advisory.workflow-recommend" })
if ($registration.Count -ne 1) {
    throw "Capability registry must contain exactly one advisory.workflow-recommend declaration."
}

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-workflow-recommend-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tempRoot | Out-Null
$priorManifest = $env:CODE_INTEL_INTEGRATIONS_MANIFEST
try {
    $env:CODE_INTEL_INTEGRATIONS_MANIFEST = $manifestPath
    $snapshotText = (& $rustCli snapshot identity --repo $resolvedRepo --working-tree-policy explicit_overlay --scope . | Out-String)
    if ($LASTEXITCODE -ne 0) {
        throw "Workflow recommendation snapshot failed with exit code $LASTEXITCODE."
    }
    $snapshot = $snapshotText | ConvertFrom-Json
    $request = [ordered]@{
        schema = "code-intel-capability-request.v1"
        capability = "advisory.workflow-recommend"
        contractVersion = 1
        implementation = $registration[0].capabilityDeclaration.implementation
        snapshot = $snapshot.snapshot
        options = [ordered]@{ repoPath = $resolvedRepo; auto = [bool]$Auto }
        inputs = @()
        effectPolicy = [ordered]@{ allowedEffects = @() }
    }
    $requestPath = Join-Path $tempRoot "request.json"
    $out = Join-Path $tempRoot "out"
    [IO.File]::WriteAllText($requestPath, ($request | ConvertTo-Json -Depth 20 -Compress), [Text.UTF8Encoding]::new($false))
    $null = (& $rustCli capability exec advisory.workflow-recommend --request $requestPath --out $out | Out-String)
    if ($LASTEXITCODE -ne 0) {
        throw "Workflow recommendation capability failed with exit code $LASTEXITCODE."
    }
    $proposalText = Get-Content -LiteralPath (Join-Path $out "workflow-recommendation.json") -Raw
    if ($Json) {
        return $proposalText
    }
    return $proposalText | ConvertFrom-Json -AsHashtable
}
finally {
    $env:CODE_INTEL_INTEGRATIONS_MANIFEST = $priorManifest
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
