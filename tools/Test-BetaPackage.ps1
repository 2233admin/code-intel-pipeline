#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ZipPath,
    [string]$ChecksumPath = "$ZipPath.sha256",
    [string]$ManifestPath,
    [switch]$AllowDirty,
    [switch]$DevelopmentOnlyAllowUnresolvedLicense,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $false

function Assert-Condition {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

$resolvedZip = (Resolve-Path -LiteralPath $ZipPath).Path
$resolvedChecksum = (Resolve-Path -LiteralPath $ChecksumPath).Path
if (-not $ManifestPath) {
    $leaf = Split-Path -Leaf $resolvedZip
    $baseName = if ($leaf.EndsWith('.zip', [StringComparison]::OrdinalIgnoreCase)) { $leaf.Substring(0, $leaf.Length - 4) } else { $leaf }
    $ManifestPath = Join-Path (Split-Path -Parent $resolvedZip) "$baseName.release-manifest.json"
}
$resolvedManifest = (Resolve-Path -LiteralPath $ManifestPath).Path

$checksumLine = (Get-Content -LiteralPath $resolvedChecksum -Raw).Trim()
$checksumMatch = [regex]::Match($checksumLine, '^(?<hash>[0-9a-f]{64})  (?<file>[^\\/]+\.zip)$')
Assert-Condition $checksumMatch.Success "Checksum sidecar must use '<lowercase sha256>  <zip filename>'."
Assert-Condition ($checksumMatch.Groups['file'].Value -eq (Split-Path -Leaf $resolvedZip)) "Checksum sidecar names a different ZIP."
$actualZipHash = (Get-FileHash -LiteralPath $resolvedZip -Algorithm SHA256).Hash.ToLowerInvariant()
Assert-Condition ($actualZipHash -ceq $checksumMatch.Groups['hash'].Value) "ZIP SHA256 does not match its checksum sidecar."

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($resolvedZip)
try {
    foreach ($entry in $archive.Entries) {
        $entryPath = $entry.FullName.Replace('\', '/')
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($entryPath)) "ZIP contains an empty entry path."
        Assert-Condition (-not $entryPath.StartsWith('/')) "ZIP contains an absolute entry: $entryPath"
        Assert-Condition (-not [System.IO.Path]::IsPathRooted($entryPath)) "ZIP contains a rooted entry: $entryPath"
        Assert-Condition (-not (@($entryPath.Split('/')) -contains '..')) "ZIP contains a traversal entry: $entryPath"
        Assert-Condition ($entryPath.StartsWith('code-intel-pipeline/', [StringComparison]::Ordinal)) "ZIP entry is outside the package root: $entryPath"
    }
}
finally {
    $archive.Dispose()
}

$extractRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-beta-verify-" + [guid]::NewGuid().ToString('N'))
try {
    [System.IO.Compression.ZipFile]::ExtractToDirectory($resolvedZip, $extractRoot)
    $packageRoot = Join-Path $extractRoot "code-intel-pipeline"
    $embeddedManifestPath = Join-Path $packageRoot "release-manifest.json"
    Assert-Condition (Test-Path -LiteralPath $embeddedManifestPath -PathType Leaf) "Package is missing release-manifest.json."

    $embeddedManifestHash = (Get-FileHash -LiteralPath $embeddedManifestPath -Algorithm SHA256).Hash
    $sidecarManifestHash = (Get-FileHash -LiteralPath $resolvedManifest -Algorithm SHA256).Hash
    Assert-Condition ($embeddedManifestHash -ceq $sidecarManifestHash) "Embedded and sidecar release manifests differ."
    $manifest = Get-Content -LiteralPath $embeddedManifestPath -Raw | ConvertFrom-Json -Depth 100
    Assert-Condition ($manifest.schema -ceq 'code-intel-beta-release-manifest.v1') "Unexpected release manifest schema."
    Assert-Condition ($manifest.product -ceq 'code-intel-pipeline') "Unexpected release product."
    Assert-Condition ($manifest.channel -ceq 'beta') "Package is not marked as beta."
    Assert-Condition ($manifest.source.commit -match '^[0-9a-f]{40}$') "Manifest source commit is invalid."
    Assert-Condition ($AllowDirty -or $manifest.source.dirty -eq $false) "Official beta packages must come from a clean worktree."
    if ($DevelopmentOnlyAllowUnresolvedLicense -and -not $AllowDirty) {
        throw "-DevelopmentOnlyAllowUnresolvedLicense is permitted only with -AllowDirty."
    }
    if (-not $DevelopmentOnlyAllowUnresolvedLicense) {
        Assert-Condition ($manifest.license.resolved -eq $true) "Official beta package license is unresolved."
        Assert-Condition ($manifest.license.filePresent -eq $true) "Official beta package is missing LICENSE."
        Assert-Condition ($manifest.license.declared -ceq $manifest.license.readme) "README and Cargo license declarations differ."
        Assert-Condition (Test-Path -LiteralPath (Join-Path $packageRoot "LICENSE") -PathType Leaf) "Package is missing LICENSE."
    }
    Assert-Condition ($manifest.provenance.cargoLockSha256 -match '^[0-9a-f]{64}$') "Cargo.lock provenance digest is invalid."
    Assert-Condition ($manifest.provenance.executableSha256 -match '^[0-9a-f]{64}$') "Executable provenance digest is invalid."
    Assert-Condition ($manifest.dependencyInventory.format -ceq 'cargo-metadata-v1') "Dependency inventory format is invalid."
    Assert-Condition ($manifest.dependencyInventory.locked -eq $true) "Dependency inventory must come from the locked graph."
    Assert-Condition (@($manifest.dependencyInventory.packages).Count -gt 0) "Dependency inventory is empty."

    $codenexus = @($manifest.optionalCapabilities | Where-Object { $_.id -ceq 'codenexus' })
    Assert-Condition ($codenexus.Count -eq 1) "Manifest must describe CodeNexus optionality exactly once."
    Assert-Condition ($codenexus[0].required -eq $false) "CodeNexus must not be a beta package requirement."
    $semanticMemory = @($manifest.optionalCapabilities | Where-Object { $_.id -ceq 'semantic_memory' })
    Assert-Condition ($semanticMemory.Count -eq 1) "Manifest must describe semantic-memory optionality exactly once."
    Assert-Condition ($semanticMemory[0].required -eq $false) "Repowise semantic memory must not be a beta package requirement."

    $expectedPaths = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
    foreach ($item in @($manifest.files)) {
        $relativePath = [string]$item.path
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($relativePath)) "Manifest contains an empty file path."
        Assert-Condition (-not [System.IO.Path]::IsPathRooted($relativePath)) "Manifest contains a rooted file path: $relativePath"
        Assert-Condition (-not (@($relativePath.Split('/')) -contains '..')) "Manifest contains a traversal path: $relativePath"
        Assert-Condition ($expectedPaths.Add($relativePath)) "Manifest contains a duplicate path: $relativePath"
        $filePath = Join-Path $packageRoot $relativePath
        Assert-Condition (Test-Path -LiteralPath $filePath -PathType Leaf) "Manifest file is missing: $relativePath"
        $file = Get-Item -LiteralPath $filePath
        Assert-Condition ($file.Length -eq [long]$item.size) "File size mismatch: $relativePath"
        $hash = (Get-FileHash -LiteralPath $filePath -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-Condition ($hash -ceq [string]$item.sha256) "File SHA256 mismatch: $relativePath"
    }
    $actualPaths = @(Get-ChildItem -LiteralPath $packageRoot -File -Recurse | ForEach-Object {
        [System.IO.Path]::GetRelativePath($packageRoot, $_.FullName).Replace([System.IO.Path]::DirectorySeparatorChar, '/')
    } | Where-Object { $_ -cne 'release-manifest.json' })
    Assert-Condition ($actualPaths.Count -eq $expectedPaths.Count) "Package has files not covered by the release manifest."
    foreach ($actualPath in $actualPaths) {
        Assert-Condition ($expectedPaths.Contains($actualPath)) "Package file is not covered by the release manifest: $actualPath"
    }

    $parserErrors = New-Object System.Collections.Generic.List[object]
    foreach ($scriptPath in @(Get-ChildItem -LiteralPath $packageRoot -Filter '*.ps1' -File -Recurse)) {
        $tokens = $null
        $errors = $null
        $null = [System.Management.Automation.Language.Parser]::ParseFile($scriptPath.FullName, [ref]$tokens, [ref]$errors)
        foreach ($parseError in @($errors)) { $parserErrors.Add($parseError) }
    }
    Assert-Condition ($parserErrors.Count -eq 0) "Packaged PowerShell scripts contain parser errors."

    $executable = Join-Path $packageRoot "bin/code-intel.exe"
    Assert-Condition (Test-Path -LiteralPath $executable -PathType Leaf) "Package is missing bin/code-intel.exe."
    $helpText = (& $executable --help 2>&1) -join "`n"
    Assert-Condition ($LASTEXITCODE -eq 0) "Packaged code-intel.exe --help failed."
    Assert-Condition ($helpText -match 'code-intel') "Packaged CLI help output is unexpected."

    $invokeWrapper = Join-Path $packageRoot "invoke-code-intel.ps1"
    Assert-Condition (Test-Path -LiteralPath $invokeWrapper -PathType Leaf) "Package is missing invoke-code-intel.ps1."
    $pwshExecutable = (Get-Process -Id $PID).Path
    $originalPath = $env:PATH
    try {
        $pathSeparator = [System.IO.Path]::PathSeparator
        $env:PATH = (@($originalPath.Split($pathSeparator) | Where-Object {
            $entry = $_
            -not (Test-Path -LiteralPath (Join-Path $entry 'cargo.exe') -PathType Leaf) -and
            -not (Test-Path -LiteralPath (Join-Path $entry 'cargo') -PathType Leaf) -and
            -not (Test-Path -LiteralPath (Join-Path $entry 'repowise.exe') -PathType Leaf) -and
            -not (Test-Path -LiteralPath (Join-Path $entry 'repowise') -PathType Leaf)
        }) -join $pathSeparator)
        Assert-Condition ($null -eq (Get-Command cargo -ErrorAction SilentlyContinue)) "Package smoke must run without Cargo on PATH."
        Assert-Condition ($null -eq (Get-Command repowise -ErrorAction SilentlyContinue)) "Package smoke must run without Repowise on PATH."
        $wrapperSmoke = (& $pwshExecutable -NoProfile -File $invokeWrapper -ValidateInstallation 2>&1) -join "`n"
    }
    finally {
        $env:PATH = $originalPath
    }
    Assert-Condition ($LASTEXITCODE -eq 0) "Packaged invoke-code-intel.ps1 -ValidateInstallation failed: $wrapperSmoke"
    Assert-Condition ($wrapperSmoke -match 'installation validation passed') "Packaged wrapper smoke output is unexpected."
    Assert-Condition ($wrapperSmoke -match 'default route is the manifest-bound Rust DAG') "Packaged wrapper did not validate the authoritative Rust DAG route."

    $result = [pscustomobject][ordered]@{
        ok = $true
        channel = $manifest.channel
        version = $manifest.version
        commit = $manifest.source.commit
        zipSha256 = $actualZipHash
        verifiedFiles = $actualPaths.Count
        verifiedDependencies = @($manifest.dependencyInventory.packages).Count
        powershellParserErrors = $parserErrors.Count
        cliHelpSmoke = "passed"
        packagedWrapperSmoke = "passed"
        cargoRequiredForPackageSmoke = $false
        semanticIndexingRequired = $semanticMemory[0].required
        codenexusRequired = $codenexus[0].required
    }
}
finally {
    if (Test-Path -LiteralPath $extractRoot) {
        Remove-Item -LiteralPath $extractRoot -Recurse -Force
    }
}

if ($Json) {
    $result | ConvertTo-Json -Depth 8
}
else {
    $result | Format-List
}
