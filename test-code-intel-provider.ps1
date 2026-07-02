param(
    [string]$Provider = "",
    [string]$Model = "",
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

function Set-EnvFromUserRegistry {
    param([string]$Name)

    if (-not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($Name, "Process"))) {
        return
    }
    $value = [Environment]::GetEnvironmentVariable($Name, "User")
    if (-not [string]::IsNullOrWhiteSpace($value)) {
        [Environment]::SetEnvironmentVariable($Name, $value, "Process")
    }
}

function Set-CodeIntelAnthropicEnv {
    # Dedicated CODE_INTEL_ANTHROPIC_* vars take priority over any inherited
    # ANTHROPIC_* values, which on a dev machine may point at the Claude Code
    # proxy (e.g. headroom on 127.0.0.1) and must not be repurposed globally.
    # ANTHROPIC_* is only ever set at PROCESS scope here, never persisted.
    $map = @{
        "CODE_INTEL_ANTHROPIC_API_KEY"  = "ANTHROPIC_API_KEY"
        "CODE_INTEL_ANTHROPIC_BASE_URL" = "ANTHROPIC_BASE_URL"
    }
    foreach ($name in $map.Keys) {
        $value = Get-CodeIntelEnvValue $name
        if (-not [string]::IsNullOrWhiteSpace($value)) {
            [Environment]::SetEnvironmentVariable($map[$name], $value, "Process")
        }
    }
}

function Get-RepowisePython {
    # The preflight uses the repowise uv tool venv python so the anthropic /
    # openai / httpx SDKs are guaranteed present without polluting the system
    # python. Fall back to plain python for pip-installed setups.
    if (-not [string]::IsNullOrWhiteSpace($env:APPDATA)) {
        $venvPython = Join-Path $env:APPDATA "uv\tools\repowise\Scripts\python.exe"
        if (Test-Path -LiteralPath $venvPython -PathType Leaf) {
            return $venvPython
        }
    }
    return "python"
}

Set-EnvFromUserRegistry "ANTHROPIC_API_KEY"
Set-EnvFromUserRegistry "ANTHROPIC_BASE_URL"
Set-CodeIntelAnthropicEnv
Set-EnvFromUserRegistry "CODE_INTEL_PROVIDER"
Set-EnvFromUserRegistry "CODE_INTEL_MODEL"
Set-EnvFromUserRegistry "CODE_INTEL_API_KEY"
Set-EnvFromUserRegistry "CODE_INTEL_BASE_URL"

# Explicit params win; otherwise fall back to CODE_INTEL_* env (process, then user).
if ([string]::IsNullOrWhiteSpace($Provider)) {
    $Provider = Get-CodeIntelEnvValue "CODE_INTEL_PROVIDER"
}
if ([string]::IsNullOrWhiteSpace($Provider)) {
    $Provider = "anthropic"
}
if ([string]::IsNullOrWhiteSpace($Model)) {
    $Model = Get-CodeIntelEnvValue "CODE_INTEL_MODEL"
}
if ([string]::IsNullOrWhiteSpace($Model)) {
    $Model = ""
}

$python = @'
import json
import os
import sys

def env(name):
    return (os.environ.get(name) or "").strip()

provider = env("CODE_INTEL_PROVIDER").lower() or "anthropic"
model = env("CODE_INTEL_MODEL")
api_key = env("CODE_INTEL_API_KEY")
base_url = env("CODE_INTEL_BASE_URL")

DEFAULT_MODELS = {"anthropic": "MiniMax-M2.7"}
model = model or DEFAULT_MODELS.get(provider, "")

result = {
    "ok": False,
    "provider": provider,
    "model": model,
    "category": "",
    "message": "",
}

try:
    if provider == "anthropic":
        key = api_key or env("ANTHROPIC_API_KEY")
        url = base_url or env("ANTHROPIC_BASE_URL") or None
        if not key:
            raise RuntimeError("No API key: set CODE_INTEL_API_KEY or CODE_INTEL_ANTHROPIC_API_KEY")
        from anthropic import Anthropic
        client = Anthropic(api_key=key, base_url=url, max_retries=1)
        client.messages.create(
            model=model,
            max_tokens=128,
            messages=[{"role": "user", "content": "reply ok"}],
        )
        result["message"] = "provider preflight ok"
        result["ok"] = True
    elif provider == "ollama":
        import httpx
        url = (base_url or env("OLLAMA_BASE_URL") or "http://localhost:11434").rstrip("/")
        resp = httpx.get(url + "/api/tags", timeout=10)
        resp.raise_for_status()
        names = [m.get("name", "") for m in resp.json().get("models", [])]
        if model and model not in names and f"{model}:latest" not in names:
            raise RuntimeError(f"model {model!r} not found on {url}; available: {names}")
        result["message"] = f"ollama reachable at {url}; {len(names)} model(s) installed"
        result["ok"] = True
    elif provider == "openai":
        key = api_key or env("OPENAI_API_KEY")
        url = base_url or env("OPENAI_BASE_URL") or None
        if not key:
            raise RuntimeError("No API key: set CODE_INTEL_API_KEY or OPENAI_API_KEY")
        if not model:
            raise RuntimeError("No model: set CODE_INTEL_MODEL for openai-compatible endpoints")
        from openai import OpenAI
        client = OpenAI(api_key=key, base_url=url, timeout=30, max_retries=1)
        client.chat.completions.create(
            model=model,
            max_tokens=16,
            messages=[{"role": "user", "content": "reply ok"}],
        )
        result["message"] = "provider preflight ok"
        result["ok"] = True
    else:
        raise RuntimeError(
            f"Unsupported provider preflight: {provider!r} "
            "(supported: anthropic, openai, ollama)"
        )
except Exception as exc:
    text = str(exc)
    lower = text.lower()
    if "429" in lower or "rate_limit" in lower or "quota" in lower or "usage limit" in lower:
        result["category"] = "provider_quota"
    else:
        result["category"] = "local_tool_error"
    result["message"] = text[:1000]

print(json.dumps(result, ensure_ascii=False))
sys.exit(0 if result["ok"] else 1)
'@

$env:CODE_INTEL_PROVIDER = $Provider
if (-not [string]::IsNullOrWhiteSpace($Model)) {
    $env:CODE_INTEL_MODEL = $Model
}
$pythonExe = Get-RepowisePython
$raw = & $pythonExe -c $python
$exitCode = $LASTEXITCODE
$result = $raw | ConvertFrom-Json

if ($Json) {
    $result | ConvertTo-Json -Depth 4
}
else {
    if ($result.ok) {
        Write-Host "Provider preflight: OK $($result.provider)/$($result.model)"
    }
    else {
        Write-Host "Provider preflight: FAILED $($result.category) $($result.provider)/$($result.model)"
        Write-Host $result.message
    }
}

exit $exitCode
