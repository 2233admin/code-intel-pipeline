#requires -Version 7.2

param(
    [string]$Repo = "",
    [string]$RepoPath = "",
    [string[]]$Repos = @(),
    [switch]$All,

    [string]$Config = "",

    [ValidateSet("auto", "windows", "macos", "linux")]
    [string]$Platform = "auto",

    [ValidateSet("lite", "normal", "full")]
    [string]$Mode = "normal",

    [switch]$RepowiseDocs,
    [string]$RepowiseProvider = "",
    [string]$RepowiseModel = "",
    [string]$RepowiseReasoning = "",
    [switch]$SaveSentruxBaseline,
[switch]$AutoSaveMissingSentruxBaseline,
[switch]$RequireUnderstandGraph,
[switch]$SkipGitHubResearch,
[switch]$SkipRepowise,
[switch]$NoIndexUpdate,
[switch]$ValidateInstallation,
[switch]$LegacyCompatibility,
[ValidateSet("auto", "enabled", "disabled")]
[string]$ProactiveSkillSuggestions = "auto",
[ValidateSet("auto", "ask", "enabled", "disabled")]
[string]$AutomaticPullRequests = "auto",
[string]$BugSkill = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
if ([string]::IsNullOrWhiteSpace($Config)) {
    $Config = Join-Path $root "pipeline.config.json"
}
$doctor = Join-Path $root "check-code-intel-tools.ps1"
$runner = Join-Path $root "run-code-intel.ps1"
$indexer = Join-Path $root "update-code-intel-index.ps1"
$platformModule = Join-Path (Join-Path $root "tools") "code-intel-platform.psm1"
Import-Module $platformModule -Force
$binaryName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
$rustCliCandidates = @(
    (Join-Path $root "bin/$binaryName"),
    (Join-Path $root "target/release/$binaryName"),
    (Join-Path $root "target/debug/$binaryName")
)
$rustCli = @(
    $rustCliCandidates |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
        ForEach-Object { Get-Item -LiteralPath $_ } |
        Sort-Object LastWriteTimeUtc -Descending |
        Select-Object -First 1 -ExpandProperty FullName
)
if ($rustCli.Count -gt 0) { $rustCli = $rustCli[0] } else { $rustCli = $null }

if ($SkipRepowise -and $RepowiseDocs) {
    throw "-SkipRepowise cannot be combined with -RepowiseDocs."
}

function Get-JsonProperty {
    param(
        [object]$Object,
        [string]$Name
    )

    if ($null -eq $Object) { return $null }
    $prop = $Object.PSObject.Properties[$Name]
    if ($null -eq $prop) { return $null }
    return $prop.Value
}

function Get-RepoSelector {
    param([string]$RepoName, [string]$DirectRepoPath)

    if (-not [string]::IsNullOrWhiteSpace($DirectRepoPath)) {
        return @{ RepoPath = $DirectRepoPath }
    }
    return @{ Repo = $RepoName }
}

function Get-RunnerParameters {
    param([string]$RepoName, [string]$DirectRepoPath)

    $parameters = @{
        Config = $Config
        Mode = $Mode
        Platform = $Platform
        RepowiseProvider = $RepowiseProvider
        RepowiseModel = $RepowiseModel
        RepowiseReasoning = $RepowiseReasoning
        ProactiveSkillSuggestions = $ProactiveSkillSuggestions
        AutomaticPullRequests = $AutomaticPullRequests
        BugSkill = $BugSkill
    }
    foreach ($entry in (Get-RepoSelector -RepoName $RepoName -DirectRepoPath $DirectRepoPath).GetEnumerator()) {
        $parameters[$entry.Key] = $entry.Value
    }
    foreach ($switchEntry in @(
        @{ Name = "RepowiseDocs"; Enabled = $RepowiseDocs },
        @{ Name = "SaveSentruxBaseline"; Enabled = $SaveSentruxBaseline },
        @{ Name = "AutoSaveMissingSentruxBaseline"; Enabled = $AutoSaveMissingSentruxBaseline },
        @{ Name = "RequireUnderstandGraph"; Enabled = $RequireUnderstandGraph },
        @{ Name = "SkipGitHubResearch"; Enabled = $SkipGitHubResearch },
        @{ Name = "SkipRepowise"; Enabled = $SkipRepowise }
    )) {
        if ($switchEntry.Enabled) { $parameters[$switchEntry.Name] = $true }
    }
    return $parameters
}

function Get-DoctorParameters {
    param([string]$RepoName, [string]$DirectRepoPath)

    $parameters = @{
        Config = $Config
        Platform = $Platform
        RequireRepowise = [bool]$RepowiseDocs
        RequireUnderstand = [bool]$RequireUnderstandGraph
    }
    foreach ($entry in (Get-RepoSelector -RepoName $RepoName -DirectRepoPath $DirectRepoPath).GetEnumerator()) {
        $parameters[$entry.Key] = $entry.Value
    }
    return $parameters
}

function Resolve-InvocationRepoPath {
    param([string]$RepoName, [string]$DirectRepoPath)

    if (-not [string]::IsNullOrWhiteSpace($DirectRepoPath)) {
        return (Get-Item -LiteralPath $DirectRepoPath -ErrorAction Stop).FullName
    }
    $configData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
    $reposConfig = Get-JsonProperty $configData "repos"
    $repoConfig = Get-JsonProperty $reposConfig $RepoName
    $configuredPath = Get-JsonProperty $repoConfig "path"
    if ([string]::IsNullOrWhiteSpace([string]$configuredPath)) {
        throw "Repository alias has no configured path: $RepoName"
    }
    return (Get-Item -LiteralPath ([string]$configuredPath) -ErrorAction Stop).FullName
}

function Get-InvocationArtifactRoot {
    $configData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
    $configured = Get-JsonProperty $configData "artifactRoot"
    if (-not [string]::IsNullOrWhiteSpace([string]$configured)) {
        return [System.IO.Path]::GetFullPath([string]$configured)
    }
    return (Get-CodeIntelArtifactRoot -Platform $Platform)
}

function Publish-AuthoritativeCoreRun {
    param([string]$ResolvedRepoPath)

    $artifactRoot = Get-InvocationArtifactRoot
    $repoName = Split-Path -Leaf $ResolvedRepoPath
    $repoAuthority = Join-Path $artifactRoot $repoName
    New-Item -ItemType Directory -Force -Path $repoAuthority | Out-Null
    $temporaryRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
    $sourceRoot = Join-Path $temporaryRoot ("code-intel-a09-{0}-{1}" -f $PID, [guid]::NewGuid().ToString("N"))
    $finalName = (Get-Date -Format "yyyyMMdd-HHmmss-fff") + "-core"
    $committed = $false
    try {
        Write-Host "Code intel invoke: authoritative DAG $ResolvedRepoPath"
        $dagArguments = @(
            "run", "execute",
            "--repo", $ResolvedRepoPath,
            "--out", $sourceRoot,
            "--authority-root", $repoAuthority,
            "--final-name", $finalName,
            "--profile", "default",
            "--doctor-require-repowise", ([bool]$RepowiseDocs).ToString().ToLowerInvariant(),
            "--doctor-require-understand", ([bool]$RequireUnderstandGraph).ToString().ToLowerInvariant()
        )
        $dagOutput = @(& $rustCli @dagArguments 2>&1)
        $dagExitCode = $LASTEXITCODE
        $executionResult = try {
            ($dagOutput -join [Environment]::NewLine) | ConvertFrom-Json -ErrorAction Stop
        }
        catch {
            $dagOutput | Out-Host
            Write-Error "Authoritative execution kernel did not return a valid result: $($_.Exception.Message)"
            return $(if ($dagExitCode -ne 0) { $dagExitCode } else { 3 })
        }
        if ([string](Get-JsonProperty $executionResult "schema") -ne "code-intel-execution-result.v1") {
            Write-Error "Authoritative execution kernel returned an unsupported result schema."
            return 3
        }
        $executionSchema = Join-Path $root "orchestration/schemas/code-intel-execution-result.v1.schema.json"
        if (-not (($executionResult | ConvertTo-Json -Depth 100 -Compress) |
                Test-Json -SchemaFile $executionSchema -ErrorAction Stop)) {
            Write-Error "Authoritative execution kernel result violates its checked-in schema."
            return 3
        }
        $dagOutcome = [string](Get-JsonProperty $executionResult "outcome")
        if ([string]::IsNullOrWhiteSpace($dagOutcome)) {
            Write-Error "Authoritative execution result has no outcome."
            return 3
        }
        $manifest = Get-JsonProperty $executionResult "manifest"
        if ([string](Get-JsonProperty $manifest "outcome") -ne $dagOutcome) {
            Write-Error "Authoritative execution result outcome does not match its manifest."
            return 3
        }
        $reportedExitCode = 0
        if (-not [int]::TryParse(
                [string](Get-JsonProperty $executionResult "exitCode"),
                [ref]$reportedExitCode
            ) -or $reportedExitCode -ne $dagExitCode) {
            Write-Error "Authoritative execution result exitCode does not match the process exit code."
            return 3
        }
        $publication = Get-JsonProperty $executionResult "publication"
        $publishedPath = [string](Get-JsonProperty $publication "path")
        $expectedPublishedPath = [System.IO.Path]::GetFullPath((Join-Path $repoAuthority $finalName))
        if ([string](Get-JsonProperty $publication "status") -ne "committed" -or
            [string](Get-JsonProperty $publication "name") -ne $finalName -or
            [string]::IsNullOrWhiteSpace($publishedPath) -or
            [System.IO.Path]::GetFullPath($publishedPath) -ne $expectedPublishedPath -or
            -not (Test-Path -LiteralPath (Join-Path $publishedPath "run-complete.json") -PathType Leaf)) {
            Write-Error "Authoritative execution kernel did not publish a committed run."
            return 3
        }
        $committed = $true
        Write-Host "Code intel invoke: authoritative run committed $repoName/$finalName outcome=$dagOutcome"
        if ($dagExitCode -ne 0) {
            Write-Warning "Authoritative DAG outcome is $dagOutcome; the committed run is retained as failure evidence."
            return $dagExitCode
        }
        if ($dagOutcome -ne "completed") {
            Write-Warning "Authoritative execution kernel returned success for non-completed outcome $dagOutcome."
            return 3
        }
        return 0
    }
    finally {
        $resolvedSource = [System.IO.Path]::GetFullPath($sourceRoot)
        if ($committed -and $resolvedSource.StartsWith($temporaryRoot, [System.StringComparison]::OrdinalIgnoreCase) -and
            (Test-Path -LiteralPath $resolvedSource -PathType Container)) {
            Remove-Item -LiteralPath $resolvedSource -Recurse -Force
        }
        elseif (-not $committed -and (Test-Path -LiteralPath $resolvedSource -PathType Container)) {
            Write-Warning "Authoritative DAG staging retained for recovery: $resolvedSource"
        }
    }
}

function Invoke-OneRepo {
    param(
        [string]$RepoName,
        [string]$DirectRepoPath = ""
    )

    $label = if (-not [string]::IsNullOrWhiteSpace($DirectRepoPath)) { $DirectRepoPath } else { $RepoName }
    $resolvedRepoPath = Resolve-InvocationRepoPath -RepoName $RepoName -DirectRepoPath $DirectRepoPath
    $legacyCode = 0
    if ($LegacyCompatibility) {
        Write-Warning "Legacy compatibility pipeline is enabled for $label; its artifacts are non-authoritative."
        Write-Host "Code intel invoke: legacy doctor $label"
        $global:LASTEXITCODE = 0
        $doctorParams = Get-DoctorParameters -RepoName $RepoName -DirectRepoPath $DirectRepoPath
        & $doctor @doctorParams
        if ($LASTEXITCODE -ne 0) {
            return [pscustomobject][ordered]@{
                repo = $label
                ok = $false
                stage = "legacy_doctor"
                exitCode = $LASTEXITCODE
            }
        }

        Write-Host "Code intel invoke: legacy compatibility pipeline $label"
        $invokeParams = Get-RunnerParameters -RepoName $RepoName -DirectRepoPath $DirectRepoPath
        & $runner @invokeParams
        $legacyCode = $LASTEXITCODE
    }

    $publicationCode = Publish-AuthoritativeCoreRun -ResolvedRepoPath $resolvedRepoPath
    $code = if ($legacyCode -ne 0) { $legacyCode } else { $publicationCode }
    return [pscustomobject][ordered]@{
        repo = $label
        ok = $code -eq 0
        stage = if ($publicationCode -ne 0) { "authoritative_publication" } elseif ($legacyCode -ne 0) { "legacy_compatibility" } else { "authoritative_pipeline" }
        exitCode = $code
        legacyCompatibility = [bool]$LegacyCompatibility
        legacyExitCode = $legacyCode
        publicationExitCode = $publicationCode
    }
}

if ($LegacyCompatibility) {
    if (-not (Test-Path -LiteralPath $doctor -PathType Leaf)) {
        throw "Legacy doctor script missing: $doctor"
    }
    if (-not (Test-Path -LiteralPath $runner -PathType Leaf)) {
        throw "Legacy pipeline script missing: $runner"
    }
}
if ($null -eq $rustCli) {
    Push-Location $root
    try {
        & cargo build -p code-intel | Out-Host
    }
    finally {
        Pop-Location
    }
    $rustCli = Join-Path $root "target/debug/$binaryName"
}
if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
    throw "Rust integration orchestrator missing. Checked: $($rustCliCandidates -join ', ')"
}

