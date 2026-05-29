param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,

    [string]$ShadowRoot = "",
    [string[]]$ScopePaths = @(),
    [string[]]$RootFiles = @(),
    [int]$CommitLimit = 25,
    [switch]$Docs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-Dir {
    param([string]$Path)
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        throw "Not a directory: $Path"
    }
    return $item.FullName
}

function Get-DefaultShadowRoot {
    $fromEnv = [Environment]::GetEnvironmentVariable("CODE_INTEL_SHADOW_ROOT", "User")
    if (-not [string]::IsNullOrWhiteSpace($fromEnv)) { return $fromEnv }
    if (-not [string]::IsNullOrWhiteSpace($env:CODE_INTEL_SHADOW_ROOT)) { return $env:CODE_INTEL_SHADOW_ROOT }
    $base = if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { $env:LOCALAPPDATA } else { (Join-Path $HOME ".code-intel") }
    return (Join-Path $base "code-intel\repowise")
}

function Resolve-RelativePath {
    param(
        [string]$Base,
        [string]$Path
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "Path cannot be empty"
    }
    if ([System.IO.Path]::IsPathRooted($Path)) {
        throw "Scope paths must be relative: $Path"
    }
    return Join-Path $Base $Path
}

function Invoke-RobocopyMirror {
    param(
        [string]$Source,
        [string]$Destination
    )

    if (-not (Test-Path -LiteralPath $Source -PathType Container)) {
        if (Test-Path -LiteralPath $Destination -PathType Container) {
            Remove-Item -LiteralPath $Destination -Recurse -Force
        }
        return
    }

    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    & robocopy $Source $Destination /MIR /XD .git .repowise node_modules .venv venv __pycache__ .pytest_cache .mypy_cache tmp dist build target .understand-anything .sentrux "*.egg-info" /XF uv.lock uv.lock.bak "*.bak" "=*" /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -gt 7) {
        throw "robocopy failed for $Source -> $Destination (exit $LASTEXITCODE)"
    }
}

function Copy-ScopedFile {
    param(
        [string]$Source,
        [string]$Destination
    )

    if (Test-Path -LiteralPath $Source -PathType Leaf) {
        $parent = Split-Path -Parent $Destination
        if (-not [string]::IsNullOrWhiteSpace($parent)) {
            New-Item -ItemType Directory -Force -Path $parent | Out-Null
        }
        Copy-Item -LiteralPath $Source -Destination $Destination -Force
    }
    elseif (Test-Path -LiteralPath $Destination -PathType Leaf) {
        Remove-Item -LiteralPath $Destination -Force
    }
}

function Write-ScopedConfig {
    param(
        [string]$ShadowPath,
        [int]$CommitLimit
    )

    $configDir = Join-Path $ShadowPath ".repowise"
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null
    $configPath = Join-Path $configDir "config.yaml"
    $lines = @(
        "provider: anthropic",
        "model: MiniMax-M2.7",
        "embedder: mock",
        "reasoning: auto",
        "commit_limit: $CommitLimit",
        "editor_files:",
        "  claude_md: false"
    )
    $lines | Set-Content -LiteralPath $configPath -Encoding UTF8
}

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

function Remove-ScopedNoise {
    param([string]$Root)

    $rootItem = Get-Item -LiteralPath $Root -ErrorAction Stop
    $rootFull = $rootItem.FullName.TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
    $dirNames = @("tmp", "__pycache__", ".pytest_cache", ".mypy_cache", "node_modules", "target", "dist", "build")
    $dirs = @(
        Get-ChildItem -LiteralPath $Root -Force -Recurse -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -in $dirNames -or $_.Name -like "*.egg-info" }
    )
    foreach ($dir in $dirs) {
        if ($dir.FullName.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
            Remove-Item -LiteralPath $dir.FullName -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    $files = @(
        Get-ChildItem -LiteralPath $Root -Force -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -in @("uv.lock", "uv.lock.bak") -or $_.Name -like "*.bak" -or $_.Name -like "=*" }
    )
    foreach ($file in $files) {
        if ($file.FullName.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
            Remove-Item -LiteralPath $file.FullName -Force -ErrorAction SilentlyContinue
        }
    }
}

