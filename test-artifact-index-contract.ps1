param(
    [string]$RepoPath = $PSScriptRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

$root = (Resolve-Path -LiteralPath $RepoPath).Path
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-a08-contract-" + [guid]::NewGuid().ToString("N"))
try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null
    cargo build -p code-intel --manifest-path (Join-Path $root "Cargo.toml") | Out-Null
    Assert-True ($LASTEXITCODE -eq 0) "Rust CLI build failed"

    $output = Join-Path $temp "index.md"
    $result = & (Join-Path $root "update-code-intel-index.ps1") -ArtifactRoot $temp -OutputPath $output | ConvertFrom-Json
    Assert-True ($LASTEXITCODE -eq 0) "Committed-only facade failed"
    Assert-True ($result.schema -eq "code-intel-artifact-index.v1") "Facade did not route to the A08 Rust schema"
    Assert-True ($result.mode -eq "committed-only") "Normal facade mode is not committed-only"
    $index = Get-Content -LiteralPath ([System.IO.Path]::ChangeExtension($output, ".json")) -Raw | ConvertFrom-Json
    Assert-True ($index.schema -eq "code-intel-artifact-index.v1") "Rust index JSON was not published"

    $legacyOutput = Join-Path $temp "legacy-index.md"
    $legacy = & (Join-Path $root "update-code-intel-index.ps1") -ArtifactRoot $temp -OutputPath $legacyOutput -LegacyCompatibilityMode | ConvertFrom-Json
    Assert-True ($LASTEXITCODE -eq 0) "Explicit legacy compatibility mode failed"
    Assert-True ($legacy.ok -eq $true) "Legacy compatibility result is invalid"

    $script = Get-Content -LiteralPath (Join-Path $root "update-code-intel-index.ps1") -Raw
    Assert-True ($script.Contains('"artifact", "index"')) "Facade lacks the production Rust route"
    Assert-True ($script.Contains("LegacyCompatibilityMode")) "Facade lacks explicit legacy compatibility mode"

    [ordered]@{
        ok = $true
        schema = "code-intel-artifact-index-contract-test.v1"
        productionSchema = $result.schema
        legacyExplicit = $true
    } | ConvertTo-Json
}
finally {
    if (Test-Path -LiteralPath $temp) {
        Remove-Item -LiteralPath $temp -Recurse -Force
    }
}
