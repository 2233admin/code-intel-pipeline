#requires -Version 7.2

param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,

    [string]$TargetPath = "",
    [string]$RunDir = "",
    [string]$DsmPath = "",
    [string]$HotspotsPath = "",
[string]$OutputPath = "",
[int]$MaxFiles = 8,
[int]$MaxReferencesPerFile = 12,
[int]$MaxCommitsPerFile = 0,
[string]$AdapterRequestPath = "",
[string]$ExpectedSnapshotIdentity = "",
[string]$SourceSnapshotIdentity = "",
[string]$SourceRevision = "",
[long]$ObservedAt = 0,
[ValidateSet("explicit_fallback", "legacy_rollback")]
[string]$AdapterActivation = "explicit_fallback",
[switch]$Quiet
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-Directory {
    param([string]$Path)

    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        throw "Not a directory: $Path"
    }
    return $item.FullName
}

function Read-JsonFileSafe {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path) -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $null
    }
    try {
        return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    }
    catch {
        return $null
    }
}

function Get-RelativePathSafe {
    param(
        [string]$Base,
        [string]$Path
    )

    try {
        return [System.IO.Path]::GetRelativePath($Base, $Path)
    }
    catch {
        try {
            $baseFull = [System.IO.Path]::GetFullPath($Base)
            $pathFull = [System.IO.Path]::GetFullPath($Path)
            if (-not $baseFull.EndsWith([System.IO.Path]::DirectorySeparatorChar)) {
                $baseFull = $baseFull + [System.IO.Path]::DirectorySeparatorChar
            }
            if ((Test-Path -LiteralPath $pathFull -PathType Container) -and -not $pathFull.EndsWith([System.IO.Path]::DirectorySeparatorChar)) {
                $pathFull = $pathFull + [System.IO.Path]::DirectorySeparatorChar
            }
            $relative = ([uri]$baseFull).MakeRelativeUri([uri]$pathFull).ToString()
            $relative = [uri]::UnescapeDataString($relative).Replace("/", [System.IO.Path]::DirectorySeparatorChar)
            if ([string]::IsNullOrWhiteSpace($relative)) { return "." }
            return $relative
        }
        catch {
            return $Path
        }
    }
}

