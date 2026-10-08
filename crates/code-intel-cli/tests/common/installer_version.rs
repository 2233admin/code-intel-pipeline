//! Isolated installer compatibility scenarios using the real native version owner.

#[path = "../../src/env_contract.rs"]
mod env_contract;

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
    [Parameter(Mandatory = $true)][string]$Workspace,
    [Parameter(Mandatory = $true)][string]$NativeCli,
    [string]$OwnerRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = if ([string]::IsNullOrWhiteSpace($OwnerRoot)) { Join-Path $Workspace "owner" } else { $OwnerRoot }
if ([string]::IsNullOrWhiteSpace($OwnerRoot) -and $Scenario -ne "missing-owner") {
    $ownerBin = Join-Path $repoRoot "bin"
    New-Item -ItemType Directory -Force -Path $ownerBin | Out-Null
    $ownerExe = Join-Path $ownerBin $(if ($IsWindows) { "code-intel.exe" } else { "code-intel" })
    Copy-Item -LiteralPath $NativeCli -Destination $ownerExe
    if (-not $IsWindows) { & chmod +x $ownerExe }
}

$ast = [System.Management.Automation.Language.Parser]::ParseFile($Installer, [ref]$null, [ref]$null)
foreach ($name in @("Test-ToolVersionProbeAllowed", "Invoke-RepowiseVersionOwner", "Get-ToolVersion", "Install-MissingTool", "Add-VersionComplianceChecks")) {
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
    if ($Name -eq "code-intel") { return $null }
    if ($script:StubCommandSource) { return [pscustomobject]@{ Source = $script:StubCommandSource } }
    return $null
}

$at032 = New-VersionStub "repowise-032" "repowise, version 0.32.0"
$at036 = New-VersionStub "repowise-036" "repowise, version 0.36.0"
$at037 = New-VersionStub "repowise-037" "repowise, version 0.37.0"
$at038 = New-VersionStub "repowise-038" "repowise, version 0.38.0"
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
        Set-Content -LiteralPath $script -Encoding ascii -Value @('Write-Output "repowise, version 0.38.0"')
        Write-ProbeResult (Get-ToolVersion $script -ExpectedName "repowise")
        break
    }
    "parse-noise-before-version" {
        # A deprecation banner carrying its own version-shaped number must not
        # win the match when the tool name anchors the real line.
        $noisy = New-VersionStub "noisy" "DeprecationWarning from setuptools 3.11.0"
        Add-Content -LiteralPath $noisy -Value $(if ($IsWindows) { "echo repowise, version 0.38.0" } else { "echo 'repowise, version 0.38.0'" })
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
        $actions.Add([ordered]@{ name = "repowise"; status = "version_drift"; detail = "reports version 0.32.0; pinned version is 0.38.0"; fix = "f" })
        $actions.Add([ordered]@{ name = "mystery"; status = "version_drift"; detail = "reports version unknown; pinned version is 0.38.0"; fix = "f" })
        $actions.Add([ordered]@{ name = "rg"; status = "already_present"; detail = "/usr/bin/rg"; fix = "" })
        $actions.Add([ordered]@{ name = "fine"; status = "upgraded"; detail = "now 0.38.0, was 0.32.0"; fix = "" })

        Add-VersionComplianceChecks $checks $actions
        @{
            emitted  = @($checks | ForEach-Object { $_.name })
            required = @($checks | Where-Object { $_.required } | ForEach-Object { $_.name })
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
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.38.0" }
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
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.38.0" }
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
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "upgrade" {
        $InstallMissing = $true
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.38.0" }
        $script:StubCommandSource = $at032
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { $script:StubCommandSource = $at038 } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "upgrade-failed" {
        $InstallMissing = $true
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.38.0" }
        $script:StubCommandSource = $at032
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "pep-floor-rc" {
        $InstallMissing = $false
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.55.0" }
        $script:StubCommandSource = New-VersionStub "repowise-rc" "repowise, version 0.55.0rc1"
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { throw "read-only decision must not install" } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "pep-floor-post" {
        $InstallMissing = $true
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.55.0" }
        $script:StubCommandSource = New-VersionStub "repowise-post" "repowise, version 0.55.0.post1"
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { throw "post release must not be downgraded" } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "pep-upgrade-newer" {
        $InstallMissing = $true
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.38.0" }
        $script:StubCommandSource = $at032
        $newer = New-VersionStub "repowise-post-upgrade" "repowise, version 0.38.0.post1"
        Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { $script:StubCommandSource = $newer } "fix"
        $script:Recorded | ConvertTo-Json -Compress
        break
    }
    "missing-owner" {
        $InstallMissing = $true
        $script:StubMetadata = [ordered]@{ packageManager = "pip"; requiresElevation = $false; pinnedVersion = "0.55.0" }
        $script:StubCommandSource = $at032
        $script:InstallerCalled = $false
        $blocked = $false
        try {
            Install-MissingTool ([System.Collections.Generic.List[object]]::new()) "repowise" { $script:InstallerCalled = $true } "fix"
        } catch { $blocked = $true }
        @{ blocked = $blocked; installerCalled = $script:InstallerCalled } | ConvertTo-Json -Compress
        break
    }
    default { throw "unknown scenario: $Scenario" }
}
"##;

pub(super) fn scenario(tag: &str) -> Value {
    let installer = repo_root().join("legacy/install-code-intel-pipeline.ps1");
    scenario_from_installer(tag, &installer, None)
}

pub(super) fn scenario_from_installer(
    tag: &str,
    installer: &Path,
    owner_root: Option<&Path>,
) -> Value {
    let temp = Temp::new(tag);
    let driver = temp.0.join("driver.ps1");
    fs::write(&driver, DRIVER).expect("write driver");
    let mut command = Command::new("pwsh");
    for name in env_contract::PIPELINE_VARS {
        command.env_remove(name);
    }
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&driver)
        .arg("-Installer")
        .arg(installer)
        .arg("-Scenario")
        .arg(tag)
        .arg("-Workspace")
        .arg(&temp.0)
        .arg("-NativeCli")
        .arg(env!("CARGO_BIN_EXE_code-intel"));
    if let Some(root) = owner_root {
        command.arg("-OwnerRoot").arg(root);
    }
    let output = command.output().expect("run version gate driver");

    assert!(
        output.status.success(),
        "driver failed for scenario {tag}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|err| panic!("scenario {tag} emitted non-JSON ({err}): {stdout}"))
}