function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock]$Script,
        [string]$Description = "native command"
    )

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $global:LASTEXITCODE = 0
        $ErrorActionPreference = "Continue"
        $output = & $Script 2>&1
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }

    $text = ($output | ForEach-Object { $_.ToString() } | Out-String).Trim()
    if ($global:LASTEXITCODE -ne 0) {
        throw "$Description failed with exit code $global:LASTEXITCODE. $text"
    }
    return $text
}

$repoPath = Resolve-Dir $RepoPath
if ([string]::IsNullOrWhiteSpace($ShadowRoot)) {
    $ShadowRoot = Get-DefaultShadowRoot
}
if (-not (Test-Path -LiteralPath (Join-Path $repoPath ".git"))) {
    throw "Repo is not a git repository: $repoPath"
}

$repoName = Split-Path -Leaf $repoPath
$scopeDirs = @($ScopePaths | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } | Select-Object -Unique)
$scopeFiles = @($RootFiles | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } | Select-Object -Unique)

if ($scopeDirs.Count -eq 0 -and $scopeFiles.Count -eq 0) {
    throw "At least one scope path or root file is required."
}

$scopeKey = (@($scopeDirs + $scopeFiles) -join "-")
$scopeSlug = $scopeKey -replace '[:/\\]+', '-'
$scopeSlug = $scopeSlug -replace '[^A-Za-z0-9._-]', '-'
$scopeSlug = $scopeSlug.Trim("-")
if ([string]::IsNullOrWhiteSpace($scopeSlug)) {
    $scopeSlug = "scoped"
}
if ($scopeSlug.Length -gt 80) {
    $scopeSlug = $scopeSlug.Substring(0, 80).Trim("-")
}
$shadowPath = Join-Path $ShadowRoot "$repoName-$scopeSlug"

$head = (git -C $repoPath rev-parse HEAD).Trim()

if (-not (Test-Path -LiteralPath $shadowPath -PathType Container)) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $shadowPath) | Out-Null
    [void](Invoke-NativeCommand -Description "git worktree add" -Script { git -C $repoPath worktree add --detach $shadowPath $head })
}
else {
    $shadowGit = Join-Path $shadowPath ".git"
    if (-not (Test-Path -LiteralPath $shadowGit)) {
        throw "Shadow path exists but is not a git worktree: $shadowPath"
    }
    $shadowHead = (git -C $shadowPath rev-parse HEAD).Trim()
    if ($shadowHead -ne $head) {
        [void](Invoke-NativeCommand -Description "git checkout shadow" -Script { git -C $shadowPath checkout --detach --force $head })
    }
}

Push-Location $shadowPath
try {
    git sparse-checkout init --no-cone | Out-Null
    $patterns = New-Object System.Collections.Generic.List[string]
    foreach ($dir in $scopeDirs) {
        $patterns.Add("/$($dir.Trim('/'))/")
    }
    foreach ($file in $scopeFiles) {
        $patterns.Add("/$($file.Trim('/'))")
    }
    $patterns | git sparse-checkout set --stdin | Out-Null
    git read-tree -mu HEAD | Out-Null
}
finally {
    Pop-Location
}

foreach ($dir in $scopeDirs) {
    $sourceDir = Resolve-RelativePath $repoPath $dir
    $destDir = Resolve-RelativePath $shadowPath $dir
    Invoke-RobocopyMirror $sourceDir $destDir
}

foreach ($file in $scopeFiles) {
    $sourceFile = Resolve-RelativePath $repoPath $file
    $destFile = Resolve-RelativePath $shadowPath $file
    Copy-ScopedFile $sourceFile $destFile
}

Remove-ScopedNoise $shadowPath
Write-ScopedConfig $shadowPath $CommitLimit

