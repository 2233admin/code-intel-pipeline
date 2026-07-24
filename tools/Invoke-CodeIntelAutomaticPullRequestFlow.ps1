#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RepoPath,
    [Parameter(Mandatory = $true)][ValidatePattern('^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$')][string]$Repository,
    [Parameter(Mandatory = $true)][string]$BaseBranch,
    [Parameter(Mandatory = $true)][string]$Title,
    [Parameter(Mandatory = $true)][string]$Body,
    [string]$ActorId = "user",
    [string]$Source = "native-console",
    [string]$DecisionResponsePath,
    [ValidateSet("keep_disabled", "enable_once_for_snapshot")][string]$DecisionOption,
    [string]$DecisionStore,
    [string]$ArtifactDirectory,
    [string]$CodeIntelCommand = "code-intel",
    [string]$GhCommand = "gh",
    [ValidateRange(60, 86400)][int]$ConsentLifetimeSeconds = 900,
    [switch]$NonInteractive,
    [switch]$AllowRepositoryMutation,
    [switch]$AllowNetworkPrCreate,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$authorityKind = "repository_mutation_and_network_pr_create"
$branchScope = "automatic_pr_execution"

function Write-JsonFile {
    param([string]$Path, [object]$Value)
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 20 -Compress), [Text.UTF8Encoding]::new($false))
}

function Get-Sha256Text {
    param([string]$Text)
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text)
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
}

function Get-RepositorySnapshot {
    param([string]$Repo)
    $headLines = @(& git -C $Repo rev-parse HEAD 2>$null)
    if ($LASTEXITCODE -ne 0 -or $headLines.Count -ne 1 -or [string]$headLines[0] -notmatch '^[0-9a-f]{40}$') { throw "Repository HEAD could not be resolved." }
    $head = ([string]$headLines[0]).ToLowerInvariant()
    $status = @(& git -C $Repo status --porcelain=v1 --untracked-files=all 2>$null | ForEach-Object { $_.ToString() })
    if ($LASTEXITCODE -ne 0) { throw "Repository working-tree state could not be resolved." }
    $canonical = "code-intel-auto-pr-snapshot.v1`nhead=$head`n" + (($status | Sort-Object) -join "`n")
    [ordered]@{ head = $head; identity = Get-Sha256Text $canonical }
}

function Resolve-CodeIntel {
    param([string]$Command)
    if (Test-Path -LiteralPath $Command -PathType Leaf) { return [IO.Path]::GetFullPath($Command) }
    $resolved = Get-Command $Command -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $resolved) { return [string]$resolved.Source }
    foreach ($candidate in @((Join-Path $root "bin\code-intel.exe"), (Join-Path $root "target\release\code-intel.exe"), (Join-Path $root "target\debug\code-intel.exe"))) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    throw "Code Intel CLI is unavailable."
}

