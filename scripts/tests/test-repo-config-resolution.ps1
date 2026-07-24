#requires -Version 7.2

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$scriptPath = Join-Path $root "run-code-intel.ps1"
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -gt 0) { throw $parseErrors[0].Message }

$functionNames = @("Resolve-Repo", "Get-JsonProperty", "Find-RepoConfigByPath")
$functions = $ast.FindAll({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $functionNames -contains $node.Name
}, $true)
. ([scriptblock]::Create(($functions.Extent.Text -join "`n`n")))

$scratch = Join-Path $env:TEMP ("code-intel-repo-config-{0}" -f [guid]::NewGuid().ToString("N"))
try {
    $repo = New-Item -ItemType Directory -Path (Join-Path $scratch "ConfiguredRepo") -Force
    $repos = [pscustomobject]@{
        configured = [pscustomobject]@{
            path = $repo.FullName + [System.IO.Path]::DirectorySeparatorChar
            sentruxPath = "configured-scope"
            repowiseScopePaths = @("scope-a", "scope-b")
            repowiseRootFiles = @("README.md", "pyproject.toml")
        }
        aliasOnly = [pscustomobject]@{
            path = (Join-Path $scratch "missing")
            sentruxPath = "alias-scope"
        }
    }

    $resolved = Resolve-Repo (Join-Path $repo.FullName ".")
    $byPath = Find-RepoConfigByPath -ReposConfig $repos -ResolvedRepoPath $resolved
    if ($byPath.sentruxPath -ne "configured-scope") { throw "-RepoPath did not load configured sentruxPath" }
    if (($byPath.repowiseScopePaths -join ",") -ne "scope-a,scope-b") { throw "-RepoPath did not load configured Repowise scopes" }
    if (($byPath.repowiseRootFiles -join ",") -ne "README.md,pyproject.toml") { throw "-RepoPath did not load configured Repowise root files" }

    $byAlias = Get-JsonProperty $repos "aliasOnly"
    if ($byAlias.sentruxPath -ne "alias-scope") { throw "alias lookup regressed" }

    Write-Host "PASS: RepoPath reverse lookup loads configured scope and alias lookup remains intact."
}
finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
