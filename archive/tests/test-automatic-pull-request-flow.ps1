#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$flow = Join-Path $root "Invoke-CodeIntelAutomaticPullRequestFlow.ps1"
$codeIntel = Join-Path $root "target\debug\code-intel.exe"
$temp = Join-Path ([IO.Path]::GetTempPath()) ("code-intel-auto-pr-flow-" + [guid]::NewGuid().ToString("N"))

function Invoke-Flow {
    param([string]$Repo, [string]$Artifacts, [string]$Store, [string]$Option, [string]$FakeGh, [switch]$Execute)
    $args = @(
        "-NoProfile", "-File", $flow, "-RepoPath", $Repo, "-Repository", "example/project", "-BaseBranch", "main",
        "-Title", "Draft: orchestrated fix", "-Body", "Created by the one-command orchestration test.",
        "-ActorId", "test-user", "-Source", "test-structured-input", "-DecisionStore", $Store,
        "-ArtifactDirectory", $Artifacts, "-CodeIntelCommand", $codeIntel, "-GhCommand", $FakeGh, "-NonInteractive", "-Json"
    )
    if ($Option) { $args += @("-DecisionOption", $Option) }
    if ($Execute) { $args += @("-AllowRepositoryMutation", "-AllowNetworkPrCreate") }
    $lines = @(& pwsh @args 2>&1 | ForEach-Object { $_.ToString() })
    [pscustomobject]@{ exitCode = $LASTEXITCODE; value = (($lines -join "`n") | ConvertFrom-Json) }
}

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null
    if (-not (Test-Path -LiteralPath $codeIntel -PathType Leaf)) {
        & cargo build -p code-intel --quiet
        if ($LASTEXITCODE -ne 0) { throw "code-intel build failed" }
    }
    $repo = Join-Path $temp "repo"
    New-Item -ItemType Directory -Force -Path $repo | Out-Null
    & git -C $repo init --quiet
    & git -C $repo config user.name "Code Intel Test"
    & git -C $repo config user.email "code-intel-test@example.invalid"
    [IO.File]::WriteAllText((Join-Path $repo "README.md"), "fixture`n", [Text.UTF8Encoding]::new($false))
    & git -C $repo add README.md
    & git -C $repo commit --quiet -m fixture
    & git -C $repo branch -M feature/auto-pr
    if ($LASTEXITCODE -ne 0) { throw "fixture repository setup failed" }
    $initialHead = [string]@(& git -C $repo rev-parse HEAD)[0]
    $initialStatus = (@(& git -C $repo status --porcelain=v1 --untracked-files=all) -join "`n")

    $log = Join-Path $temp "gh.log"
    $fakeGh = Join-Path $temp "fake-gh.ps1"
    @'
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest)
Add-Content -LiteralPath $env:CODE_INTEL_AUTO_PR_FLOW_LOG -Value ($Rest -join "`t")
Write-Output "https://github.com/example/project/pull/77"
exit 0
'@ | Set-Content -LiteralPath $fakeGh -Encoding UTF8
    $env:CODE_INTEL_AUTO_PR_FLOW_LOG = $log

    $pendingArtifacts = Join-Path $temp "pending"
    $pending = Invoke-Flow -Repo $repo -Artifacts $pendingArtifacts -Store (Join-Path $temp "pending-store") -FakeGh $fakeGh
    if ($pending.exitCode -ne 10 -or $pending.value.status -ne "pending" -or @($pending.value.observedEffects | Where-Object { $_ }).Count -ne 0) { throw "noninteractive pending contract failed: exit=$($pending.exitCode) status=$($pending.value.status) reason=$($pending.value.reason)" }
    $pendingProposal = Join-Path $pendingArtifacts "automatic-pr-proposal.json"
    $pendingRequest = Get-Content -LiteralPath (Join-Path $pendingArtifacts "automatic-pr-consent.request.json") -Raw | ConvertFrom-Json
    $pendingHash = (Get-FileHash -LiteralPath $pendingProposal -Algorithm SHA256).Hash.ToLowerInvariant()
    if ([string]$pendingRequest.evidenceRefs[0].sha256 -ne $pendingHash -or (Test-Path -LiteralPath $log)) { throw "pending request is not proposal-bound or invoked gh" }
    if (Test-Path -LiteralPath (Join-Path $pendingArtifacts "automatic-pr-authorization.json")) { throw "pending flow created authorization" }

    $insideRepo = Invoke-Flow -Repo $repo -Artifacts (Join-Path $repo ".forbidden-auto-pr") -Store (Join-Path $temp "inside-repo-store") -FakeGh $fakeGh
    if ($insideRepo.exitCode -eq 0 -or $insideRepo.value.status -ne "failed" -or (Test-Path -LiteralPath (Join-Path $repo ".forbidden-auto-pr"))) { throw "flow allowed pre-consent artifacts inside the target repository" }

    $insideStorePath = Join-Path $repo ".forbidden-decision-store"
    $insideStore = Invoke-Flow -Repo $repo -Artifacts (Join-Path $temp "outside-artifacts") -Store $insideStorePath -FakeGh $fakeGh
    if ($insideStore.exitCode -eq 0 -or $insideStore.value.status -ne "failed" -or (Test-Path -LiteralPath $insideStorePath)) { throw "flow allowed its decision store inside the target repository" }

    $declineArtifacts = Join-Path $temp "decline"
    $declineStore = Join-Path $temp "decline-store"
    $declined = Invoke-Flow -Repo $repo -Artifacts $declineArtifacts -Store $declineStore -Option "keep_disabled" -FakeGh $fakeGh
    if ($declined.exitCode -ne 0 -or $declined.value.status -ne "declined" -or @($declined.value.observedEffects | Where-Object { $_ }).Count -ne 0 -or (Test-Path -LiteralPath $log)) { throw "decline flow caused an effect or wrong status: exit=$($declined.exitCode) status=$($declined.value.status) reason=$($declined.value.reason)" }
    if (Test-Path -LiteralPath (Join-Path $declineArtifacts "automatic-pr-authorization.json")) { throw "decline flow created execution authorization" }
    $declineRecordDirs = @(Get-ChildItem -LiteralPath $declineStore -Directory -Filter "decision-*" -ErrorAction SilentlyContinue)
    if ($declineRecordDirs.Count -ne 1) { throw "decline decision was not durably recorded" }

    $approvedPendingArtifacts = Join-Path $temp "approved-pending"
    $approvedPending = Invoke-Flow -Repo $repo -Artifacts $approvedPendingArtifacts -Store (Join-Path $temp "approved-pending-store") -Option "enable_once_for_snapshot" -FakeGh $fakeGh
    if ($approvedPending.exitCode -ne 10 -or $approvedPending.value.status -ne "approved_pending_execution" -or (Test-Path -LiteralPath $log) -or (Test-Path -LiteralPath (Join-Path $approvedPendingArtifacts "automatic-pr-authorization.json"))) { throw "approved pending flow crossed the effect boundary" }

    $approveArtifacts = Join-Path $temp "approve"
    $approved = Invoke-Flow -Repo $repo -Artifacts $approveArtifacts -Store (Join-Path $temp "approve-store") -Option "enable_once_for_snapshot" -FakeGh $fakeGh -Execute
    if ($approved.exitCode -ne 0 -or $approved.value.status -ne "created" -or @($approved.value.observedEffects).Count -ne 2) { throw "approved flow did not create exactly one draft PR" }
    if (-not ($approved.value | ConvertTo-Json -Depth 20 | Test-Json -SchemaFile (Join-Path $root "orchestration/schemas/code-intel-auto-pr-flow-result.v1.schema.json") -ErrorAction Stop)) { throw "flow result schema validation failed" }
    $calls = @(Get-Content -LiteralPath $log)
    if ($calls.Count -ne 1 -or $calls[0] -notmatch '^pr\s+create\s+' -or $calls[0] -notmatch '\s--draft$') { throw "fake gh invocation was not one draft PR" }
    foreach ($file in @("automatic-pr-proposal.json", "automatic-pr-consent.request.json", "automatic-pr-consent.response.json", "automatic-pr-decision-resolution.json", "automatic-pr-decision-replay.json", "automatic-pr-authorization.json")) {
        if (-not (Test-Path -LiteralPath (Join-Path $approveArtifacts $file) -PathType Leaf)) { throw "approved flow artifact missing: $file" }
    }
    $auth = Get-Content -LiteralPath (Join-Path $approveArtifacts "automatic-pr-authorization.json") -Raw
    if (-not ($auth | Test-Json -SchemaFile (Join-Path $root "orchestration/schemas/code-intel-auto-pr-authorization.v1.schema.json") -ErrorAction Stop)) { throw "orchestrated authorization schema failed" }
    $proposal = Get-Content -LiteralPath (Join-Path $approveArtifacts "automatic-pr-proposal.json") -Raw
    if (-not ($proposal | Test-Json -SchemaFile (Join-Path $root "orchestration/schemas/code-intel-auto-pr-proposal.v1.schema.json") -ErrorAction Stop)) { throw "orchestrated proposal schema failed" }
    if ([string]@(& git -C $repo rev-parse HEAD)[0] -ne $initialHead -or (@(& git -C $repo status --porcelain=v1 --untracked-files=all) -join "`n") -ne $initialStatus) { throw "flow changed repository HEAD or worktree" }

    $duplicate = Invoke-Flow -Repo $repo -Artifacts (Join-Path $temp "duplicate") -Store (Join-Path $temp "duplicate-store") -Option "enable_once_for_snapshot" -FakeGh $fakeGh -Execute
    if ($duplicate.exitCode -eq 0 -or @((Get-Content -LiteralPath $log)).Count -ne 1) { throw "same exact proposal executed twice through a fresh decision record" }

    Write-Host "Automatic pull request one-command flow tests passed."
} finally {
    Remove-Item Env:CODE_INTEL_AUTO_PR_FLOW_LOG -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
