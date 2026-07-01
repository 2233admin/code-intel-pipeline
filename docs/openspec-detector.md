# OpenSpec Enterprise 检测器

检测项目是否需要 OpenSpec Enterprise 工作流，已集成到 Code Intel Pipeline。

## 功能

1. **项目特征扫描** - 检测代码规模、文件类型、协作模式
2. **需求推断** - 根据特征判断是否需要 OpenSpec
3. **用户确认** - 提示用户确认是否启用
4. **自动集成** - 集成到 Code Intel Pipeline 的 `run-code-intel.ps1`

## 集成到 Code Intel Pipeline

### 参数

```powershell
-SkipOpenSpec    # 跳过 OpenSpec 检测
-AutoOpenSpec    # 自动模式，不询问用户
```

### 输出位置

- `summary.md` - OpenSpec Enterprise 段落
- `report.json` - `openSpec` 数据块
- notes 数组 - 检测结果摘要

## 项目特征检测

### 规模指标

| 指标 | 小型 | 中型 | 大型 |
|------|------|------|------|
| 代码行数 | < 5,000 | 5,000 - 50,000 | > 50,000 |
| 文件数 | < 100 | 100 - 1,000 | > 1,000 |
| 模块数 | < 10 | 10 - 100 | > 100 |
| 多人协作 | 1人 | 2-3人 | > 3人 |

### 治理需求检测

| 特征 | 权重 |
|------|------|
| 存在 design.md, spec.md 等规范文件 | +30 |
| 存在 security, audit 相关目录 | +25 |
| README 提到 governance/compliance | +20 |
| 代码库年龄 > 1年 | +15 |
| 有 CI/CD 流水线 | +10 |
| 存在 data-model 或 schema 目录 | +25 |

### 推断规则

```
Score >= 80: 强烈推荐 OpenSpec Enterprise
Score >= 50: 建议 OpenSpec 标准
Score >= 30: 可选 OpenSpec
Score < 30: 不需要
```

## 实现

```powershell
# OpenSpec Detector Module

function Get-OpenSpecProjectScore {
    param([string]$RepoPath)

    $score = 0

    # 1. 规模检测
    $lines = (Get-ChildItem -Path $RepoPath -Recurse -File -Include *.ts,*.js,*.rs,*.py |
        Get-Content | Measure-Object -Line).Lines
    if ($lines -gt 50000) { $score += 40 }
    elseif ($lines -gt 5000) { $score += 20 }

    # 2. 治理文件检测
    if (Test-Path "$RepoPath/design.md") { $score += 30 }
    if (Test-Path "$RepoPath/specs") { $score += 25 }
    if (Test-Path "$RepoPath/security-review.md") { $score += 25 }

    # 3. 协作检测
    $contributors = (git -C $RepoPath log --format=%ae | Sort-Object -Unique).Count
    if ($contributors -gt 3) { $score += 20 }
    elseif ($contributors -gt 1) { $score += 10 }

    # 4. CI/CD 检测
    if (Test-Path "$RepoPath/.github/workflows") { $score += 10 }
    if (Test-Path "$RepoPath/.gitlab-ci.yml") { $score += 10 }

    return $score
}

function Invoke-OpenSpecSuggestion {
    param([string]$RepoPath)

    $score = Get-OpenSpecProjectScore -RepoPath $RepoPath

    if ($score -ge 80) {
        return @{
            recommendation = "strongly_recommended"
            score = $score
            message = @"
检测到这是大型项目，强烈建议使用 OpenSpec Enterprise 工作流。

特征:
- 代码规模: $score 分
- 支持 DAG 依赖管理
- 企业级治理
- Agent 协议集成

[Y] 启用 OpenSpec Enterprise
[n] 跳过
[?] 了解更多
"@
        }
    }
    elseif ($score -ge 50) {
        return @{
            recommendation = "recommended"
            score = $score
            message = "建议使用 OpenSpec 标准工作流"
        }
    }
    else {
        return @{
            recommendation = "not_needed"
            score = $score
            message = "项目规模较小，暂不需要 OpenSpec"
        }
    }
}
```

## 集成到 Code Intel Pipeline

在 `run-code-intel.ps1` 中添加检测步骤：

```powershell
# 在扫描开始前
$openSpecSuggestion = Invoke-OpenSpecSuggestion -RepoPath $RepoPath
if ($openSpecSuggestion.recommendation -eq "strongly_recommended") {
    Write-Host $openSpecSuggestion.message
    $response = Read-Host "选择"
    if ($response -eq "Y") {
        # 初始化 OpenSpec
        Initialize-OpenSpecWorkflow -RepoPath $RepoPath -Schema "spec-driven-enterprise"
    }
}
```

## 框架对比

| 框架 | 特点 | 适合场景 |
|------|------|----------|
| OpenSpec Enterprise | DAG + 治理 + Agent | 大型多人协作 |
| OpenSpec 标准 | 轻量 DAG | 中型项目 |
| Dome (GS) | 阶段门控 | 瀑布风格 |
| AJ Stock (GSD) | 目标驱动 | 敏捷增强 |

## 下一步

1. 将此模块集成到 Code Intel Pipeline
2. 添加多框架对比功能
3. 实现智能推荐引擎
