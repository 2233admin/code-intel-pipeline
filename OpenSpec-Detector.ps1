# OpenSpec Enterprise 检测器
# 自动推断项目是否需要 OpenSpec 工作流

param(
    [string]$RepoPath,
    [switch]$Auto,        # 自动模式，不询问用户
    [switch]$Verbose
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ============ 特征检测 (优化版) ============

function Get-CodeMetrics {
    param([string]$Path)

    # 快速估算：只统计主要目录
    $quickDirs = @("src", "lib", "app", "packages", "crates")
    $totalLines = 0
    $totalFiles = 0

    foreach ($dir in $quickDirs) {
        $dirPath = Join-Path $Path $dir
        if (Test-Path $dirPath) {
            $files = Get-ChildItem -Path $dirPath -Recurse -File -Include *.ts,*.tsx,*.js,*.jsx,*.rs,*.py,*.go -ErrorAction SilentlyContinue
            $totalFiles += $files.Count
            foreach ($file in $files) {
                try {
                    $totalLines += (Get-Content $file.FullName -ErrorAction SilentlyContinue | Measure-Object -Line).Lines
                }
                catch { }
            }
        }
    }

    # 如果快速扫描没有结果，扫描根目录
    if ($totalFiles -eq 0) {
        $files = Get-ChildItem -Path $Path -File -Include *.ts,*.tsx,*.js,*.jsx,*.rs,*.py,*.go -ErrorAction SilentlyContinue | Select-Object -First 100
        $totalFiles = $files.Count
        foreach ($file in $files) {
            try {
                $totalLines += (Get-Content $file.FullName -ErrorAction SilentlyContinue | Measure-Object -Line).Lines
            }
            catch { }
        }
        $totalLines = $totalLines * 10  # 估算
    }

    return @{
        lines = $totalLines
        files = $totalFiles
        estimated = ($totalFiles -eq 0)
    }
}

function Get-GovernanceIndicators {
    param([string]$Path)

    $indicators = @{
        hasDesign = Test-Path "$Path/design.md"
        hasSpecs = Test-Path "$Path/specs"
        hasSecurityReview = (Test-Path "$Path/security-review.md") -or (Test-Path "$Path/docs/security-review.md")
        hasArchitecture = Test-Path "$Path/architecture.md"
        hasOpenSpec = Test-Path "$Path/openspec"
        hasADRs = (Test-Path "$Path/docs/adr") -or (Test-Path "$Path/adr")
        hasConstitution = Test-Path "$Path/constitution.md"
    }

    return $indicators
}

function Get-CollaborationMetrics {
    param([string]$Path)

    try {
        $contributors = @(& git -C $Path log --format=%ae 2>$null | Sort-Object -Unique)
        $lastCommit = & git -C $Path log -1 --format=%ci 2>$null
        $repoAge = if ($lastCommit) {
            ((Get-Date) - [DateTime]::Parse($lastCommit)).Days
        } else { 0 }

        return @{
            contributors = $contributors.Count
            repoAgeDays = $repoAge
        }
    }
    catch {
        return @{
            contributors = 0
            repoAgeDays = 0
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

function Get-TestCoverage {
    param([string]$Path)

    $hasTests = $false
    $testPatterns = @("*/test/*", "*/tests/*", "*/__tests__/*", "*_test.*", "*_tests.*", "*.spec.*", "*.test.*")

    foreach ($pattern in $testPatterns) {
        if (Get-ChildItem -Path $Path -Recurse -Include $pattern -ErrorAction SilentlyContinue) {
            $hasTests = $true
            break
        }
    }

    return $hasTests
}

# ============ 推断引擎 ============

function Get-OpenSpecRecommendation {
    param(
        [hashtable]$Metrics,
        [hashtable]$Governance,
        [hashtable]$Collaboration,
        [int]$CICDScore,
        [bool]$HasTests
    )

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
    if ($Governance.hasOpenSpec) {
        return @{
            recommendation = "already_using"
            score = 100
            schema = "spec-driven-enterprise"
            message = "项目已在使用 OpenSpec"
            reasons = @("检测到 openspec/ 目录")
        }
    }

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

    # 推断结果
    $schema = if ($score -ge 80) { "spec-driven-enterprise" }
              elseif ($score -ge 50) { "spec-driven" }
              else { $null }

    $recommendation = if ($score -ge 80) { "strongly_recommended" }
                      elseif ($score -ge 50) { "recommended" }
                      elseif ($score -ge 30) { "optional" }
                      else { "not_needed" }

    return @{
        score = $score
        recommendation = $recommendation
        schema = $schema
        reasons = $reasons
        metrics = $Metrics
        governance = $Governance
        collaboration = $Collaboration
    }
}

# ============ 用户确认 ============

function Show-OpenSpecSuggestion {
    param([hashtable]$Result)

    $emoji = switch ($Result.recommendation) {
        "already_using" { "[OK]" }
        "strongly_recommended" { "[HOT]" }
        "recommended" { "[IDE]" }
        "optional" { "[?]" }
        default { "[---]" }
    }

    $schemaNote = if ($Result.schema -eq "spec-driven-enterprise") {
        "`n- DAG 依赖管理`n- 企业级治理`n- Agent 协议"
    } elseif ($Result.schema -eq "spec-driven") {
        "`n- 轻量 DAG`n- 规范驱动"
    } else { "" }

    $message = @"

$emoji OpenSpec 工作流建议

**推荐级别**: $($Result.recommendation -replace '_', ' ')
**评分**: $($Result.score)/100

**特征检测**:
$($Result.reasons -join "`n- ")

$schemaNote

"@

    return $message
}

function Initialize-OpenSpecWorkflow {
    param(
        [string]$Path,
        [string]$Schema = "spec-driven"
    )

    $openSpecDir = Join-Path $Path "openspec"

    # 创建目录结构
    $null = New-Item -Path "$openSpecDir/changes" -ItemType Directory -Force
    $null = New-Item -Path "$openSpecDir/schemas" -ItemType Directory -Force

    # 如果有 openspec-enterprise 仓库，克隆 schemas
    # 这里可以添加从 Gitea 克隆的逻辑

    Write-Host "OpenSpec 工作流已初始化: $openSpecDir"
    Write-Host "下一步: 运行 'node skillctl.mjs init' 初始化工作流"
}

# ============ 主程序 ============

if ([string]::IsNullOrWhiteSpace($RepoPath)) {
    $RepoPath = Get-Location
}

Write-Host "OpenSpec Enterprise 检测器"
Write-Host "=" * 40

# 收集特征
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

# 推断
$result = Get-OpenSpecRecommendation -Metrics $metrics -Governance $governance -Collaboration $collaboration -CICDScore $cicdScore -HasTests $hasTests

# 输出结果
Write-Host "`n" + ("=" * 40)
Write-Host (Show-OpenSpecSuggestion -Result $result)

# 用户确认
if (-not $Auto -and $result.recommendation -eq "strongly_recommended") {
    $response = Read-Host "`n是否初始化 OpenSpec Enterprise 工作流? (Y/n)"

    if ($response -ne "n" -and $response -ne "N") {
        Initialize-OpenSpecWorkflow -Path $RepoPath -Schema $result.schema
    }
}

# 返回结果用于管道
return $result
