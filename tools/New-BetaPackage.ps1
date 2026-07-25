#requires -Version 7.2

[CmdletBinding()]
param(
    [string]$RepoPath = (Split-Path -Parent $PSScriptRoot),
    [string]$ExecutablePath,
    [string]$OutputDirectory,
    [string]$PackageName = "code-intel-pipeline-windows-beta",
    [string]$ExpectedVersion,
    [switch]$AllowDirty,
    [switch]$DevelopmentOnlyAllowUnresolvedLicense
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $false

function Invoke-GitText {
    param([string[]]$Arguments)

    $output = @(& git -C $script:RepoRoot @Arguments)
    if ($LASTEXITCODE -ne 0) {
        throw "git $($Arguments -join ' ') failed with exit code $LASTEXITCODE."
    }
    return $output
}

function Get-RelativeSlashPath {
    param([string]$BasePath, [string]$Path)

    return [System.IO.Path]::GetRelativePath($BasePath, $Path).Replace([System.IO.Path]::DirectorySeparatorChar, '/')
}

$script:RepoRoot = (Resolve-Path -LiteralPath $RepoPath).Path
if (-not $ExecutablePath) {
    $ExecutablePath = Join-Path $script:RepoRoot "target/release/code-intel.exe"
}
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $script:RepoRoot "dist"
}
$resolvedExecutable = (Resolve-Path -LiteralPath $ExecutablePath).Path
$outputRoot = [System.IO.Path]::GetFullPath($OutputDirectory)

$status = @(Invoke-GitText -Arguments @("status", "--porcelain=v1", "--untracked-files=all"))
$dirty = $status.Count -gt 0
if ($dirty -and -not $AllowDirty) {
    throw "Refusing to package a dirty worktree. Commit or clean the release snapshot first."
}

$commit = (Invoke-GitText -Arguments @("rev-parse", "HEAD") | Select-Object -First 1).Trim()
if ($commit -notmatch '^[0-9a-f]{40}$') {
    throw "Unable to resolve a full Git commit for release provenance."
}

$manifestPath = Join-Path $script:RepoRoot "crates/code-intel-cli/Cargo.toml"
$manifestText = Get-Content -LiteralPath $manifestPath -Raw
$versionMatch = [regex]::Match($manifestText, '(?m)^version\s*=\s*"(?<version>[^"]+)"\s*$')
if (-not $versionMatch.Success) {
    throw "Unable to read the code-intel version from crates/code-intel-cli/Cargo.toml."
}
$version = $versionMatch.Groups['version'].Value
if ($ExpectedVersion -and $ExpectedVersion -cne $version) {
    throw "Release version mismatch: expected '$ExpectedVersion' from the tag, Cargo.toml declares '$version'."
}
$licenseMatch = [regex]::Match($manifestText, '(?m)^license\s*=\s*"(?<license>[^"]+)"\s*$')
if (-not $licenseMatch.Success) {
    throw "Unable to read the declared license from crates/code-intel-cli/Cargo.toml."
}
$declaredLicense = $licenseMatch.Groups['license'].Value
$readmeText = Get-Content -LiteralPath (Join-Path $script:RepoRoot "README.md") -Raw
$readmeLicenseMatch = [regex]::Match($readmeText, '(?ms)^## License\s*\r?\n\s*(?<license>[^\r\n]+)')
$readmeLicense = if ($readmeLicenseMatch.Success) { $readmeLicenseMatch.Groups['license'].Value.Trim() } else { "missing" }
$licenseFilePresent = Test-Path -LiteralPath (Join-Path $script:RepoRoot "LICENSE") -PathType Leaf
$licenseResolved = $licenseFilePresent -and $readmeLicense -ceq $declaredLicense
if ($DevelopmentOnlyAllowUnresolvedLicense -and -not $AllowDirty) {
    throw "-DevelopmentOnlyAllowUnresolvedLicense is permitted only with -AllowDirty and must never be used for an official package."
}
if (-not $licenseResolved -and -not $DevelopmentOnlyAllowUnresolvedLicense) {
    throw "Release license is unresolved: Cargo.toml declares '$declaredLicense', README declares '$readmeLicense', LICENSE present=$licenseFilePresent."
}

$metadataText = (& cargo metadata --manifest-path $manifestPath --locked --format-version 1 2>$null) -join "`n"
if ($LASTEXITCODE -ne 0) {
    throw "cargo metadata --locked failed with exit code $LASTEXITCODE."
}
$cargoMetadata = $metadataText | ConvertFrom-Json -Depth 100
$dependencies = @($cargoMetadata.packages | Sort-Object name, version | ForEach-Object {
    [ordered]@{
        name = $_.name
        version = $_.version
        source = if ($null -eq $_.source) { "workspace" } else { [string]$_.source }
        license = if ([string]::IsNullOrWhiteSpace([string]$_.license)) { "unknown" } else { [string]$_.license }
    }
})

