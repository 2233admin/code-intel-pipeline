#requires -Version 7.2
<#
.SYNOPSIS
    T1 parity observation harness (issue #46 / campaign #55).

.DESCRIPTION
    Runs the SAME repository snapshot through both the legacy PowerShell
    pipeline (run-code-intel.ps1) and the Rust CLI (code-intel run execute),
    then diffs a curated set of comparable artifacts: the run-manifest/report
    node-or-step set and verdicts, the hospital-report.json status/diagnosis
    fields, and the sentrux structural-evidence metrics.

    This is an OBSERVATION tool, not a gate. Divergence between the two
    paths is EXPECTED right now (that is the entire point of T1: freeze the
    PS1 contract and measure the gap before T2-T5 port it). The harness
    reports divergence precisely -- it does not paper over field-shape
    differences with a lossy "close enough" comparison, and it does not
    retry/skip a mismatch into a false pass.

    Exit code: 0 when every comparison point matched, 1 otherwise. The JSON
    verdict on stdout (and at -VerdictPath) is the authoritative result;
    the exit code exists only so CI can turn continue-on-error tracking on
    the process boundary.

.PARAMETER RepoPath
    Repository to scan. Defaults to the repo root two levels above this
    script (scripts/tests/../..).

.PARAMETER VerdictPath
    Where to write the JSON verdict file. Defaults to a temp file so
    repeated local runs never dirty the working tree; CI passes an explicit
    path under $env:RUNNER_TEMP for artifact upload.

.PARAMETER KeepArtifacts
    Do not delete the two scratch output trees on exit. Useful when a
    divergence needs to be inspected by hand.
#>
param(
    [string]$RepoPath = "",
    [string]$VerdictPath = "",
    [switch]$KeepArtifacts
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ---------------------------------------------------------------------------
# 0. Resolve paths
# ---------------------------------------------------------------------------

if ([string]::IsNullOrWhiteSpace($RepoPath)) {
    $RepoPath = Join-Path $PSScriptRoot "../.."
}
$repoRoot = [System.IO.Path]::GetFullPath($RepoPath)

if ([string]::IsNullOrWhiteSpace($VerdictPath)) {
    $VerdictPath = Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-parity-verdict-" + [guid]::NewGuid().ToString("N") + ".json")
}

$exeName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
$rustCli = Join-Path $repoRoot (Join-Path "target/release" $exeName)

$manifestPath = Join-Path $repoRoot "orchestration/integrations.json"
$ps1Script = Join-Path $repoRoot "archive/run-code-intel.ps1"

if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "Cannot find orchestration manifest: $manifestPath"
}
if (-not (Test-Path -LiteralPath $ps1Script -PathType Leaf)) {
    throw "Cannot find run-code-intel.ps1 at: $ps1Script"
}

# ---------------------------------------------------------------------------
# 1. Locate or build the release Rust CLI
# ---------------------------------------------------------------------------

if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
    Write-Host "Release binary not found at $rustCli -- building (cargo build -p code-intel --release --locked)..."
    Push-Location $repoRoot
    try {
        cargo build -p code-intel --release --locked
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build -p code-intel --release --locked failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
}
if (-not (Test-Path -LiteralPath $rustCli -PathType Leaf)) {
    throw "Release binary still missing after build: $rustCli"
}

# ---------------------------------------------------------------------------
# 2. Scratch roots (one per path, always fresh, always outside the repo tree)
# ---------------------------------------------------------------------------

$runToken = [guid]::NewGuid().ToString("N").Substring(0, 10)
$tempRoot = [System.IO.Path]::GetTempPath()

$rustOut = Join-Path $tempRoot "code-intel-parity-rust-out-$runToken"
$rustAuthority = Join-Path $tempRoot "code-intel-parity-rust-auth-$runToken"
$ps1ArtifactRoot = Join-Path $tempRoot "code-intel-parity-ps1-root-$runToken"

New-Item -ItemType Directory -Force -Path $rustAuthority | Out-Null
New-Item -ItemType Directory -Force -Path $ps1ArtifactRoot | Out-Null

function Remove-ParityScratch {
    if ($KeepArtifacts) {
        Write-Host "KeepArtifacts set -- leaving scratch trees:"
        Write-Host "  rust out:       $rustOut"
        Write-Host "  rust authority: $rustAuthority"
        Write-Host "  ps1 artifacts:  $ps1ArtifactRoot"
        return
    }
    foreach ($path in @($rustOut, $rustAuthority, $ps1ArtifactRoot)) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# ---------------------------------------------------------------------------
# 3. Run the RUST path
#
#    This mirrors ci.yml's "Authoritative self-scan (release gate parity)"
#    step exactly (same subcommand, same flags) so this harness observes
#    the identical invocation shape CI already treats as authoritative.
# ---------------------------------------------------------------------------

Write-Host "=== Running Rust path: code-intel run execute ==="
$rustStdout = & $rustCli run execute `
    --repo $repoRoot `
    --out $rustOut `
    --authority-root $rustAuthority `
    --final-name parity-rust `
    --manifest $manifestPath `
    --doctor-require-repowise false 2>&1
$rustExitCode = $LASTEXITCODE
Write-Host "Rust path exit code: $rustExitCode"

# ---------------------------------------------------------------------------
# 4. Run the PS1 path
#
#    Flags chosen from the param() block (see docs/ps1-exit/contract-inventory.md
#    for the full inventory) to approximate "local, no-external-providers run"
#    without depending on state this harness cannot guarantee exists:
#
#      -Mode normal          Default mode. NOT "lite": lite mode skips the
#                             entire Sentrux stage (run-code-intel.ps1:3773),
#                             which would make the sentrux-metrics comparison
#                             vacuous. NOT "full": full only appends "--full"
#                             to an advisory /understand command string
#                             (:3612-3614) and changes nothing this harness
#                             reads.
#      -ArtifactRoot <fresh>  Isolates output the same way -out does for the
#                             Rust side.
#      -SkipRepowise          Skips the whole Repowise stage (:3646). Repowise
#                             is not one of the 3 diffed artifact categories,
#                             and skipping avoids a dependency on the
#                             external repowise tool / mock-provider wiring.
#      -SkipOpenSpec          REQUIRED, not optional. When NOT skipped, the
#                             workflow-recommendation block (:3449-3453)
#                             unconditionally resolves $defaultRustCli =
#                             target/debug/<exe> (a DIFFERENT build than the
#                             target/release binary this harness builds) and
#                             THROWS if that debug binary is missing:
#                               if (-not (Test-Path -LiteralPath $rustCli...)) {
#                                   throw "Workflow recommendation capability
#                                   binary not found: $rustCli"
#                               }
#                             There is no graceful fallback on this call site
#                             (unlike the Sentrux DSM call site, which degrades
#                             to the PowerShell implementation). OpenSpec /
#                             workflow-recommendation is not one of the 3
#                             diffed artifact categories either, so skipping
#                             it costs nothing here.
#      -SkipSentruxGate        Skips baseline-regression comparison (:3824).
#                             The gate compares against a COMMITTED
#                             .sentrux/baseline.json snapshot; a scratch
#                             harness run legitimately drifts from that
#                             baseline and a gate failure there is not a
#                             PS1-vs-Rust divergence signal. -SkipSentruxCheck
#                             is deliberately NOT passed: the check step is
#                             what produces the metrics this harness compares
#                             against Rust's evidence.sentrux/sentrux-payload.json.
#      -SkipGitHubResearch     Passed for parity with the existing CI smoke
#                             invocation, but note (see contract-inventory.md,
#                             kill-candidate row): this flag is currently
#                             DEAD -- $githubResearch is unconditionally
#                             New-GitHubSolutionResearchNotApplicable()
#                             (:3919) regardless of this switch. Passing it
#                             is a no-op today; kept for readability and in
#                             case the flag is wired up again later.
#
#    Deliberately NOT passed:
#      -RequireUnderstandGraph  Left off (default false) so a missing
#                             .understand-anything/knowledge-graph.json
#                             (true for a scratch temp checkout) degrades to
#                             "manual_required" (:3633-3640, exit-eligible 0)
#                             instead of a hard failure.
#
#    run-code-intel.ps1 must run as a genuinely separate pwsh PROCESS, not be
#    dot-sourced or call-operator-invoked in this session: the script ends
#    with unconditional `exit 0` / `exit 1` statements (:4721,:4723) that
#    would terminate THIS harness's process if invoked in-process.
# ---------------------------------------------------------------------------

Write-Host "=== Running PS1 path: run-code-intel.ps1 ==="
$pwshExe = (Get-Process -Id $PID).Path
if ([string]::IsNullOrWhiteSpace($pwshExe) -or -not (Test-Path -LiteralPath $pwshExe -PathType Leaf)) {
    $pwshExe = "pwsh"
}
$ps1Stdout = & $pwshExe -NoProfile -File $ps1Script `
    -RepoPath $repoRoot `
    -Mode normal `
    -ArtifactRoot $ps1ArtifactRoot `
    -SkipRepowise `
    -SkipOpenSpec `
    -SkipSentruxGate `
    -SkipGitHubResearch 2>&1
$ps1ExitCode = $LASTEXITCODE
Write-Host "PS1 path exit code: $ps1ExitCode"

# The PS1 path writes to <ArtifactRoot>/<repoName>/<timestamp>/, then does an
# atomic rename off a ".staging-<nonce>" suffix on success. Rather than
# reconstruct $repoName/$timestamp formatting, find the produced report.json
# directly -- robust to both the naming scheme and to a failed run leaving
# only the staging directory behind.
$ps1ReportFile = Get-ChildItem -LiteralPath $ps1ArtifactRoot -Recurse -Filter "report.json" -File -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
$ps1RunDir = if ($null -ne $ps1ReportFile) { $ps1ReportFile.DirectoryName } else { $null }

# ---------------------------------------------------------------------------
# 5. Normalization helpers (volatile-field redaction)
#
#    Adapted from scripts/tests/test-parity-baseline.ps1's
#    ConvertTo-NormalizedCanonicalValue (the repo's existing precedent for
#    excluding timestamps/absolute paths from golden-fixture comparison).
#    Extended here with a hash/identity-field allowlist because the Rust
#    artifacts carry content-addressed identities (snapshotIdentity,
#    admissionIdentity, sourceRevision, runIdentity, sha256, ...) that are
#    expected to differ between two independently-executed scans and are not
#    themselves a meaningful parity signal at T1.
# ---------------------------------------------------------------------------

$script:TimeFieldNames = @(
    "generatedAt", "startedAt", "finishedAt", "completedAt", "observedAt", "publishedAt", "parsed_at"
)
$script:IdentityFieldNames = @(
    "snapshotIdentity", "consumedSnapshotIdentity", "admissionIdentity", "sourceRevision",
    "runIdentity", "sha256", "rollbackIdentity"
)
$script:DurationFieldNames = @(
    "durationMs", "duration_ms", "elapsedMs"
)
$script:MachineFieldNames = @(
    "machine", "machineName", "hostname", "host_name"
)

function ConvertTo-ParityNormalizedValue {
    param($Value, [string]$PropertyName = "", [string]$TempRootA = "", [string]$TempRootB = "")

    if ($null -eq $Value) { return $null }

    if ($Value -is [string]) {
        if ($PropertyName -in $script:TimeFieldNames) { return "<TIME>" }
        if ($PropertyName -in $script:IdentityFieldNames) { return "<IDENTITY>" }
        if ($PropertyName -in $script:MachineFieldNames) { return "<MACHINE>" }

        $normalized = $Value.Replace("\", "/")
        foreach ($root in @($TempRootA, $TempRootB)) {
            if ([string]::IsNullOrWhiteSpace($root)) { continue }
            $normalizedRoot = $root.Replace("\", "/").TrimEnd("/")
            if ($normalized.Equals($normalizedRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
                return "<TEMP_ROOT>"
            }
            if ($normalized.StartsWith($normalizedRoot + "/", [System.StringComparison]::OrdinalIgnoreCase)) {
                return "<TEMP_ROOT>" + $normalized.Substring($normalizedRoot.Length)
            }
        }
        return $Value
    }
    if ($PropertyName -in $script:DurationFieldNames) { return "<DURATION>" }
    if ($Value -is [System.Collections.IDictionary]) {
        $canonical = [ordered]@{}
        foreach ($key in @($Value.Keys | ForEach-Object { [string]$_ } | Sort-Object -CaseSensitive)) {
            $canonical[$key] = ConvertTo-ParityNormalizedValue $Value[$key] $key $TempRootA $TempRootB
        }
        return $canonical
    }
    if ($Value -is [System.Management.Automation.PSCustomObject]) {
        $canonical = [ordered]@{}
        foreach ($property in @($Value.PSObject.Properties | Sort-Object Name -CaseSensitive)) {
            $canonical[$property.Name] = ConvertTo-ParityNormalizedValue $property.Value $property.Name $TempRootA $TempRootB
        }
        return $canonical
    }
    if ($Value -is [string[]] -or ($Value -is [System.Collections.IEnumerable] -and -not ($Value -is [string]))) {
        $items = @($Value | ForEach-Object { ConvertTo-ParityNormalizedValue $_ "" $TempRootA $TempRootB })
        return , $items
    }
    return $Value
}

function Get-JsonPathValue {
    <# Safe multi-segment property navigation: returns $null instead of
       throwing when any segment is missing. Segments may address either
       object properties or, when a segment is an integer literal, array
       indices. #>
    param($Object, [string[]]$Segments)

    $current = $Object
    foreach ($segment in $Segments) {
        if ($null -eq $current) { return $null }
        if ($current -is [System.Collections.IDictionary]) {
            if (-not $current.Contains($segment)) { return $null }
            $current = $current[$segment]
            continue
        }
        if ($current -is [System.Management.Automation.PSCustomObject]) {
            $prop = $current.PSObject.Properties[$segment]
            if ($null -eq $prop) { return $null }
            $current = $prop.Value
            continue
        }
        if ($current -is [System.Collections.IEnumerable] -and -not ($current -is [string])) {
            $index = 0
            if ([int]::TryParse($segment, [ref]$index)) {
                $asArray = @($current)
                if ($index -lt 0 -or $index -ge $asArray.Count) { return $null }
                $current = $asArray[$index]
                continue
            }
        }
        return $null
    }
    return $current
}

function Read-JsonArtifact {
    param([string]$Path)
    if ([string]::IsNullOrWhiteSpace($Path) -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $null
    }
    try {
        return (Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -Depth 100)
    }
    catch {
        return $null
    }
}

function New-Comparison {
    param([string]$Label, $RustValue, $PsValue, [string]$Reason = "")
    $rustNorm = ConvertTo-ParityNormalizedValue $RustValue "" $rustOut $ps1RunDir
    $psNorm = ConvertTo-ParityNormalizedValue $PsValue "" $rustOut $ps1RunDir
    $rustJson = if ($null -eq $rustNorm) { "null" } else { ($rustNorm | ConvertTo-Json -Depth 20 -Compress) }
    $psJson = if ($null -eq $psNorm) { "null" } else { ($psNorm | ConvertTo-Json -Depth 20 -Compress) }
    [pscustomobject][ordered]@{
        label = $Label
        rust = $rustJson
        ps1 = $psJson
        matched = ($rustJson -ceq $psJson)
        reason = $Reason
    }
}

# ---------------------------------------------------------------------------
# 6. Load artifacts (tolerant of missing files -- a missing artifact is
#    itself a reportable divergence, not a harness crash)
# ---------------------------------------------------------------------------

$rustManifest = Read-JsonArtifact (Join-Path $rustOut "run-manifest.json")
$rustHospital = Read-JsonArtifact (Join-Path $rustOut "diagnosis.hospital/hospital-report.json")
$rustSentrux = Read-JsonArtifact (Join-Path $rustOut "evidence.sentrux/sentrux-payload.json")

$ps1Report = if ($null -ne $ps1RunDir) { Read-JsonArtifact (Join-Path $ps1RunDir "report.json") } else { $null }
$ps1Hospital = if ($null -ne $ps1RunDir) { Read-JsonArtifact (Join-Path $ps1RunDir "hospital-report.json") } else { $null }

$comparisons = [System.Collections.Generic.List[object]]::new()

# ---------------------------------------------------------------------------
# 7a. Node / step set
#
#    The two pipelines do not share a stage-identifier vocabulary (Rust: DAG
#    node ids like "evidence.sentrux"; PS1: free-text step names like
#    "sentrux check"). There is exactly one pairing this harness asserts with
#    direct evidence (both sides route through the same underlying `sentrux
#    check` invocation); everything else is reported as a one-sided set
#    member rather than guessed at, per the ticket's instruction to prefer
#    under-claiming over a wrong row.
# ---------------------------------------------------------------------------

$rustNodeNames = @()
if ($null -ne $rustManifest -and $null -ne $rustManifest.nodes) {
    $rustNodeNames = @($rustManifest.nodes.PSObject.Properties.Name | Sort-Object)
}
$ps1StepNames = @()
if ($null -ne $ps1Report -and $null -ne $ps1Report.steps) {
    $ps1StepNames = @($ps1Report.steps | ForEach-Object { [string]$_.name } | Sort-Object)
}

function Get-NormalizedRustNodeVerdict {
    param($Node)
    if ($null -eq $Node) { return $null }
    $status = [string]$Node.status
    switch ($status) {
        "succeeded" { return [string]$Node.verdict } # pass | unknown | not_applicable
        "domain_failed" { return "fail" }
        "process_failed" { return "fail-process" }
        "dependency_blocked" { return "blocked" }
        default { return $status } # pending | running
    }
}

function Get-NormalizedPs1StepVerdict {
    param($Step)
    if ($null -eq $Step) { return $null }
    switch ([string]$Step.status) {
        "passed" { return "pass" }
        "failed" { return "fail" }
        "skipped" { return "not_applicable" }
        default { return [string]$Step.status }
    }
}

$rustSentruxNode = if ($null -ne $rustManifest -and $null -ne $rustManifest.nodes) { $rustManifest.nodes."evidence.sentrux" } else { $null }
$ps1SentruxCheckStep = if ($ps1StepNames.Count -gt 0) { $ps1Report.steps | Where-Object { $_.name -eq "sentrux check" } | Select-Object -First 1 } else { $null }
$comparisons.Add((New-Comparison "nodes.evidence.sentrux_vs_step.sentrux-check.verdict" `
    (Get-NormalizedRustNodeVerdict $rustSentruxNode) `
    (Get-NormalizedPs1StepVerdict $ps1SentruxCheckStep) `
    "Only stage pairing with direct evidence that both sides observe the same underlying tool invocation (sentrux check)."))

$pairedRustNames = @("evidence.sentrux")
$pairedPs1Names = @("sentrux check")
foreach ($name in ($rustNodeNames | Where-Object { $_ -notin $pairedRustNames })) {
    $comparisons.Add((New-Comparison "nodes.rust-only.$name" $name $null "Rust DAG node with no evidenced PS1 step counterpart at T1; T2-T5 will establish the mapping."))
}
foreach ($name in ($ps1StepNames | Where-Object { $_ -notin $pairedPs1Names })) {
    $comparisons.Add((New-Comparison "nodes.ps1-only.$name" $null $name "PS1 step with no evidenced Rust node counterpart at T1; T2-T5 will establish the mapping."))
}

# ---------------------------------------------------------------------------
# 7b. hospital-report.json status / diagnosis
#
#    Both sides claim schema "code-intel-hospital.v1", but the top-level
#    shapes have already drifted (see contract-inventory.md): PS1 nests its
#    verdict under `triage` (status/primary_diagnosis/overall_score), Rust
#    nests it under `diagnosis` (impression/risk/findings) plus a top-level
#    `domainVerdict`. There is no lossless field-for-field mapping, so this
#    section compares the fields that ARE structurally shared verbatim
#    (schema, state_machine.current_state) and separately surfaces the two
#    verdict shapes side by side as an explicit, expected divergence rather
#    than silently picking one interpretation.
# ---------------------------------------------------------------------------

$comparisons.Add((New-Comparison "hospital.schema" (Get-JsonPathValue $rustHospital @("schema")) (Get-JsonPathValue $ps1Hospital @("schema"))))
$comparisons.Add((New-Comparison "hospital.state_machine.current_state" (Get-JsonPathValue $rustHospital @("state_machine", "current_state")) (Get-JsonPathValue $ps1Hospital @("state_machine", "current_state"))))
$comparisons.Add((New-Comparison "hospital.verdict_shape" `
    ([ordered]@{ domainVerdict = (Get-JsonPathValue $rustHospital @("domainVerdict")); risk = (Get-JsonPathValue $rustHospital @("diagnosis", "risk")); impression = (Get-JsonPathValue $rustHospital @("diagnosis", "impression")) }) `
    ([ordered]@{ status = (Get-JsonPathValue $ps1Hospital @("triage", "status")); primary_diagnosis = (Get-JsonPathValue $ps1Hospital @("triage", "primary_diagnosis")); overall_score = (Get-JsonPathValue $ps1Hospital @("triage", "overall_score")) }) `
    "No lossless field mapping exists yet between Rust's diagnosis/domainVerdict and PS1's triage block (see contract-inventory.md). Reported side by side, not forced to a shared vocabulary; a T2-T5 ticket must define the mapping before this can be a real pass/fail gate."))

# ---------------------------------------------------------------------------
# 7c. Sentrux structural-evidence metrics
#
#    Rust: evidence.sentrux/sentrux-payload.json -> data.structuralEvidence.rules[]
#    (one entry per rule kind, each with a verdict). PS1: the raw `sentrux
#    check` step captured in report.json's steps[] (status/output only --
#    PS1 does not re-parse the sentrux CLI's stdout into a per-rule verdict
#    array anywhere in run-code-intel.ps1; see contract-inventory.md gap
#    section). Compare what both sides actually expose: presence/pass-fail
#    of the check as a whole, plus the raw rule list from Rust for the
#    record (PS1 has no equivalent to diff it against, which is itself the
#    signal T1 is meant to capture).
# ---------------------------------------------------------------------------

$rustSentruxRules = Get-JsonPathValue $rustSentrux @("data", "structuralEvidence", "rules")
$comparisons.Add((New-Comparison "sentrux.rules_present" `
    ($null -ne $rustSentruxRules -and @($rustSentruxRules).Count -gt 0) `
    ($null -ne $ps1SentruxCheckStep) `
    "Rust exposes a structured per-rule verdict array; PS1 exposes only the check step's pass/fail + raw stdout. Field-shape gap, not a value mismatch -- tracked as a gap in contract-inventory.md, not fixed here."))
$comparisons.Add((New-Comparison "sentrux.check_verdict" `
    (Get-NormalizedRustNodeVerdict $rustSentruxNode) `
    (Get-NormalizedPs1StepVerdict $ps1SentruxCheckStep)))

# ---------------------------------------------------------------------------
# 8. Emit verdict
# ---------------------------------------------------------------------------

$diverged = @($comparisons | Where-Object { -not $_.matched })
$matchedCount = $comparisons.Count - $diverged.Count

$verdict = [ordered]@{
    schema = "code-intel-ps1-rust-parity-verdict.v1"
    ok = ($diverged.Count -eq 0)
    compared = $comparisons.Count
    matched = $matchedCount
    rustExitCode = $rustExitCode
    ps1ExitCode = $ps1ExitCode
    rustArtifactsFound = [ordered]@{
        runManifest = ($null -ne $rustManifest)
        hospitalReport = ($null -ne $rustHospital)
        sentruxPayload = ($null -ne $rustSentrux)
    }
    ps1ArtifactsFound = [ordered]@{
        runDir = ($null -ne $ps1RunDir)
        report = ($null -ne $ps1Report)
        hospitalReport = ($null -ne $ps1Hospital)
    }
    diverged = @($diverged | ForEach-Object {
        [ordered]@{
            label = $_.label
            rust = $_.rust
            ps1 = $_.ps1
            reason = $_.reason
        }
    })
}

$verdictJson = $verdict | ConvertTo-Json -Depth 20
Write-Host "=== Parity verdict ==="
Write-Output $verdictJson
Set-Content -LiteralPath $VerdictPath -Value $verdictJson -Encoding UTF8
Write-Host "Verdict written to: $VerdictPath"

if (-not $ps1RunDir) {
    Write-Host "--- PS1 stdout/stderr tail (no run directory was produced) ---"
    Write-Host (($ps1Stdout | Select-Object -Last 60 | ForEach-Object { $_.ToString() }) -join "`n")
}
if ($null -eq $rustManifest) {
    Write-Host "--- Rust stdout/stderr tail (no run-manifest.json was produced) ---"
    Write-Host (($rustStdout | Select-Object -Last 60 | ForEach-Object { $_.ToString() }) -join "`n")
}

Remove-ParityScratch

if ($verdict.ok) {
    exit 0
}
else {
    exit 1
}
