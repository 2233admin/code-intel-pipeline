#requires -Version 7.2

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$RepoPath,

    [Parameter(Mandatory = $true)]
    [string]$AuthorizationPath,

    [Parameter(Mandatory = $true)]
    [string]$DecisionStore,

    [Parameter(Mandatory = $true)]
    [string]$DecisionReplayQueryPath,

    [Parameter(Mandatory = $true)]
    [string]$ProposalPath,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-f]{40}$')]
    [string]$ExpectedHead,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-f]{64}$')]
    [string]$ExpectedSnapshotIdentity,

    [switch]$AllowRepositoryMutation,

    [switch]$AllowNetworkPrCreate,

    [string]$GhCommand = "gh",

    [string]$CodeIntelCommand = "code-intel",

    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-Sha256Text {
    param([Parameter(Mandatory = $true)][string]$Text)

    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text)
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
}

function Get-CurrentSnapshot {
    param([Parameter(Mandatory = $true)][string]$Repo)

    $headLines = @(& git -C $Repo rev-parse HEAD 2>$null)
    if ($LASTEXITCODE -ne 0 -or $headLines.Count -ne 1 -or [string]$headLines[0] -notmatch '^[0-9a-f]{40}$') {
        throw "Repository HEAD could not be resolved."
    }
    $head = ([string]$headLines[0]).ToLowerInvariant()

    $statusLines = @(& git -C $Repo status --porcelain=v1 --untracked-files=all 2>$null | ForEach-Object { $_.ToString() })
    if ($LASTEXITCODE -ne 0) { throw "Repository working-tree state could not be resolved." }
    $canonical = "code-intel-auto-pr-snapshot.v1`nhead=$head`n" + (($statusLines | Sort-Object) -join "`n")
    return [ordered]@{
        head = $head
        identity = Get-Sha256Text -Text $canonical
    }
}

