#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet("claude_cli", "opencode_cli", "codex_cli")][string]$Adapter,
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$OutputPath,
    [ValidateRange(1, 3600)][int]$LifetimeSeconds = 300
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$path = (Resolve-Path -LiteralPath $Executable -ErrorAction Stop).Path
$item = Get-Item -LiteralPath $path -ErrorAction Stop
if (-not $item.PSIsContainer -and $item.Length -ge 0) {
    $observedAt = [DateTimeOffset]::UtcNow
    $sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    $identityMaterial = @(
        "code-intel-model-executable-handle.v1",
        $Adapter,
        $path,
        $sha256,
        [string]$item.Length,
        [string]$item.LastWriteTimeUtc.Ticks,
        $observedAt.ToString("O"),
        $observedAt.AddSeconds($LifetimeSeconds).ToString("O")
    ) -join "`n"
    $identityBytes = [Text.Encoding]::UTF8.GetBytes($identityMaterial)
    $identity = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($identityBytes)).ToLowerInvariant()
    $handle = [ordered]@{
        schema = "code-intel-model-executable-handle.v1"
        adapter = $Adapter
        executablePath = $path
        sha256 = $sha256
        length = [long]$item.Length
        lastWriteTimeUtcTicks = [long]$item.LastWriteTimeUtc.Ticks
        observedAt = $observedAt.ToString("O")
        expiresAt = $observedAt.AddSeconds($LifetimeSeconds).ToString("O")
        identity = $identity
    }
    $parent = Split-Path -Parent ([IO.Path]::GetFullPath($OutputPath))
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    [IO.File]::WriteAllText([IO.Path]::GetFullPath($OutputPath), ($handle | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
    $handle | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
throw "executable must be a file"