function Test-PathInside {
    param([string]$Candidate, [string]$Parent)
    $candidateFull = [IO.Path]::GetFullPath($Candidate).TrimEnd('\', '/')
    $parentFull = [IO.Path]::GetFullPath($Parent).TrimEnd('\', '/')
    return $candidateFull.Equals($parentFull, [StringComparison]::OrdinalIgnoreCase) -or
        $candidateFull.StartsWith(($parentFull + [IO.Path]::DirectorySeparatorChar), [StringComparison]::OrdinalIgnoreCase)
}

function Invoke-CodeIntelJson {
    param([string]$Command, [string[]]$Arguments, [string]$Label)
    $lines = @(& $Command @Arguments 2>&1 | ForEach-Object { $_.ToString() })
    if ($LASTEXITCODE -ne 0) { throw "$Label failed: $($lines -join ' ')" }
    try { return (($lines -join "`n") | ConvertFrom-Json) } catch { throw "$Label returned malformed JSON." }
}

function Complete-Flow {
    param([string]$Status, [string]$Reason, [string]$Artifacts, [object]$Execution, [int]$ExitCode)
    $result = [ordered]@{
        schema = "code-intel-auto-pr-flow-result.v1"
        status = $Status
        reason = $Reason
        artifactDirectory = $Artifacts
        execution = $Execution
        observedEffects = if ($null -ne $Execution -and $Execution.status -in @("created", "execution_indeterminate")) { @("repo_mutation", "network") } else { @() }
    }
    if ($Json) { $result | ConvertTo-Json -Depth 16 } else { Write-Host "Automatic PR flow: $Status"; Write-Host "Reason: $Reason"; Write-Host "Artifacts: $Artifacts" }
    exit $ExitCode
}

$artifacts = ""
try {
    if ([string]::IsNullOrWhiteSpace($ActorId) -or [string]::IsNullOrWhiteSpace($Source)) { throw "ActorId and Source are required." }
    if ($DecisionResponsePath -and $DecisionOption) { throw "DecisionResponsePath and DecisionOption are mutually exclusive." }
    if ($NonInteractive -and $DecisionOption -and (-not $PSBoundParameters.ContainsKey("ActorId") -or -not $PSBoundParameters.ContainsKey("Source"))) {
        throw "Noninteractive DecisionOption requires explicit ActorId and Source provenance."
    }
    if ($NonInteractive -and $DecisionResponsePath -and -not $PSBoundParameters.ContainsKey("ActorId")) {
        throw "Noninteractive DecisionResponsePath requires the expected ActorId."
    }
    if ($AllowRepositoryMutation.IsPresent -xor $AllowNetworkPrCreate.IsPresent) { throw "Full execution requires both effect switches; supply neither to stop after authorization." }
    $repo = [IO.Path]::GetFullPath($RepoPath)
    if (-not (Test-Path -LiteralPath $repo -PathType Container)) { throw "Repository path does not exist." }
    & git -C $repo rev-parse --is-inside-work-tree *> $null
    if ($LASTEXITCODE -ne 0) { throw "Repository path is not a Git worktree." }
    $headBranchLines = @(& git -C $repo branch --show-current 2>$null)
    if ($LASTEXITCODE -ne 0 -or $headBranchLines.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$headBranchLines[0])) { throw "Current branch could not be resolved." }
    $headBranch = [string]$headBranchLines[0]
    if ($headBranch -eq $BaseBranch) { throw "Base branch and current head branch must differ." }

    $dataRoot = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "code-intel" } else { Join-Path ([IO.Path]::GetTempPath()) "code-intel" }
    $artifacts = if ($ArtifactDirectory) { [IO.Path]::GetFullPath($ArtifactDirectory) } else { Join-Path $dataRoot ("auto-pr-flows\" + [guid]::NewGuid().ToString("N")) }
    $store = if ($DecisionStore) { [IO.Path]::GetFullPath($DecisionStore) } else { Join-Path $dataRoot "decision-records\auto-pr" }
    if ((Test-PathInside $artifacts $repo) -or (Test-PathInside $store $repo)) {
        throw "ArtifactDirectory and DecisionStore must remain outside the target repository."
    }
    New-Item -ItemType Directory -Force -Path $artifacts,$store | Out-Null
    $cli = Resolve-CodeIntel $CodeIntelCommand
    $snapshot = Get-RepositorySnapshot $repo
    $issuedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $expiresAt = $issuedAt + $ConsentLifetimeSeconds
    $suffix = Get-Sha256Text "$Repository`n$($snapshot.identity)`n$Title`n$Body"
    $correlationId = "auto-pr-" + $suffix.Substring(0, 24)

    $proposal = [ordered]@{
        schema = "code-intel-auto-pr-proposal.v1"; repository = $Repository; snapshotIdentity = $snapshot.identity
        expectedHead = $snapshot.head; baseBranch = $BaseBranch; headBranch = $headBranch; title = $Title; body = $Body; draft = $true
    }
    $proposalPath = Join-Path $artifacts "automatic-pr-proposal.json"
    Write-JsonFile $proposalPath $proposal
    $proposalDigest = (Get-FileHash -LiteralPath $proposalPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $evidence = [ordered]@{ refId = "automatic-pr-proposal"; sha256 = $proposalDigest; observedAt = $issuedAt; expiresAt = $expiresAt }
    $options = @(
        [ordered]@{ id = "keep_disabled"; label = "Keep automatic PR disabled"; consequence = "No pull request is created." },
        [ordered]@{ id = "enable_once_for_snapshot"; label = "Create one draft PR"; consequence = "One exact snapshot-bound draft pull request may be created." }
    )
    $request = [ordered]@{
        schema = "code-intel-decision-request.v1"; correlationId = $correlationId; gapId = "automatic-pr-consent"
        question = "Create one draft pull request for '$Title' from '$headBranch' to '$BaseBranch' in '$Repository'?"
        recommendation = [ordered]@{ optionId = "keep_disabled"; rationale = "Keep repository and network effects disabled unless the user explicitly approves this exact proposal." }
        evidenceRefs = @($evidence); options = $options; authorityNeeded = [ordered]@{ kind = $authorityKind; actorIds = @($ActorId) }
        issuedAt = $issuedAt; expiresAt = $expiresAt; affectedBranches = @($branchScope)
    }
    $requestPath = Join-Path $artifacts "automatic-pr-consent.request.json"
    Write-JsonFile $requestPath $request

    $responsePath = ""
    if ($DecisionResponsePath) {
        $responsePath = [IO.Path]::GetFullPath($DecisionResponsePath)
        if (-not (Test-Path -LiteralPath $responsePath -PathType Leaf)) { throw "Decision response file does not exist." }
    } elseif ($NonInteractive -and -not $DecisionOption) {
        Complete-Flow "pending" "A structured decision response is required; no repository or network effect occurred." $artifacts $null 10
    } else {
        $choice = $DecisionOption
        if (-not $choice) {
            Write-Host $request.question
            Write-Host "Repository: $Repository"
            Write-Host "Branches: $headBranch -> $BaseBranch"
            Write-Host "Snapshot: $($snapshot.identity)"
            Write-Host "Draft: true"
            Write-Host "Options: keep_disabled | enable_once_for_snapshot"
            $answer = Read-Host "Decision [keep_disabled]"
            $choice = if ($answer -in @("enable_once_for_snapshot", "yes", "y")) { "enable_once_for_snapshot" } else { "keep_disabled" }
        }
        $response = [ordered]@{
            schema = "code-intel-decision-response.v1"; correlationId = $correlationId; gapId = "automatic-pr-consent"
            answer = [ordered]@{ kind = "choice"; optionId = $choice }
            actorProvenance = [ordered]@{ actorId = $ActorId; authorityKind = $authorityKind; source = $Source }
            timestamp = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
        }
        $responsePath = Join-Path $artifacts "automatic-pr-consent.response.json"
        Write-JsonFile $responsePath $response
    }
    $responseDocument = Get-Content -LiteralPath $responsePath -Raw -Encoding UTF8 | ConvertFrom-Json
    $decisionNow = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $exchange = Invoke-CodeIntelJson $cli @("decision", "request-response", "--request", $requestPath, "--response", $responsePath, "--now", [string]$decisionNow, "--branch", $branchScope) "Decision exchange"
    if ([string]$exchange.schema -ne "code-intel-decision-exchange-result.v1" -or [string]$exchange.status -ne "resolved" -or @($exchange.effects).Count -ne 0) { throw "Decision exchange did not resolve cleanly." }

    $gap = [ordered]@{
        schema = "code-intel-decision-gap.v1"; id = "automatic-pr-consent"; kind = "risk_acceptance"
        blockedDecision = "Create one automatic draft pull request"; discoverableFactsChecked = @([ordered]@{ factId = "automatic-pr-proposal-current"; status = "resolved" })
        options = $options; recommendedAnswer = [ordered]@{ kind = "proposal"; optionId = "keep_disabled"; rationale = $request.recommendation.rationale }
        affectedBranches = @($branchScope); authorityRequired = $true; authorityState = "unresolved"; effects = @()
    }
    $recordedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $authorityEvent = [ordered]@{
        schema = "code-intel-authority-event.v1"; id = "authority-" + $correlationId; decision = "approved"
        approver = [ordered]@{ id = [string]$responseDocument.actorProvenance.actorId; role = [string]$responseDocument.actorProvenance.authorityKind }
        evidenceIds = @("automatic-pr-proposal"); issuedAt = [long]$responseDocument.timestamp; expiresAt = $expiresAt
    }
    $resolution = [ordered]@{
        schema = "code-intel-decision-record-request.v1"; gap = $gap; request = $request; response = $responseDocument
        authorityEvent = $authorityEvent; snapshotIdentity = $snapshot.identity; recordedAt = $recordedAt
    }
    $resolutionPath = Join-Path $artifacts "automatic-pr-decision-resolution.json"
    Write-JsonFile $resolutionPath $resolution
    $recordOutcome = Invoke-CodeIntelJson $cli @("decision", "record", "--resolution", $resolutionPath, "--store", $store) "Decision record"
    if ([string]$recordOutcome.status -notin @("recorded", "replay") -or $null -eq $recordOutcome.record) { throw "C07 did not commit the decision record." }

    $replayQuery = [ordered]@{
        schema = "code-intel-decision-replay-query.v1"; gapId = "automatic-pr-consent"; snapshotIdentity = $snapshot.identity
        evidenceRefs = @($evidence); affectedBranches = @($branchScope); now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    }
    $replayPath = Join-Path $artifacts "automatic-pr-decision-replay.json"
    Write-JsonFile $replayPath $replayQuery
    $replay = Invoke-CodeIntelJson $cli @("decision", "replay", "--query", $replayPath, "--store", $store) "Decision replay"
    if ([string]$replay.status -ne "replay" -or [bool]$replay.questionRequired -ne $false -or $null -eq $replay.record) { throw "C07 replay did not authorize continuation." }

    $choice = [string]$responseDocument.answer.optionId
    if ($choice -ne "enable_once_for_snapshot") {
        Complete-Flow "declined" "The decision was recorded and replayed; automatic PR remains disabled." $artifacts $null 0
    }
    if (-not $AllowRepositoryMutation -and -not $AllowNetworkPrCreate) {
        Complete-Flow "approved_pending_execution" "Approval is recorded, but effect switches were not supplied." $artifacts $null 10
    }

    $record = $replay.record
    $authorization = [ordered]@{
        schema = "code-intel-auto-pr-authorization.v1"; authorizationId = "auto-pr-" + ([string]$record.bindingDigest).Substring(0, 24); decision = "approved"
        repository = $Repository; snapshotIdentity = $snapshot.identity; expectedHead = $snapshot.head; baseBranch = $BaseBranch; headBranch = $headBranch
        title = $Title; body = $Body; draft = $true
        consent = [ordered]@{ kind = "enable_once_for_snapshot"; actorId = [string]$responseDocument.actorProvenance.actorId; source = [string]$responseDocument.actorProvenance.source; recordedAt = [long]$responseDocument.timestamp }
        decisionRecord = [ordered]@{ id = [string]$record.id; bindingDigest = [string]$record.bindingDigest; authorityEventId = [string]$record.authorityEvent.id }
        issuedAt = [long]$responseDocument.timestamp; expiresAt = $expiresAt
    }
    $authorizationPath = Join-Path $artifacts "automatic-pr-authorization.json"
    Write-JsonFile $authorizationPath $authorization
    $executor = Join-Path $root "Invoke-CodeIntelAutomaticPullRequest.ps1"
    $executorArguments = @("-NoProfile", "-File", $executor, "-RepoPath", $repo, "-AuthorizationPath", $authorizationPath, "-DecisionStore", $store, "-DecisionReplayQueryPath", $replayPath, "-ProposalPath", $proposalPath, "-ExpectedHead", $snapshot.head, "-ExpectedSnapshotIdentity", $snapshot.identity, "-CodeIntelCommand", $cli, "-GhCommand", $GhCommand, "-AllowRepositoryMutation", "-AllowNetworkPrCreate", "-Json")
    $executorLines = @(& pwsh @executorArguments 2>&1 | ForEach-Object { $_.ToString() })
    $executorExit = $LASTEXITCODE
    try { $execution = (($executorLines -join "`n") | ConvertFrom-Json) } catch { throw "Automatic PR executor returned malformed JSON." }
    if ($executorExit -ne 0 -and [string]$execution.status -ne "execution_indeterminate") { throw "Automatic PR executor rejected execution: $($execution.reason)" }
    Complete-Flow ([string]$execution.status) ([string]$execution.reason) $artifacts $execution $executorExit
} catch {
    Complete-Flow "failed" $_.Exception.Message $artifacts $null 65
}
