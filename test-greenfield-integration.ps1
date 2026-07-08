param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
$scratch = Join-Path $env:TEMP ("cip-greenfield-{0}" -f [guid]::NewGuid().ToString("N").Substring(0, 8))
$repo = Join-Path $scratch "repo"
$artifacts = Join-Path $scratch "artifacts"

try {
    New-Item -ItemType Directory -Force -Path $repo | Out-Null
    Set-Content -LiteralPath (Join-Path $repo "README.md") -Value "# sample" -Encoding UTF8

    $raw = & pwsh -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root "Invoke-GreenfieldSpecExtraction.ps1") `
        -RepoPath $repo `
        -ArtifactDir $artifacts `
        -Exclude "runtime,community" `
        -Json

    if ($LASTEXITCODE -ne 0) {
        throw "Greenfield adapter exited with code $LASTEXITCODE"
    }

    $manifest = $raw | ConvertFrom-Json
    if ($manifest.schema -ne "code-intel-greenfield-spec-extraction.v1") {
        throw "Unexpected Greenfield schema: $($manifest.schema)"
    }
    if ($manifest.status -ne "planned") {
        throw "Expected planned status, got $($manifest.status)"
    }
    if (-not (Test-Path -LiteralPath (Join-Path $artifacts "greenfield-manifest.json") -PathType Leaf)) {
        throw "Missing greenfield-manifest.json"
    }
    if (-not (Test-Path -LiteralPath (Join-Path $artifacts "greenfield-plan.md") -PathType Leaf)) {
        throw "Missing greenfield-plan.md"
    }
    if ($manifest.prompt -notmatch "/analyze") {
        throw "Prompt missing /analyze command"
    }
    if ($manifest.prompt -notmatch "--exclude runtime,community") {
        throw "Prompt missing normalized exclude list: $($manifest.prompt)"
    }

    $badOutput = & pwsh -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root "Invoke-GreenfieldSpecExtraction.ps1") `
        -RepoPath $repo `
        -ArtifactDir (Join-Path $scratch "bad-artifacts") `
        -Exclude "not-a-source" `
        -Json 2>&1

    if ($LASTEXITCODE -eq 0) {
        throw "Invalid Greenfield exclude should fail"
    }

    $rustCli = Join-Path $root "target\debug\code-intel.exe"
    if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
        Push-Location $root
        try {
            & cargo build -p code-intel | Out-Host
        } finally {
            Pop-Location
        }
    }

    $planRaw = & $rustCli orchestrate --action Plan --capability behavioral_specification --repo $repo --json
    if ($LASTEXITCODE -ne 0) {
        throw "Orchestrator Greenfield plan failed"
    }
    $plan = $planRaw | ConvertFrom-Json
    $greenfield = @($plan.plan | Where-Object { $_.id -eq "spec.greenfield" })
    if ($greenfield.Count -ne 1) {
        throw "Expected one spec.greenfield integration"
    }
    if ([bool]$greenfield[0].required) {
        throw "spec.greenfield must stay optional"
    }

    Write-Host "Greenfield integration smoke passed"
} finally {
    if (Test-Path -LiteralPath $scratch) {
        Remove-Item -LiteralPath $scratch -Recurse -Force
    }
}
