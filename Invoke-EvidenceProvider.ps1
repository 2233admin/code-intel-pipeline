param(
    [Parameter(Mandatory)]
    [ValidateSet("compete", "react-doctor")]
    [string]$Provider,

    [Parameter(Mandatory)]
    [ValidateSet("prepare", "status", "scan", "adapt")]
    [string]$Operation,

    [string]$RepoPath = "",
    [string]$ArtifactDir = "",
    [string]$Request = "",
    [long]$EvaluatedAt = -1,
    [long]$MaxAgeSeconds = 86400
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$CompeteRevision = "ec13028fc8da620c73a114ffe403a772b29a78cb"
$ReactDoctorIntegrity = "sha512-G3spmtZJE/gWWPRJ3rpgUWTPRDJpEmdRja7iNZ7RAXlfpEO+NWVzPTca/cPI9hLwPo2Aq5/BZggo5JDBrwGrlA=="

function Get-UnixTime {
    return [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
}

function Get-Sha256Text([string]$Text) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Text)
    $hash = [Security.Cryptography.SHA256]::HashData($bytes)
    return [Convert]::ToHexString($hash).ToLowerInvariant()
}

function Resolve-Repo([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw "-RepoPath is required for $Operation" }
    $resolved = (Resolve-Path -LiteralPath $Path).Path
    if (-not (Test-Path -LiteralPath (Join-Path $resolved ".git"))) {
        & git -C $resolved rev-parse --is-inside-work-tree 2>$null | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "RepoPath is not a Git worktree: $resolved" }
    }
    return $resolved
}

function Initialize-ArtifactDir([string]$Path, [string]$Repo) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw "-ArtifactDir is required" }
    $full = [IO.Path]::GetFullPath($Path)
    $repoFull = [IO.Path]::GetFullPath($Repo).TrimEnd([IO.Path]::DirectorySeparatorChar)
    if ($full.Equals($repoFull, [StringComparison]::OrdinalIgnoreCase) -or
        $full.StartsWith($repoFull + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "ArtifactDir must be outside the target repository"
    }
    New-Item -ItemType Directory -Force -Path $full | Out-Null
    return (Resolve-Path -LiteralPath $full).Path
}

function Get-SnapshotIdentity([string]$Repo) {
    $head = (& git -C $Repo rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "Cannot resolve repository HEAD" }
    $remote = (& git -C $Repo config --get remote.origin.url 2>$null)
    if ([string]::IsNullOrWhiteSpace($remote)) { $remote = $Repo }
    $diff = (& git -C $Repo diff --binary HEAD 2>$null) -join "`n"
    $untracked = @(& git -C $Repo ls-files --others --exclude-standard)
    $untrackedEvidence = foreach ($relative in $untracked) {
        $path = Join-Path $Repo $relative
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            "$relative=$((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant())"
        }
    }
    return Get-Sha256Text "$remote`n$head`n$diff`n$($untrackedEvidence -join "`n")"
}

function Write-JsonFile([string]$Path, $Value) {
    [IO.File]::WriteAllText(
        $Path,
        ($Value | ConvertTo-Json -Depth 30),
        [Text.UTF8Encoding]::new($false)
    )
}

function New-ArtifactRef([string]$Path, [string]$Root, [string]$Schema, [string]$Type, [string]$Snapshot) {
    return [ordered]@{
        schema = "code-intel-artifact-ref.v1"
        artifactSchema = $Schema
        type = $Type
        path = [IO.Path]::GetRelativePath($Root, $Path).Replace("\", "/")
        sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
        consumedSnapshotIdentity = $Snapshot
    }
}

function Find-CodeIntelCli {
    $exe = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
    foreach ($candidate in @(
        (Join-Path $PSScriptRoot "target/debug/$exe"),
        (Join-Path $PSScriptRoot "target/release/$exe"),
        (Join-Path $PSScriptRoot "bin/$exe")
    )) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    $command = Get-Command code-intel -ErrorAction SilentlyContinue
    if ($null -ne $command) { return $command.Source }
    throw "code-intel CLI is unavailable; run cargo build -p code-intel"
}

function Test-NpmUnavailable([string]$StderrPath) {
    if (-not (Test-Path -LiteralPath $StderrPath -PathType Leaf)) { return $false }
    $stderrText = Get-Content -LiteralPath $StderrPath -Raw
    return $stderrText -match "EAI_AGAIN|ENOTFOUND|ECONNREFUSED|ETIMEDOUT|TAR_BAD_ARCHIVE|npm error code E404|network (request|connectivity)"
}

function Invoke-Adapter([string]$NativePath, [string]$Root, [long]$At) {
    $cli = Find-CodeIntelCli
    $raw = @(& $cli provider "$Provider-adapt" --request $NativePath --artifact-root $Root --evaluated-at $At --max-age-seconds $MaxAgeSeconds)
    if ($LASTEXITCODE -ne 0) { throw "$Provider adapter failed" }
    $result = ($raw -join "`n") | ConvertFrom-Json
    Write-JsonFile (Join-Path $Root "$Provider-route-result.json") $result
    return $result
}

