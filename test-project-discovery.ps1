param(
    [string]$RepoPath = $PSScriptRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = (Resolve-Path -LiteralPath $RepoPath).Path
$script = Join-Path $root "Find-CodeIntelProjects.ps1"
if (-not (Test-Path -LiteralPath $script -PathType Leaf)) {
    throw "Missing Find-CodeIntelProjects.ps1"
}

$base = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-project-discovery-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $base | Out-Null

try {
    $cargoRepo = Join-Path $base "cargo-repo"
    $nodeRepo = Join-Path $base "nested\node-repo"
    $plainDir = Join-Path $base "plain"
    New-Item -ItemType Directory -Force -Path (Join-Path $cargoRepo ".git") | Out-Null
    New-Item -ItemType Directory -Force -Path $nodeRepo | Out-Null
    New-Item -ItemType Directory -Force -Path $plainDir | Out-Null
    Set-Content -LiteralPath (Join-Path $cargoRepo "Cargo.toml") -Value "[package]`nname = `"demo`"" -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $nodeRepo "package.json") -Value "{ `"name`": `"demo`" }" -Encoding UTF8

    $filesystemJson = & $script -Root $base -MaxDepth 4 -Json | ConvertFrom-Json
    $filesystemPaths = @($filesystemJson | ForEach-Object { $_.path })
    if ($filesystemPaths -notcontains (Resolve-Path -LiteralPath $cargoRepo).Path) {
        throw "Filesystem discovery did not find cargo repo."
    }
    if ($filesystemPaths -notcontains (Resolve-Path -LiteralPath $nodeRepo).Path) {
        throw "Filesystem discovery did not find node repo."
    }
    if ($filesystemPaths -contains (Resolve-Path -LiteralPath $plainDir).Path) {
        throw "Filesystem discovery should not report plain directory."
    }

    $csv = Join-Path $base "wiztree.csv"
    $csvRows = @(
        [pscustomobject]@{ "File Name" = "$cargoRepo\"; Size = "1234"; Modified = "2026-01-01 00:00:00" },
        [pscustomobject]@{ "File Name" = (Join-Path $cargoRepo ".git") + "\"; Size = "10"; Modified = "2026-01-01 00:00:00" },
        [pscustomobject]@{ "File Name" = (Join-Path $nodeRepo "package.json"); Size = "20"; Modified = "2026-01-01 00:00:00" }
    )
    $csvRows | Export-Csv -LiteralPath $csv -NoTypeInformation -Encoding UTF8

    $wizTreeJson = & $script -WizTreeCsv $csv -Json | ConvertFrom-Json
    $wizTreePaths = @($wizTreeJson | ForEach-Object { $_.path })
    if ($wizTreePaths -notcontains (Resolve-Path -LiteralPath $cargoRepo).Path) {
        throw "WizTree CSV discovery did not find cargo repo."
    }
    if ($wizTreePaths -notcontains (Resolve-Path -LiteralPath $nodeRepo).Path) {
        throw "WizTree CSV discovery did not find node repo."
    }

    Write-Host "Project discovery tests passed."
}
finally {
    if (Test-Path -LiteralPath $base) {
        Remove-Item -LiteralPath $base -Recurse -Force
    }
}
