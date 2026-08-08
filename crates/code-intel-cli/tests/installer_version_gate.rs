//! Contract tests for the installer's version gate.
//!
//! Before this gate existed, `Install-MissingTool` returned on presence alone:
//! a machine with `repowise` 0.32.0 on PATH reported `already_present` while
//! the supply-chain-003 pin declared 0.36.0, so the pin was a declaration
//! nothing enforced. Presence and correctness were indistinguishable in the
//! install report.
//!
//! The functions under test live inside `legacy/install-code-intel-pipeline.ps1`,
//! whose top level performs a real installation — dot-sourcing it would install.
//! The driver written here lifts the two functions out by AST and evaluates
//! them with their collaborators stubbed. Per AGENTS.md the driver is generated
//! into a temp directory rather than committed, so this adds no new PowerShell
//! to the tree; assertions live on the Rust side, matching the #78/#80
//! direction of porting PowerShell call points to Rust.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root")
}

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "code-intel-version-gate-{tag}-{}-{nonce}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("scratch");
        Self(dir)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Lifts `Get-ToolVersion` and `Install-MissingTool` out of the installer by
/// AST, stubs their collaborators, runs one scenario, and prints one JSON
/// document. Kept in a temp file rather than the tree: AGENTS.md forbids
/// adding PowerShell scripts to the repository.
///
/// Delimited with `r##"` rather than `r#"`: the POSIX stub writes a `"#!/bin/sh"`
/// line, and the `"#` inside it would otherwise close the raw string.
const DRIVER: &str = r##"
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$Scenario,
    [Parameter(Mandatory = $true)][string]$Workspace
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ast = [System.Management.Automation.Language.Parser]::ParseFile($Installer, [ref]$null, [ref]$null)
foreach ($name in @("Test-ToolVersionProbeAllowed", "Get-ToolVersion", "Install-MissingTool", "Add-VersionComplianceChecks")) {
    $fn = $ast.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
        }, $true)
    if (-not $fn) { throw "function not found in installer: $name" }
    . ([scriptblock]::Create($fn.Extent.Text))
}

function New-VersionStub {
    # Platform-correct stubs. `cross-platform-smoke` runs `cargo test -p
    # code-intel --locked` on macos-latest and ubuntu-latest, where a `.cmd`
    # batch file is not executable — `& $Source` would throw, Get-ToolVersion
    # would swallow it as "unknown", and every scenario below would fail for a
    # reason unrelated to the gate.
    param([string]$Tag, [string]$Output)
    if ($IsWindows) {
        $path = Join-Path $Workspace "$Tag.cmd"
        Set-Content -LiteralPath $path -Encoding ascii -Value @("@echo off", "echo $Output")
    }
    else {
        $path = Join-Path $Workspace $Tag
        Set-Content -LiteralPath $path -Encoding ascii -Value @("#!/bin/sh", "echo '$Output'")
        & chmod +x $path
    }
    return $path
}

function Get-MissingToolPath {
    param()
    if ($IsWindows) { return (Join-Path $Workspace "does-not-exist.cmd") }
    return (Join-Path $Workspace "does-not-exist")
}

function Write-ProbeResult {
    # $null means the probe was REFUSED (never executed); "" means it RAN and
    # produced no readable version. Collapsing the two would let an
    # unverifiable source read as drift and induce a reinstall.
    param($Value)
    @{
        refused = ($null -eq $Value)
        parsed  = if ($null -eq $Value) { "" } else { [string]$Value }
    } | ConvertTo-Json -Compress
}

$script:Recorded = $null
$script:StubMetadata = $null
$script:StubCommandSource = $null

function Get-InstallMetadata { param([string]$CommandName) return $script:StubMetadata }
function Get-CodeIntelPythonCommand { return $null }

function Add-InstallAction {
    param(
        $Actions, [string]$Name, [string]$Status, [string]$Detail = "",
        [string]$Fix = "", [string]$PackageManager = "", [bool]$RequiresElevation = $false
    )
    $script:Recorded = [ordered]@{ name = $Name; status = $Status; detail = $Detail; fix = $Fix }
}

function Get-Command {
    # Remaining-args sink so the caller's `-ErrorAction SilentlyContinue` binds
    # here instead of colliding with the common parameter.
    param(
        [Parameter(Position = 0)][string]$Name,
        [Parameter(ValueFromRemainingArguments = $true)]$Rest
    )
    if ($script:StubCommandSource) { return [pscustomobject]@{ Source = $script:StubCommandSource } }
    return $null
}

