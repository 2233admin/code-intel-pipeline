[CmdletBinding()]
param(
    [string]$FixturePath = "",
    [switch]$SkipInstall
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
if ([string]::IsNullOrWhiteSpace($FixturePath)) {
    $FixturePath = Join-Path ([System.IO.Path]::GetTempPath()) ("sentrux-vlang-fixture-{0}" -f ([System.Guid]::NewGuid().ToString("N").Substring(0, 8)))
}

function Invoke-SentruxText {
    param([string[]]$Arguments)

    $oldPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & sentrux @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $oldPreference
    }

    return [pscustomobject][ordered]@{
        exitCode = $exitCode
        text = ($output | ForEach-Object { $_.ToString() } | Out-String).Trim()
    }
}

if (-not $SkipInstall) {
    & (Join-Path $root "Install-SentruxVlangOverlay.ps1") | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Install-SentruxVlangOverlay.ps1 failed"
    }
}

if (-not (Get-Command sentrux -ErrorAction SilentlyContinue)) {
    throw "sentrux CLI not found in PATH"
}

$pluginPath = Join-Path $env:USERPROFILE ".sentrux\plugins\vlang"
& sentrux plugin validate $pluginPath | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw "sentrux plugin validate failed for $pluginPath"
}

$pluginList = Invoke-SentruxText @("plugin", "list")
if ($pluginList.exitCode -ne 0 -or $pluginList.text -notmatch "vlang\s+v0\.2\.0\s+\[v\]") {
    throw "sentrux plugin list did not show vlang v0.2.0 [v]"
}

New-Item -ItemType Directory -Force -Path (Join-Path $FixturePath ".sentrux") | Out-Null

@'
module main

import os
import helper

struct User {
	name string
}

interface Greeter {
	greet() string
}

enum Status {
	ok
	failed
}

fn greet_user(user User) string {
	return 'hello ${user.name}'
}

fn main() {
	user := User{
		name: os.getenv('USER')
	}
	helper.audit_user(user.name)
	println(greet_user(user))
}
'@ | Set-Content -LiteralPath (Join-Path $FixturePath "main.v") -NoNewline

@'
module helper

pub fn audit_user(name string) {
	println('audit ${name}')
}
'@ | Set-Content -LiteralPath (Join-Path $FixturePath "helper.v") -NoNewline

@'
[constraints]
max_cycles = 0
max_coupling = "B"
max_cc = 25
no_god_files = true
'@ | Set-Content -LiteralPath (Join-Path $FixturePath ".sentrux\rules.toml") -NoNewline

$check = Invoke-SentruxText @("check", $FixturePath)
$checkText = $check.text
if ($check.exitCode -ne 0) {
    throw "sentrux check failed: $checkText"
}
if ($checkText -notmatch "2 files" -or $checkText -notmatch "1 import, 1 call") {
    throw "sentrux check did not build the expected V structure graph: $checkText"
}

$save = Invoke-SentruxText @("gate", "--save", $FixturePath)
$saveText = $save.text
if ($save.exitCode -ne 0) {
    throw "sentrux gate --save failed: $saveText"
}

$gate = Invoke-SentruxText @("gate", $FixturePath)
$gateText = $gate.text
if ($gate.exitCode -ne 0 -or $gateText -notmatch "No degradation detected") {
    throw "sentrux gate failed: $gateText"
}

[pscustomobject][ordered]@{
    ok = $true
    fixture = $FixturePath
    plugin = $pluginPath
    checkGraph = "2 files, 1 import, 1 call"
    quality = if ($gateText -match "Quality:\s+([0-9]+)\s+->\s+([0-9]+)") { $Matches[2] } else { "" }
} | ConvertTo-Json -Depth 4
