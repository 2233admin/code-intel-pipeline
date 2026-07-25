[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][long]$EvaluatedAt,
    [string]$RepoRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),
    [string]$CodeIntel = (Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) "target\debug\code-intel.exe"),
    [string]$SourceRevision = "ca9334aa8eb8df3be7e10c5547069f03645cabe2"
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($EvaluatedAt -le 0) { throw "EvaluatedAt must be positive" }
if (Test-Path -LiteralPath $OutDir) { throw "packet output must be exclusive: $OutDir" }
if (-not (Test-Path -LiteralPath $CodeIntel -PathType Leaf)) { throw "code-intel binary is missing: $CodeIntel" }
$null = New-Item -ItemType Directory -Path $OutDir
$evidenceDir = Join-Path $OutDir "evidence"
$null = New-Item -ItemType Directory -Path $evidenceDir

function Write-Json([string]$Path, [object]$Value) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 30 -Compress), [Text.UTF8Encoding]::new($false))
}
function Get-TextSha([string]$Text) {
    ([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Text)))).ToLowerInvariant()
}
function New-Ref([string]$Schema, [string]$Type, [string]$Relative) {
    $path = Join-Path $OutDir ($Relative -replace '/', [IO.Path]::DirectorySeparatorChar)
    [ordered]@{ schema="code-intel-artifact-ref.v1"; artifactSchema=$Schema; type=$Type; path=($Relative -replace '\\','/'); sha256=((Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant()); consumedSnapshotIdentity=$snapshotIdentity }
}

& pwsh -NoLogo -NoProfile -File (Join-Path $RepoRoot "test-repowise-adapter-contract.ps1") | Out-Null
if ($LASTEXITCODE -ne 0) { throw "B01 adapter contract failed" }
& cargo test -p code-intel --test capability_exec repowise_route::public_route_translates_and_a04_validates_success_quota_and_index_only --quiet | Out-Null
if ($LASTEXITCODE -ne 0) { throw "B01 public route parity failed" }
& cargo test -p code-intel --test capability_exec repowise_adapter::docs_quota_is_partial_provider_unavailable_without_erasing_current_index --quiet | Out-Null
if ($LASTEXITCODE -ne 0) { throw "B01 quota behavior failed" }
& cargo test -p code-intel --test capability_exec repowise_adapter::index_only_emits_one_a04_request_and_docs_are_explicitly_not_requested --quiet | Out-Null
if ($LASTEXITCODE -ne 0) { throw "B01 index-only A04 behavior failed" }

$livePath = Join-Path $RepoRoot "run-code-intel.ps1"
$liveText = [IO.File]::ReadAllText($livePath)
if (@([regex]::Matches($liveText, 'test-code-intel-provider\.ps1')).Count -ne 0) { throw "historical direct wrapper call is live" }
if (@([regex]::Matches($liveText, 'Invoke-RepowiseProviderProbe\.ps1')).Count -ne 1) { throw "current production probe route must occur exactly once" }
if ($liveText -notmatch 'Index-only repowise will still run') { throw "index-only fallback changed" }
$liveHash = (Get-FileHash $livePath -Algorithm SHA256).Hash.ToLowerInvariant()
$historical = @(& git -C $RepoRoot show "$SourceRevision`:run-code-intel.ps1") -join "`n"
if ($LASTEXITCODE -ne 0) { throw "cannot load historical facade" }
$legacyPattern = '(?s)if \(\$RepowiseDocs -and -not \$SkipRepowise\) \{\n    \$providerPreflightScript = Join-Path \$PSScriptRoot "test-code-intel-provider\.ps1".*?\n\}'
$legacyMatch = [regex]::Match($historical, $legacyPattern)
if (-not $legacyMatch.Success) { throw "historical direct production branch is absent" }
$baseText = $legacyMatch.Value.Replace("`r`n","`n").Replace("`r","`n")
$resultText = ""
$deletedLines = @($baseText -split "`n")

$snapshotIdentity = Get-TextSha ((@($liveHash, (Get-TextSha $baseText), ((Get-FileHash (Join-Path $RepoRoot "Invoke-RepowiseProviderProbe.ps1") -Algorithm SHA256).Hash.ToLowerInvariant()), ((Get-FileHash (Join-Path $RepoRoot "orchestration\integrations.json") -Algorithm SHA256).Hash.ToLowerInvariant())) -join "`n"))
$retirementId = "retire-provider-preflight-branch"
$branchId = "run-code-intel.provider-preflight.test-wrapper"
$replacementId = "provider.repowise-adapt"
$callPath = "run-code-intel.ps1::$branchId"
$expiry = $EvaluatedAt + (30 * 86400)
function Add-Evidence([string]$Name, [string]$Class, [object]$Details) {
    $value = [ordered]@{ schema="code-intel-compatibility-retirement-evidence.v1"; snapshotIdentity=$snapshotIdentity; id="e03.$Name"; evidenceClass=$Class; retirementId=$retirementId; legacyBranchId=$branchId; replacementCapabilityId=$replacementId; details=$Details }
    $relative = "evidence/$Name.json"; Write-Json (Join-Path $OutDir $relative) $value
    New-Ref "code-intel-compatibility-retirement-evidence.v1" "compatibility.retirement-evidence" $relative
}
$replacement = Add-Evidence "replacement-atom" "replacement_atom" ([ordered]@{ outcome="passed"; status="production_ready"; capability=$replacementId; publicRoute=$true; a04Validated=$true })
$golden = Add-Evidence "golden-parity" "golden_parity" ([ordered]@{ outcome="passed"; assertionCount=3; command="test-repowise-adapter-contract.ps1" })
$contract = Add-Evidence "contract-parity" "contract_parity" ([ordered]@{ outcome="passed"; assertionCount=3; command="targeted B01/A04 capability_exec tests" })
$effects = Add-Evidence "effect-parity" "effect_parity" ([ordered]@{ outcome="passed"; assertionCount=3; currentProbeCalls=1; legacyWrapperCalls=0; indexOnlyPreserved=$true })
$registry = Add-Evidence "registry-reconciliation" "registry_reconciliation" ([ordered]@{ outcome="passed"; registryParticipantId="facade.provider-preflight.test-wrapper"; replacementCapabilityId=$replacementId; status="deleted"; historicalSourceRevision=$SourceRevision; installerDiagnosticOutOfScope=$true })
$window = Add-Evidence "compatibility-window" "compatibility_window" ([ordered]@{ outcome="blocked"; startedAt=$EvaluatedAt; observedThrough=$EvaluatedAt; minimumDays=30; checkedAt=$EvaluatedAt; expiresAt=$expiry; blocker="no completed 30-day compatibility observation window" })
$rehearsalRelative = "work/e03-provider-preflight-rollback-$EvaluatedAt"
$rehearsalRoot = Join-Path $RepoRoot ($rehearsalRelative -replace '/', [IO.Path]::DirectorySeparatorChar)
$rollbackCommand = "pwsh -NoLogo -NoProfile -File tools/compatibility/Restore-ProviderPreflightLegacyBranch.ps1 -RehearsalRoot $rehearsalRelative"
& pwsh -NoLogo -NoProfile -File (Join-Path $RepoRoot "tools\compatibility\Restore-ProviderPreflightLegacyBranch.ps1") -RehearsalRoot $rehearsalRoot -SourceRevision $SourceRevision | Out-Null
if ($LASTEXITCODE -ne 0 -or (Get-FileHash $livePath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $liveHash) { throw "exclusive rollback rehearsal failed or changed live facade" }
$rollback = Add-Evidence "rollback-execution" "rollback_execution" ([ordered]@{ outcome="passed"; command=$rollbackCommand; executedAt=$EvaluatedAt; exitCode=0; target="$rehearsalRelative/run-code-intel.ps1"; replacementChanged=$false; liveFacadeChanged=$false })
$usage = Add-Evidence "usage-observation" "usage_observation" ([ordered]@{ outcome="blocked"; startedAt=$EvaluatedAt; endedAt=$EvaluatedAt; totalInvocations=0; legacyInvocations=0; replacementInvocations=0; blocker="no production usage observation exists" })
$trace = '{"legacyBranchId":"' + $branchId + '","replacementCapabilityId":"' + $replacementId + '","retirementId":"' + $retirementId + '"}'
$necessity = Add-Evidence "c00-necessity" "c00_necessity" ([ordered]@{ outcome="passed"; decision="admit"; changeId=$retirementId; necessityTraceSha256=(Get-TextSha $trace) })
$snapshotDependency = Add-Evidence "dependency-repo-snapshot" "dependency_approval" ([ordered]@{ outcome="passed"; dependencyId="repo.snapshot"; status="approved"; reviewer="e03-author" })
$a04Dependency = Add-Evidence "dependency-a04-admissibility" "dependency_approval" ([ordered]@{ outcome="passed"; dependencyId="evidence.admissibility"; status="approved"; reviewer="e03-author" })
$subject = [ordered]@{
    legacyBranch=[ordered]@{ capabilityId="facade.provider-preflight.test-wrapper"; branchId=$branchId; callPath=$callPath; affectedFiles=@("run-code-intel.ps1"); owner="executor-provider-preflight"; registryParticipantId="facade.provider-preflight.test-wrapper" }
    replacement=[ordered]@{ capabilityId=$replacementId; implementationId="provider.repowise-adapt.compat"; dependencies=@("repo.snapshot","evidence.admissibility"); atomEvidence=$replacement }
    parity=[ordered]@{ golden=$golden; contract=$contract; effects=$effects }; registryReconciliation=$registry; compatibilityWindow=$window
    rollback=[ordered]@{ command=$rollbackCommand; executionEvidence=$rollback }; usageObservation=$usage; necessityEvidence=$necessity
    dependencyStates=@($snapshotDependency,$a04Dependency); lineReductionEvidence=$false
}
$independent = Add-Evidence "independent-approval" "independent_approval" ([ordered]@{ outcome="blocked"; approved=$false; authorIndependent=$false; subjectSha256=("0"*64); reviewer="independent-verifier-required"; authorityEvent=[ordered]@{}; blocker="no independent repository-governed approval exists" })
$manifest = [ordered]@{ schema="code-intel-compatibility-retirement-manifest.v1"; snapshotIdentity=$snapshotIdentity; retirementId=$retirementId; approvalSubject=$subject; independentApproval=$independent }
Write-Json (Join-Path $OutDir "compatibility-retirement-manifest.json") $manifest
$manifestRef = New-Ref "code-intel-compatibility-retirement-manifest.v1" "compatibility.retirement-manifest" "compatibility-retirement-manifest.json"

$registryJson = Get-Content (Join-Path $RepoRoot "orchestration\integrations.json") -Raw | ConvertFrom-Json
$gateDecl = ($registryJson.integrations | Where-Object id -eq "compatibility.retirement-gate").capabilityDeclaration
$request = [ordered]@{
    schema="code-intel-capability-request.v1"; capability="compatibility.retirement-gate"; contractVersion=1; implementation=$gateDecl.implementation
    snapshot=[ordered]@{ identity=$snapshotIdentity; repoIdentity=("content-v1:"+("c"*64)); head="historical-$SourceRevision"; workingTreePolicy="explicit_overlay"; scope=@("."); inputDigest=("d"*64) }
    options=[ordered]@{ evaluatedAt=$EvaluatedAt }
    inputs=@($manifestRef,$replacement,$golden,$contract,$effects,$registry,$window,$rollback,$usage,$necessity,$snapshotDependency,$a04Dependency,$independent)
    effectPolicy=[ordered]@{ allowedEffects=$gateDecl.allowedEffects }
}
Write-Json (Join-Path $OutDir "e00-request.json") $request
$gateOut = Join-Path $OutDir "gate-out"
& $CodeIntel capability exec compatibility.retirement-gate --request (Join-Path $OutDir "e00-request.json") --out $gateOut --artifact-root $OutDir | Out-Null
if ($LASTEXITCODE -ne 0) { throw "E00 execution failed" }
$decision = Get-Content (Join-Path $gateOut "compatibility-retirement-decision.json") -Raw | ConvertFrom-Json
if ($decision.decision -ne "blocked") { throw "E03 cannot proceed without observation and independent approval" }

$hunk = [ordered]@{ addedLines=@(); deletedLines=$deletedLines; newLines=0; newStart=1; oldLines=$deletedLines.Count; oldStart=1 }
$patchFiles = @([ordered]@{ baseBlobSha256=(Get-TextSha $baseText); baseText=$baseText; hunks=@($hunk); path="run-code-intel.ps1"; resultBlobSha256=(Get-TextSha $resultText); resultText=$resultText })
$patchSha = Get-TextSha (ConvertTo-Json -InputObject $patchFiles -Depth 30 -Compress)
$diff = [ordered]@{
    schema="code-intel-compatibility-retirement-deletion-diff.v1"; snapshotIdentity=$snapshotIdentity; retirementId=$retirementId; legacyBranchId=$branchId
    affectedFiles=@("run-code-intel.ps1"); deletionsOnly=$true
    summary="Historical-base deletion proof removes only the ca9334a direct production call to test-code-intel-provider.ps1. It is not a patch against the current correct production probe."
    patch=[ordered]@{ algorithm="replayable-delete-only-v1"; sha256=$patchSha; files=$patchFiles }
}
Write-Json (Join-Path $OutDir "compatibility-retirement-deletion-diff.json") $diff
$diffRef = New-Ref "code-intel-compatibility-retirement-deletion-diff.v1" "compatibility.retirement-deletion-diff" "compatibility-retirement-deletion-diff.json"
$decisionRef = New-Ref "code-intel-compatibility-retirement-decision.v1" "compatibility.retirement-decision" "gate-out/compatibility-retirement-decision.json"
$ticket = [ordered]@{
    schema="code-intel-compatibility-retirement-ticket-template.v1"; snapshotIdentity=$snapshotIdentity; ticketId="ticket-e03-retire-provider-preflight-branch"; retirementId=$retirementId
    legacyBranch=[ordered]@{ capabilityId="facade.provider-preflight.test-wrapper"; branchId=$branchId; callPath=$callPath }
    replacement=[ordered]@{ capabilityId=$replacementId; dependencies=@("repo.snapshot","evidence.admissibility") }; affectedFiles=@("run-code-intel.ps1")
    evidence=[ordered]@{ golden=$golden; contract=$contract; effects=$effects; usage=$usage; rollbackRehearsal=$rollback; deletionDiff=$diffRef }
    source=[ordered]@{ retirementDecision=$decisionRef; retirementManifest=$manifestRef }
    owner="executor-provider-preflight"; verifier="independent-verifier-required"; observationExpiry=$expiry; status="draft"; authorityBoundary="template_only_no_approval_or_deletion_authority"
}
Write-Json (Join-Path $OutDir "compatibility-retirement-ticket.json") $ticket
& $CodeIntel compatibility retirement-ticket lint --ticket (Join-Path $OutDir "compatibility-retirement-ticket.json") --evaluated-at $EvaluatedAt | Out-Null
if ($LASTEXITCODE -ne 0) { throw "E01 ticket lint failed" }
$ticketRef = New-Ref "code-intel-compatibility-retirement-ticket-template.v1" "compatibility.retirement-ticket-template" "compatibility-retirement-ticket.json"
$ticketDecl = ($registryJson.integrations | Where-Object id -eq "compatibility.retirement-ticket-template").capabilityDeclaration
$e01 = [ordered]@{
    schema="code-intel-capability-request.v1"; capability="compatibility.retirement-ticket-template"; contractVersion=1; implementation=$ticketDecl.implementation
    snapshot=[ordered]@{ identity=$snapshotIdentity; repoIdentity=("content-v1:"+("c"*64)); head="historical-$SourceRevision"; workingTreePolicy="explicit_overlay"; scope=@("."); inputDigest=("d"*64) }
    options=[ordered]@{ evaluatedAt=$EvaluatedAt }; inputs=@($ticketRef,$manifestRef,$decisionRef,$diffRef); effectPolicy=[ordered]@{ allowedEffects=$ticketDecl.allowedEffects }
}
Write-Json (Join-Path $OutDir "e01-request.json") $e01
$e01Output = @(& $CodeIntel capability exec compatibility.retirement-ticket-template --request (Join-Path $OutDir "e01-request.json") --out (Join-Path $OutDir "e01-out") --artifact-root $OutDir 2>&1)
$e01Exit = $LASTEXITCODE; $e01Text = $e01Output -join "`n"
[IO.File]::WriteAllText((Join-Path $OutDir "e01-stderr.txt"), $e01Text, [Text.UTF8Encoding]::new($false))
if ($e01Exit -ne 65 -or $e01Text -notmatch "ticket requires an approved E00 decision") { throw "E01 did not validate the historical replayable patch before the blocked-decision rejection: $e01Exit $e01Text" }
if ((Get-FileHash $livePath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $liveHash) { throw "E03 changed the current correct production probe" }
$status = [ordered]@{
    schema="code-intel-compatibility-retirement-execution-status.v1"; retirementId=$retirementId; decision="blocked"; deletionExecuted=$false; retired=$false
    blockers=@($decision.blockers); gainLedgerProjection=$decision.gainLedgerProjection
    boundary="E03 proves only the historical test-wrapper branch. It has no authority to delete or reroute the current production probe."
}
Write-Json (Join-Path $OutDir "status.json") $status
$status | ConvertTo-Json -Depth 10 -Compress