function Invoke-CompetePrepare {
    $repo = Resolve-Repo $RepoPath
    $root = Initialize-ArtifactDir $ArtifactDir $repo
    $snapshot = Get-SnapshotIdentity $repo
    $promptPath = Join-Path $root "compete-prompt.md"
    $requestPath = Join-Path $root "compete-request.json"
    $nativePath = Join-Path $root "compete-native-result.json"
    $prompt = @"
# Compete research task

Analyze the repository at $repo using Compete revision $CompeteRevision.
Use Claude Code WebSearch/WebFetch for research. Do not install the plugin and do
not modify the repository. Write every output under $root.

Required datasets: product.json, competitors.json, companies.json, pricing.json,
techstack.json, social.json, marketing.json, seo.json, features.json.
Also write report.json, report.html, and provenance source artifacts.

Validate datasets with Compete's own schemas. Then replace $nativePath with a
code-intel-compete-native-result.v1 envelope. Every Artifact
Ref path must be relative to the artifact directory, carry a lowercase SHA-256,
and use consumedSnapshotIdentity $snapshot.
"@
    [IO.File]::WriteAllText($promptPath, $prompt, [Text.UTF8Encoding]::new($false))
    Write-JsonFile $requestPath ([ordered]@{
        schema = "code-intel-compete-request.v1"
        snapshotIdentity = $snapshot
        repo = $repo
        artifactRoot = $root
        prompt = "compete-prompt.md"
        requiredDatasets = @("product", "competitors", "companies", "pricing", "techstack", "social", "marketing", "seo", "features")
        tool = [ordered]@{ revision = $CompeteRevision; license = "MIT" }
    })
    Write-JsonFile $nativePath ([ordered]@{
        schema = "code-intel-compete-native-result.v1"
        snapshotIdentity = $snapshot
        status = "not_run"
        observedAt = Get-UnixTime
        tool = [ordered]@{ revision = $CompeteRevision; license = "MIT" }
        artifacts = $null
        error = $null
    })
    return [ordered]@{ ok = $true; status = "prepared"; request = $requestPath; prompt = $promptPath; nativeResult = $nativePath }
}

function Get-CompeteStatus {
    $repo = Resolve-Repo $RepoPath
    $root = Initialize-ArtifactDir $ArtifactDir $repo
    $route = Join-Path $root "compete-route-result.json"
    $native = Join-Path $root "compete-native-result.json"
    $requestPath = Join-Path $root "compete-request.json"
    if (Test-Path -LiteralPath $route) {
        return [ordered]@{ ok = $true; status = "adapted"; routeResult = $route }
    }
    if (Test-Path -LiteralPath $native) {
        $value = Get-Content -LiteralPath $native -Raw | ConvertFrom-Json
        return [ordered]@{ ok = $true; status = if ($value.status -eq "not_run") { "prepared" } else { "ready_for_adapt" }; nativeStatus = $value.status; nativeResult = $native }
    }
    return [ordered]@{ ok = $true; status = if (Test-Path -LiteralPath $requestPath) { "prepared" } else { "not_run" } }
}