function Assert-ExactProperties {
    param(
        [Parameter(Mandatory = $true)][object]$Value,
        [Parameter(Mandatory = $true)][string[]]$Required,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $actual = @($Value.PSObject.Properties.Name | Sort-Object)
    $expected = @($Required | Sort-Object)
    if (@(Compare-Object $actual $expected).Count -ne 0) {
        throw "$Label must contain exactly: $($Required -join ', ')."
    }
}

function Assert-Authorization {
    param([Parameter(Mandatory = $true)][object]$Authorization)

    Assert-ExactProperties $Authorization @(
        "schema", "authorizationId", "decision", "repository", "snapshotIdentity",
        "expectedHead", "baseBranch", "headBranch", "title", "body", "draft",
        "consent", "decisionRecord", "issuedAt", "expiresAt"
    ) "authorization"
    Assert-ExactProperties $Authorization.consent @("kind", "actorId", "source", "recordedAt") "authorization consent"
    Assert-ExactProperties $Authorization.decisionRecord @("id", "bindingDigest", "authorityEventId") "authorization decisionRecord"

    Assert-AuthorizationIdentity -Authorization $Authorization
    Assert-AuthorizationPullRequest -Authorization $Authorization
    Assert-AuthorizationConsent -Authorization $Authorization
}

function Assert-AuthorizationIdentity {
    param([Parameter(Mandatory = $true)][object]$Authorization)

    if ([string]$Authorization.schema -ne "code-intel-auto-pr-authorization.v1") { throw "Authorization schema is invalid." }
    if ([string]$Authorization.authorizationId -notmatch '^[A-Za-z0-9._-]+$') { throw "Authorization id is invalid." }
    if ([string]$Authorization.decision -ne "approved") { throw "Authorization decision is not approved." }
    if ([string]$Authorization.repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') { throw "Authorization repository must be owner/name." }
    if ([string]$Authorization.snapshotIdentity -notmatch '^[0-9a-f]{64}$') { throw "Authorization snapshot identity is invalid." }
    if ([string]$Authorization.expectedHead -notmatch '^[0-9a-f]{40}$') { throw "Authorization expected HEAD is invalid." }
    if ([string]$Authorization.decisionRecord.id -notmatch '^decision-record-v1:[0-9a-f]{64}$') { throw "Authorization decision record id is invalid." }
    if ([string]$Authorization.decisionRecord.bindingDigest -notmatch '^[0-9a-f]{64}$') { throw "Authorization decision binding digest is invalid." }
    if ([string]::IsNullOrWhiteSpace([string]$Authorization.decisionRecord.authorityEventId)) { throw "Authorization authority event id is required." }
}

function Assert-AuthorizationPullRequest {
    param([Parameter(Mandatory = $true)][object]$Authorization)

    if ([string]::IsNullOrWhiteSpace([string]$Authorization.baseBranch) -or [string]::IsNullOrWhiteSpace([string]$Authorization.headBranch)) { throw "Authorization branches are required." }
    if ([string]$Authorization.baseBranch -eq [string]$Authorization.headBranch) { throw "Authorization base and head branches must differ." }
    if ([string]::IsNullOrWhiteSpace([string]$Authorization.title) -or [string]::IsNullOrWhiteSpace([string]$Authorization.body)) { throw "Authorization title and body are required." }
    if ([bool]$Authorization.draft -ne $true) { throw "Beta automatic pull requests must be drafts." }
}

function Assert-AuthorizationConsent {
    param([Parameter(Mandatory = $true)][object]$Authorization)

    if ([string]$Authorization.consent.kind -ne "enable_once_for_snapshot") { throw "Authorization consent must be one-time and snapshot-bound." }
    if ([string]::IsNullOrWhiteSpace([string]$Authorization.consent.actorId) -or [string]::IsNullOrWhiteSpace([string]$Authorization.consent.source)) { throw "Authorization actor provenance is required." }

    $issuedAt = [long]$Authorization.issuedAt
    $expiresAt = [long]$Authorization.expiresAt
    $recordedAt = [long]$Authorization.consent.recordedAt
    if ($issuedAt -lt 0 -or $recordedAt -lt $issuedAt -or $expiresAt -le $recordedAt) { throw "Authorization timestamps are invalid." }
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    if ($now -ge $expiresAt) { throw "Authorization has expired." }
}

function Resolve-GhCommand {
    param([Parameter(Mandatory = $true)][string]$Command)

    if (Test-Path -LiteralPath $Command -PathType Leaf) {
        return [IO.Path]::GetFullPath($Command)
    }
    $resolved = Get-Command $Command -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $resolved) { throw "GitHub CLI command is unavailable." }
    return [string]$resolved.Source
}

function Resolve-ApplicationCommand {
    param([Parameter(Mandatory = $true)][string]$Command, [Parameter(Mandatory = $true)][string]$Label)

    if (Test-Path -LiteralPath $Command -PathType Leaf) { return [IO.Path]::GetFullPath($Command) }
    $resolved = Get-Command $Command -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $resolved) { throw "$Label command is unavailable." }
    return [string]$resolved.Source
}

