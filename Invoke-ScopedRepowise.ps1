param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,

    [string]$ShadowRoot = "D:\projects\_cache\code-intel\repowise",
    [string[]]$ScopePaths = @(),
    [string[]]$RootFiles = @(),
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
    & robocopy $Source $Destination /MIR /XD .git .repowise node_modules .venv venv __pycache__ dist build target .understand-anything .sentrux /NFL /NDL /NJH /NJS /NP | Out-Null
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
    param([string]$ShadowPath)

    $configDir = Join-Path $ShadowPath ".repowise"
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null
    $configPath = Join-Path $configDir "config.yaml"
    $lines = @(
        "provider: anthropic",
        "model: MiniMax-M2.7",
        "embedder: mock",
        "reasoning: auto",
        "commit_limit: 100",
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

$repoPath = Resolve-Dir $RepoPath
if (-not (Test-Path -LiteralPath (Join-Path $repoPath ".git"))) {
    throw "Repo is not a git repository: $repoPath"
}

$repoName = Split-Path -Leaf $repoPath
$shadowPath = Join-Path $ShadowRoot $repoName
$scopeDirs = @($ScopePaths | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } | Select-Object -Unique)
$scopeFiles = @($RootFiles | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } | Select-Object -Unique)

if ($scopeDirs.Count -eq 0 -and $scopeFiles.Count -eq 0) {
    throw "At least one scope path or root file is required."
}

$head = (git -C $repoPath rev-parse HEAD).Trim()

if (-not (Test-Path -LiteralPath $shadowPath -PathType Container)) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $shadowPath) | Out-Null
    git -C $repoPath worktree add --detach $shadowPath $head | Out-Null
}
else {
    $shadowGit = Join-Path $shadowPath ".git"
    if (-not (Test-Path -LiteralPath $shadowGit)) {
        throw "Shadow path exists but is not a git worktree: $shadowPath"
    }
    $shadowHead = (git -C $shadowPath rev-parse HEAD).Trim()
    if ($shadowHead -ne $head) {
        git -C $shadowPath checkout --detach --force $head | Out-Null
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

Write-ScopedConfig $shadowPath

[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$OutputEncoding = [System.Text.UTF8Encoding]::new()
$env:PYTHONIOENCODING = "utf-8"
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
            repowise init . --index-only -y
        }
        $dbPath = Join-Path $shadowPath ".repowise\wiki.db"
        if (Test-Path -LiteralPath $dbPath -PathType Leaf) {
            Remove-Item -LiteralPath $dbPath -Force
        }
        $scriptPath = Join-Path $PSScriptRoot "Run-ScopedRepowiseDocs.py"
        python $scriptPath --repo $shadowPath --coverage-pct 0.02 --concurrency 1
    }
    else {
        if (Test-Path -LiteralPath $statePath -PathType Leaf) {
            repowise update --no-workspace --index-only
        }
        else {
            @("n") | repowise init . --index-only -y
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
    $status = repowise status --no-workspace 2>&1
    if (Test-Path -LiteralPath $statePath -PathType Leaf) {
        $state = Get-Content -LiteralPath $statePath -Raw
    }
}
finally {
    Pop-Location
}

Write-Host "Scoped repowise complete"
Write-Host "Shadow: $shadowPath"
Write-Host "HEAD: $head"
Write-Host "ScopeDirs: $($scopeDirs -join ', ')"
Write-Host "RootFiles: $($scopeFiles -join ', ')"
Write-Host "Status:"
Write-Host ($status | Out-String).Trim()
if (-not [string]::IsNullOrWhiteSpace($state)) {
    Write-Host "State:"
    Write-Host $state
}