Write-Host "Code intel invoke: validate integration orchestration"
Push-Location $root
try {
    & $rustCli orchestrate --action Validate | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "Integration orchestration validation failed"
    }
}
finally {
    Pop-Location
}
if ($ValidateInstallation) {
    if ($LegacyCompatibility) {
        $doctorCommand = Get-Command -Name $doctor -ErrorAction Stop
        $runnerCommand = Get-Command -Name $runner -ErrorAction Stop
        if (-not $doctorCommand.Parameters.ContainsKey('RequireRepowise')) {
            throw "Legacy doctor does not expose the optional Repowise contract."
        }
        if (-not $runnerCommand.Parameters.ContainsKey('SkipRepowise')) {
            throw "Legacy pipeline runner does not expose the optional Repowise contract."
        }
    }
    Write-Host "Code intel invoke: installation validation passed; default route is the manifest-bound Rust DAG ($rustCli)"
    exit 0
}

$targetRepos = @()
if ($All) {
    $configData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
    $reposConfig = Get-JsonProperty $configData "repos"
    if ($null -eq $reposConfig) {
        throw "No repos configured in: $Config"
    }
    $targetRepos = @($reposConfig.PSObject.Properties.Name)
}
elseif ($Repos.Count -gt 0) {
    $targetRepos = @($Repos)
}
elseif (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
    $targetRepos = @([pscustomobject]@{ repo = ""; path = $RepoPath })
}
elseif (-not [string]::IsNullOrWhiteSpace($Repo)) {
    $targetRepos = @($Repo)
}
else {
    throw "Specify -Repo <alias>, -RepoPath <path>, -Repos <alias[]> or -All."
}