[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$OutputEncoding = [System.Text.UTF8Encoding]::new()
$env:PYTHONIOENCODING = "utf-8"
$env:PYTHONUTF8 = "1"
$env:TERM = "xterm"
$env:NO_COLOR = "1"
$env:RICH_FORCE_TERMINAL = "0"
$env:REPOWISE_SKIP_HOOK_INSTALL = "1"
Set-EnvFromUserRegistry "ANTHROPIC_API_KEY"
Set-EnvFromUserRegistry "ANTHROPIC_BASE_URL"
Set-EnvFromUserRegistry "REPOWISE_PROVIDER"

$statePath = Join-Path $shadowPath ".repowise\state.json"
$docsEnabled = $false
if (Test-Path -LiteralPath $statePath -PathType Leaf) {
    try {
        $existingState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
        $docsEnabled = [bool](Get-Member -InputObject $existingState -Name "docs_enabled" -MemberType NoteProperty) -and [bool]$existingState.docs_enabled
    }
    catch {
        $docsEnabled = $false
    }
}
Push-Location $shadowPath
try {
    if ($Docs) {
        if (-not (Test-Path -LiteralPath $statePath -PathType Leaf)) {
            [void](Invoke-NativeCommand -Description "repowise init" -Script { @("n") | repowise init . --index-only -y --no-claude-md --no-onboarding --skip-tests --skip-infra --commit-limit $CommitLimit --embedder mock --provider mock -x "tmp/**" -x "**/tmp/**" -x "**/*.egg-info/**" -x "uv.lock" -x "**/uv.lock" -x "*.bak" -x "**/*.bak" })
        }
        $dbPath = Join-Path $shadowPath ".repowise\wiki.db"
        if (Test-Path -LiteralPath $dbPath -PathType Leaf) {
            Remove-Item -LiteralPath $dbPath -Force
        }
        $scriptPath = Join-Path $PSScriptRoot "Run-ScopedRepowiseDocs.py"
        [void](Invoke-NativeCommand -Description "repowise scoped docs" -Script { python $scriptPath --repo $shadowPath --coverage-pct 0.02 --concurrency 1 })
    }
    else {
        if (Test-Path -LiteralPath $statePath -PathType Leaf) {
            [void](Invoke-NativeCommand -Description "repowise update" -Script { repowise update --no-workspace --index-only })
        }
        else {
            [void](Invoke-NativeCommand -Description "repowise init" -Script { @("n") | repowise init . --index-only -y --no-claude-md --no-onboarding --skip-tests --skip-infra --commit-limit $CommitLimit --embedder mock --provider mock -x "tmp/**" -x "**/tmp/**" -x "**/*.egg-info/**" -x "uv.lock" -x "**/uv.lock" -x "*.bak" -x "**/*.bak" })
        }
    }
}
finally {
    Pop-Location
}

$status = $null
$state = ""
Push-Location $shadowPath
try {
    $status = Invoke-NativeCommand -Description "repowise status" -Script { repowise status --no-workspace }
    if (Test-Path -LiteralPath $statePath -PathType Leaf) {
        $state = Get-Content -LiteralPath $statePath -Raw
    }
}
finally {
    Pop-Location
}

$dbPath = Join-Path $shadowPath ".repowise\wiki.db"
if ((-not (Test-Path -LiteralPath $statePath -PathType Leaf)) -and (-not (Test-Path -LiteralPath $dbPath -PathType Leaf))) {
    throw "Scoped repowise finished without .repowise\state.json or .repowise\wiki.db: $shadowPath"
}

Write-Output "Scoped repowise complete"
Write-Output "Shadow: $shadowPath"
Write-Output "HEAD: $head"
Write-Output "ScopeDirs: $($scopeDirs -join ', ')"
Write-Output "RootFiles: $($scopeFiles -join ', ')"
Write-Output "Status:"
Write-Output ($status | Out-String).Trim()
if (-not [string]::IsNullOrWhiteSpace($state)) {
    Write-Output "State:"
    Write-Output $state
}
