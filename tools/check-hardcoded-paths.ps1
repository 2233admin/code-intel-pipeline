#requires -Version 7.2

[CmdletBinding()]
param(
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $false

$root = Split-Path -Parent $PSScriptRoot
$slash = [string][char]92
$literalPatterns = @(
    ("C:" + $slash + "Users" + $slash + "Administrator"),
    ("power" + "shell" + ".exe"),
    ("LOCAL" + "APP" + "DATA"),
    ("USER" + "PRO" + "FILE"),
    ("APP" + "DATA")
)
$patternParts = @($literalPatterns | ForEach-Object { [regex]::Escape($_) })
$patternParts += "(?<![A-Za-z])[A-Za-z]:\\(?:[^\s`"'\\]*\\)*code-intel-pipeline\b"
$pattern = [regex]::new($patternParts -join "|")
$envVarPattern = [regex]::new("\`$env:[A-Za-z_][A-Za-z0-9_]*", [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)
$globs = @("*.ps1", "*.psm1", "*.md", "*.yml")

Push-Location $root
try {
    $files = @(& git ls-files -- $globs)
    if ($LASTEXITCODE -ne 0) {
        throw "git ls-files failed with exit code $LASTEXITCODE"
    }

    $hits = New-Object System.Collections.Generic.List[object]
    foreach ($file in $files) {
        if ([string]::IsNullOrWhiteSpace($file)) { continue }
        $lineNumber = 0
        foreach ($line in Get-Content -LiteralPath $file -ErrorAction Stop) {
            $lineNumber++
            $scanText = $envVarPattern.Replace($line, "")
            if ($pattern.IsMatch($scanText)) {
                $hits.Add([pscustomobject][ordered]@{
                    file = $file
                    line = $lineNumber
                    text = "$file`:$lineNumber`:$line"
                })
            }
        }
    }

    $result = [pscustomobject][ordered]@{
        ok = $hits.Count -eq 0
        scannedFiles = $files.Count
        hits = $hits
    }
}
finally {
    Pop-Location
}

if ($Json) {
    $result | ConvertTo-Json -Depth 6
}
else {
    if ($result.ok) {
        Write-Host "Hardcoded path scan: OK ($($result.scannedFiles) files)"
    }
    else {
        Write-Host "Hardcoded path scan: FAILED"
        $result.hits | ForEach-Object { Write-Host $_.text }
    }
}

if (-not $result.ok) { exit 1 }
exit 0
