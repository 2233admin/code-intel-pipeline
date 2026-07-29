param(
    [ValidateSet("Validate", "List", "Plan")]
    [string]$Action = "Validate",

    [string]$Capability = "",
    [string]$RepoPath = "",

    [ValidateSet("lite", "normal", "full")]
    [string]$Mode = "normal",

    [string]$Manifest = "",
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSCommandPath
if ([string]::IsNullOrWhiteSpace($Manifest)) {
    $Manifest = Join-Path $root "orchestration\integrations.json"
}

function Get-JsonProperty {
    param(
        [object]$Object,
        [string]$Name
    )

    if ($null -eq $Object) { return $null }
    $prop = $Object.PSObject.Properties[$Name]
    if ($null -eq $prop) { return $null }
    return $prop.Value
}

function ConvertTo-Array {
    param([object]$Value)

    if ($null -eq $Value) { return @() }
    if ($Value -is [System.Array]) { return @($Value) }
    return @($Value)
}

function Expand-CommandTemplate {
    param(
        [string]$Template,
        [string]$RepoPath,
        [string]$Mode
    )

    $expanded = $Template.Replace("<mode>", $Mode)
    if (-not [string]::IsNullOrWhiteSpace($RepoPath)) {
        $expanded = $expanded.Replace("<repo-path>", $RepoPath)
    }
    return $expanded
}

if (-not (Test-Path -LiteralPath $Manifest -PathType Leaf)) {
    throw "Orchestration manifest missing: $Manifest"
}

$manifestData = Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json
$stages = ConvertTo-Array (Get-JsonProperty $manifestData "stages")
$integrations = ConvertTo-Array (Get-JsonProperty $manifestData "integrations")

$errors = New-Object System.Collections.Generic.List[string]
$stageIds = @{}
foreach ($stage in $stages) {
    $id = [string](Get-JsonProperty $stage "id")
    if ([string]::IsNullOrWhiteSpace($id)) {
        $errors.Add("stage id is empty")
        continue
    }
    if ($stageIds.ContainsKey($id)) {
        $errors.Add("duplicate stage id: $id")
    }
    else {
        $stageIds[$id] = $stage
    }
}

$integrationIds = @{}
foreach ($integration in $integrations) {
    $id = [string](Get-JsonProperty $integration "id")
    $stage = [string](Get-JsonProperty $integration "stage")
    $entrypoint = [string](Get-JsonProperty $integration "entrypoint")
    $capabilities = ConvertTo-Array (Get-JsonProperty $integration "capabilities")

    if ([string]::IsNullOrWhiteSpace($id)) {
        $errors.Add("integration id is empty")
        continue
    }
    if ($integrationIds.ContainsKey($id)) {
        $errors.Add("duplicate integration id: $id")
    }
    else {
        $integrationIds[$id] = $integration
    }
    if (-not $stageIds.ContainsKey($stage)) {
        $errors.Add("integration $id references unknown stage: $stage")
    }
    if ([string]::IsNullOrWhiteSpace($entrypoint)) {
        $errors.Add("integration $id has no entrypoint")
    }
    elseif ($entrypoint -like "*.ps1" -or $entrypoint -like "*.py" -or $entrypoint -like "*.toml" -or $entrypoint -like "*.rs") {
        $candidate = Join-Path $root $entrypoint
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            $errors.Add("integration $id entrypoint missing: $entrypoint")
        }
    }
    if ($capabilities.Count -eq 0) {
        $errors.Add("integration $id exposes no capabilities")
    }
}

$stageOrder = @{}
foreach ($stage in $stages) {
    $stageOrder[[string](Get-JsonProperty $stage "id")] = [int](Get-JsonProperty $stage "order")
}

$selected = @($integrations | Where-Object {
    if ([string]::IsNullOrWhiteSpace($Capability)) { return $true }
    $caps = ConvertTo-Array (Get-JsonProperty $_ "capabilities")
    return ($caps -contains $Capability) -or ([string](Get-JsonProperty $_ "id") -eq $Capability) -or ([string](Get-JsonProperty $_ "stage") -eq $Capability)
} | Sort-Object @{ Expression = { $stageOrder[[string](Get-JsonProperty $_ "stage")] } }, @{ Expression = { [string](Get-JsonProperty $_ "id") } })

$plan = @($selected | ForEach-Object {
    $commands = Get-JsonProperty $_ "commands"
    $expandedCommands = [ordered]@{}
    if ($null -ne $commands) {
        foreach ($command in $commands.PSObject.Properties) {
            $expandedCommands[$command.Name] = Expand-CommandTemplate ([string]$command.Value) $RepoPath $Mode
        }
    }

    [pscustomobject][ordered]@{
        id = [string](Get-JsonProperty $_ "id")
        stage = [string](Get-JsonProperty $_ "stage")
        kind = [string](Get-JsonProperty $_ "kind")
        required = [bool](Get-JsonProperty $_ "required")
        entrypoint = [string](Get-JsonProperty $_ "entrypoint")
        capabilities = @(ConvertTo-Array (Get-JsonProperty $_ "capabilities"))
        commands = $expandedCommands
        artifactContract = @(ConvertTo-Array (Get-JsonProperty $_ "artifactContract"))
        extensionPoint = [string](Get-JsonProperty $_ "extensionPoint")
    }
})

$result = [pscustomobject][ordered]@{
    ok = $errors.Count -eq 0
    action = $Action
    manifest = $Manifest
    policy = Get-JsonProperty $manifestData "policy"
    errors = @($errors)
    stages = @($stages | Sort-Object @{ Expression = { [int](Get-JsonProperty $_ "order") } })
    integrations = if ($Action -eq "Validate") { @() } else { $plan }
    plan = if ($Action -eq "Plan") { $plan } else { @() }
}

if ($Json) {
    $result | ConvertTo-Json -Depth 12
}
else {
    if (-not $result.ok) {
        Write-Host "Code Intel orchestration: FAILED"
        foreach ($errorText in $errors) { Write-Host "- $errorText" }
    }
    elseif ($Action -eq "Validate") {
        Write-Host "Code Intel orchestration: OK"
        Write-Host "Manifest: $Manifest"
        Write-Host "Stages: $($stages.Count)"
        Write-Host "Integrations: $($integrations.Count)"
    }
    else {
        Write-Host "Code Intel orchestration: $Action"
        foreach ($item in $plan) {
            Write-Host "$($item.stage): $($item.id) [$($item.kind)] entry=$($item.entrypoint)"
            if ($Action -eq "Plan") {
                foreach ($command in $item.commands.GetEnumerator()) {
                    Write-Host "  $($command.Key): $($command.Value)"
                }
            }
        }
    }
}

if (-not $result.ok) { exit 1 }
exit 0