$results = New-Object System.Collections.Generic.List[object]
foreach ($target in $targetRepos) {
    if ($target -is [pscustomobject]) {
        $results.Add((Invoke-OneRepo $target.repo $target.path))
    }
    else {
        $results.Add((Invoke-OneRepo $target))
    }
}

if (-not $NoIndexUpdate -and (Test-Path -LiteralPath $indexer -PathType Leaf)) {
    Write-Host "Code intel invoke: update artifact index"
    $indexParams = @{}
    if (Test-Path -LiteralPath $Config -PathType Leaf) {
        $indexConfigData = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
        $configuredArtifactRoot = Get-JsonProperty $indexConfigData "artifactRoot"
        if (-not [string]::IsNullOrWhiteSpace([string]$configuredArtifactRoot)) {
            $indexParams.ArtifactRoot = [string]$configuredArtifactRoot
        }
    }
    $indexParams.Platform = $Platform
    & $indexer @indexParams | Out-Host
}

Write-Host "Code intel invoke: batch summary"
foreach ($result in $results) {
    $mark = if ($result.ok) { "OK" } else { "FAILED" }
    Write-Host "$mark $($result.repo) stage=$($result.stage) exit=$($result.exitCode)"
}

$failed = @($results | Where-Object { -not $_.ok })
if ($failed.Count -gt 0) {
    exit 1
}
exit 0