function Get-ValidatedDecisionRecord {
    param(
        [Parameter(Mandatory = $true)][string]$Store,
        [Parameter(Mandatory = $true)][string]$QueryPath,
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][object]$Authorization,
        [Parameter(Mandatory = $true)][string]$ProposalSha256
    )

    if (-not (Test-Path -LiteralPath $Store -PathType Container)) { throw "Decision record store does not exist." }
    if (-not (Test-Path -LiteralPath $QueryPath -PathType Leaf)) { throw "Decision replay query does not exist." }
    $query = Get-Content -LiteralPath $QueryPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-ExactProperties $query @("schema", "gapId", "snapshotIdentity", "evidenceRefs", "affectedBranches", "now") "decision replay query"
    if ([string]$query.schema -ne "code-intel-decision-replay-query.v1") { throw "Decision replay query schema is invalid." }
    if ([string]$query.gapId -ne "automatic-pr-consent") { throw "Decision replay query is not scoped to automatic PR consent." }
    if (@($query.affectedBranches).Count -ne 1 -or [string]$query.affectedBranches[0] -ne "automatic_pr_execution") { throw "Decision replay query branch scope is invalid." }
    if ([string]$query.snapshotIdentity -ne ([string]$Authorization.snapshotIdentity).ToLowerInvariant()) { throw "Decision replay query snapshot does not match the authorization." }

    $query.now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $runtimeQuery = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-auto-pr-replay-" + [guid]::NewGuid().ToString("N") + ".json")
    try {
        [IO.File]::WriteAllText($runtimeQuery, ($query | ConvertTo-Json -Depth 12 -Compress), [Text.UTF8Encoding]::new($false))
        $resolved = Resolve-ApplicationCommand -Command $Command -Label "Code Intel"
        $lines = @(& $resolved decision replay --query $runtimeQuery --store $Store 2>&1 | ForEach-Object { $_.ToString() })
        if ($LASTEXITCODE -ne 0) { throw "Decision replay validation failed." }
        $outcome = ($lines -join "`n") | ConvertFrom-Json
    } finally {
        Remove-Item -LiteralPath $runtimeQuery -Force -ErrorAction SilentlyContinue
    }

    Assert-ExactProperties $outcome @("schema", "status", "questionRequired", "record", "reason", "diagnostics") "decision replay result"
    if ([string]$outcome.schema -ne "code-intel-decision-record-operation-result.v1" -or
        [string]$outcome.status -ne "replay" -or [bool]$outcome.questionRequired -ne $false -or
        $null -ne $outcome.reason -or $null -eq $outcome.record -or @($outcome.diagnostics).Count -ne 0) {
        throw "A clean, current, replay-valid user decision is required."
    }
    $record = $outcome.record
    if ([string]$record.id -ne [string]$Authorization.decisionRecord.id -or
        [string]$record.bindingDigest -ne [string]$Authorization.decisionRecord.bindingDigest -or
        [string]$record.authorityEvent.id -ne [string]$Authorization.decisionRecord.authorityEventId) { throw "Authorization is not bound to the replayed decision record." }
    if ([string]$record.gap.id -ne "automatic-pr-consent" -or [string]$record.request.gapId -ne "automatic-pr-consent" -or [string]$record.response.gapId -ne "automatic-pr-consent") { throw "Decision record is not an automatic PR consent decision." }
    if ([string]$record.acceptedChoice.kind -ne "choice" -or [string]$record.acceptedChoice.optionId -ne "enable_once_for_snapshot") { throw "User decision did not enable one automatic PR." }
    if (@($record.affectedBranches).Count -ne 1 -or [string]$record.affectedBranches[0] -ne "automatic_pr_execution") { throw "Decision record branch scope is invalid." }
    if ([string]$record.snapshotIdentity -ne ([string]$Authorization.snapshotIdentity).ToLowerInvariant()) { throw "Decision record snapshot does not match the authorization." }
    $proposalEvidence = @($record.evidenceBinding.refs | Where-Object { [string]$_.refId -eq "automatic-pr-proposal" -and [string]$_.sha256 -eq $ProposalSha256 })
    if ($proposalEvidence.Count -ne 1) { throw "Decision record does not bind the exact automatic PR proposal." }
    if ([string]$record.authorityEvent.decision -ne "approved") { throw "Decision authority event is not approved." }
    if ([string]$record.response.actorProvenance.actorId -ne [string]$record.authorityEvent.approver.id -or
        [string]$record.response.actorProvenance.actorId -ne [string]$Authorization.consent.actorId -or
        [string]$record.response.actorProvenance.source -ne [string]$Authorization.consent.source) { throw "Authorization consent provenance does not match the decision record." }
    if ([string]$record.response.actorProvenance.authorityKind -ne "repository_mutation_and_network_pr_create" -or
        [string]$record.authorityEvent.approver.role -ne "repository_mutation_and_network_pr_create") { throw "Decision record lacks automatic PR authority." }
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    if ($now -ge [long]$record.request.expiresAt -or $now -ge [long]$record.authorityEvent.expiresAt) { throw "Decision record authority has expired." }
    return $record
}

