param(
    [string]$RepoPath = "",
    [string]$Remote = "",
    [Parameter(Mandatory = $true)]
    [string]$ArtifactDir,
    [ValidateSet("xml", "markdown", "json", "plain")]
    [string]$Style = "markdown",
    [switch]$Compress,
    [switch]$Skip
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepomixOutputExtension {
    param([string]$Style)

    switch ($Style) {
        "xml" { return "xml" }
        "markdown" { return "md" }
        "json" { return "json" }
        "plain" { return "txt" }
        default { return "txt" }
    }
}

function Get-BoundedText {
    param([string]$Text, [int]$MaxLength = 1200)

    if ([string]::IsNullOrWhiteSpace($Text)) { return "" }
    $trimmed = $Text.Trim()
    if ($trimmed.Length -le $MaxLength) { return $trimmed }
    return $trimmed.Substring(0, $MaxLength)
}

New-Item -ItemType Directory -Force -Path $ArtifactDir | Out-Null
$extension = Get-RepomixOutputExtension $Style
$packPath = Join-Path $ArtifactDir ("repomix-output.{0}" -f $extension)
$summaryPath = Join-Path $ArtifactDir "repomix-summary.json"

$base = [ordered]@{
    schema = "code-intel-repomix-pack.v1"
    generatedAt = (Get-Date).ToString("o")
    status = "skipped"
    style = $Style
    path = $packPath
    summaryPath = $summaryPath
    repoPath = $RepoPath
    remote = $Remote
    compress = [bool]$Compress
    command = @()
    stdout = ""
    error = ""
}

if ($Skip) {
    $base["reason"] = "Skipped by -Skip."
    $base | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $summaryPath -Encoding UTF8
    return $base
}

$repomix = Get-Command repomix -ErrorAction SilentlyContinue
if (-not $repomix) {
    $base["reason"] = "repomix not found on PATH."
    $base | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $summaryPath -Encoding UTF8
    return $base
}

if ([string]::IsNullOrWhiteSpace($RepoPath) -and [string]::IsNullOrWhiteSpace($Remote)) {
    throw "Specify -RepoPath or -Remote."
}

$args = @()
if (-not [string]::IsNullOrWhiteSpace($Remote)) {
    $args += @("--remote", $Remote)
}
else {
    $args += $RepoPath
}
$args += @("-o", $packPath, "--style", $Style)
if ($Compress) {
    $args += "--compress"
}

$base["command"] = @("repomix") + $args
try {
    $global:LASTEXITCODE = 0
    $output = & repomix @args 2>&1
    $text = ($output | ForEach-Object { $_.ToString() } | Out-String).Trim()
    $base["stdout"] = Get-BoundedText $text
    if ($global:LASTEXITCODE -ne 0) {
        $base["status"] = "failed"
        $base["error"] = "repomix exited with code $global:LASTEXITCODE"
    }
    elseif (Test-Path -LiteralPath $packPath -PathType Leaf) {
        $packItem = Get-Item -LiteralPath $packPath
        $base["status"] = "ok"
        $base["bytes"] = $packItem.Length
    }
    else {
        $base["status"] = "failed"
        $base["error"] = "repomix completed but did not write $packPath"
    }
}
catch {
    $base["status"] = "failed"
    $base["error"] = $_.Exception.Message
}

$base | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $summaryPath -Encoding UTF8
return $base