$at032 = New-VersionStub "repowise-032" "repowise, version 0.32.0"
$at036 = New-VersionStub "repowise-036" "repowise, version 0.36.0"
$at037 = New-VersionStub "repowise-037" "repowise, version 0.37.0"
$prerelease = New-VersionStub "code-intel" "code-intel 0.7.0-beta.2"
$silent = New-VersionStub "silent" "no version here"

switch ($Scenario) {
    "parse-standard" { Write-ProbeResult (Get-ToolVersion $at032 -ExpectedName "repowise"); break }
    "parse-prerelease" { Write-ProbeResult (Get-ToolVersion $prerelease -ExpectedName "code-intel"); break }
    "parse-unparseable" { Write-ProbeResult (Get-ToolVersion $silent); break }
    "parse-empty-source" { Write-ProbeResult (Get-ToolVersion ""); break }
    "parse-missing-tool" { Write-ProbeResult (Get-ToolVersion (Get-MissingToolPath)); break }
    "parse-relative-source" {
        # A bare/relative name must never be executed: PowerShell would resolve
        # it against the current directory, which is the repository under
        # analysis.
        Write-ProbeResult (Get-ToolVersion "repowise")
        break
    }
    "parse-script-source" {
        # The load-bearing security case. A `.ps1` on PATH resolves to an
        # ExternalScriptInfo whose Source is the script path; `& $Source` would
        # run it INSIDE the installer process.
        $script = Join-Path $Workspace "repowise.ps1"
        Set-Content -LiteralPath $script -Encoding ascii -Value @('Write-Output "repowise, version 0.36.0"')
        Write-ProbeResult (Get-ToolVersion $script -ExpectedName "repowise")
        break
    }
    "parse-noise-before-version" {
        # A deprecation banner carrying its own version-shaped number must not
        # win the match when the tool name anchors the real line.
        $noisy = New-VersionStub "noisy" "DeprecationWarning from setuptools 3.11.0"
        Add-Content -LiteralPath $noisy -Value $(if ($IsWindows) { "echo repowise, version 0.36.0" } else { "echo 'repowise, version 0.36.0'" })
        Write-ProbeResult (Get-ToolVersion $noisy -ExpectedName "repowise")
        break
    }
    "compliance-checks" {
        # `ok` is computed from $checks only. This asserts drift actually
        # reaches that computation, and that an unmeasurable version does not
        # fail the install.
        $checks = [System.Collections.Generic.List[object]]::new()
        function Add-Check {
            param($Checks, [string]$Name, [string]$Category, [bool]$Required, [bool]$Ok, [string]$Detail = "", [string]$Fix = "")
            $Checks.Add([ordered]@{ name = $Name; category = $Category; required = $Required; ok = $Ok })
        }
        $actions = [System.Collections.Generic.List[object]]::new()
        $actions.Add([ordered]@{ name = "repowise"; status = "version_drift"; detail = "reports version 0.32.0; pinned version is 0.36.0"; fix = "f" })
        $actions.Add([ordered]@{ name = "mystery"; status = "version_drift"; detail = "reports version unknown; pinned version is 0.36.0"; fix = "f" })
        $actions.Add([ordered]@{ name = "rg"; status = "already_present"; detail = "/usr/bin/rg"; fix = "" })
        $actions.Add([ordered]@{ name = "fine"; status = "upgraded"; detail = "now 0.36.0, was 0.32.0"; fix = "" })

        Add-VersionComplianceChecks $checks $actions
        @{
            emitted  = @($checks | ForEach-Object { $_.name })
            required = @($checks | Where-Object { $_.required } | ForEach-Object { $_.name })
        } | ConvertTo-Json -Compress
        break
    }
    "wiring" {
        # Everything above stubs Get-InstallMetadata, so nothing so far proves
        # the REAL switch actually carries the pin. Without this scenario,
        # deleting `pinnedVersion = $script:RepowisePinnedVersion` from the
        # installer restores the presence-only bug and every other test still
        # passes.
        $installerText = Get-Content -Raw -LiteralPath $Installer
        $pinMatch = [regex]::Match($installerText, '\$script:RepowisePinnedVersion\s*=\s*"([^"]+)"')
        if (-not $pinMatch.Success) { throw "RepowisePinnedVersion assignment not found in installer" }
        $script:RepowisePinnedVersion = $pinMatch.Groups[1].Value
        $script:EffectivePlatform = if ($IsWindows) { "windows" } elseif ($IsMacOS) { "macos" } else { "linux" }

        $realMetadata = $ast.Find({
                param($node)
                $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq "Get-InstallMetadata"
            }, $true)
        if (-not $realMetadata) { throw "Get-InstallMetadata not found in installer" }
        . ([scriptblock]::Create($realMetadata.Extent.Text))

        $repowise = Get-InstallMetadata "repowise"
        $rg = Get-InstallMetadata "rg"
        # Report a sentinel rather than dereferencing a missing key: under
        # StrictMode that would surface as a property-access exception, which
        # reads as a broken test rather than as the regression it is.
        @{
            pinLiteral     = $script:RepowisePinnedVersion
            repowisePinned = if ($repowise.Contains("pinnedVersion")) { [string]$repowise.pinnedVersion } else { "<no pinnedVersion key>" }
            rgHasPin       = [bool]$rg.Contains("pinnedVersion")
        } | ConvertTo-Json -Compress
        break
    }
    "match" {
        $InstallMissing = $false
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.32.0" }
        $script:StubCommandSource = $at032
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { throw "installer must not run when the version matches" } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "drift" {
        $InstallMissing = $false
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.36.0" }
        $script:StubCommandSource = $at032
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { throw "installer must not run without -InstallMissing" } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "newer" {
        $InstallMissing = $true
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.36.0" }
        $script:StubCommandSource = $at037
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { throw "installer must not downgrade a tool newer than the pin" } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "drift-unknown" {
        $InstallMissing = $false
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.36.0" }
        $script:StubCommandSource = $silent
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { throw "installer must not run without -InstallMissing" } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "unpinned" {
        $InstallMissing = $false
        $script:StubMetadata = [ordered]@{ packageManager = "winget"; requiresElevation = $false }
        $script:StubCommandSource = $at032
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "rg" { throw "installer must not run for a present unpinned tool" } "fix"
        $result = $script:Recorded
        $result["expectedDetail"] = $at032
        $result | ConvertTo-Json -Compress
        break
    }
    "upgrade" {
        $InstallMissing = $true
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.36.0" }
        $script:StubCommandSource = $at032
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { $script:StubCommandSource = $at036 } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "upgrade-failed" {
        $InstallMissing = $true
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.36.0" }
        $script:StubCommandSource = $at032
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    default { throw "unknown scenario: $Scenario" }
}
"##;

fn scenario(tag: &str) -> Value {
    let temp = Temp::new(tag);
    let driver = temp.0.join("driver.ps1");
    fs::write(&driver, DRIVER).expect("write driver");

    let installer = repo_root().join("legacy/install-code-intel-pipeline.ps1");
    let output = Command::new("pwsh")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&driver)
        .arg("-Installer")
        .arg(&installer)
        .arg("-Scenario")
        .arg(tag)
        .arg("-Workspace")
        .arg(&temp.0)
        .output()
        .expect("run version gate driver");

    assert!(
        output.status.success(),
        "driver failed for scenario {tag}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|err| panic!("scenario {tag} emitted non-JSON ({err}): {stdout}"))
}

#[test]
fn tool_version_parses_the_formats_the_gate_actually_meets() {
    let standard = scenario("parse-standard");
    assert_eq!(standard["refused"], false);
    assert_eq!(
        standard["parsed"], "0.32.0",
        "`repowise, version X` is what the pinned tool prints"
    );

    let prerelease = scenario("parse-prerelease");
    assert_eq!(prerelease["refused"], false);
    assert_eq!(
        prerelease["parsed"], "0.7.0-beta.2",
        "a prerelease suffix must survive whole, or every beta reads as drift"
    );
}

#[test]
fn the_probe_refuses_sources_the_project_forbids_launching() {
    // tool_path.rs states the rule for every tool launch in this project:
    // "only ever launches by absolute path", "relative PATH entries are
    // skipped outright". A `.ps1` is worse than merely relative — `& $Source`
    // would execute it inside the installer's own process.
    for tag in [
        "parse-empty-source",
        "parse-missing-tool",
        "parse-relative-source",
        "parse-script-source",
    ] {
        assert_eq!(
            scenario(tag)["refused"],
            true,
            "{tag} must be refused, not executed"
        );
    }
}

#[test]
fn a_version_shaped_number_in_a_banner_does_not_win_the_match() {
    let result = scenario("parse-noise-before-version");
    assert_eq!(result["refused"], false);
    assert_eq!(
        result["parsed"], "0.36.0",
        "the name-anchored line wins over `setuptools 3.11.0` noise"
    );
}

#[test]
fn a_probe_that_ran_but_read_nothing_is_unknown_not_a_match() {
    // The gate exists to surface exactly this state; treating it as a match
    // would restore the presence-only behaviour it replaces.
    for tag in ["parse-unparseable"] {
        assert_eq!(
            scenario(tag)["refused"],
            false,
            "{tag} is executable; it ran"
        );
        assert_eq!(
            scenario(tag)["parsed"],
            "",
            "{tag} must read as unknown, not as a version"
        );
    }
}

#[test]
fn a_matching_pinned_version_stays_already_present() {
    let result = scenario("match");
    assert_eq!(result["status"], "already_present");
    assert!(
        result["detail"]
            .as_str()
            .unwrap()
            .contains("version 0.32.0"),
        "the matching case reports what it observed: {}",
        result["detail"]
    );
}

#[test]
fn newer_than_pin_stays_already_present_and_is_never_downgraded() {
    // The pin is a floor, not an exact target. A user who upgraded past the
    // pin must not be reported as drifted, and -InstallMissing must not
    // downgrade them back to the pin on every rerun — the scenario's
    // installer block throws if invoked.
    let result = scenario("newer");
    assert_eq!(result["status"], "already_present");
    let detail = result["detail"].as_str().unwrap();
    assert!(
        detail.contains("0.37.0") && detail.contains("newer than pin 0.36.0"),
        "names both the observed version and the floor: {detail}"
    );
}

#[test]
fn drift_is_reported_even_when_the_gate_may_not_fix_it() {
    // Without -InstallMissing the installer must not touch the system, but
    // staying silent is the failure this branch exists to prevent: a
    // present-but-wrong version previously read as already_present.
    let result = scenario("drift");
    assert_eq!(result["status"], "version_drift");
    let detail = result["detail"].as_str().unwrap();
    assert!(
        detail.contains("0.32.0"),
        "names the observed version: {detail}"
    );
    assert!(
        detail.contains("0.36.0"),
        "names the pinned version: {detail}"
    );
    assert!(
        result["fix"].as_str().unwrap().contains("-InstallMissing"),
        "tells the operator how to resolve it"
    );
}

#[test]
fn an_unreadable_version_reports_drift_rather_than_passing() {
    let result = scenario("drift-unknown");
    assert_eq!(result["status"], "version_drift");
    assert!(
        result["detail"].as_str().unwrap().contains("unknown"),
        "got: {}",
        result["detail"]
    );
}

#[test]
fn tools_without_a_pin_keep_their_previous_behaviour() {
    let result = scenario("unpinned");
    assert_eq!(result["status"], "already_present");
    assert_eq!(
        result["detail"], result["expectedDetail"],
        "an unpinned tool's detail stays the bare source path, with no version probe appended"
    );
}

#[test]
fn install_missing_upgrades_a_drifted_tool_to_the_pin() {
    let result = scenario("upgrade");
    assert_eq!(result["status"], "upgraded");
    assert!(
        result["detail"].as_str().unwrap().contains("was 0.32.0"),
        "an upgrade records what it replaced: {}",
        result["detail"]
    );
}

#[test]
fn a_reinstall_that_does_not_reach_the_pin_fails_loudly() {
    assert_eq!(scenario("upgrade-failed")["status"], "upgrade_failed");
}

#[test]
fn confirmed_drift_reaches_the_ok_computation_but_uncertainty_does_not() {
    // The installer's `ok` is derived from `$checks`, never from
    // `$installActions`, and bootstrap-new-machine.ps1 reads only
    // `installResult.ok`. Drift that stays in installActions is invisible to
    // every consumer.
    let result = scenario("compliance-checks");

    let emitted: Vec<&str> = result["emitted"]
        .as_array()
        .expect("emitted")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        emitted,
        vec!["version:repowise", "version:mystery"],
        "only drift and upgrade_failed become checks; already_present and upgraded do not"
    );

    let required: Vec<&str> = result["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        required,
        vec!["version:repowise"],
        "a measured mismatch fails the install; an unreadable version is uncertainty and must not"
    );
}

#[test]
fn the_real_metadata_switch_carries_the_pin() {
    // Every scenario above stubs Get-InstallMetadata, so none of them prove the
    // production switch is wired. Deleting `pinnedVersion =
    // $script:RepowisePinnedVersion` from the installer restores the exact
    // presence-only bug this change fixes; without this test that deletion is
    // invisible.
    let result = scenario("wiring");

    assert_eq!(
        result["repowisePinned"], result["pinLiteral"],
        "the repowise metadata entry must carry the supply-chain-003 pin, not a literal that drifted from it"
    );
    assert_eq!(
        result["repowisePinned"], "0.36.0",
        "if the pin moves, this assertion is the deliberate place to notice"
    );
    assert_eq!(
        result["rgHasPin"], false,
        "unpinned tools must not gain a pinnedVersion key, or they acquire a version probe they never had"
    );
}
