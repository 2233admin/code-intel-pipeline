param(
    [string[]]$Root = @("."),
    [string]$WizTreeCsv = "",
    [string]$WizTreeExe = "",
    [string]$ExportCsv = "",
    [int]$MaxDepth = 5,
    [int]$MinScore = 25,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$signalSpecs = @(
    [ordered]@{ name = ".git"; type = "directory"; score = 60 },
    [ordered]@{ name = "Cargo.toml"; type = "file"; score = 25 },
    [ordered]@{ name = "package.json"; type = "file"; score = 25 },
    [ordered]@{ name = "pyproject.toml"; type = "file"; score = 25 },
    [ordered]@{ name = "go.mod"; type = "file"; score = 25 },
    [ordered]@{ name = "pom.xml"; type = "file"; score = 20 },
    [ordered]@{ name = "build.gradle"; type = "file"; score = 20 },
    [ordered]@{ name = "pnpm-workspace.yaml"; type = "file"; score = 20 },
    [ordered]@{ name = "README.md"; type = "file"; score = 5 }
)

$signalScores = @{}
foreach ($spec in $signalSpecs) {
    $signalScores[[string]$spec.name] = [int]$spec.score
}

function Convert-ToInt64 {
    param([object]$Value)

    if ($null -eq $Value) { return 0 }
    $text = ([string]$Value).Trim() -replace ",", ""
    if ([string]::IsNullOrWhiteSpace($text)) { return 0 }
    $parsed = 0L
    if ([long]::TryParse($text, [ref]$parsed)) { return $parsed }
    return 0
}

function Get-PropertyValue {
    param(
        [object]$Object,
        [string[]]$Names
    )

    foreach ($name in $Names) {
        $prop = $Object.PSObject.Properties[$name]
        if ($null -ne $prop -and -not [string]::IsNullOrWhiteSpace([string]$prop.Value)) {
            return [string]$prop.Value
        }
    }

    return ""
}

function Add-Candidate {
    param(
        [hashtable]$Candidates,
        [string]$Path,
        [string]$Signal,
        [long]$SizeBytes,
        [string]$Source,
        [datetime]$LastWriteTime
    )

    if ([string]::IsNullOrWhiteSpace($Path)) { return }
    $fullPath = [System.IO.Path]::GetFullPath($Path)

    if (-not $Candidates.ContainsKey($fullPath)) {
        $Candidates[$fullPath] = [ordered]@{
            path = $fullPath
            score = 0
            signals = New-Object System.Collections.Generic.List[string]
            sizeBytes = 0L
            lastWriteTime = $null
            source = $Source
        }
    }

    $candidate = $Candidates[$fullPath]
    if (-not $candidate.signals.Contains($Signal)) {
        $candidate.signals.Add($Signal)
        $candidate.score = [int]$candidate.score + [int]$signalScores[$Signal]
    }

    if ($SizeBytes -gt [long]$candidate.sizeBytes) {
        $candidate.sizeBytes = $SizeBytes
    }
    if ($null -ne $LastWriteTime -and ($null -eq $candidate.lastWriteTime -or $LastWriteTime -gt $candidate.lastWriteTime)) {
        $candidate.lastWriteTime = $LastWriteTime
    }
}

function Invoke-WizTreeExport {
    param(
        [string]$Exe,
        [string]$ScanRoot,
        [string]$CsvPath
    )

    if ([string]::IsNullOrWhiteSpace($Exe)) {
        $cmd = Get-Command "WizTree64.exe" -ErrorAction SilentlyContinue
        if ($null -eq $cmd) { $cmd = Get-Command "WizTree.exe" -ErrorAction SilentlyContinue }
        if ($null -eq $cmd) { throw "WizTree CLI not found. Pass -WizTreeCsv or -WizTreeExe." }
        $Exe = $cmd.Source
    }

    & $Exe $ScanRoot "/export=$CsvPath" "/exportfolders=1" "/exportfiles=1" "/exportmaxdepth=$MaxDepth" "/admin=0" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "WizTree export failed with exit code $LASTEXITCODE"
    }
    if (-not (Test-Path -LiteralPath $CsvPath -PathType Leaf)) {
        throw "WizTree export did not create CSV: $CsvPath"
    }
}