function Invoke-TextCommand {
    param([scriptblock]$Body)

    try {
        $global:LASTEXITCODE = 0
        $output = & $Body 2>&1
        $text = ($output | ForEach-Object { $_.ToString() } | Out-String).Trim()
        if ($global:LASTEXITCODE -ne 0 -and [string]::IsNullOrWhiteSpace($text)) {
            return @()
        }
        return @($text -split "\r?\n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    }
    catch {
        return @()
    }
}

function Test-CodeNexusGeneratedPath {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) { return $false }
    $normalized = $Path.Replace('\', '/')
    while ($normalized.StartsWith('./', [System.StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    return $normalized -match '(?i)(^|/)(work|artifact|artifacts|staging|\.code-intel|\.git|node_modules|target|dist|build|\.venv|__pycache__)(/|$)'
}

function Select-HotspotFiles {
    param(
        [string]$RepoPath,
        [string]$TargetPath,
        [object]$Hotspots,
        [object]$Dsm,
        [int]$MaxFiles
    )

    $seen = @{}
    $items = New-Object System.Collections.Generic.List[object]

    if ($null -ne $Hotspots -and $null -ne $Hotspots.files) {
        foreach ($file in @($Hotspots.files)) {
            if ($items.Count -ge $MaxFiles) { break }
            $path = [string]$file.path
            if ([string]::IsNullOrWhiteSpace($path) -or (Test-CodeNexusGeneratedPath $path) -or $seen.ContainsKey($path)) { continue }
            $seen[$path] = $true
            $items.Add([pscustomobject][ordered]@{
                path = $path
                reason = "sentrux_hotspot"
                maxComplexity = if ($null -ne $file.maxComplexity) { [int]$file.maxComplexity } else { $null }
                functionCount = if ($null -ne $file.functionCount) { [int]$file.functionCount } else { $null }
                riskScore = $null
            })
        }
    }

    if ($items.Count -lt $MaxFiles -and $null -ne $Dsm -and $null -ne $Dsm.modules) {
        foreach ($module in @($Dsm.modules | Sort-Object { $_.metrics.risk } -Descending)) {
            foreach ($path in @($module.files)) {
                if ($items.Count -ge $MaxFiles) { break }
                $pathText = [string]$path
                if ([string]::IsNullOrWhiteSpace($pathText) -or (Test-CodeNexusGeneratedPath $pathText) -or $seen.ContainsKey($pathText)) { continue }
                $seen[$pathText] = $true
                $items.Add([pscustomobject][ordered]@{
                    path = $pathText
                    reason = "sentrux_module_risk"
                    maxComplexity = $null
                    functionCount = $null
                    riskScore = $module.metrics.risk
                })
            }
            if ($items.Count -ge $MaxFiles) { break }
        }
    }

    if ($items.Count -lt $MaxFiles) {
        $root = if ([string]::IsNullOrWhiteSpace($TargetPath)) { $RepoPath } else { $TargetPath }
        $fallbackFiles = Get-ChildItem -LiteralPath $root -Recurse -File -ErrorAction SilentlyContinue |
            Where-Object {
                $relativePath = Get-RelativePathSafe $RepoPath $_.FullName
                $_.Length -le 1048576 -and
                -not (Test-CodeNexusGeneratedPath $relativePath) -and
                $_.Extension.ToLowerInvariant() -in @(".ps1", ".py", ".rs", ".go", ".ts", ".tsx", ".js", ".jsx", ".java", ".cs")
            } |
            Sort-Object Length -Descending |
            Select-Object -First $MaxFiles
        foreach ($file in $fallbackFiles) {
            if ($items.Count -ge $MaxFiles) { break }
            $path = Get-RelativePathSafe $RepoPath $file.FullName
            if ($seen.ContainsKey($path)) { continue }
            $seen[$path] = $true
            $items.Add([pscustomobject][ordered]@{
                path = $path
                reason = "largest_code_file"
                maxComplexity = $null
                functionCount = $null
                riskScore = $null
            })
        }
    }

    return $items.ToArray()
}

function Get-RecentCommits {
    param(
        [string]$RepoPath,
        [string]$RelativePath,
        [int]$Limit
    )

    if ($Limit -le 0) { return ,@() }
    $lines = Invoke-TextCommand { git -C $RepoPath --no-pager log --oneline --max-count=$Limit -- $RelativePath }
    return ,@($lines)
}

function Get-References {
    param(
        [string]$RepoPath,
        [string]$RelativePath,
        [int]$Limit
    )

    if (-not (Get-Command rg -ErrorAction SilentlyContinue)) {
        return ,@()
    }
    if ($Limit -le 0) { return ,@() }

    $stem = [System.IO.Path]::GetFileNameWithoutExtension($RelativePath)
    if ([string]::IsNullOrWhiteSpace($stem) -or $stem.Length -lt 3) {
        return ,@()
    }

    $lines = Invoke-TextCommand {
        rg -n -m $Limit --hidden `
            -g "!**/work/**" `
            -g "!**/artifact/**" `
            -g "!**/artifacts/**" `
            -g "!**/staging/**" `
            -g "!**/.code-intel/**" `
            -g "!**/.git/**" `
            -g "!**/node_modules/**" `
            -g "!**/target/**" `
            -g "!**/dist/**" `
            -g "!**/build/**" `
            -g "!**/.venv/**" `
            -g "!**/__pycache__/**" `
            --fixed-strings $stem $RepoPath
    }
    return ,@($lines | Select-Object -First $Limit)
}

function Get-FileDigest {
    param(
        [string]$RepoPath,
        [string]$RelativePath
    )

    $path = Join-Path $RepoPath $RelativePath
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        return [ordered]@{
            exists = $false
            loc = 0
            firstLines = @()
        }
    }

    $lines = @()
    try {
        $lines = @((Get-Content -LiteralPath $path -TotalCount 12 -ErrorAction Stop) | ForEach-Object { [string]$_ })
    }
    catch {
        $lines = @()
    }
    $loc = 0
    try {
        $loc = @(Get-Content -LiteralPath $path -ErrorAction Stop).Count
    }
    catch {
        $loc = 0
    }

    return [ordered]@{
        exists = $true
        loc = $loc
        firstLines = $lines
    }
}

$repoRoot = Resolve-Directory $RepoPath
$targetRoot = if ([string]::IsNullOrWhiteSpace($TargetPath)) { $repoRoot } else { Resolve-Directory $TargetPath }
if ([string]::IsNullOrWhiteSpace($RunDir)) {
    $RunDir = Join-Path $repoRoot ".code-intel"
}
New-Item -ItemType Directory -Force -Path $RunDir | Out-Null
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $RunDir "codenexus-context.json"
}

$hotspots = Read-JsonFileSafe $HotspotsPath
$dsm = Read-JsonFileSafe $DsmPath
$selectedFiles = Select-HotspotFiles $repoRoot $targetRoot $hotspots $dsm $MaxFiles
$fileContexts = @()
foreach ($file in $selectedFiles) {
    $relative = [string]$file.path
    $fileContexts += [ordered]@{
        path = $relative
        reason = $file.reason
        maxComplexity = $file.maxComplexity
        functionCount = $file.functionCount
        riskScore = $file.riskScore
        digest = Get-FileDigest $repoRoot $relative
        recentCommits = Get-RecentCommits $repoRoot $relative $MaxCommitsPerFile
        references = Get-References $repoRoot $relative $MaxReferencesPerFile
    }
}

$payload = [ordered]@{
    tool = "codenexus-lite"
    generatedAt = (Get-Date).ToUniversalTime().ToString("o")
    repo = $repoRoot
    target = $targetRoot
    output = $OutputPath
    sources = [ordered]@{
        dsm = $DsmPath
        hotspots = $HotspotsPath
    }
    summary = [ordered]@{
        files = $fileContexts.Count
        references = [int](($fileContexts | ForEach-Object { @($_.references).Count } | Measure-Object -Sum).Sum)
        recentCommits = [int](($fileContexts | ForEach-Object { @($_.recentCommits).Count } | Measure-Object -Sum).Sum)
    }
    files = $fileContexts
    nextQueries = @(
        "Inspect top files by reason=sentrux_hotspot before editing.",
        "Use references to estimate blast radius before changing public functions.",
        "Use recentCommits to identify ownership or churn before accepting a baseline."
    )
    limitations = @(
        "This is deterministic CodeNexus-lite context, not a semantic embedding graph.",
        "It is designed to be portable on a fresh machine and can be replaced by a full CodeNexus backend later."
    )
}

$payload | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $OutputPath -Encoding UTF8

if (-not [string]::IsNullOrWhiteSpace($AdapterRequestPath)) {
    foreach ($identity in @($ExpectedSnapshotIdentity, $SourceSnapshotIdentity)) {
        if ($identity -notmatch '^[0-9a-f]{64}$') {
            throw "CodeNexus adapter snapshot identities must be lowercase SHA-256 values"
        }
    }
    if ([string]::IsNullOrWhiteSpace($SourceRevision)) {
        throw "CodeNexus adapter source revision is required"
    }
    if ($ObservedAt -le 0) {
        $ObservedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    }
    $evidencePayloadPath = Join-Path $RunDir "codenexus-evidence-payload.json"
    $evidencePayload = [ordered]@{
        schema = "code-intel-evidence-payload.v1"
        data = [ordered]@{
            codenexus = [ordered]@{
                schema = "code-intel-codenexus-evidence.v1"
                snapshotIdentity = $SourceSnapshotIdentity
                provider = [ordered]@{
                    mode = "lite"
                    providerId = "codenexus.lite-compat"
                    implementationId = "invoke-codenexus-lite.ps1"
                    activation = $AdapterActivation
                }
                provenance = [ordered]@{
                    sourceRevision = $SourceRevision
                    observedAt = $ObservedAt
                }
                completeness = "complete"
                availability = "available"
                providerData = $payload
            }
        }
    }
    [System.IO.File]::WriteAllText(
        $evidencePayloadPath,
        ($evidencePayload | ConvertTo-Json -Depth 20 -Compress),
        [System.Text.UTF8Encoding]::new($false)
    )
    $payloadDigest = (Get-FileHash -LiteralPath $evidencePayloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $implementationDigest = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $artifactRelativePath = [System.IO.Path]::GetRelativePath($RunDir, $evidencePayloadPath).Replace('\', '/')
    $adapterRequest = [ordered]@{
        schema = "code-intel-codenexus-native-result.v1"
        providerMode = "lite"
        status = "current"
        providerId = "codenexus.lite-compat"
        implementation = [ordered]@{
            id = "invoke-codenexus-lite.ps1"
            version = "compat-v1"
            digest = $implementationDigest
        }
        sourceRevision = $SourceRevision
        expectedSnapshotIdentity = $ExpectedSnapshotIdentity
        sourceSnapshotIdentity = $SourceSnapshotIdentity
        collectedAt = $ObservedAt
        observedAt = $ObservedAt
        payload = [ordered]@{
            schema = "code-intel-artifact-ref.v1"
            artifactSchema = "code-intel-evidence-payload.v1"
            type = "observed.evidence.payload"
            path = $artifactRelativePath
            sha256 = $payloadDigest
            consumedSnapshotIdentity = $SourceSnapshotIdentity
        }
        activation = $AdapterActivation
        effects = @(
            "read_repository",
            "read_git_history",
            "read_sentrux_artifacts",
            "write_compatibility_artifact"
        )
    }
    [System.IO.File]::WriteAllText(
        $AdapterRequestPath,
        ($adapterRequest | ConvertTo-Json -Depth 20 -Compress),
        [System.Text.UTF8Encoding]::new($false)
    )
}
if (-not $Quiet) {
    $payload | ConvertTo-Json -Depth 8
}