$sourceArgs = @("ls-files")
if ($AllowDirty) {
    $sourceArgs += @("--cached", "--others", "--exclude-standard")
}
$sourceFiles = @(Invoke-GitText -Arguments $sourceArgs | Where-Object {
    -not [string]::IsNullOrWhiteSpace($_) -and $_ -notmatch '^(dist|target|work)/'
} | Sort-Object -Unique)
if ($licenseFilePresent -and -not ($sourceFiles -contains "LICENSE")) {
    throw "The repository-root LICENSE is not part of the release snapshot."
}

New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$packageRoot = Join-Path $outputRoot "code-intel-pipeline"
$zipPath = Join-Path $outputRoot "$PackageName.zip"
$checksumPath = "$zipPath.sha256"
$sidecarManifestPath = Join-Path $outputRoot "$PackageName.release-manifest.json"
foreach ($path in @($packageRoot, $zipPath, $checksumPath, $sidecarManifestPath)) {
    $fullPath = [System.IO.Path]::GetFullPath($path)
    if (-not $fullPath.StartsWith(($outputRoot.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar), [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Package output escaped the requested output directory: $fullPath"
    }
    if (Test-Path -LiteralPath $fullPath) {
        Remove-Item -LiteralPath $fullPath -Recurse -Force
    }
}
New-Item -ItemType Directory -Force -Path (Join-Path $packageRoot "bin") | Out-Null

foreach ($relativePath in $sourceFiles) {
    $sourcePath = [System.IO.Path]::GetFullPath((Join-Path $script:RepoRoot $relativePath))
    $expectedPrefix = $script:RepoRoot.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    if (-not $sourcePath.StartsWith($expectedPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Git returned a path outside the repository: $relativePath"
    }
    $targetPath = Join-Path $packageRoot $relativePath
    $targetParent = Split-Path -Parent $targetPath
    if ($targetParent) {
        New-Item -ItemType Directory -Force -Path $targetParent | Out-Null
    }
    Copy-Item -LiteralPath $sourcePath -Destination $targetPath -Force
}
Copy-Item -LiteralPath $resolvedExecutable -Destination (Join-Path $packageRoot "bin/code-intel.exe") -Force

$inventory = @(Get-ChildItem -LiteralPath $packageRoot -File -Recurse | Sort-Object FullName | ForEach-Object {
    [ordered]@{
        path = Get-RelativeSlashPath -BasePath $packageRoot -Path $_.FullName
        size = $_.Length
        sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
})
$lockPath = Join-Path $script:RepoRoot "Cargo.lock"
$binaryHash = (Get-FileHash -LiteralPath (Join-Path $packageRoot "bin/code-intel.exe") -Algorithm SHA256).Hash.ToLowerInvariant()
$generatedAt = [DateTimeOffset]::UtcNow.ToString('o')
$releaseManifest = [ordered]@{
    schema = "code-intel-beta-release-manifest.v1"
    product = "code-intel-pipeline"
    channel = "beta"
    version = $version
    generatedAt = $generatedAt
    source = [ordered]@{
        repository = "https://github.com/2233admin/code-intel-pipeline"
        commit = $commit
        dirty = $dirty
    }
    license = [ordered]@{
        declared = $declaredLicense
        readme = $readmeLicense
        filePresent = $licenseFilePresent
        resolved = $licenseResolved
    }
    provenance = [ordered]@{
        builder = if ($env:GITHUB_ACTIONS -eq 'true') { "github-actions/windows-latest" } else { "local-pwsh" }
        workflow = [string]$env:GITHUB_WORKFLOW
        runId = [string]$env:GITHUB_RUN_ID
        runAttempt = [string]$env:GITHUB_RUN_ATTEMPT
        cargoLockSha256 = (Get-FileHash -LiteralPath $lockPath -Algorithm SHA256).Hash.ToLowerInvariant()
        executableSha256 = $binaryHash
    }
    optionalCapabilities = @(
        [ordered]@{
            id = "semantic_memory"
            required = $false
            note = "Repowise remains in the default plan, but indexing and docs are optional for the beta core and may be bypassed with -SkipRepowise."
        },
        [ordered]@{
            id = "codenexus"
            required = $false
            note = "Optional localization/index assistance; not required for install, doctor, CLI help, or beta package smoke."
        }
    )
    dependencyInventory = [ordered]@{
        format = "cargo-metadata-v1"
        locked = $true
        packages = $dependencies
    }
    files = $inventory
}
$releaseManifest | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $packageRoot "release-manifest.json") -Encoding UTF8
Copy-Item -LiteralPath (Join-Path $packageRoot "release-manifest.json") -Destination $sidecarManifestPath -Force

Compress-Archive -LiteralPath $packageRoot -DestinationPath $zipPath -CompressionLevel Optimal -Force
$zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
"$zipHash  $(Split-Path -Leaf $zipPath)" | Set-Content -LiteralPath $checksumPath -Encoding ascii

[pscustomobject][ordered]@{
    ok = $true
    channel = "beta"
    version = $version
    commit = $commit
    dirty = $dirty
    zip = $zipPath
    sha256 = $zipHash
    checksum = $checksumPath
    manifest = $sidecarManifestPath
    fileCount = $inventory.Count
    dependencyCount = $dependencies.Count
}
