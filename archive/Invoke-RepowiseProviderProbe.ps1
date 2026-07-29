#requires -Version 7.2

param(
    [string]$Provider = "",
    [string]$Model = "",
    [ValidateSet("", "model_not_found", "authentication_error", "rate_limit", "local_import_error")]
    [string]$FailureFixture = "",
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-CodeIntelEnvValue {
    param([string]$Name)
    $value = [Environment]::GetEnvironmentVariable($Name, "Process")
    if ([string]::IsNullOrWhiteSpace($value)) {
        $value = [Environment]::GetEnvironmentVariable($Name, "User")
    }
    return $value
}

function Import-ProcessEnv {
    param([string]$Name)
    if (-not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($Name, "Process"))) { return }
    $value = [Environment]::GetEnvironmentVariable($Name, "User")
    if (-not [string]::IsNullOrWhiteSpace($value)) {
        [Environment]::SetEnvironmentVariable($Name, $value, "Process")
    }
}

function Get-RepowisePython {
    if (-not [string]::IsNullOrWhiteSpace($env:APPDATA)) {
        $candidate = Join-Path $env:APPDATA "uv\tools\repowise\Scripts\python.exe"
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    return "python"
}

function Protect-ProviderMessage {
    param([string]$Message)
    $safe = [string]$Message
    foreach ($name in @(
        "CODE_INTEL_API_KEY", "CODE_INTEL_ANTHROPIC_API_KEY", "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY", "ANTHROPIC_AUTH_TOKEN"
    )) {
        $value = Get-CodeIntelEnvValue $name
        if (-not [string]::IsNullOrWhiteSpace($value)) {
            $safe = $safe.Replace($value, "[REDACTED]")
        }
    }
    $safe = $safe -replace '(?i)Bearer\s+[A-Za-z0-9._~+/-]+', 'Bearer [REDACTED]'
    $safe = $safe -replace '(?i)\b(sk-[A-Za-z0-9_-]{6,})\b', '[REDACTED]'
    if ($safe.Length -gt 1000) { $safe = $safe.Substring(0, 1000) }
    return $safe
}

foreach ($name in @(
    "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL", "OPENAI_API_KEY", "OPENAI_BASE_URL",
    "CODE_INTEL_PROVIDER", "CODE_INTEL_MODEL", "CODE_INTEL_API_KEY", "CODE_INTEL_BASE_URL",
    "REPOWISE_PROVIDER", "REPOWISE_MODEL", "REPOWISE_REASONING"
)) {
    Import-ProcessEnv $name
}

$dedicatedKey = Get-CodeIntelEnvValue "CODE_INTEL_ANTHROPIC_API_KEY"
$dedicatedUrl = Get-CodeIntelEnvValue "CODE_INTEL_ANTHROPIC_BASE_URL"
if (-not [string]::IsNullOrWhiteSpace($dedicatedKey)) { $env:ANTHROPIC_API_KEY = $dedicatedKey }
if (-not [string]::IsNullOrWhiteSpace($dedicatedUrl)) { $env:ANTHROPIC_BASE_URL = $dedicatedUrl }

if ([string]::IsNullOrWhiteSpace($Provider)) { $Provider = Get-CodeIntelEnvValue "CODE_INTEL_PROVIDER" }
if ([string]::IsNullOrWhiteSpace($Provider)) { $Provider = "anthropic" }
if ($Provider -ieq "ccw") { $Provider = "codex_cli" }
if ([string]::IsNullOrWhiteSpace($Model)) { $Model = Get-CodeIntelEnvValue "CODE_INTEL_MODEL" }
if ($null -eq $Model) { $Model = "" }

$result = $null
switch -Regex ($Provider) {
    "^mock$" {
        $result = [ordered]@{ ok = $true; provider = $Provider; model = $Model; category = ""; message = "mock provider" }
        break
    }
    "^codex_cli$" {
        $available = $null -ne (Get-Command codex -ErrorAction SilentlyContinue)
        $result = [ordered]@{
            ok = $available; provider = $Provider; model = $Model
            category = if ($available) { "" } else { "local_tool_error" }
            message = if ($available) { "codex CLI available" } else { "codex CLI not found for Repowise provider" }
        }
        break
    }
    "^opencode$" {
        $available = $null -ne (Get-Command opencode -ErrorAction SilentlyContinue)
        $result = [ordered]@{
            ok = $available; provider = $Provider; model = $Model
            category = if ($available) { "" } else { "local_tool_error" }
            message = if ($available) { "opencode CLI available" } else { "opencode CLI not found for Repowise provider" }
        }
        break
    }
}

if ($null -eq $result) {
    $python = @'
import json
import os
import sys

def env(name):
    return (os.environ.get(name) or "").strip()

def classify_failure(exc):
    text = str(exc)
    lower = text.lower()
    type_name = type(exc).__name__.lower()
    status = getattr(exc, "status_code", None)

    if isinstance(exc, (ImportError, ModuleNotFoundError, FileNotFoundError)):
        return "local_tool_error"
    if status == 429 or any(term in lower for term in ("429", "rate_limit", "quota", "usage limit", "status_code': 2056", '"status_code":2056')):
        return "provider_quota"
    if status == 404 or any(term in lower for term in ("model_not_found", "not_found_error", "error code: 404", "status code: 404")):
        return "provider_unavailable"
    if status in (401, 403) or any(term in lower for term in (
        "authentication_error", "invalid api key", "not authorized", "token not match",
        "no api key configured", "requires key and model", "unsupported repowise provider",
        "invalid params", "invalid_request_error", "error code: 400", "status code: 400",
        "status_code': 1004", '"status_code":1004', "status_code': 2049", '"status_code":2049'
    )):
        return "config_error"
    if status is not None and status >= 500:
        return "provider_unavailable"
    if any(term in lower or term in type_name for term in (
        "connection", "connecterror", "timeout", "timed out", "service unavailable",
        "internal server error", "bad gateway", "gateway timeout"
    )):
        return "provider_unavailable"
    return "provider_error"

provider = env("CODE_INTEL_PROVIDER").lower() or "anthropic"
model = env("CODE_INTEL_MODEL")
api_key = env("CODE_INTEL_API_KEY")
base_url = env("CODE_INTEL_BASE_URL")
model = model or {"anthropic": "MiniMax-M2.7"}.get(provider, "")
result = {"ok": False, "provider": provider, "model": model, "category": "", "message": ""}

try:
    fixture = env("CODE_INTEL_PROVIDER_PROBE_FAILURE_FIXTURE")
    if fixture == "model_not_found":
        raise RuntimeError("Error code: 404 - {'type':'error','error':{'type':'not_found_error','code':'model_not_found','message':'model not found'}}")
    if fixture == "authentication_error":
        raise RuntimeError("Error code: 401 - {'type':'authentication_error','message':'invalid API key'}")
    if fixture == "rate_limit":
        raise RuntimeError("Error code: 429 - {'type':'rate_limit_error','message':'usage limit exceeded'}")
    if fixture == "local_import_error":
        raise ModuleNotFoundError("No module named 'anthropic'")
    if provider == "anthropic":
        key = api_key or env("ANTHROPIC_API_KEY")
        url = base_url or env("ANTHROPIC_BASE_URL") or None
        if not key:
            raise RuntimeError("No API key configured for the Repowise Anthropic provider")
        from anthropic import Anthropic
        Anthropic(api_key=key, base_url=url, max_retries=1).messages.create(
            model=model, max_tokens=16, messages=[{"role": "user", "content": "reply ok"}]
        )
    elif provider == "openai":
        key = api_key or env("OPENAI_API_KEY")
        url = base_url or env("OPENAI_BASE_URL") or None
        if not key or not model:
            raise RuntimeError("OpenAI-compatible Repowise provider requires key and model")
        from openai import OpenAI
        OpenAI(api_key=key, base_url=url, timeout=30, max_retries=1).chat.completions.create(
            model=model, max_tokens=16, messages=[{"role": "user", "content": "reply ok"}]
        )
    elif provider == "ollama":
        import httpx
        url = (base_url or env("OLLAMA_BASE_URL") or "http://localhost:11434").rstrip("/")
        response = httpx.get(url + "/api/tags", timeout=10)
        response.raise_for_status()
        names = [item.get("name", "") for item in response.json().get("models", [])]
        if model and model not in names and f"{model}:latest" not in names:
            raise RuntimeError("configured Ollama model is unavailable")
    else:
        raise RuntimeError("unsupported Repowise provider health probe")
    result["ok"] = True
    result["message"] = "provider health probe passed"
except Exception as exc:
    text = str(exc)
    result["category"] = classify_failure(exc)
    result["message"] = text

print(json.dumps(result, ensure_ascii=False))
sys.exit(0 if result["ok"] else 1)
'@
    $env:CODE_INTEL_PROVIDER = $Provider
    $env:CODE_INTEL_MODEL = $Model
    $env:CODE_INTEL_PROVIDER_PROBE_FAILURE_FIXTURE = $FailureFixture
    $raw = & (Get-RepowisePython) -c $python
    $probeExit = $LASTEXITCODE
    $result = $raw | ConvertFrom-Json
    $result.message = Protect-ProviderMessage ([string]$result.message)
}
else {
    $probeExit = if ($result.ok) { 0 } else { 1 }
    $result.message = Protect-ProviderMessage ([string]$result.message)
}

$output = [ordered]@{
    schema = "code-intel-repowise-provider-health.v1"
    kind = "health"
    evidence = $false
    ok = [bool]$result.ok
    provider = [string]$result.provider
    model = [string]$result.model
    category = [string]$result.category
    message = [string]$result.message
}
if ($Json) { $output | ConvertTo-Json -Depth 4 }
elseif ($output.ok) { Write-Host "Repowise provider health: OK $($output.provider)/$($output.model)" }
else { Write-Host "Repowise provider health: FAILED $($output.category) $($output.message)" }

exit $probeExit
