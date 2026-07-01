param(
    [string]$Provider = "anthropic",
    [string]$Model = "MiniMax-M3",
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

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

Set-EnvFromUserRegistry "ANTHROPIC_API_KEY"
Set-EnvFromUserRegistry "ANTHROPIC_BASE_URL"

$python = @'
import json
import os
import sys

provider = os.environ.get("CODE_INTEL_PROVIDER", "anthropic")
model = os.environ.get("CODE_INTEL_MODEL", "MiniMax-M3")

result = {
    "ok": False,
    "provider": provider,
    "model": model,
    "category": "",
    "message": "",
}

try:
    if provider != "anthropic":
        raise RuntimeError(f"Unsupported provider preflight: {provider}")
    from anthropic import Anthropic
    client = Anthropic(
        api_key=os.environ.get("ANTHROPIC_API_KEY"),
        base_url=os.environ.get("ANTHROPIC_BASE_URL"),
    )
    client.messages.create(
        model=model,
        max_tokens=128,
        messages=[{"role": "user", "content": "reply ok"}],
    )
    result["ok"] = True
    result["message"] = "provider preflight ok"
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
$env:CODE_INTEL_MODEL = $Model
if (Get-Command uv -ErrorAction SilentlyContinue) {
    $raw = & uv --quiet run --with anthropic python -c $python
}
else {
    $raw = & python -c $python
}
$exitCode = $LASTEXITCODE
$result = $raw | ConvertFrom-Json

if ($Json) {
    $result | ConvertTo-Json -Depth 4
}
else {
    if ($result.ok) {
        Write-Host "Provider preflight: OK $Provider/$Model"
    }
    else {
        Write-Host "Provider preflight: FAILED $($result.category) $Provider/$Model"
        Write-Host $result.message
    }
}

exit $exitCode
