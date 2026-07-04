# 三栈工作流推荐器 (Workflow Stack Recommender)

按项目真实特征分层推荐用户实际在用的三个工作流体系，已集成到 Code Intel Pipeline。三层互补、可同时推荐，不是互斥选择。

> 历史注：本模块原名 "OpenSpec Enterprise 检测器"，只判定是否需要 OpenSpec。现已泛化为三栈推荐器；`report.json` 中 `openSpec` 字段为向后兼容保留（内容 = 下方 spec-driven 层判定结果），新增 `workflows` 数组承载三层完整输出。

## 三栈定义

### 1. matt-flow — idea→ship 主流程

来源：mattpocock/skills。核心动线 `/grill-with-docs` → `/to-prd` → `/to-issues` → `/implement`，`/triage` 负责收拢外来 issue。

- **verdict = recommended** 当：活跃开发（最近提交 ≤90 天）且属于用户在建项目（源码文件数 >5）
- **entrySkills** 按特征叠加：
  - 检测到 `.github/ISSUE_TEMPLATE` 或明显的 issue 协作迹象 → 加 `/triage`
  - 默认加 `/grill-with-docs`
  - 大项目（`lines > 20000` 或 `contributors > 2`）→ 加 `/to-prd /to-issues`

### 2. gstack — 交付/质量层

覆盖 `/review /ship /qa /design-review /browse /canary /benchmark`。

- **verdict = recommended** 当：活跃开发（最近提交 ≤90 天）
- **entrySkills** 按特征叠加：
  - 检测到 web 前端（`package.json` 含 `react/vue/next/svelte/vite`，或存在 `frontend/ web/ ui/` 目录）→ `/qa /design-review`
  - 检测到部署迹象（`Dockerfile`、`docker-compose.yml`，或 `.github/workflows/*.yml` 内含 `deploy` 字样）→ `/ship /canary`
  - 都没有 → 默认至少给 `/review`

### 3. spec-driven — 规范驱动治理层

在两个真实项目之间按需求推荐其一，纯推荐 UX，不做深集成：

| 工具 | 来源 | 特点 | 适合场景 |
|------|------|------|----------|
| **OpenSpec OPSX** | Fission-AI/OpenSpec | 流动迭代式 spec 工作流，actions 非 phases（propose/explore/apply/sync/archive），artifact 链 proposal→specs→design→tasks→implement，`openspec init` 生成 `.claude/skills` | 存量 (brownfield) 项目做持续变更管理 |
| **spec-kit** | github/spec-kit | `.specify/` 结构 + `/specify /plan /tasks` + `constitution.md`，Spec-Driven Development 起步套件 | 从零起步 (greenfield) 的 0→1 构建 |

**选型规则**：

1. 仓里已有 `openspec/` 目录 → `verdict=already_adopted`，`tool=openspec-opsx`
2. 已有 `.specify/` 目录 → `verdict=already_adopted`，`tool=spec-kit`
3. 否则跑治理打分（沿用规模/治理文件/协作/仓龄/CI/测试维度），决定这层要不要推：
   - `score >= 50` → `recommended`
   - `score >= 30` → `optional`
   - `score < 30` → `not_needed`
4. 工具选择（仅当 verdict 非 not_needed 时有意义）：
   - 源码文件数 >5 且仓龄 >90 天（存量项目）→ `openspec-opsx`，entrySkills = `["openspec init"]`
   - 近乎空仓/新项目（源码少或仓龄短）→ `spec-kit`，entrySkills = `["specify init"]`

打分维度权重不变：

| 维度 | 权重 |
|------|------|
| 大型代码库 (>50,000 行) | +40 |
| 中型代码库 (10,000-50,000 行) | +25 |
| 较小代码库 (5,000-10,000 行) | +10 |
| 存在 design.md | +20 |
| 存在 architecture.md | +15 |
| 存在 specs/ 目录 | +25 |
| 存在安全审查文件 | +25 |
| 存在 ADR 文档 | +15 |
| 存在 constitution.md | +20 |
| 多人协作 (>5人) | +25 |
| 少量协作 (>2人) | +15 |
| 成熟项目 (仓龄 >365 天) | +10 |
| 存在 CI/CD 配置 | +10~50（每种 CI 系统 +10，可叠加） |
| 有测试 | +5 / 无测试 -5 |

## 代码规模扫描

- 扫描范围：仓库根目录递归，排除 `node_modules/ .git/ target/ dist/ build/ vendor/ venv/ .venv/ __pycache__/`
- 扩展名：`.ts .tsx .js .jsx .rs .py .go .ps1 .psm1 .cs .java .kt .swift .vue .svelte .v`
- 不再局限于 `src/lib/app/packages/crates` 五个白名单目录，避免像 `tools/` 下的脚本被漏统计

## 集成到 Code Intel Pipeline

### 参数

```powershell
-SkipOpenSpec    # 跳过三栈检测器（保留历史参数名/语义）
-AutoOpenSpec    # 自动模式，不询问用户（本检测器本身不做交互提示）
```

### 输出位置

- `summary.md` — "Workflow Stack Recommendations" 段落，三行式（每栈一行：栈名/verdict/入口技能），并额外输出 `spec-driven brief`，包含 recommended/confidence/do first/guardrails/acceptance。
- `report.json` — `workflows[].recommendationBrief` 承载推荐简报；旧兼容 `openSpec.recommendationBrief` 同步 spec-driven 层结果。
- `recommendationBrief` 字段：`recommended`、`verdict`、`confidence`、`why[]`、`whyNot[]`、`doFirst[]`、`doNotDoYet[]`、`fallback`、`acceptance[]`、`sourceMethod`。
- 简报吸收 `EternallLight/improving-ai-agent-openspec` 的方法论：PRD 分解、阶段计划、需求覆盖、验收测试、done criteria。它是推荐/治理说明，不是运行时依赖。
- `report.json`
  - `openSpec` 块 — 向后兼容，内容等同 `workflows` 数组中 `stack=spec-driven` 的一项
  - `workflows` 数组 — 三层完整判定：`{stack, tool?, verdict, score?, reasons[], entrySkills[]}`
- `notes` 数组 — spec-driven 层判定摘要

### 独立脚本

`OpenSpec-Detector.ps1` 是同一套逻辑的独立可执行版本（不依赖 pipeline），供单独跑三栈检测：

```powershell
.\OpenSpec-Detector.ps1 -RepoPath <path> -Auto
```

> ⚠️ 实现同步提醒：`run-code-intel.ps1`（内联函数 `Get-CodeMetrics` / `Get-GovernanceIndicators` / `Get-SpecDrivenRecommendation` / `Get-MattFlowRecommendation` / `Get-GstackRecommendation` / `Invoke-WorkflowStackDetector` 等）与独立的 `OpenSpec-Detector.ps1` 是两份重复实现，改动其中一份逻辑时务必同步另一份。

## 下一步

1. 视需要把 matt-flow / gstack 的特征探测再细化（例如更精确的部署迹象识别）
2. spec-driven 层目前仅推荐，不做 `openspec init` / `specify init` 的自动调用；如需深集成需另行设计
