#requires -Version 7.2

param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$atom = Join-Path $root "Invoke-CodeIntelAutomaticPullRequest.ps1"
$temp = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-auto-pr-" + [guid]::NewGuid().ToString("N"))

foreach ($schemaName in @(
    "code-intel-auto-pr-authorization.v1.schema.json",
    "code-intel-auto-pr-proposal.v1.schema.json",
    "code-intel-auto-pr-execution-result.v1.schema.json",
    "code-intel-auto-pr-execution-receipt.v1.schema.json"
)) {
    $schemaPath = Join-Path $root ("orchestration\schemas\" + $schemaName)
    if (-not (Test-Path -LiteralPath $schemaPath -PathType Leaf)) { throw "missing schema: $schemaName" }
    Get-Content -LiteralPath $schemaPath -Raw | ConvertFrom-Json | Out-Null
}

function Get-SnapshotIdentity {
    param([string]$Repo, [string]$Head)
    $lines = @(& git -C $Repo status --porcelain=v1 --untracked-files=all | ForEach-Object { $_.ToString() })
    if ($LASTEXITCODE -ne 0) { throw "fixture status failed" }
    $text = "code-intel-auto-pr-snapshot.v1`nhead=$Head`n" + (($lines | Sort-Object) -join "`n")
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($text)
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
}

function Invoke-Atom {
    param([string]$Repo, [string]$Auth, [string]$DecisionStore, [string]$ReplayQuery, [string]$Proposal, [string]$CodeIntel, [string]$Head, [string]$Snapshot, [string]$FakeGh, [switch]$RepoPermission, [switch]$NetworkPermission)
    $arguments = @("-NoProfile", "-File", $atom, "-RepoPath", $Repo, "-AuthorizationPath", $Auth, "-DecisionStore", $DecisionStore, "-DecisionReplayQueryPath", $ReplayQuery, "-ProposalPath", $Proposal, "-CodeIntelCommand", $CodeIntel, "-ExpectedHead", $Head, "-ExpectedSnapshotIdentity", $Snapshot, "-GhCommand", $FakeGh, "-Json")
    if ($RepoPermission) { $arguments += "-AllowRepositoryMutation" }
    if ($NetworkPermission) { $arguments += "-AllowNetworkPrCreate" }
    $output = @(& pwsh @arguments 2>&1 | ForEach-Object { $_.ToString() })
    return [pscustomobject]@{ exitCode = $LASTEXITCODE; result = (($output -join "`n") | ConvertFrom-Json) }
}

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null
    $repo = Join-Path $temp "repo"
    New-Item -ItemType Directory -Force -Path $repo | Out-Null
    & git -C $repo init --quiet
    & git -C $repo config user.name "Code Intel Test"
    & git -C $repo config user.email "code-intel-test@example.invalid"
    "fixture" | Set-Content -LiteralPath (Join-Path $repo "README.md") -Encoding UTF8
    & git -C $repo add README.md
    & git -C $repo commit --quiet -m "fixture"
    & git -C $repo branch -M feature/auto-pr
    if ($LASTEXITCODE -ne 0) { throw "fixture repository setup failed" }

    $head = [string]@(& git -C $repo rev-parse HEAD)[0]
    $snapshot = Get-SnapshotIdentity -Repo $repo -Head $head
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $codeIntel = Join-Path $root "target\debug\code-intel.exe"
    if (-not (Test-Path -LiteralPath $codeIntel -PathType Leaf)) {
        & cargo build -p code-intel --quiet
        if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $codeIntel -PathType Leaf)) { throw "code-intel decision CLI build failed" }
    }
    $decisionStore = Join-Path $temp "decision-store"
    New-Item -ItemType Directory -Force -Path $decisionStore | Out-Null
    $proposal = [ordered]@{
        schema = "code-intel-auto-pr-proposal.v1"; repository = "example/project"; snapshotIdentity = $snapshot; expectedHead = $head
        baseBranch = "main"; headBranch = "feature/auto-pr"; title = "Draft: verified fix"; body = "Created by the isolated beta atom test."; draft = $true
    }
    $proposalPath = Join-Path $temp "automatic-pr-proposal.json"
    [IO.File]::WriteAllText($proposalPath, ($proposal | ConvertTo-Json -Depth 8 -Compress), [Text.UTF8Encoding]::new($false))
    if (-not (Get-Content -LiteralPath $proposalPath -Raw | Test-Json -SchemaFile (Join-Path $root "orchestration/schemas/code-intel-auto-pr-proposal.v1.schema.json") -ErrorAction Stop)) { throw "proposal fixture failed its schema" }
    $proposalSha256 = (Get-FileHash -LiteralPath $proposalPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $evidence = [ordered]@{ refId = "automatic-pr-proposal"; sha256 = $proposalSha256; observedAt = $now - 10; expiresAt = $now + 600 }
    $options = @(
        [ordered]@{ id = "keep_disabled"; label = "Keep disabled"; consequence = "No pull request is created." },
        [ordered]@{ id = "enable_once_for_snapshot"; label = "Enable once"; consequence = "One draft pull request may be created." }
    )
    $gap = [ordered]@{
        schema = "code-intel-decision-gap.v1"; id = "automatic-pr-consent"; kind = "risk_acceptance"
        blockedDecision = "Create one automatic draft pull request"
        discoverableFactsChecked = @([ordered]@{ factId = "pipeline-evidence-current"; status = "resolved" })
        options = $options
        recommendedAnswer = [ordered]@{ kind = "proposal"; optionId = "keep_disabled"; rationale = "Keep external effects disabled by default." }
        affectedBranches = @("automatic_pr_execution"); authorityRequired = $true; authorityState = "unresolved"; effects = @()
    }
    $request = [ordered]@{
        schema = "code-intel-decision-request.v1"; correlationId = "auto-pr-test-consent"; gapId = "automatic-pr-consent"
        question = "Enable one automatic draft pull request?"
        recommendation = [ordered]@{ optionId = "keep_disabled"; rationale = "Keep external effects disabled by default." }
        evidenceRefs = @($evidence); options = $options
        authorityNeeded = [ordered]@{ kind = "repository_mutation_and_network_pr_create"; actorIds = @("test-user") }
        issuedAt = $now - 5; expiresAt = $now + 500; affectedBranches = @("automatic_pr_execution")
    }
    $resolution = [ordered]@{
        schema = "code-intel-decision-record-request.v1"; gap = $gap; request = $request
        response = [ordered]@{
            schema = "code-intel-decision-response.v1"; correlationId = "auto-pr-test-consent"; gapId = "automatic-pr-consent"
            answer = [ordered]@{ kind = "choice"; optionId = "enable_once_for_snapshot" }
            actorProvenance = [ordered]@{ actorId = "test-user"; authorityKind = "repository_mutation_and_network_pr_create"; source = "test-native-ui" }
            timestamp = $now
        }
        authorityEvent = [ordered]@{
            schema = "code-intel-authority-event.v1"; id = "auto-pr-authority-test-1"; decision = "approved"
            approver = [ordered]@{ id = "test-user"; role = "repository_mutation_and_network_pr_create" }
            evidenceIds = @("automatic-pr-proposal"); issuedAt = $now - 1; expiresAt = $now + 400
        }
        snapshotIdentity = $snapshot; recordedAt = $now
    }
    $resolutionPath = Join-Path $temp "decision-resolution.json"
    [IO.File]::WriteAllText($resolutionPath, ($resolution | ConvertTo-Json -Depth 16), [Text.UTF8Encoding]::new($false))
    $recordOutput = @(& $codeIntel decision record --resolution $resolutionPath --store $decisionStore 2>&1 | ForEach-Object { $_.ToString() })
    if ($LASTEXITCODE -ne 0) { throw "decision record fixture failed: $($recordOutput -join ' ')" }
    $record = (($recordOutput -join "`n") | ConvertFrom-Json).record
    $replayQuery = [ordered]@{
        schema = "code-intel-decision-replay-query.v1"; gapId = "automatic-pr-consent"; snapshotIdentity = $snapshot
        evidenceRefs = @($evidence); affectedBranches = @("automatic_pr_execution"); now = $now
    }
    $replayQueryPath = Join-Path $temp "decision-replay-query.json"
    [IO.File]::WriteAllText($replayQueryPath, ($replayQuery | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
    $authorization = [ordered]@{
        schema = "code-intel-auto-pr-authorization.v1"
        authorizationId = "auto-pr-test-1"
        decision = "approved"
        repository = "example/project"
        snapshotIdentity = $snapshot
        expectedHead = $head
        baseBranch = "main"
        headBranch = "feature/auto-pr"
        title = "Draft: verified fix"
        body = "Created by the isolated beta atom test."
        draft = $true
        consent = [ordered]@{ kind = "enable_once_for_snapshot"; actorId = "test-user"; source = "test-native-ui"; recordedAt = $now }
        decisionRecord = [ordered]@{ id = [string]$record.id; bindingDigest = [string]$record.bindingDigest; authorityEventId = [string]$record.authorityEvent.id }
        issuedAt = $now
        expiresAt = $now + 600
    }
    $authPath = Join-Path $temp "authorization.json"
    [IO.File]::WriteAllText($authPath, ($authorization | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    if (-not (Get-Content -LiteralPath $authPath -Raw | Test-Json -SchemaFile (Join-Path $root "orchestration/schemas/code-intel-auto-pr-authorization.v1.schema.json") -ErrorAction Stop)) {
        throw "authorization fixture failed its schema"
    }
    $log = Join-Path $temp "gh.log"
    $fakeGh = Join-Path $temp "fake-gh.ps1"
    @'
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest)
Add-Content -LiteralPath $env:CODE_INTEL_AUTO_PR_TEST_LOG -Value ($Rest -join "`t")
Write-Output "https://github.com/example/project/pull/42"
exit 0
'@ | Set-Content -LiteralPath $fakeGh -Encoding UTF8
    $env:CODE_INTEL_AUTO_PR_TEST_LOG = $log

    $snapshot = Get-SnapshotIdentity -Repo $repo -Head $head

    $noPermissions = Invoke-Atom -Repo $repo -Auth $authPath -DecisionStore $decisionStore -ReplayQuery $replayQueryPath -Proposal $proposalPath -CodeIntel $codeIntel -Head $head -Snapshot $snapshot -FakeGh $fakeGh
    if (-not ($noPermissions.result | ConvertTo-Json -Depth 10 | Test-Json -SchemaFile (Join-Path $root "orchestration/schemas/code-intel-auto-pr-execution-result.v1.schema.json") -ErrorAction Stop)) { throw "default-disabled result failed its schema" }
    if ($noPermissions.exitCode -eq 0 -or (Test-Path -LiteralPath $log)) { throw "default-disabled path invoked gh" }

    $onePermission = Invoke-Atom -Repo $repo -Auth $authPath -DecisionStore $decisionStore -ReplayQuery $replayQueryPath -Proposal $proposalPath -CodeIntel $codeIntel -Head $head -Snapshot $snapshot -FakeGh $fakeGh -RepoPermission
    if ($onePermission.exitCode -eq 0 -or (Test-Path -LiteralPath $log)) { throw "single-permission path invoked gh" }

    $wrongSnapshot = "0" * 64
    $drifted = Invoke-Atom -Repo $repo -Auth $authPath -DecisionStore $decisionStore -ReplayQuery $replayQueryPath -Proposal $proposalPath -CodeIntel $codeIntel -Head $head -Snapshot $wrongSnapshot -FakeGh $fakeGh -RepoPermission -NetworkPermission
    if ($drifted.exitCode -eq 0 -or (Test-Path -LiteralPath $log)) { throw "snapshot-drift path invoked gh" }

    $forgedStore = Join-Path $temp "forged-empty-store"
    New-Item -ItemType Directory -Force -Path $forgedStore | Out-Null
    $forged = Invoke-Atom -Repo $repo -Auth $authPath -DecisionStore $forgedStore -ReplayQuery $replayQueryPath -Proposal $proposalPath -CodeIntel $codeIntel -Head $head -Snapshot $snapshot -FakeGh $fakeGh -RepoPermission -NetworkPermission
    if ($forged.exitCode -eq 0 -or (Test-Path -LiteralPath $log)) { throw "hand-written authorization bypassed the decision record store" }

    Add-Content -LiteralPath $proposalPath -Value " " -Encoding UTF8
    $changedProposal = Invoke-Atom -Repo $repo -Auth $authPath -DecisionStore $decisionStore -ReplayQuery $replayQueryPath -Proposal $proposalPath -CodeIntel $codeIntel -Head $head -Snapshot $snapshot -FakeGh $fakeGh -RepoPermission -NetworkPermission
    if ($changedProposal.exitCode -eq 0 -or (Test-Path -LiteralPath $log)) { throw "changed proposal bypassed decision evidence binding" }
    [IO.File]::WriteAllText($proposalPath, ($proposal | ConvertTo-Json -Depth 8 -Compress), [Text.UTF8Encoding]::new($false))

    $created = Invoke-Atom -Repo $repo -Auth $authPath -DecisionStore $decisionStore -ReplayQuery $replayQueryPath -Proposal $proposalPath -CodeIntel $codeIntel -Head $head -Snapshot $snapshot -FakeGh $fakeGh -RepoPermission -NetworkPermission
    if ($created.exitCode -ne 0 -or $created.result.status -ne "created") {
        throw "fully authorized path did not create the draft PR: exit=$($created.exitCode); reason=$($created.result.reason)"
    }
    if (-not ($created.result | ConvertTo-Json -Depth 10 | Test-Json -SchemaFile (Join-Path $root "orchestration/schemas/code-intel-auto-pr-execution-result.v1.schema.json") -ErrorAction Stop)) { throw "created result failed its schema" }
    if (-not (Get-Content -LiteralPath $created.result.receipt -Raw | Test-Json -SchemaFile (Join-Path $root "orchestration/schemas/code-intel-auto-pr-execution-receipt.v1.schema.json") -ErrorAction Stop)) { throw "created receipt failed its schema" }
    $calls = @(Get-Content -LiteralPath $log)
    if ($calls.Count -ne 1) { throw "fake gh must be invoked exactly once" }
    if ($calls[0] -notmatch '^pr\s+create\s+' -or $calls[0] -notmatch '\s--draft$') { throw "fake gh did not receive one draft PR creation command" }

    $replay = Invoke-Atom -Repo $repo -Auth $authPath -DecisionStore $decisionStore -ReplayQuery $replayQueryPath -Proposal $proposalPath -CodeIntel $codeIntel -Head $head -Snapshot $snapshot -FakeGh $fakeGh -RepoPermission -NetworkPermission
    if ($replay.exitCode -eq 0 -or @((Get-Content -LiteralPath $log)).Count -ne 1) { throw "consumed authorization was replayed" }

    $authorization.authorizationId = "auto-pr-test-wrapper-2"
    [IO.File]::WriteAllText($authPath, ($authorization | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $rewrapped = Invoke-Atom -Repo $repo -Auth $authPath -DecisionStore $decisionStore -ReplayQuery $replayQueryPath -Proposal $proposalPath -CodeIntel $codeIntel -Head $head -Snapshot $snapshot -FakeGh $fakeGh -RepoPermission -NetworkPermission
    if ($rewrapped.exitCode -eq 0 -or @((Get-Content -LiteralPath $log)).Count -ne 1) { throw "same decision record was replayed through a second authorization id" }

    Write-Host "Automatic pull request beta atom tests passed."
} finally {
    Remove-Item Env:CODE_INTEL_AUTO_PR_TEST_LOG -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
