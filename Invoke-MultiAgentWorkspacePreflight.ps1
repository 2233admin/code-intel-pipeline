#requires -Version 7.2

param(
    [string]$RepoPath = ".",

    [ValidateSet("mutation", "observation")]
    [string]$Intent = "mutation",

    [string]$Policy = (Join-Path $PSScriptRoot "orchestration\multi-agent-workspace-policy.v1.json"),

    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-ReadOnlyGit {
    param(
        [string]$WorkingDirectory,
        [string[]]$Arguments
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = "git"
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [System.Text.UTF8Encoding]::new($false)
    $startInfo.StandardErrorEncoding = [System.Text.UTF8Encoding]::new($false)
    $startInfo.Environment["GIT_OPTIONAL_LOCKS"] = "0"
    [void]$startInfo.ArgumentList.Add("--no-optional-locks")
    [void]$startInfo.ArgumentList.Add("-C")
    [void]$startInfo.ArgumentList.Add($WorkingDirectory)
    foreach ($argument in $Arguments) {
        [void]$startInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        [void]$process.Start()
        $stdout = $process.StandardOutput.ReadToEnd()
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        return [pscustomobject]@{
            exitCode = $process.ExitCode
            stdout = $stdout
            stderr = $stderr.Trim()
        }
    } finally {
        $process.Dispose()
    }
}

function Get-NormalizedPath {
    param([string]$Path)
    return $Path.Replace("\", "/")
}

function Get-ChangeGroup {
    param([string]$Status)

    if ($Status -eq "??") { return "untracked" }
    if ($Status -match "U" -or $Status -in @("AA", "DD")) { return "unmerged" }
    if ($Status -match "R") { return "renamed" }
    if ($Status -match "C") { return "copied" }
    if ($Status -match "D") { return "deleted" }
    if ($Status -match "A") { return "added" }
    if ($Status -match "T") { return "typeChanged" }
    if ($Status -match "M") { return "modified" }
    return "other"
}

function Get-Sha256 {
    param([string]$Text)
    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($Text)
    $digest = [System.Security.Cryptography.SHA256]::HashData($bytes)
    return [Convert]::ToHexString($digest).ToLowerInvariant()
}

function Write-ResultAndExit {
    param(
        [System.Collections.IDictionary]$Result,
        [int]$ExitCode,
        [switch]$AsJson
    )

    if ($AsJson) {
        $Result | ConvertTo-Json -Depth 20
    } else {
        $state = if ([bool]$Result.allowed) { "allowed" } else { "denied" }
        Write-Host "Multi-Agent Workspace Preflight: $state ($($Result.decision))"
        Write-Host "Intent: $($Result.intent)"
        Write-Host "Repository: $($Result.repo)"
        Write-Host "Changes: tracked=$($Result.counts.tracked) untracked=$($Result.counts.untracked) total=$($Result.counts.total)"
        Write-Host "Inventory SHA-256: $($Result.inventoryHash)"
        Write-Host "Reason: $($Result.reason)"
    }
    exit $ExitCode
}

try {
    $policyDocument = Get-Content -Raw -LiteralPath $Policy | ConvertFrom-Json -Depth 20
} catch {
    [Console]::Error.WriteLine("Invalid multi-agent workspace policy: $($_.Exception.Message)")
    exit 2
}

$policyValid = $false
try {
    $policyValid = [string]$policyDocument.schema -eq "code-intel-multi-agent-workspace-policy.v1" -and
        [string]$policyDocument.defaultIntent -eq "mutation" -and
        [string]$policyDocument.inspectionAuthority -eq "observation_only" -and
        [bool]$policyDocument.requirements.repositoryRootRequired -and
        [bool]$policyDocument.requirements.dirtyRootBlocksMutation -and
        [bool]$policyDocument.requirements.observationMustBeExplicit -and
        -not [bool]$policyDocument.requirements.observationMayModifyRepository -and
        [string]$policyDocument.requirements.inventoryFormat -eq "git-status-porcelain-v1-z" -and
        [string]$policyDocument.requirements.inventoryHash -eq "sha256-canonical-json-v1"
} catch {
    $policyValid = $false
}
if (-not $policyValid) {
    [Console]::Error.WriteLine("Multi-agent workspace policy shape or fail-closed boundary is invalid.")
    exit 2
}

$repo = [System.IO.Path]::GetFullPath($RepoPath)
$emptyCounts = [ordered]@{
    tracked = 0
    untracked = 0
    total = 0
    staged = 0
    worktree = 0
}
$emptyGroups = [ordered]@{
    added = 0
    modified = 0
    deleted = 0
    renamed = 0
    copied = 0
    typeChanged = 0
    unmerged = 0
    untracked = 0
    other = 0
}

if (-not (Test-Path -LiteralPath $repo -PathType Container)) {
    $result = [ordered]@{
        schema = "code-intel-multi-agent-workspace-preflight.v1"
        authority = "observation_only"
        repo = $repo
        gitRoot = $null
        isRepositoryRoot = $false
        intent = $Intent
        dirty = $null
        allowed = $false
        decision = "deny_inspection_failed"
        reason = "repository path does not exist or is not a directory"
        counts = $emptyCounts
        groups = $emptyGroups
        inventoryHashAlgorithm = "sha256-canonical-json-v1"
        inventoryHash = $null
        entries = @()
    }
    Write-ResultAndExit -Result $result -ExitCode 22 -AsJson:$Json
}

$rootResult = Invoke-ReadOnlyGit -WorkingDirectory $repo -Arguments @("rev-parse", "--show-toplevel")
if ($rootResult.exitCode -ne 0 -or [string]::IsNullOrWhiteSpace($rootResult.stdout)) {
    $result = [ordered]@{
        schema = "code-intel-multi-agent-workspace-preflight.v1"
        authority = "observation_only"
        repo = $repo
        gitRoot = $null
        isRepositoryRoot = $false
        intent = $Intent
        dirty = $null
        allowed = $false
        decision = "deny_inspection_failed"
        reason = $(if ([string]::IsNullOrWhiteSpace($rootResult.stderr)) { "path is not a Git worktree" } else { $rootResult.stderr })
        counts = $emptyCounts
        groups = $emptyGroups
        inventoryHashAlgorithm = "sha256-canonical-json-v1"
        inventoryHash = $null
        entries = @()
    }
    Write-ResultAndExit -Result $result -ExitCode 22 -AsJson:$Json
}

$gitRoot = [System.IO.Path]::GetFullPath($rootResult.stdout.Trim())
$comparison = if ($IsWindows) { [System.StringComparison]::OrdinalIgnoreCase } else { [System.StringComparison]::Ordinal }
$trimChars = @([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
$isRepositoryRoot = $repo.TrimEnd($trimChars).Equals($gitRoot.TrimEnd($trimChars), $comparison)

$statusResult = Invoke-ReadOnlyGit -WorkingDirectory $gitRoot -Arguments @("status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignored=no")
if ($statusResult.exitCode -ne 0) {
    $result = [ordered]@{
        schema = "code-intel-multi-agent-workspace-preflight.v1"
        authority = "observation_only"
        repo = $repo
        gitRoot = $gitRoot
        isRepositoryRoot = $isRepositoryRoot
        intent = $Intent
        dirty = $null
        allowed = $false
        decision = "deny_inspection_failed"
        reason = $(if ([string]::IsNullOrWhiteSpace($statusResult.stderr)) { "git status inspection failed" } else { $statusResult.stderr })
        counts = $emptyCounts
        groups = $emptyGroups
        inventoryHashAlgorithm = "sha256-canonical-json-v1"
        inventoryHash = $null
        entries = @()
    }
    Write-ResultAndExit -Result $result -ExitCode 22 -AsJson:$Json
}

$entries = [System.Collections.Generic.List[object]]::new()
$tokens = @($statusResult.stdout.Split([char]0, [System.StringSplitOptions]::RemoveEmptyEntries))
for ($index = 0; $index -lt $tokens.Count; $index++) {
    $token = [string]$tokens[$index]
    if ($token.Length -lt 4 -or $token[2] -ne " ") {
        $result = [ordered]@{
            schema = "code-intel-multi-agent-workspace-preflight.v1"
            authority = "observation_only"
            repo = $repo
            gitRoot = $gitRoot
            isRepositoryRoot = $isRepositoryRoot
            intent = $Intent
            dirty = $null
            allowed = $false
            decision = "deny_inspection_failed"
            reason = "unrecognized git status porcelain entry"
            counts = $emptyCounts
            groups = $emptyGroups
            inventoryHashAlgorithm = "sha256-canonical-json-v1"
            inventoryHash = $null
            entries = @()
        }
        Write-ResultAndExit -Result $result -ExitCode 22 -AsJson:$Json
    }

    $status = $token.Substring(0, 2)
    $path = Get-NormalizedPath $token.Substring(3)
    $originalPath = $null
    if ($status -match "[RC]") {
        $index++
        if ($index -ge $tokens.Count) {
            $result = [ordered]@{
                schema = "code-intel-multi-agent-workspace-preflight.v1"
                authority = "observation_only"
                repo = $repo
                gitRoot = $gitRoot
                isRepositoryRoot = $isRepositoryRoot
                intent = $Intent
                dirty = $null
                allowed = $false
                decision = "deny_inspection_failed"
                reason = "rename or copy entry is missing its original path"
                counts = $emptyCounts
                groups = $emptyGroups
                inventoryHashAlgorithm = "sha256-canonical-json-v1"
                inventoryHash = $null
                entries = @()
            }
            Write-ResultAndExit -Result $result -ExitCode 22 -AsJson:$Json
        }
        $originalPath = Get-NormalizedPath ([string]$tokens[$index])
    }

    $entries.Add([pscustomobject][ordered]@{
        status = $status
        group = Get-ChangeGroup $status
        path = $path
        originalPath = $originalPath
    })
}

$orderedEntries = @($entries | Sort-Object path, originalPath, status)
$trackedCount = @($orderedEntries | Where-Object status -ne "??").Count
$untrackedCount = @($orderedEntries | Where-Object status -eq "??").Count
$stagedCount = @($orderedEntries | Where-Object { $_.status -ne "??" -and $_.status[0] -ne " " }).Count
$worktreeCount = @($orderedEntries | Where-Object { $_.status -ne "??" -and $_.status[1] -ne " " }).Count
$counts = [ordered]@{
    tracked = $trackedCount
    untracked = $untrackedCount
    total = $orderedEntries.Count
    staged = $stagedCount
    worktree = $worktreeCount
}
$groups = [ordered]@{}
foreach ($name in @("added", "modified", "deleted", "renamed", "copied", "typeChanged", "unmerged", "untracked", "other")) {
    $groups[$name] = @($orderedEntries | Where-Object group -eq $name).Count
}
$canonicalInventory = [ordered]@{
    schema = "code-intel-multi-agent-workspace-inventory.v1"
    entries = $orderedEntries
}
$canonicalJson = $canonicalInventory | ConvertTo-Json -Depth 10 -Compress
$inventoryHash = Get-Sha256 $canonicalJson
$dirty = $orderedEntries.Count -gt 0

if (-not $isRepositoryRoot) {
    $allowed = $false
    $decision = "deny_repository_root_required"
    $reason = "preflight must be run against the Git worktree root"
    $exitCode = 21
} elseif ($dirty -and $Intent -eq "mutation") {
    $allowed = $false
    $decision = "deny_dirty_root"
    $reason = "dirty repository root cannot authorize mutation-oriented agent work"
    $exitCode = 20
} elseif ($Intent -eq "observation") {
    $allowed = $true
    $decision = "allow_observation"
    $reason = "explicit observation-only intent does not grant repository mutation authority"
    $exitCode = 0
} else {
    $allowed = $true
    $decision = "allow_clean_mutation"
    $reason = "repository root is clean"
    $exitCode = 0
}

$result = [ordered]@{
    schema = "code-intel-multi-agent-workspace-preflight.v1"
    authority = "observation_only"
    repo = $repo
    gitRoot = $gitRoot
    isRepositoryRoot = $isRepositoryRoot
    intent = $Intent
    dirty = $dirty
    allowed = $allowed
    decision = $decision
    reason = $reason
    counts = $counts
    groups = $groups
    inventoryHashAlgorithm = "sha256-canonical-json-v1"
    inventoryHash = $inventoryHash
    entries = $orderedEntries
}
Write-ResultAndExit -Result $result -ExitCode $exitCode -AsJson:$Json
