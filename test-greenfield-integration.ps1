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

    $fakeBin = Join-Path $scratch "fake-claude"
    New-Item -ItemType Directory -Force -Path $fakeBin | Out-Null
    $fakeClaude = Join-Path $fakeBin "claude.cmd"
    @'
@echo off
echo fixture specification output
exit /b 0
'@ | Set-Content -LiteralPath $fakeClaude -Encoding ASCII
    $oldPath = $env:PATH
    try {
        $env:PATH = $fakeBin + [IO.Path]::PathSeparator + $oldPath
        $analyzeArtifacts = Join-Path $scratch "analyze-artifacts"
        $analyzeRaw = & pwsh -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root "Invoke-GreenfieldSpecExtraction.ps1") `
            -RepoPath $repo `
            -ArtifactDir $analyzeArtifacts `
            -Analyze `
            -Json
        if ($LASTEXITCODE -ne 0) { throw "Greenfield analyze fixture failed" }
        $analyze = $analyzeRaw | ConvertFrom-Json
        if ($analyze.status -ne "completed" -or -not [bool]$analyze.analyzeRequested) {
            throw "Analyze fixture must complete only after explicit -Analyze."
        }
        if (-not (Test-Path -LiteralPath $analyze.outputs.stdout -PathType Leaf)) {
            throw "Analyze fixture must capture provider stdout locally."
        }
        if ((Get-Content -LiteralPath $analyze.outputs.stdout -Raw) -notmatch "fixture specification output") {
            throw "Analyze fixture stdout was not preserved."
        }
    }
    finally {
        $env:PATH = $oldPath
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
    if ($greenfield.Count -ne 0) {
        throw "Retired external spec.greenfield integration must not appear in production plans"
    }
    $registry = Get-Content -LiteralPath (Join-Path $root "orchestration/integrations.json") -Raw | ConvertFrom-Json
    if (@($registry.integrations | Where-Object { [string]$_.id -eq "spec.greenfield" }).Count -ne 0) {
        throw "Retired external spec.greenfield integration remains registered"
    }

    Write-Host "Greenfield integration smoke passed"
} finally {
    if (Test-Path -LiteralPath $scratch) {
        Remove-Item -LiteralPath $scratch -Recurse -Force
    }
}
