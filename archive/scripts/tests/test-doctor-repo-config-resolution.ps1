#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$doctor = Join-Path $root "check-code-intel-tools.ps1"
$scratch = Join-Path $env:TEMP ("code-intel-doctor-config-{0}" -f [guid]::NewGuid().ToString("N"))

try {
    $repo = New-Item -ItemType Directory -Path (Join-Path $scratch "ConfiguredRepo") -Force
    $sentruxDir = New-Item -ItemType Directory -Path (Join-Path $repo.FullName "backend\.sentrux") -Force
    New-Item -ItemType File -Path (Join-Path $sentruxDir.FullName "rules.toml") -Force | Out-Null
    New-Item -ItemType File -Path (Join-Path $sentruxDir.FullName "baseline.json") -Force | Out-Null

    $configPath = Join-Path $scratch "pipeline.config.json"
    [ordered]@{
        repos = [ordered]@{
            fixture = [ordered]@{
                path = $repo.FullName + [System.IO.Path]::DirectorySeparatorChar
                sentruxPath = "backend"
            }
        }
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $configPath

    $output = & $doctor -Config $configPath -RepoPath (Join-Path $repo.FullName ".") -Json 2>$null
    $result = ($output -join "`n") | ConvertFrom-Json
    $expectedScope = (Get-Item -LiteralPath (Join-Path $repo.FullName "backend")).FullName

    if ($result.checks.repo.sentruxScope -ne $expectedScope) { throw "Doctor did not resolve configured sentruxPath" }
    if (-not $result.checks.repo.sentruxRules) { throw "Doctor did not find scoped Sentrux rules" }
    if (-not $result.checks.repo.sentruxBaseline) { throw "Doctor did not find scoped Sentrux baseline" }

    Write-Host "PASS: Doctor RepoPath reverse lookup finds configured Sentrux scope, rules, and baseline."
}
finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