function Read-WizTreeCandidates {
    param(
        [string]$CsvPath,
        [hashtable]$Candidates
    )

    $folderSizes = @{}
    foreach ($row in (Import-Csv -LiteralPath $CsvPath)) {
        $rawPath = Get-PropertyValue $row @("File Name", "Filename", "Full Path", "Path", "Name")
        if ([string]::IsNullOrWhiteSpace($rawPath)) { continue }

        $normalized = $rawPath.Trim().Trim('"')
        $isFolder = $normalized.EndsWith("\") -or $normalized.EndsWith("/")
        $normalized = $normalized.TrimEnd("\", "/")
        $leaf = [System.IO.Path]::GetFileName($normalized)
        $sizeBytes = Convert-ToInt64 (Get-PropertyValue $row @("Size", "Allocated", "Allocated Size"))
        $modifiedText = Get-PropertyValue $row @("Modified", "Last Modified", "Date Modified")
        $lastWrite = $null
        $parsedDate = [datetime]::MinValue
        if ([datetime]::TryParse($modifiedText, [ref]$parsedDate)) { $lastWrite = $parsedDate }

        if ($isFolder) {
            $folderSizes[$normalized] = $sizeBytes
        }

        foreach ($spec in $signalSpecs) {
            if ($leaf -ne [string]$spec.name) { continue }
            if ([string]$spec.type -eq "directory" -and -not $isFolder) { continue }
            if ([string]$spec.type -eq "file" -and $isFolder) { continue }

            $candidatePath = Split-Path -Parent $normalized
            if ([string]$spec.name -eq ".git") {
                $candidatePath = Split-Path -Parent $normalized
            }

            $candidateSize = if ($folderSizes.ContainsKey($candidatePath)) { [long]$folderSizes[$candidatePath] } else { $sizeBytes }
            Add-Candidate $Candidates $candidatePath ([string]$spec.name) $candidateSize "wiztree_csv" $lastWrite
        }
    }
}

function Read-DirectoryCandidates {
    param(
        [string[]]$Roots,
        [hashtable]$Candidates
    )

    $skip = @(".git", "node_modules", "target", "dist", "build", "vendor", "third_party", "external", ".repowise", ".sentrux")
    $queue = New-Object System.Collections.Generic.Queue[object]

    foreach ($rootInput in $Roots) {
        if (-not (Test-Path -LiteralPath $rootInput -PathType Container)) { continue }
        $queue.Enqueue([pscustomobject]@{ path = (Resolve-Path -LiteralPath $rootInput).Path; depth = 0 })
    }

    while ($queue.Count -gt 0) {
        $item = $queue.Dequeue()
        $dir = [string]$item.path
        $depth = [int]$item.depth

        foreach ($spec in $signalSpecs) {
            $signalPath = Join-Path $dir ([string]$spec.name)
            $pathType = if ([string]$spec.type -eq "directory") { "Container" } else { "Leaf" }
            $exists = if ([string]$spec.name -eq ".git") {
                Test-Path -LiteralPath $signalPath
            }
            else {
                Test-Path -LiteralPath $signalPath -PathType $pathType
            }
            if ($exists) {
                $signalItem = Get-Item -LiteralPath $signalPath -Force -ErrorAction SilentlyContinue
                $lastWrite = if ($null -ne $signalItem) { $signalItem.LastWriteTime } else { (Get-Item -LiteralPath $dir -Force).LastWriteTime }
                Add-Candidate $Candidates $dir ([string]$spec.name) 0 "filesystem" $lastWrite
            }
        }

        if ($depth -ge $MaxDepth) { continue }
        foreach ($child in Get-ChildItem -LiteralPath $dir -Directory -Force -ErrorAction SilentlyContinue) {
            if ($skip -contains $child.Name) { continue }
            $queue.Enqueue([pscustomobject]@{ path = $child.FullName; depth = $depth + 1 })
        }
    }
}

$candidates = @{}

if (-not [string]::IsNullOrWhiteSpace($WizTreeCsv)) {
    Read-WizTreeCandidates -CsvPath $WizTreeCsv -Candidates $candidates
}
elseif (-not [string]::IsNullOrWhiteSpace($WizTreeExe)) {
    $csv = if (-not [string]::IsNullOrWhiteSpace($ExportCsv)) {
        $ExportCsv
    }
    else {
        Join-Path ([System.IO.Path]::GetTempPath()) ("code-intel-wiztree-" + [guid]::NewGuid().ToString("N") + ".csv")
    }
    foreach ($scanRoot in $Root) {
        Invoke-WizTreeExport -Exe $WizTreeExe -ScanRoot $scanRoot -CsvPath $csv
        Read-WizTreeCandidates -CsvPath $csv -Candidates $candidates
    }
}
else {
    Read-DirectoryCandidates -Roots $Root -Candidates $candidates
}

$result = @(
    foreach ($candidate in $candidates.Values) {
        if ([int]$candidate.score -lt $MinScore) { continue }
        [pscustomobject][ordered]@{
            path = [string]$candidate.path
            score = [int]$candidate.score
            signals = @($candidate.signals)
            sizeBytes = [long]$candidate.sizeBytes
            lastWriteTime = if ($null -ne $candidate.lastWriteTime) { ([datetime]$candidate.lastWriteTime).ToString("o") } else { "" }
            source = [string]$candidate.source
            recommendedCommand = ".\invoke-code-intel.ps1 -RepoPath `"$($candidate.path)`" -Mode normal"
        }
    }
) | Sort-Object score, sizeBytes, path -Descending

if ($Json) {
    $result | ConvertTo-Json -Depth 6
}
else {
    $result | Format-Table -AutoSize path, score, signals, sizeBytes, source
}