function Get-ValidatedProposal {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][object]$Authorization)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Automatic PR proposal does not exist." }
    $proposal = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-ExactProperties $proposal @("schema", "repository", "snapshotIdentity", "expectedHead", "baseBranch", "headBranch", "title", "body", "draft") "automatic PR proposal"
    if ([string]$proposal.schema -ne "code-intel-auto-pr-proposal.v1" -or [bool]$proposal.draft -ne $true) { throw "Automatic PR proposal is invalid." }
    foreach ($field in @("repository", "snapshotIdentity", "expectedHead", "baseBranch", "headBranch", "title", "body", "draft")) {
        if ($proposal.$field -ne $Authorization.$field) { throw "Authorization does not match the exact automatic PR proposal field: $field." }
    }
    $canonicalProposal = [ordered]@{
        schema = [string]$proposal.schema
        repository = [string]$proposal.repository
        snapshotIdentity = [string]$proposal.snapshotIdentity
        expectedHead = [string]$proposal.expectedHead
        baseBranch = [string]$proposal.baseBranch
        headBranch = [string]$proposal.headBranch
        title = [string]$proposal.title
        body = [string]$proposal.body
        draft = [bool]$proposal.draft
    }
    return [ordered]@{
        value = $proposal
        sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
        semanticIdentity = Get-Sha256Text -Text ("code-intel-auto-pr-proposal-semantics.v1`n" + ($canonicalProposal | ConvertTo-Json -Depth 4 -Compress))
    }
}

function Write-Result {
    param([Parameter(Mandatory = $true)][object]$Result)

    if ($Json) {
        $Result | ConvertTo-Json -Depth 10
    } else {
        Write-Host "Automatic PR: $($Result.status)"
        Write-Host "Reason: $($Result.reason)"
        if (-not [string]::IsNullOrWhiteSpace([string]$Result.url)) { Write-Host "URL: $($Result.url)" }
    }
}

$repo = [IO.Path]::GetFullPath($RepoPath)
$authorizationFile = [IO.Path]::GetFullPath($AuthorizationPath)
$decisionStorePath = [IO.Path]::GetFullPath($DecisionStore)
$decisionQueryFile = [IO.Path]::GetFullPath($DecisionReplayQueryPath)
$proposalFile = [IO.Path]::GetFullPath($ProposalPath)
$lockPath = $null
$lockOwned = $false
$networkAttempted = $false
$authorizationId = ""

