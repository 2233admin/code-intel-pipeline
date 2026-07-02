# 三栈工作流推荐器 (Workflow Stack Recommender) - standalone script
# 按项目特征分层推荐: matt-flow (idea->ship) / gstack (delivery/quality) / spec-driven (openspec-opsx | spec-kit)
# NOTE: this logic is duplicated in run-code-intel.ps1 (inline, ~line 86+). Keep both copies in sync when editing.

param(
    [string]$RepoPath,
    [switch]$Auto,        # 自动模式，不询问用户（本检测器不做交互提示，保留兼容旧参数）
    [switch]$Verbose
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ============ 特征检测 ============

function Get-CodeMetrics {
    param([string]$Path)

    $excludeDirNames = @("node_modules", ".git", "target", "dist", "build", "vendor", "venv", ".venv", "__pycache__")
    $includeExt = @(
        "*.ts", "*.tsx", "*.js", "*.jsx", "*.rs", "*.py", "*.go",
        "*.ps1", "*.psm1", "*.cs", "*.java", "*.kt", "*.swift", "*.vue", "*.svelte", "*.v"
    )

    $allFiles = @(Get-ChildItem -Path $Path -Recurse -File -Include $includeExt -ErrorAction SilentlyContinue |
        Where-Object {
            $full = $_.FullName
            -not ($excludeDirNames | Where-Object { $full -match [regex]::Escape("\$_\") -or $full -match [regex]::Escape("/$_/") })
        })

    $totalFiles = $allFiles.Count
    $totalLines = 0
    foreach ($file in $allFiles) {
        try {
            $totalLines += (Get-Content $file.FullName -ErrorAction SilentlyContinue | Measure-Object -Line).Lines
        }
        catch { }
    }

    return @{
        lines = $totalLines
        files = $totalFiles
        estimated = $false
    }
}

function Get-GovernanceIndicators {
    param([string]$Path)

    return @{
        hasDesign = Test-Path "$Path/design.md"
        hasSpecs = Test-Path "$Path/specs"
        hasSecurityReview = (Test-Path "$Path/security-review.md") -or (Test-Path "$Path/docs/security-review.md")
        hasArchitecture = Test-Path "$Path/architecture.md"
        hasOpenSpec = Test-Path "$Path/openspec"
        hasSpecKit = Test-Path "$Path/.specify"
        hasADRs = (Test-Path "$Path/docs/adr") -or (Test-Path "$Path/adr")
        hasConstitution = Test-Path "$Path/constitution.md"
        hasIssueTemplates = Test-Path "$Path/.github/ISSUE_TEMPLATE"
    }
}

function Get-CollaborationMetrics {
    param([string]$Path)

    try {
        $contributors = @(& git -C $Path log --format=%ae 2>$null | Sort-Object -Unique)
        $lastCommit = & git -C $Path log -1 --format=%ci 2>$null
        $firstCommit = & git -C $Path log --reverse --format=%ci 2>$null | Select-Object -First 1
        # repoAgeDays = age since FIRST commit (brownfield detection);
        # lastCommitAgeDays = staleness since LAST commit (activity detection).
        # Using last-commit age for both would judge every active old repo "greenfield".
        $lastCommitAgeDays = if ($lastCommit) {
            ((Get-Date) - [DateTime]::Parse($lastCommit)).Days
        } else { 9999 }
        $repoAge = if ($firstCommit) {
            ((Get-Date) - [DateTime]::Parse($firstCommit)).Days
        } else { 0 }

        return @{
            contributors = $contributors.Count
            repoAgeDays = $repoAge
            lastCommitAgeDays = $lastCommitAgeDays
        }
    }
    catch {
        return @{
            contributors = 0
            repoAgeDays = 0
            lastCommitAgeDays = 9999
        }
    }
}

function Get-CICDScore {
    param([string]$Path)

    $score = 0

    if (Test-Path "$Path/.github/workflows") { $score += 10 }
    if (Test-Path "$Path/.gitlab-ci.yml") { $score += 10 }
    if (Test-Path "$Path/Jenkinsfile") { $score += 10 }
    if (Test-Path "$Path/azure-pipelines.yml") { $score += 10 }
    if (Test-Path "$Path/.circleci") { $score += 10 }

    return $score
}

function Test-CodeIntelHasDeployIndicators {
    param([string]$Path)

    if (Test-Path "$Path/Dockerfile") { return $true }
    if (Test-Path "$Path/docker-compose.yml") { return $true }
    if (Test-Path "$Path/docker-compose.yaml") { return $true }

    $workflowsDir = "$Path/.github/workflows"
    if (Test-Path $workflowsDir) {
        $matches = @(Get-ChildItem -Path $workflowsDir -Filter "*.yml" -ErrorAction SilentlyContinue) +
                   @(Get-ChildItem -Path $workflowsDir -Filter "*.yaml" -ErrorAction SilentlyContinue)
        foreach ($wf in $matches) {
            try {
                $content = Get-Content -LiteralPath $wf.FullName -Raw -ErrorAction SilentlyContinue
                if ($content -match "(?i)deploy") { return $true }
            }
            catch { }
        }
    }
    return $false
}

function Test-CodeIntelHasWebFrontend {
    param([string]$Path)

    $packageJsonPath = "$Path/package.json"
    if (Test-Path $packageJsonPath) {
        try {
            $pkg = Get-Content -LiteralPath $packageJsonPath -Raw -ErrorAction SilentlyContinue | ConvertFrom-Json
            $deps = @()
            if ($pkg.PSObject.Properties["dependencies"]) { $deps += $pkg.dependencies.PSObject.Properties.Name }
            if ($pkg.PSObject.Properties["devDependencies"]) { $deps += $pkg.devDependencies.PSObject.Properties.Name }
            $frontendMarkers = @("react", "vue", "next", "svelte", "vite")
            foreach ($marker in $frontendMarkers) {
                if ($deps | Where-Object { $_ -match "(?i)$marker" }) { return $true }
            }
        }
        catch { }
    }

    foreach ($dir in @("frontend", "web", "ui")) {
        if (Test-Path "$Path/$dir") { return $true }
    }
    return $false
}

function Get-TestCoverage {
    param([string]$Path)

    $hasTests = $false
    $testPatterns = @("*/test/*", "*/tests/*", "*/__tests__/*", "*_test.*", "*_tests.*", "*.spec.*", "*.test.*")

    foreach ($pattern in $testPatterns) {
        $found = @(Get-ChildItem -Path $Path -Recurse -Include $pattern -ErrorAction SilentlyContinue)
        if ($found.Count -gt 0) {
            $hasTests = $true
            break
        }
    }

    return $hasTests
}

# ============ 推断引擎 ============

function Get-SpecDrivenRecommendation {
    param(
        [hashtable]$Metrics,
        [hashtable]$Governance,
        [hashtable]$Collaboration,
        [int]$CICDScore,
        [bool]$HasTests
    )

    if ($Governance.hasOpenSpec) {
        return @{
            stack = "spec-driven"
            tool = "openspec-opsx"
            verdict = "already_adopted"
            score = 100
            reasons = @("检测到 openspec/ 目录 (已在用 OpenSpec OPSX)")
            entrySkills = @()
        }
    }
    if ($Governance.hasSpecKit) {
        return @{
            stack = "spec-driven"
            tool = "spec-kit"
            verdict = "already_adopted"
            score = 100
            reasons = @("检测到 .specify/ 目录 (已在用 spec-kit)")
            entrySkills = @()
        }
    }

    $score = 0
    $reasons = @()

    # 规模评分
    if ($Metrics.lines -gt 50000) {
        $score += 40
        $reasons += "大型代码库 ($($Metrics.lines) 行)"
    }
    elseif ($Metrics.lines -gt 10000) {
        $score += 25
        $reasons += "中型代码库 ($($Metrics.lines) 行)"
    }
    elseif ($Metrics.lines -gt 5000) {
        $score += 10
        $reasons += "较小代码库 ($($Metrics.lines) 行)"
    }

    # 治理文件评分
    if ($Governance.hasDesign) { $score += 20; $reasons += "存在 design.md" }
    if ($Governance.hasArchitecture) { $score += 15; $reasons += "存在 architecture.md" }
    if ($Governance.hasSpecs) { $score += 25; $reasons += "存在 specs/ 目录" }
    if ($Governance.hasSecurityReview) { $score += 25; $reasons += "存在安全审查文件" }
    if ($Governance.hasADRs) { $score += 15; $reasons += "存在 ADR 文档" }
    if ($Governance.hasConstitution) { $score += 20; $reasons += "存在 constitution.md" }

    # 协作评分
    if ($Collaboration.contributors -gt 5) {
        $score += 25
        $reasons += "多人协作 ($($Collaboration.contributors) 人)"
    }
    elseif ($Collaboration.contributors -gt 2) {
        $score += 15
        $reasons += "少量协作 ($($Collaboration.contributors) 人)"
    }

    if ($Collaboration.repoAgeDays -gt 365) {
        $score += 10
        $reasons += "成熟项目 ($($Collaboration.repoAgeDays) 天)"
    }

    # CI/CD 评分
    if ($CICDScore -gt 0) {
        $score += $CICDScore
        $reasons += "存在 CI/CD 配置"
    }

    # 测试评分
    if ($HasTests) { $score += 5 } else { $score -= 5 }

    $verdict = if ($score -ge 50) { "recommended" }
               elseif ($score -ge 30) { "optional" }
               else { "not_needed" }

    # brownfield (存量, 源码多+仓龄久) -> openspec-opsx; greenfield (近乎空仓/新项目) -> spec-kit
    $isBrownfield = ($Metrics.files -gt 5) -and ($Collaboration.repoAgeDays -gt 90)
    $tool = if ($isBrownfield) { "openspec-opsx" } else { "spec-kit" }
    if ($isBrownfield) {
        $reasons += "存量项目 (files=$($Metrics.files), repoAgeDays=$($Collaboration.repoAgeDays)) -> OpenSpec OPSX 适合持续变更管理"
    } else {
        $reasons += "新建/近乎空仓 (files=$($Metrics.files), repoAgeDays=$($Collaboration.repoAgeDays)) -> spec-kit 适合 0->1 起步"
    }

    $entrySkills = @(if ($verdict -eq "not_needed") { }
                   elseif ($tool -eq "openspec-opsx") { "openspec init" }
                   else { "specify init" })

    return @{
        stack = "spec-driven"
        tool = $tool
        verdict = $verdict
        score = $score
        reasons = $reasons
        entrySkills = $entrySkills
        metrics = $Metrics
        governance = $Governance
        collaboration = $Collaboration
    }
}

function Get-MattFlowRecommendation {
    param(
        [hashtable]$Metrics,
        [hashtable]$Governance,
        [hashtable]$Collaboration
    )

    $reasons = @()
    $isActive = $Collaboration.lastCommitAgeDays -le 90
    $hasSource = $Metrics.files -gt 5

    $verdict = if ($isActive -and $hasSource) { "recommended" } else { "not_needed" }

    if ($isActive) { $reasons += "活跃开发 (最近提交 $($Collaboration.lastCommitAgeDays) 天前)" }
    else { $reasons += "90天内无提交 (最近提交 $($Collaboration.lastCommitAgeDays) 天前)" }

    if ($hasSource) { $reasons += "在建项目 (files=$($Metrics.files))" }
    else { $reasons += "源码文件过少 (files=$($Metrics.files))" }

    $entrySkills = @()
    if ($verdict -eq "recommended") {
        if ($Governance.hasIssueTemplates) {
            $entrySkills += "/triage"
            $reasons += "检测到 .github/ISSUE_TEMPLATE -> 外来 issue 分诊"
        }
        $entrySkills += "/grill-with-docs"
        if ($Metrics.lines -gt 20000 -or $Collaboration.contributors -gt 2) {
            $entrySkills += "/to-prd"
            $entrySkills += "/to-issues"
            $reasons += "大项目 (lines=$($Metrics.lines), contributors=$($Collaboration.contributors)) -> 加 PRD/issue 拆解"
        }
    }

    return @{
        stack = "matt-flow"
        verdict = $verdict
        reasons = $reasons
        entrySkills = $entrySkills
    }
}

function Get-GstackRecommendation {
    param(
        [string]$Path,
        [hashtable]$Collaboration
    )

    $reasons = @()
    $isActive = $Collaboration.lastCommitAgeDays -le 90
    $verdict = if ($isActive) { "recommended" } else { "not_needed" }

    if ($isActive) { $reasons += "活跃开发 (最近提交 $($Collaboration.lastCommitAgeDays) 天前)" }
    else { $reasons += "90天内无提交 (最近提交 $($Collaboration.lastCommitAgeDays) 天前)" }

    $entrySkills = @()
    if ($verdict -eq "recommended") {
        $hasWebFrontend = Test-CodeIntelHasWebFrontend -Path $Path
        $hasDeploy = Test-CodeIntelHasDeployIndicators -Path $Path

        if ($hasWebFrontend) {
            $entrySkills += "/qa"
            $entrySkills += "/design-review"
            $reasons += "检测到 web 前端 -> QA + 设计评审"
        }
        if ($hasDeploy) {
            $entrySkills += "/ship"
            $entrySkills += "/canary"
            $reasons += "检测到部署迹象 (Dockerfile/compose/CI deploy) -> ship + canary"
        }
        if ($entrySkills.Count -eq 0) {
            $entrySkills += "/review"
            $reasons += "默认交付质量闸"
        }
    }

    return @{
        stack = "gstack"
        verdict = $verdict
        reasons = $reasons
        entrySkills = $entrySkills
    }
}

# ============ 主程序 ============

if ([string]::IsNullOrWhiteSpace($RepoPath)) {
    $RepoPath = Get-Location
}

Write-Host "三栈工作流推荐器 (Workflow Stack Recommender)"
Write-Host ("=" * 40)

Write-Host "`n[1/5] 分析代码规模..."
$metrics = Get-CodeMetrics -Path $RepoPath

Write-Host "[2/5] 检测治理文件..."
$governance = Get-GovernanceIndicators -Path $RepoPath

Write-Host "[3/5] 分析协作模式..."
$collaboration = Get-CollaborationMetrics -Path $RepoPath

Write-Host "[4/5] 检查 CI/CD..."
$cicdScore = Get-CICDScore -Path $RepoPath

Write-Host "[5/5] 检查测试覆盖..."
$hasTests = Get-TestCoverage -Path $RepoPath

$specDriven = Get-SpecDrivenRecommendation -Metrics $metrics -Governance $governance -Collaboration $collaboration -CICDScore $cicdScore -HasTests $hasTests
$mattFlow = Get-MattFlowRecommendation -Metrics $metrics -Governance $governance -Collaboration $collaboration
$gstack = Get-GstackRecommendation -Path $RepoPath -Collaboration $collaboration

$workflows = @($mattFlow, $gstack, $specDriven)

Write-Host ("`n" + ("=" * 40))
Write-Host "`n工作流推荐结果`n"
foreach ($wf in $workflows) {
    $toolText = if ($wf.ContainsKey("tool") -and $wf.tool) { " (tool=$($wf.tool))" } else { "" }
    $skillsText = if ($wf.entrySkills.Count -gt 0) { $wf.entrySkills -join " " } else { "(none)" }
    Write-Host "- $($wf.stack)${toolText}: $($wf.verdict) -> $skillsText"
    foreach ($reason in $wf.reasons) {
        Write-Host "    * $reason"
    }
}
Write-Host ""

# 返回结果用于管道 (向后兼容: 顶层字段沿用旧 openSpec 结果形状 = spec-driven 层)
$result = [ordered]@{
    workflows = $workflows
    specDriven = $specDriven
    mattFlow = $mattFlow
    gstack = $gstack
    # legacy top-level aliases (spec-driven layer)
    stack = $specDriven.stack
    tool = $specDriven.tool
    verdict = $specDriven.verdict
    score = $specDriven.score
    reasons = $specDriven.reasons
    entrySkills = $specDriven.entrySkills
}

return $result
