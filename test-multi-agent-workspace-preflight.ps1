param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSCommandPath
$adapter = Join-Path $root "Invoke-MultiAgentWorkspacePreflight.ps1"
$policy = Join-Path $root "orchestration\multi-agent-workspace-policy.v1.json"
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-workspace-preflight-" + [guid]::NewGuid().ToString("N"))

function Invoke-Preflight {
    param(
        [string]$Repo,
        [ValidateSet("mutation", "observation")]
        [string]$Intent = "mutation",
        [string]$PolicyPath = $policy
    )

    $output = @(& pwsh -NoProfile -File $adapter -RepoPath $Repo -Intent $Intent -Policy $PolicyPath -Json 2>&1 | ForEach-Object { $_.ToString() })
    return [pscustomobject]@{
        exitCode = $LASTEXITCODE
        output = $output -join "`n"
    }
}

function New-FixtureRepo {
    param([string]$Path)

    New-Item -ItemType Directory -Force -Path $Path | Out-Null
    & git -C $Path init --quiet
    if ($LASTEXITCODE -ne 0) { throw "fixture git init failed: $Path" }
    & git -C $Path config user.email "workspace-preflight@example.invalid"
    & git -C $Path config user.name "Workspace Preflight Test"
    Set-Content -LiteralPath (Join-Path $Path "tracked file.txt") -Value "baseline" -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $Path "rename source.txt") -Value "rename baseline" -Encoding UTF8
    & git -C $Path add -- "tracked file.txt" "rename source.txt"
    & git -C $Path commit --quiet -m "fixture baseline"
    if ($LASTEXITCODE -ne 0) { throw "fixture baseline commit failed: $Path" }
}

try {
    $cleanRepo = Join-Path $tempRoot "clean repo"
    New-FixtureRepo $cleanRepo

    $clean = Invoke-Preflight -Repo $cleanRepo
    if ($clean.exitCode -ne 0) { throw "clean mutation preflight failed: $($clean.output)" }
    $cleanResult = $clean.output | ConvertFrom-Json
    if (-not [bool]$cleanResult.allowed -or [bool]$cleanResult.dirty -or [string]$cleanResult.decision -ne "allow_clean_mutation") {
        throw "clean repository must admit mutation intent"
    }
    if ([int]$cleanResult.counts.total -ne 0 -or [string]::IsNullOrWhiteSpace([string]$cleanResult.inventoryHash)) {
        throw "clean inventory must have zero entries and a deterministic hash"
    }

    Set-Content -LiteralPath (Join-Path $cleanRepo "tracked file.txt") -Value "changed" -Encoding UTF8
    & git -C $cleanRepo mv -- "rename source.txt" "renamed file.txt"
    if ($LASTEXITCODE -ne 0) { throw "fixture rename failed" }
    New-Item -ItemType Directory -Force -Path (Join-Path $cleanRepo "untracked folder") | Out-Null
    Set-Content -LiteralPath (Join-Path $cleanRepo "untracked folder\new file.txt") -Value "new" -Encoding UTF8

    $dirty = Invoke-Preflight -Repo $cleanRepo
    if ($dirty.exitCode -ne 20) { throw "dirty mutation intent must exit 20: $($dirty.output)" }
    $dirtyResult = $dirty.output | ConvertFrom-Json
    if ([bool]$dirtyResult.allowed -or -not [bool]$dirtyResult.dirty -or [string]$dirtyResult.decision -ne "deny_dirty_root") {
        throw "dirty root must fail closed for mutation intent"
    }
    if ([int]$dirtyResult.counts.tracked -ne 2 -or [int]$dirtyResult.counts.untracked -ne 1 -or [int]$dirtyResult.counts.total -ne 3) {
        throw "tracked and untracked counts are incorrect: $($dirty.output)"
    }
    if ([int]$dirtyResult.groups.modified -ne 1 -or [int]$dirtyResult.groups.renamed -ne 1 -or [int]$dirtyResult.groups.untracked -ne 1) {
        throw "change grouping is incorrect: $($dirty.output)"
    }
    $renameEntry = @($dirtyResult.entries | Where-Object group -eq "renamed")
    if ($renameEntry.Count -ne 1 -or [string]$renameEntry[0].path -ne "renamed file.txt" -or [string]$renameEntry[0].originalPath -ne "rename source.txt") {
        throw "NUL-delimited rename paths are incorrect: $($dirty.output)"
    }

    $observed = Invoke-Preflight -Repo $cleanRepo -Intent observation
    if ($observed.exitCode -ne 0) { throw "explicit observation should be admitted: $($observed.output)" }
    $observedResult = $observed.output | ConvertFrom-Json
    if (-not [bool]$observedResult.allowed -or [string]$observedResult.authority -ne "observation_only" -or [string]$observedResult.decision -ne "allow_observation") {
        throw "observation result must remain explicitly read-only"
    }
    if ([string]$observedResult.inventoryHash -ne [string]$dirtyResult.inventoryHash) {
        throw "intent must not change the inventory hash"
    }

    $secondRepo = Join-Path $tempRoot "second repo"
    New-FixtureRepo $secondRepo
    Set-Content -LiteralPath (Join-Path $secondRepo "tracked file.txt") -Value "changed" -Encoding UTF8
    & git -C $secondRepo mv -- "rename source.txt" "renamed file.txt"
    if ($LASTEXITCODE -ne 0) { throw "second fixture rename failed" }
    New-Item -ItemType Directory -Force -Path (Join-Path $secondRepo "untracked folder") | Out-Null
    Set-Content -LiteralPath (Join-Path $secondRepo "untracked folder\new file.txt") -Value "new" -Encoding UTF8
    $secondDirty = Invoke-Preflight -Repo $secondRepo
    $secondDirtyResult = $secondDirty.output | ConvertFrom-Json
    if ($secondDirty.exitCode -ne 20 -or [string]$secondDirtyResult.inventoryHash -ne [string]$dirtyResult.inventoryHash) {
        throw "canonical inventory hash must be independent of fixture location"
    }

    $subdirectory = Join-Path $cleanRepo "untracked folder"
    $subdirectoryResult = Invoke-Preflight -Repo $subdirectory -Intent observation
    if ($subdirectoryResult.exitCode -ne 21) { throw "subdirectory invocation must fail closed with exit 21: $($subdirectoryResult.output)" }

    $notRepo = Join-Path $tempRoot "not a repo"
    New-Item -ItemType Directory -Force -Path $notRepo | Out-Null
    $notRepoResult = Invoke-Preflight -Repo $notRepo -Intent observation
    if ($notRepoResult.exitCode -ne 22) { throw "non-repository inspection must fail closed with exit 22: $($notRepoResult.output)" }

    $invalidPolicy = Join-Path $tempRoot "invalid-policy.json"
    '{"schema":"code-intel-multi-agent-workspace-policy.v1"}' | Set-Content -LiteralPath $invalidPolicy -Encoding UTF8
    $invalidPolicyResult = Invoke-Preflight -Repo $cleanRepo -Intent observation -PolicyPath $invalidPolicy
    if ($invalidPolicyResult.exitCode -ne 2) { throw "invalid policy must fail closed with exit 2" }

    Write-Host "PASS multi-agent workspace preflight: clean mutation admitted; dirty mutation, subdirectory, non-repository, and invalid policy fail closed; explicit observation remains read-only"
} finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