function Invoke-ReactDoctorScan {
    $repo = Resolve-Repo $RepoPath
    $root = Initialize-ArtifactDir $ArtifactDir $repo
    $snapshot = Get-SnapshotIdentity $repo
    $observedAt = Get-UnixTime
    $nativePath = Join-Path $root "react-doctor-native-result.json"
    $reportPath = Join-Path $root "react-doctor-report.json"
    $stderrPath = Join-Path $root "react-doctor.stderr.txt"
    $command = @("npx", "--yes", "react-doctor@0.7.8", "--json", "--no-telemetry")
    $status = "provider_unavailable"
    $errorText = $null
    $reportRef = $null
    $npx = Get-Command npx -ErrorAction SilentlyContinue
    if ($null -ne $npx) {
        $before = @(& git -C $repo status --porcelain=v1 --untracked-files=all)
        Push-Location $repo
        try {
            & $npx.Source --yes react-doctor@0.7.8 --json --no-telemetry 1> $reportPath 2> $stderrPath
            $exitCode = $LASTEXITCODE
        }
        finally {
            Pop-Location
        }
        $after = @(& git -C $repo status --porcelain=v1 --untracked-files=all)
        if (($before -join "`n") -ne ($after -join "`n")) {
            $status = "local_tool_error"
            $errorText = "React Doctor modified the target repository"
        }
        else {
            try {
                $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
                if ($report.schemaVersion -ne 3) { throw "unsupported JSON schema" }
                $notApplicable = $report.ok -eq $false -and
                    $null -ne $report.error -and
                    $report.error.name -eq "ProjectNotFoundError"
                if ($exitCode -eq 0 -or $notApplicable) {
                    $status = "completed"
                    $reportRef = New-ArtifactRef $reportPath $root "react-doctor-json-report.v3" "react-doctor-report" $snapshot
                }
                elseif (Test-NpmUnavailable $stderrPath) {
                    $status = "provider_unavailable"
                    $errorText = "npm or the React Doctor package was unavailable"
                }
                else {
                    $status = "local_tool_error"
                    $errorText = "React Doctor exited with code $exitCode"
                }
            }
            catch {
                if ($exitCode -ne 0 -and (Test-NpmUnavailable $stderrPath)) {
                    $status = "provider_unavailable"
                    $errorText = "npm or the React Doctor package was unavailable"
                }
                else {
                    $status = "local_tool_error"
                    $errorText = "React Doctor returned corrupt or unsupported JSON: $($_.Exception.Message)"
                }
            }
        }
    }
    Write-JsonFile $nativePath ([ordered]@{
        schema = "code-intel-react-doctor-native-result.v1"
        snapshotIdentity = $snapshot
        status = $status
        observedAt = $observedAt
        tool = [ordered]@{ version = "0.7.8"; integrity = $ReactDoctorIntegrity; command = $command }
        report = $reportRef
        error = $errorText
    })
    return Invoke-Adapter $nativePath $root $observedAt
}

$valid = ($Provider -eq "compete" -and $Operation -in @("prepare", "status", "adapt")) -or
    ($Provider -eq "react-doctor" -and $Operation -in @("scan", "adapt"))
if (-not $valid) { throw "Operation $Operation is not valid for provider $Provider" }

$result = switch ("$Provider/$Operation") {
    "compete/prepare" { Invoke-CompetePrepare }
    "compete/status" { Get-CompeteStatus }
    "react-doctor/scan" { Invoke-ReactDoctorScan }
    default {
        if ([string]::IsNullOrWhiteSpace($ArtifactDir)) { throw "-ArtifactDir is required" }
        $root = (Resolve-Path -LiteralPath $ArtifactDir).Path
        $native = if ([string]::IsNullOrWhiteSpace($Request)) {
            Join-Path $root "$Provider-native-result.json"
        } else {
            $Request
        }
        $at = if ($EvaluatedAt -ge 0) { $EvaluatedAt } else { Get-UnixTime }
        Invoke-Adapter $native $root $at
    }
}

$result | ConvertTo-Json -Depth 30