try {
    if (-not $AllowRepositoryMutation -or -not $AllowNetworkPrCreate) {
        throw "Automatic PR creation requires both -AllowRepositoryMutation and -AllowNetworkPrCreate."
    }
    if (-not (Test-Path -LiteralPath $repo -PathType Container)) { throw "Repository path does not exist." }
    if (-not (Test-Path -LiteralPath $authorizationFile -PathType Leaf)) { throw "Authorization file does not exist." }

    & git -C $repo rev-parse --is-inside-work-tree *> $null
    if ($LASTEXITCODE -ne 0) { throw "Repository path is not a Git worktree." }

    $authorization = Get-Content -LiteralPath $authorizationFile -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-Authorization -Authorization $authorization
    $authorizationId = [string]$authorization.authorizationId
    $proposal = Get-ValidatedProposal -Path $proposalFile -Authorization $authorization
    $decisionRecord = Get-ValidatedDecisionRecord -Store $decisionStorePath -QueryPath $decisionQueryFile -Command $CodeIntelCommand -Authorization $authorization -ProposalSha256 $proposal.sha256

    $snapshot = Get-CurrentSnapshot -Repo $repo
    if ($snapshot.head -ne $ExpectedHead.ToLowerInvariant() -or $snapshot.head -ne ([string]$authorization.expectedHead).ToLowerInvariant()) {
        throw "Current HEAD does not match the explicitly expected and authorized HEAD."
    }
    if ($snapshot.identity -ne $ExpectedSnapshotIdentity.ToLowerInvariant() -or $snapshot.identity -ne ([string]$authorization.snapshotIdentity).ToLowerInvariant()) {
        throw "Current repository snapshot does not match the explicitly expected and authorized snapshot."
    }

    $branchLines = @(& git -C $repo branch --show-current 2>$null)
    if ($LASTEXITCODE -ne 0 -or $branchLines.Count -ne 1 -or [string]$branchLines[0] -ne [string]$authorization.headBranch) {
        throw "Current branch does not match the authorized PR head branch."
    }

    $gitCommonDirLines = @(& git -C $repo rev-parse --git-common-dir 2>$null)
    if ($LASTEXITCODE -ne 0 -or $gitCommonDirLines.Count -ne 1) { throw "Git common directory could not be resolved." }
    $gitCommonDir = [string]$gitCommonDirLines[0]
    if (-not [IO.Path]::IsPathRooted($gitCommonDir)) { $gitCommonDir = Join-Path $repo $gitCommonDir }
    $receiptRoot = Join-Path ([IO.Path]::GetFullPath($gitCommonDir)) "code-intel\auto-pr-authorizations"
    New-Item -ItemType Directory -Force -Path $receiptRoot | Out-Null
    $receiptKey = "proposal-" + [string]$proposal.semanticIdentity
    $receiptPath = Join-Path $receiptRoot ($receiptKey + ".json")
    $lockPath = $receiptPath + ".lock"
    if (Test-Path -LiteralPath $receiptPath -PathType Leaf) { throw "Authorization has already been consumed." }

    try {
        $lock = [IO.File]::Open($lockPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $lock.Dispose()
        $lockOwned = $true
    } catch [IO.IOException] {
        throw "Authorization is already executing or has a stale execution lock."
    }

    $resolvedGh = Resolve-GhCommand -Command $GhCommand
    $arguments = @(
        "pr", "create", "--repo", [string]$authorization.repository,
        "--base", [string]$authorization.baseBranch,
        "--head", [string]$authorization.headBranch,
        "--title", [string]$authorization.title,
        "--body", [string]$authorization.body,
        "--draft"
    )
    $networkAttempted = $true
    $output = @(& $resolvedGh @arguments 2>&1 | ForEach-Object { $_.ToString() })
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) { throw "GitHub CLI PR creation failed with exit code $exitCode." }
    $url = @($output | Where-Object { $_ -match '^https://github\.com/.+/pull/[0-9]+/?$' } | Select-Object -Last 1)
    if ($url.Count -ne 1) { throw "GitHub CLI did not return exactly one pull request URL." }

    $receipt = [ordered]@{
        schema = "code-intel-auto-pr-execution-receipt.v1"
        authorizationId = [string]$authorization.authorizationId
        repository = [string]$authorization.repository
        expectedHead = $snapshot.head
        snapshotIdentity = $snapshot.identity
        draft = $true
        url = [string]$url[0]
        executedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
        observedEffects = @("repo_mutation", "network")
    }
    [IO.File]::WriteAllText($lockPath, ($receipt | ConvertTo-Json -Depth 8 -Compress), [Text.UTF8Encoding]::new($false))
    [IO.File]::Move($lockPath, $receiptPath)
    $lockOwned = $false

    Write-Result ([ordered]@{
        schema = "code-intel-auto-pr-execution-result.v1"
        status = "created"
        reason = "One-time snapshot-bound authorization was consumed."
        authorizationId = [string]$authorization.authorizationId
        draft = $true
        url = [string]$url[0]
        receipt = $receiptPath
        declaredEffects = @("repo_mutation", "network")
        observedEffects = @("repo_mutation", "network")
    })
    exit 0
} catch {
    $failureReason = $_.Exception.Message
    $failureStatus = "not_authorized"
    $failureEffects = @()
    $failureReceipt = ""
    if ($lockOwned -and $null -ne $lockPath -and (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
        if ($networkAttempted) {
            $failureStatus = "execution_indeterminate"
            $failureEffects = @("repo_mutation", "network")
            $failureReceipt = $lockPath.Substring(0, $lockPath.Length - 5)
            $indeterminateReceipt = [ordered]@{
                schema = "code-intel-auto-pr-execution-receipt.v1"
                authorizationId = $authorizationId
                status = "indeterminate"
                reason = $failureReason
                executedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
                observedEffects = $failureEffects
            }
            [IO.File]::WriteAllText($lockPath, ($indeterminateReceipt | ConvertTo-Json -Depth 8 -Compress), [Text.UTF8Encoding]::new($false))
            [IO.File]::Move($lockPath, $failureReceipt)
            $lockOwned = $false
        } else {
            Remove-Item -LiteralPath $lockPath -Force -ErrorAction SilentlyContinue
        }
    }
    Write-Result ([ordered]@{
        schema = "code-intel-auto-pr-execution-result.v1"
        status = $failureStatus
        reason = $failureReason
        authorizationId = $authorizationId
        draft = $true
        url = ""
        receipt = $failureReceipt
        declaredEffects = @("repo_mutation", "network")
        observedEffects = $failureEffects
    })
    exit 65
}
