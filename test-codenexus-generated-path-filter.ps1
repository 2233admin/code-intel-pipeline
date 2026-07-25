#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$fixture = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-codenexus-generated-paths-" + [guid]::NewGuid().ToString("N"))
$repo = Join-Path $fixture "repo"
$run = Join-Path $fixture "run"
$output = Join-Path $run "codenexus-context.json"

function Write-TextFile {
    param([string]$RelativePath, [string]$Content)

    $path = Join-Path $repo $RelativePath
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $path) | Out-Null
    [System.IO.File]::WriteAllText($path, $Content, [System.Text.UTF8Encoding]::new($false))
}

try {
    New-Item -ItemType Directory -Force -Path $repo, $run | Out-Null
    Write-TextFile "src\TrustedService.ps1" (@(
        "function TrustedService {",
        "    return 'trusted'",
        "}",
        "TrustedService"
    ) -join "`n")

    $generatedRoots = @("work", "artifact", "artifacts", "staging", ".git", "target")
    $generatedPaths = @()
    foreach ($generatedRoot in $generatedRoots) {
        $relative = "$generatedRoot\rollback\TrustedService.ps1"
        $generatedPaths += $relative
        Write-TextFile $relative ((1..80 | ForEach-Object { "TrustedService # generated high-reference copy $_" }) -join "`n")
    }

    $hotspotsPath = Join-Path $fixture "hotspots.json"
    $dsmPath = Join-Path $fixture "dsm.json"
    $hotspots = [ordered]@{
        files = @(
            $generatedPaths | ForEach-Object {
                [ordered]@{ path = $_; maxComplexity = 999; functionCount = 999 }
            }
        ) + @([ordered]@{ path = "src\TrustedService.ps1"; maxComplexity = 1; functionCount = 1 })
    }
    $dsm = [ordered]@{
        modules = @(
            [ordered]@{ files = $generatedPaths; metrics = [ordered]@{ risk = 999 } },
            [ordered]@{ files = @("src\TrustedService.ps1"); metrics = [ordered]@{ risk = 1 } }
        )
    }
    [System.IO.File]::WriteAllText($hotspotsPath, ($hotspots | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($dsmPath, ($dsm | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))

    & (Join-Path $root "Invoke-CodeNexusLite.ps1") `
        -RepoPath $repo `
        -RunDir $run `
        -HotspotsPath $hotspotsPath `
        -DsmPath $dsmPath `
        -OutputPath $output `
        -MaxFiles 8 `
        -MaxReferencesPerFile 100 `
        -Quiet
    if ($LASTEXITCODE -ne 0) { throw "CodeNexus-lite generated-path fixture failed" }

    $result = Get-Content -LiteralPath $output -Raw | ConvertFrom-Json
    $ranked = @($result.files | ForEach-Object { ([string]$_.path).Replace('\', '/') })
    if ($ranked.Count -eq 0 -or $ranked[0] -ne "src/TrustedService.ps1") {
        throw "generated paths displaced the trusted top file: $($ranked -join ', ')"
    }
    foreach ($generatedRoot in $generatedRoots) {
        if (@($ranked | Where-Object { $_ -match "(^|/)$([regex]::Escape($generatedRoot))(/|$)" }).Count -gt 0) {
            throw "generated path entered CodeNexus ranking: $generatedRoot"
        }
    }
    $references = @($result.files[0].references | ForEach-Object { [string]$_ })
    foreach ($generatedRoot in $generatedRoots) {
        if (@($references | Where-Object { $_ -match "[\\/]$([regex]::Escape($generatedRoot))[\\/]" }).Count -gt 0) {
            throw "generated path entered CodeNexus references: $generatedRoot"
        }
    }

    $fallbackOutput = Join-Path $run "codenexus-fallback-context.json"
    & (Join-Path $root "Invoke-CodeNexusLite.ps1") `
        -RepoPath $repo `
        -RunDir $run `
        -OutputPath $fallbackOutput `
        -MaxFiles 8 `
        -MaxReferencesPerFile 0 `
        -Quiet
    if ($LASTEXITCODE -ne 0) { throw "CodeNexus-lite fallback fixture failed" }
    $fallback = Get-Content -LiteralPath $fallbackOutput -Raw | ConvertFrom-Json
    $fallbackRanked = @($fallback.files | ForEach-Object { ([string]$_.path).Replace('\', '/') })
    if ($fallbackRanked.Count -ne 1 -or $fallbackRanked[0] -ne "src/TrustedService.ps1") {
        throw "fallback ranking admitted generated paths: $($fallbackRanked -join ', ')"
    }

    Write-Host "CodeNexus generated-path filtering passed."
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
