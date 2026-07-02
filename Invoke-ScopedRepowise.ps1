#requires -Version 7.2

param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,

    [string]$ShadowRoot = "",
    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto",
    [string[]]$ScopePaths = @(),
    [string[]]$RootFiles = @(),
    [int]$CommitLimit = 25,
    [int]$TimeoutSeconds = 600,
    [string]$Provider = "mock",
    [string]$Model = "",
    [string]$Reasoning = "auto",
    [switch]$Docs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$platformModule = Join-Path (Join-Path $PSScriptRoot "tools") "code-intel-platform.psm1"
Import-Module $platformModule -Force
$effectivePlatform = Get-CodeIntelPlatform -Platform $Platform

function Resolve-Dir {
    param([string]$Path)
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        throw "Not a directory: $Path"
    }
    return $item.FullName
}

function Get-DefaultShadowRoot {
    return (Get-CodeIntelShadowRoot -Platform $effectivePlatform)
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

    if ($effectivePlatform -eq "windows" -and (Get-Command robocopy -ErrorAction SilentlyContinue)) {
        New-Item -ItemType Directory -Force -Path $Destination | Out-Null
        & robocopy $Source $Destination /MIR /XD .git .repowise node_modules .venv venv __pycache__ .pytest_cache .mypy_cache tmp dist build target .understand-anything .sentrux "*.egg-info" /XF uv.lock uv.lock.bak "*.bak" "=*" /NFL /NDL /NJH /NJS /NP | Out-Null
        if ($LASTEXITCODE -gt 7) {
            throw "robocopy failed for $Source -> $Destination (exit $LASTEXITCODE)"
        }
        $global:LASTEXITCODE = 0
        return
    }

    if ($effectivePlatform -ne "windows" -and (Get-Command rsync -ErrorAction SilentlyContinue)) {
        New-Item -ItemType Directory -Force -Path $Destination | Out-Null
        $sourceArg = $Source.TrimEnd("/", "\") + "/"
        $destinationArg = $Destination.TrimEnd("/", "\") + "/"
        $rsyncArgs = @(
            "-a",
            "--delete",
            "--exclude=.git",
            "--exclude=.repowise",
            "--exclude=node_modules",
            "--exclude=.venv",
            "--exclude=venv",
            "--exclude=__pycache__",
            "--exclude=.pytest_cache",
            "--exclude=.mypy_cache",
            "--exclude=tmp",
            "--exclude=dist",
            "--exclude=build",
            "--exclude=target",
            "--exclude=.understand-anything",
            "--exclude=.sentrux",
            "--exclude=*.egg-info",
            "--exclude=uv.lock",
            "--exclude=uv.lock.bak",
            "--exclude=*.bak",
            "--exclude==*",
            $sourceArg,
            $destinationArg
        )
        & rsync @rsyncArgs
        if ($LASTEXITCODE -ne 0) {
            throw "rsync failed for $Source -> $Destination (exit $LASTEXITCODE)"
        }
        return
    }

    if (Test-Path -LiteralPath $Destination -PathType Container) {
        Remove-Item -LiteralPath $Destination -Recurse -Force
    }
    Copy-Item -LiteralPath $Source -Destination $Destination -Recurse -Force
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

function Get-CodeIntelEnvValue {
    param([string]$Name)

    $value = [Environment]::GetEnvironmentVariable($Name, "Process")
    if ([string]::IsNullOrWhiteSpace($value)) {
        $value = [Environment]::GetEnvironmentVariable($Name, "User")
    }
    return $value
}

function Write-ScopedConfig {
    param(
        [string]$ShadowPath,
        [int]$CommitLimit,
        [string]$Provider,
        [string]$Model,
        [string]$Reasoning
    )

    $provider = Get-CodeIntelEnvValue "CODE_INTEL_PROVIDER"
    if ([string]::IsNullOrWhiteSpace($provider)) { $provider = "anthropic" }
    $model = Get-CodeIntelEnvValue "CODE_INTEL_MODEL"
    if ([string]::IsNullOrWhiteSpace($model) -and $provider -eq "anthropic") { $model = "MiniMax-M2.7" }

    $configDir = Join-Path $ShadowPath ".repowise"
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null
    $configPath = Join-Path $configDir "config.yaml"
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("provider: $provider")
    if (-not [string]::IsNullOrWhiteSpace($model)) {
        $lines.Add("model: $model")
    }
    $lines.Add("embedder: mock")
    $lines.Add("reasoning: auto")
    $lines.Add("commit_limit: $CommitLimit")
    $lines.Add("editor_files:")
    $lines.Add("  claude_md: false")
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

function Set-CodeIntelAnthropicEnv {
    # Dedicated CODE_INTEL_ANTHROPIC_* vars take priority over any inherited
    # ANTHROPIC_* values, which on a dev machine may point at the Claude Code
    # proxy (e.g. headroom on 127.0.0.1) and must not be repurposed globally.
    $map = @{
        "CODE_INTEL_ANTHROPIC_API_KEY"  = "ANTHROPIC_API_KEY"
        "CODE_INTEL_ANTHROPIC_BASE_URL" = "ANTHROPIC_BASE_URL"
    }
    foreach ($name in $map.Keys) {
        $value = [Environment]::GetEnvironmentVariable($name, "Process")
        if ([string]::IsNullOrWhiteSpace($value)) {
            $value = [Environment]::GetEnvironmentVariable($name, "User")
        }
        if (-not [string]::IsNullOrWhiteSpace($value)) {
            [Environment]::SetEnvironmentVariable($map[$name], $value, "Process")
        }
    }
}

function Set-CodeIntelProviderEnv {
    # Generic provider selection (CODE_INTEL_PROVIDER/MODEL/API_KEY/BASE_URL);
    # consumed by Run-ScopedRepowiseDocs.py. anthropic keeps the dedicated
    # CODE_INTEL_ANTHROPIC_* fallback via Set-CodeIntelAnthropicEnv below for
    # backward compatibility.
    Set-EnvFromUserRegistry "CODE_INTEL_PROVIDER"
    Set-EnvFromUserRegistry "CODE_INTEL_MODEL"
    Set-EnvFromUserRegistry "CODE_INTEL_API_KEY"
    Set-EnvFromUserRegistry "CODE_INTEL_BASE_URL"
    Set-CodeIntelAnthropicEnv
}

function Get-RepowisePython {
    # Run-ScopedRepowiseDocs.py imports the repowise package, which lives in
    # the uv tool venv (not the system python). Fall back to plain python for
    # environments where repowise was pip-installed globally.
    if (-not [string]::IsNullOrWhiteSpace($env:APPDATA)) {
        $venvPython = Join-Path $env:APPDATA "uv\tools\repowise\Scripts\python.exe"
        if (Test-Path -LiteralPath $venvPython -PathType Leaf) {
            return $venvPython
        }
    }
    return "python"
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

function Stop-ProcessTreeSafe {
    param([int]$ProcessId)

    $children = @(
        Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object { $_.ParentProcessId -eq $ProcessId }
    )
    foreach ($child in $children) {
        Stop-ProcessTreeSafe ([int]$child.ProcessId)
    }

    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -ne $process) {
        Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-ProcessWithTimeout {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,
        [string[]]$ArgumentList = @(),
        [string]$Description = "process",
        [int]$TimeoutSeconds = 600,
        [string]$WorkingDirectory = (Get-Location).Path
    )

    $stdout = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-{0}-out.txt" -f ([System.Guid]::NewGuid().ToString("N")))
    $stderr = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-{0}-err.txt" -f ([System.Guid]::NewGuid().ToString("N")))
    try {
        $startProcessParams = @{
            FilePath = $FilePath
            ArgumentList = $ArgumentList
            WorkingDirectory = $WorkingDirectory
            RedirectStandardOutput = $stdout
            RedirectStandardError = $stderr
            PassThru = $true
        }
        if ($effectivePlatform -eq "windows") {
            $startProcessParams.WindowStyle = "Hidden"
        }
        $process = Start-Process @startProcessParams

        $finished = $process.WaitForExit([math]::Max(1, $TimeoutSeconds) * 1000)
        if (-not $finished) {
            Stop-ProcessTreeSafe ([int]$process.Id)
            throw "$Description timed out after ${TimeoutSeconds}s"
        }

        $outText = if (Test-Path -LiteralPath $stdout) { Get-Content -LiteralPath $stdout -Raw -ErrorAction SilentlyContinue } else { "" }
        $errText = if (Test-Path -LiteralPath $stderr) { Get-Content -LiteralPath $stderr -Raw -ErrorAction SilentlyContinue } else { "" }
        $text = (($outText, $errText) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }) -join "`n"
        $exitCode = if ($null -eq $process.ExitCode) { 0 } else { [int]$process.ExitCode }
        if ($exitCode -ne 0) {
            throw "$Description failed with exit code $exitCode. $text"
        }
        return $text.Trim()
    }
    finally {
        Remove-Item -LiteralPath $stdout,$stderr -Force -ErrorAction SilentlyContinue
    }
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
    [void](Invoke-NativeCommand -Description "git reset shadow" -Script { git -C $shadowPath reset --hard HEAD })
    [void](Invoke-NativeCommand -Description "git clean shadow" -Script { git -C $shadowPath clean -fdx -e .repowise })
    git sparse-checkout init --no-cone 2>&1 | Out-Null
    $patterns = New-Object System.Collections.Generic.List[string]
    foreach ($dir in $scopeDirs) {
        $patterns.Add("/$($dir.Trim('/'))/")
    }
    foreach ($file in $scopeFiles) {
        $patterns.Add("/$($file.Trim('/'))")
    }
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $patterns | git sparse-checkout set --stdin 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "git sparse-checkout set failed with exit $LASTEXITCODE"
        }
        git read-tree -mu HEAD 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "git read-tree failed with exit $LASTEXITCODE"
        }
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
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
$Provider = if ($Provider -ieq "ccw") { "codex_cli" } else { $Provider }
$providerArgs = @("--provider", $Provider)
if (-not [string]::IsNullOrWhiteSpace($Model)) { $providerArgs += @("--model", $Model) }
if (-not [string]::IsNullOrWhiteSpace($Reasoning)) { $providerArgs += @("--reasoning", $Reasoning) }
# Docs must never silently run the "mock" default (index-only legs use mock on
# purpose; mock docs would emit junk pages). When -Provider was not explicitly
# set past the default, omit --provider so Run-ScopedRepowiseDocs.py resolves
# the real provider from CODE_INTEL_PROVIDER (default anthropic).
$docsProviderArgs = @()
if (-not [string]::IsNullOrWhiteSpace($Provider) -and $Provider -ine "mock") { $docsProviderArgs += @("--provider", $Provider) }
if (-not [string]::IsNullOrWhiteSpace($Model)) { $docsProviderArgs += @("--model", $Model) }
if (-not [string]::IsNullOrWhiteSpace($Reasoning)) { $docsProviderArgs += @("--reasoning", $Reasoning) }
Write-ScopedConfig $shadowPath $CommitLimit $Provider $Model $Reasoning

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
Set-CodeIntelProviderEnv

$statePath = Join-Path (Join-Path $shadowPath ".repowise") "state.json"
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
            [void](Invoke-ProcessWithTimeout `
                -FilePath "repowise" `
                -Description "repowise init" `
                -TimeoutSeconds $TimeoutSeconds `
                -ArgumentList (@("init", ".", "--index-only", "-y", "--no-claude-md", "--no-onboarding", "--skip-tests", "--skip-infra", "--commit-limit", [string]$CommitLimit, "--embedder", "mock") + $providerArgs))
        }
        $dbPath = Join-Path (Join-Path $shadowPath ".repowise") "wiki.db"
        if (Test-Path -LiteralPath $dbPath -PathType Leaf) {
            Remove-Item -LiteralPath $dbPath -Force
        }
        $scriptPath = Join-Path $PSScriptRoot "Run-ScopedRepowiseDocs.py"
        [void](Invoke-ProcessWithTimeout `
            -FilePath (Get-RepowisePython) `
            -Description "repowise scoped docs" `
            -TimeoutSeconds $TimeoutSeconds `
            -ArgumentList (@($scriptPath, "--repo", $shadowPath, "--coverage-pct", "0.02", "--concurrency", "1") + $docsProviderArgs))
    }
    else {
        if (Test-Path -LiteralPath $statePath -PathType Leaf) {
            [void](Invoke-ProcessWithTimeout `
                -FilePath "repowise" `
                -Description "repowise update" `
                -TimeoutSeconds $TimeoutSeconds `
                -ArgumentList (@("update", "--no-workspace", "--index-only") + $providerArgs))
        }
        else {
            [void](Invoke-ProcessWithTimeout `
                -FilePath "repowise" `
                -Description "repowise init" `
                -TimeoutSeconds $TimeoutSeconds `
                -ArgumentList (@("init", ".", "--index-only", "-y", "--no-claude-md", "--no-onboarding", "--skip-tests", "--skip-infra", "--commit-limit", [string]$CommitLimit, "--embedder", "mock") + $providerArgs))
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
    $status = Invoke-ProcessWithTimeout `
        -FilePath "repowise" `
        -Description "repowise status" `
        -TimeoutSeconds $TimeoutSeconds `
        -ArgumentList @("status", "--no-workspace")
    if (Test-Path -LiteralPath $statePath -PathType Leaf) {
        $state = Get-Content -LiteralPath $statePath -Raw
    }
}
finally {
    Pop-Location
}

$dbPath = Join-Path (Join-Path $shadowPath ".repowise") "wiki.db"
if ((-not (Test-Path -LiteralPath $statePath -PathType Leaf)) -and (-not (Test-Path -LiteralPath $dbPath -PathType Leaf))) {
    throw "Scoped repowise finished without .repowise/state.json or .repowise/wiki.db: $shadowPath"
}

$global:LASTEXITCODE = 0
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
