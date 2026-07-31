#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))

$repoRoot = Split-Path -Parent $root
$binaryName = if ($IsWindows) { "code-intel.exe" } else { "code-intel" }
$rustCli = Join-Path $repoRoot "target/debug/$binaryName"
$temporaryRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$testRoot = Join-Path $temporaryRoot ("code-intel-wrapper-e2e-{0}-{1}" -f $PID, [guid]::NewGuid().ToString("N"))
$repo = Join-Path $testRoot "fixture-repo"
$artifactRoot = Join-Path $testRoot "artifacts"

function Invoke-StableWrapper {
    Push-Location $repo
    try {
        $output = @(& $rustCli --artifact-root $artifactRoot 2>&1)
        return [pscustomobject]@{
            ExitCode = $LASTEXITCODE
            Output = ($output -join [Environment]::NewLine)
        }
    }
    finally {
        Pop-Location
    }
}

try {
    New-Item -ItemType Directory -Path $repo -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $repo "assets") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $repo "src") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $repo ".sentrux") -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $repo "README.md") -Value "stable wrapper fixture" -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $repo "src/lib.rs") -Value "pub fn fixture() {}" -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $repo ".sentrux/rules.toml") -Value @"
[constraints]
max_cycles = 0
max_coupling = "F"
max_cc = 100
no_god_files = false
"@ -Encoding utf8NoBOM
    [System.IO.File]::WriteAllBytes((Join-Path $repo "assets/logo.png"), [byte[]](0x89, 0x50, 0x4e, 0x47, 0xff))
    # Provision the fixture baseline with the same built-in engine that the
    # authoritative run gates with; a PATH-resolved external Sentrux would
    # write a foreign baseline identity and trip the engine-mismatch check.
    & $rustCli sentrux --operation save_baseline --repo $repo | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "fixture Sentrux baseline creation failed" }
    & $rustCli sentrux --operation check --repo $repo | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "fixture Sentrux rules did not pass" }
    & git -C $repo init --quiet
    & git -C $repo add .
    & git -C $repo -c user.name=CodeIntelTest -c user.email=code-intel-test@example.invalid commit --quiet -m baseline
    if ($LASTEXITCODE -ne 0) { throw "fixture Git commit failed" }

    $success = Invoke-StableWrapper
    if ($success.ExitCode -ne 0) {
        $failureDetails = ""
        $authority = Join-Path $artifactRoot "fixture-repo"
        $failedRun = Get-ChildItem -LiteralPath $authority -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like "*-core" } |
            Sort-Object Name -Descending |
            Select-Object -First 1
        if ($null -ne $failedRun) {
            $failedMarkerPath = Join-Path $failedRun.FullName "run-complete.json"
            if (Test-Path -LiteralPath $failedMarkerPath -PathType Leaf) {
                $failedMarker = Get-Content -LiteralPath $failedMarkerPath -Raw | ConvertFrom-Json
                $failedManifestPath = Join-Path $failedRun.FullName ([string]$failedMarker.manifest.path)
                if (Test-Path -LiteralPath $failedManifestPath -PathType Leaf) {
                    $failedManifest = Get-Content -LiteralPath $failedManifestPath -Raw | ConvertFrom-Json
                    $failureDetails = "`nAuthoritative manifest:`n" + ($failedManifest | ConvertTo-Json -Depth 20)
                }
            }
        }
        throw "stable wrapper rejected a repository containing an unsupported binary file:`n$($success.Output)$failureDetails"
    }
    if ($success.Output -match "legacy compatibility pipeline") {
        throw "stable wrapper default route still executed the legacy pipeline"
    }
    if ($success.Output -notmatch "\[PASS\]" -or
        $success.Output -notmatch "Outcome:\s+completed" -or
        $success.Output -notmatch "Run evidence:") {
        throw "stable wrapper did not print the user-facing success summary:`n$($success.Output)"
    }
    $authority = Join-Path $artifactRoot "fixture-repo"
    $completedRun = Get-ChildItem -LiteralPath $authority -Directory |
        Where-Object { $_.Name -like "*-core" } |
        Sort-Object Name -Descending |
        Select-Object -First 1
    if ($null -eq $completedRun) { throw "stable wrapper did not publish an authoritative run" }
    $completedMarker = Get-Content -LiteralPath (Join-Path $completedRun.FullName "run-complete.json") -Raw | ConvertFrom-Json
    $completedManifest = Get-Content -LiteralPath (Join-Path $completedRun.FullName ([string]$completedMarker.manifest.path)) -Raw | ConvertFrom-Json
    if ([string]$completedManifest.outcome -ne "completed") {
        throw "expected completed authoritative run, got: $($completedManifest.outcome)"
    }
    foreach ($nodeId in @("evidence.graph", "evidence.sentrux", "diagnosis.hospital")) {
        if ([string]$completedManifest.nodes.$nodeId.status -ne "succeeded") {
            throw "authoritative default spine did not complete $nodeId"
        }
        if ([string]$completedManifest.nodes.$nodeId.verdict -ne "pass") {
            throw "authoritative default spine did not produce a pass domain verdict for $nodeId"
        }
    }
    $doctorArtifact = @($completedManifest.nodes.doctor.artifacts |
        Where-Object { [string]$_.type -eq "doctor.observation" } |
        Select-Object -First 1)
    if ($doctorArtifact.Count -ne 1) {
        throw "authoritative doctor observation was not published"
    }
    $doctorObservation = Get-Content -LiteralPath (Join-Path $completedRun.FullName ([string]$doctorArtifact[0].path)) -Raw | ConvertFrom-Json
    if ([bool]$doctorObservation.environmentPolicy.policy.requireRepowise) {
        throw "-SkipRepowise did not reach the authoritative doctor policy"
    }
    $completedIndex = Get-Content -LiteralPath (Join-Path $artifactRoot "index.json") -Raw | ConvertFrom-Json
    $completedEntry = @($completedIndex.entries | Where-Object { [string]$_.repo -eq "fixture-repo" })
    if ($completedEntry.Count -ne 1 -or [string]$completedEntry[0].run -ne $completedRun.Name -or [string]$completedEntry[0].outcome -ne "completed") {
        throw "completed authoritative run did not become the single A08 latest entry"
    }
    $payloadQuery = @(& $rustCli artifact query --artifact-root $artifactRoot --repo fixture-repo --repo-path $repo --type observed.evidence.payload 2>&1)
    if ($LASTEXITCODE -ne 0) { throw "direct provider payload query failed: $($payloadQuery -join [Environment]::NewLine)" }
    $payloadQueryJson = ($payloadQuery -join [Environment]::NewLine) | ConvertFrom-Json
    if ([string]$payloadQueryJson.freshness.status -ne "current" -or @($payloadQueryJson.matches).Count -lt 2) {
        throw "provider payload query did not close on the current completed run"
    }

    [System.IO.File]::WriteAllBytes((Join-Path $repo "broken.rs"), [byte[]](0xff, 0xfe, 0xfd))
    & git -C $repo add broken.rs
    & git -C $repo -c user.name=CodeIntelTest -c user.email=code-intel-test@example.invalid commit --quiet -m invalid-utf8-source
    if ($LASTEXITCODE -ne 0) { throw "fixture invalid-source Git commit failed" }

    $failure = Invoke-StableWrapper
    if ($failure.ExitCode -eq 0) {
        throw "stable wrapper hid an authoritative DAG failure:`n$($failure.Output)"
    }

    $latest = Get-ChildItem -LiteralPath $authority -Directory |
        Where-Object { $_.Name -like "*-core" } |
        Sort-Object Name -Descending |
        Select-Object -First 1
    if ($null -eq $latest) { throw "failed authoritative run was not retained for audit" }
    $marker = Get-Content -LiteralPath (Join-Path $latest.FullName "run-complete.json") -Raw | ConvertFrom-Json
    $manifestPath = Join-Path $latest.FullName ([string]$marker.manifest.path)
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ([string]$manifest.outcome -ne "process_failed") {
        throw "expected process_failed audit run, got: $($manifest.outcome)"
    }
    if ([string]$manifest.nodes.'evidence.native-code'.status -ne "process_failed") {
        throw "native-code failure was not preserved in the authoritative manifest"
    }
    $failedIndex = Get-Content -LiteralPath (Join-Path $artifactRoot "index.json") -Raw | ConvertFrom-Json
    $latestEntry = @($failedIndex.entries | Where-Object { [string]$_.repo -eq "fixture-repo" })
    if ($latestEntry.Count -ne 1 -or [string]$latestEntry[0].run -ne $completedRun.Name -or [string]$latestEntry[0].outcome -ne "completed") {
        throw "non-completed run replaced the last completed A08 authority"
    }
    $failedDiagnostic = @($failedIndex.diagnostics | Where-Object {
        [string]$_.repo -eq "fixture-repo" -and
        [string]$_.run -eq $latest.Name -and
        [string]$_.classification -eq "non_completed"
    })
    if ($failedDiagnostic.Count -ne 1 -or [string]$failedDiagnostic[0].reason -notmatch "process_failed") {
        throw "failed audit run was not classified outside the authoritative index"
    }
    if ($failure.Output -notmatch "\[FAIL\]" -or
        $failure.Output -notmatch "Outcome:\s+process_failed" -or
        $failure.Output -notmatch "Cause:\s+evidence\.native-code" -or
        $failure.Output -notmatch "Run evidence:") {
        throw "stable wrapper did not expose the authoritative failure in its user-facing summary:`n$($failure.Output)"
    }

    $missingRepo = Join-Path $testRoot "missing-repo"
    $invalidOutput = @(& $rustCli $missingRepo --artifact-root $artifactRoot 2>&1)
    if ($LASTEXITCODE -eq 0 -or
        ($invalidOutput -join [Environment]::NewLine) -notmatch "repository path is not a directory:" -or
        ($invalidOutput -join [Environment]::NewLine) -match "main\.rs:\d+") {
        throw "primary entry did not return a concise invalid-path error:`n$($invalidOutput -join [Environment]::NewLine)"
    }

    Write-Host "Stable wrapper E2E: OK"
}
finally {
    $resolved = [System.IO.Path]::GetFullPath($testRoot)
    $leaf = Split-Path -Leaf $resolved
    if ($env:CODE_INTEL_E2E_KEEP_TEMP -eq "1") {
        Write-Host "Stable wrapper E2E fixture retained: $resolved"
    }
    elseif ($resolved.StartsWith($temporaryRoot, [System.StringComparison]::OrdinalIgnoreCase) -and
        $leaf.StartsWith("code-intel-wrapper-e2e-", [System.StringComparison]::Ordinal) -and
        (Test-Path -LiteralPath $resolved -PathType Container)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}

# The second wrapper invocation is expected to fail so the audit path can be
# asserted. Do not let that intentionally non-zero native exit code become the
# successful test script's process exit code.
exit 0
