#requires -Version 7.2

param(
    [string]$Provider = "anthropic",
    [string]$Model = "MiniMax-M2.7",
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Set-EnvFromUserRegistry {
    param([string]$Name)
    if (-not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($Name, "Process"))) { return }
    $value = [Environment]::GetEnvironmentVariable($Name, "User")
    if (-not [string]::IsNullOrWhiteSpace($value)) {
        [Environment]::SetEnvironmentVariable($Name, $value, "Process")
    }
}

Set-EnvFromUserRegistry "ANTHROPIC_API_KEY"
Set-EnvFromUserRegistry "ANTHROPIC_BASE_URL"
Set-EnvFromUserRegistry "REPOWISE_PROVIDER"
Set-EnvFromUserRegistry "REPOWISE_MODEL"
Set-EnvFromUserRegistry "REPOWISE_REASONING"

$effectiveProvider = if ($Provider -ieq "ccw") { "codex_cli" } else { $Provider }
$result = [ordered]@{
    ok = $false
    provider = $effectiveProvider
    model = $Model
    category = ""
    message = ""
}

try {
    switch -Regex ($effectiveProvider) {
        "^mock$" {
            $result.ok = $true
            $result.message = "mock provider"
            break
        }
        "^codex_cli$" {
            if (-not (Get-Command codex -ErrorAction SilentlyContinue)) {
                throw "codex CLI not found on PATH for repowise codex_cli provider"
            }
            $result.ok = $true
            $result.message = "codex CLI available"
            break
        }
        "^opencode$" {
            if (-not (Get-Command opencode -ErrorAction SilentlyContinue)) {
                throw "opencode CLI not found on PATH for repowise opencode provider"
            }
            $result.ok = $true
            $result.message = "opencode CLI available"
            break
        }
        "^anthropic$" {
            if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable("ANTHROPIC_API_KEY", "Process"))) {
                throw "ANTHROPIC_API_KEY is not set"
            }
            $result.ok = $true
            $result.message = "anthropic environment available"
            break
        }
        default {
            if (-not (Get-Command repowise -ErrorAction SilentlyContinue)) {
                throw "repowise CLI not found on PATH"
            }
            $result.ok = $true
            $result.message = "provider deferred to repowise registry"
        }
    }
}
catch {
    $text = $_.Exception.Message
    if ($text -match "429|rate|quota|usage limit") {
        $result.category = "provider_quota"
    }
    else {
        $result.category = "provider_error"
    }
    $result.message = $text
}

if ($Json) {
    [pscustomobject]$result | ConvertTo-Json -Depth 4
}
else {
    if ($result.ok) {
        Write-Host "Provider preflight: OK $effectiveProvider/$Model"
    }
    else {
        Write-Host "Provider preflight: FAILED $($result.category) $effectiveProvider/$Model"
        Write-Host $result.message
    }
}

if ($result.ok) { exit 0 }
exit 1