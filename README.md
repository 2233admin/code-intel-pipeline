# Code Intel Pipeline

<p align="center">
  <img src="assets/gpt-musume.png" alt="GPT娘正在给代码仓库写结构诊断板" width="760">
</p>

<p align="center">
  <b>把刚 clone 下来的项目先看清楚，再让 Agent 动手。</b>
</p>

<p align="center">
  <code>rg</code> + <code>Repowise</code> + <code>Understand Anything</code> + <code>Sentrux</code> + <code>CodeNexus-lite</code>
  <br>
  一条给 AI Agent 用的本地代码理解流水线。
</p>

---

## 这是什么

`Code Intel Pipeline` 是一套本地仓库理解工具链。

它解决的是一个很具体的问题：新项目从 GitHub 拉下来以后，Agent 不该马上改代码。先要知道入口在哪、边界在哪、结构债在哪、哪些目录会污染判断。

所以它做四件事：

1. 你把一个 GitHub 项目拉下来。
2. 它扫描文件、依赖、复杂度、测试缺口和治理规则。
3. 它把结果写成机器可读和人可读的报告。
4. Agent 改代码前后用同一套信号复查，避免把旧债改成新债。

这里有一个医院隐喻，但不是卖萌。代码仓库像病人，第一步不是开刀，是拍片：

- `rg` 是快速 X 光，先数清骨头。
- `Understand Anything` 是解剖图谱，看系统器官怎么摆。
- `Sentrux` 是 CT + 门禁，盯结构退化和架构规则。
- `Repowise` 是长期病历，把项目语义记住。
- `CodeNexus-lite` 是 MRI 定位，告诉 Agent 下一步该看哪里。
- `hospital-report.json` 是会诊报告。
- `surgery-plan.md` 是第一张手术单。

看板娘只是提醒一件事：先诊断，再治疗。别让 Agent 闭眼冲进陌生仓库。

## 适合谁

适合这些场景：

- 你刚 clone 一个陌生项目，想知道它到底怎么长的。
- 你要让 Codex、Claude、OpenAI Agent 或其他 AI 工具接手代码。
- 你不想每次都靠 Agent 自己猜上下文。
- 你想在改代码前后检查结构质量有没有下降。
- 你有一个大仓库，根目录里塞了 `tools/`、`vendor/`、研究代码、外部轮子，不想它们污染核心指标。

不适合这些场景：

- 想把所有代码一次性丢给 LLM 总结。
- 只要漂亮 wiki，不关心结构门禁。
- 希望工具自动替你重构全部代码。

这套系统的边界很清楚：它负责看清楚、量出来、拦退化、给手术方向。真正动刀还是人和 Agent 一起做。

## 一分钟上手

Windows PowerShell：

```powershell
git clone https://github.com/2233admin/code-intel-pipeline.git
cd code-intel-pipeline
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo -RepairSkillLinks -InstallMissing
.\check-code-intel-tools.ps1 -RepoPath C:\path\to\your\repo
.\run-code-intel.ps1 -RepoPath C:\path\to\your\repo -Mode normal
```

稳定入口：

```powershell
.\invoke-code-intel.ps1 -RepoPath C:\path\to\your\repo -Mode normal
```

完整 smoke test：

```powershell
.\test-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo
```

大仓库建议指定核心范围：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\your\repo -Mode normal -SentruxPath backend
```

## 新机器部署

最省心：

```powershell
.\bootstrap-new-machine.ps1 -RepoPath C:\path\to\your\repo
```

它会连续执行：

```text
install -> doctor -> smoke test
```

结果写到：

```text
%LOCALAPPDATA%\code-intel\bootstrap\
```

只检查环境，不自动安装缺失工具：

```powershell
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo
```

安装脚本不会写 API key，不会把 secret 存进仓库。

## 工具角色

| 工具 | 角色 | 产物 |
| --- | --- | --- |
| `rg` | 快速文件清单、文本搜索 | `files.txt` |
| `Repowise` | 语义索引、长期记忆、病历 | `.repowise/` 或 scoped shadow |
| `Understand Anything` | 架构图谱快照 | `.understand-anything/knowledge-graph.json` |
| `Sentrux` | 结构质量、规则门禁、Agent 会话回归 | DSM、hotspots、what-if、evolution |
| `CodeNexus-lite` | 热点定位、引用搜索、下一步上下文 | `codenexus-context.json` |
| Hospital layer | 诊断、处置、出院标准 | `hospital.md`、`hospital-report.json`、`surgery-plan.md` |

这几个工具分工不同。不要把它们混成一个 RAG 糊糊。

## 输出在哪里

每次运行会创建一个带时间戳的目录：

```text
%LOCALAPPDATA%\code-intel\artifacts\<repo-name>\<timestamp>\
```

核心报告：

```text
summary.md
report.json
understanding.md
hospital.md
hospital-report.json
surgery-plan.md
surgery-plan.json
```

结构产物：

```text
sentrux-dsm.json
sentrux-file-details.json
sentrux-hotspots.json
sentrux-evolution.json
sentrux-what-if.json
codenexus-context.json
```

读报告顺序：

1. 先看 `summary.md`，它是急诊分诊牌。
2. 失败时看 `report.json`，它有完整步骤和失败分类。
3. 交接给人或 Agent 前看 `understanding.md`。
4. 做治理判断看 `hospital.md`。
5. 要开工修结构看 `surgery-plan.md`。

## Hospital Mode

Hospital Mode 是这套工具的产品层。它把工具输出变成一个诊疗流程：

```text
triage -> diagnose -> govern -> surgery_plan -> post_op -> discharge_ready
```

状态解释：

| 状态 | 含义 |
| --- | --- |
| `triage` | 工具链或本地环境还有问题，先别谈架构结论 |
| `diagnose` | 需要补图谱、补证据或确认诊断 |
| `govern` | 缺规则、缺 baseline、缺质量门禁 |
| `surgery_plan` | 系统能理解，但存在明确结构债，应该排手术 |
| `post_op` | 代码已经动过，需要复查是否退化 |
| `discharge_ready` | 可出院，当前结构信号满足放行标准 |

`hospital-report.json` 给 Agent/CI 读，关键字段：

```text
triage.status
triage.disposition
triage.primary_diagnosis
triage.overall_score
triage.next_protocol
state_machine.current_state
state_machine.transitions
report_quality.dimensions
treatment.plan
```

当 `next_protocol = surgery_plan` 时，会生成：

```text
surgery-plan.md
surgery-plan.json
```

手术单会告诉你：

- 第一手术目标文件。
- 第一热点函数。
- 对应 what-if 场景。
- CodeNexus 入口。
- 复查命令。
- 出院标准。

完整协议见：

```text
docs/hospital-mode.md
```

## Agent 工作流

Agent 开始改代码前：

```powershell
.\Invoke-SentruxAgentTool.ps1 session_start C:\path\to\repo\backend
```

Agent 改完代码后：

```powershell
.\Invoke-SentruxAgentTool.ps1 session_end C:\path\to\repo\backend
```

如果结构质量下降，`session_end` 会失败，并返回前后分数。

可用工具：

```text
scan
health
session_start
session_end
rescan
check_rules
evolution
dsm
git_stats
test_gaps
what_if
```

也支持 MCP/Agent 风格别名：

```text
sentrux_scan
sentrux_health
sentrux_dsm
sentrux_git_stats
sentrux_test_gaps
```

常用命令：

```powershell
.\Invoke-SentruxAgentTool.ps1 health C:\path\to\repo\backend
.\Invoke-SentruxAgentTool.ps1 dsm C:\path\to\repo\backend
.\Invoke-SentruxAgentTool.ps1 evolution C:\path\to\repo\backend
.\Invoke-SentruxAgentTool.ps1 what_if C:\path\to\repo\backend
```

别让 Agent 裸奔。没有 `session_start/session_end`，它改完代码以后自己也不知道有没有把结构弄坏。

## Sentrux 自动 Pro

`sentrux` 是 MIT/开源项目，本仓库会安装一个很薄的 shim：

```text
%LOCALAPPDATA%\code-intel\bin\sentrux.cmd
%LOCALAPPDATA%\code-intel\bin\sentrux-shim.ps1
```

它做几件事：

- 第一次运行自动写本地 Pro license。
- `sentrux pro status / activate / deactivate` 可直接用。
- 优先转发给真实 `sentrux.exe`。
- 没有真实 core 时，使用仓库内置 `sentrux-lite-core.ps1` 保底。

检查：

```powershell
sentrux pro status
```

预期：

```text
Tier: pro
Status: active
Features: dsm_export, file_detail_panel, evolution_details, what_if_analysis, agent_mcp, rule_gates, nine_color_modes
```

关闭自动 Pro：

```powershell
$env:SENTRUX_AUTO_PRO = "0"
sentrux pro deactivate
```

重新激活：

```powershell
sentrux pro activate OSS-LOCAL-PRO
```

真实 core 存在时会用真实 core；lite core 只保证部署闭环不断，不替代完整产品。

## Sentrux V 插件覆盖包

Sentrux 0.5.7 自带的 Windows `vlang` 插件包缺 `[grammar]` 和 `windows-x86_64.dll`。安装脚本会自动把覆盖包放到：

```text
%USERPROFILE%\.sentrux\plugins\vlang
```

覆盖包位置：

```text
overlays\sentrux\vlang
```

单独安装：

```powershell
.\Install-SentruxVlangOverlay.ps1
```

验证：

```powershell
sentrux plugin validate $env:USERPROFILE\.sentrux\plugins\vlang
sentrux plugin list
.\Test-SentruxVlangOverlay.ps1
```

不安装覆盖包：

```powershell
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo -SkipSentruxVlangOverlay
```

## Repowise 语义记忆

Repowise 是可选语义记忆层。默认单步超时 `180` 秒；超时会跳过，不拖死整轮：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal -RepowiseTimeoutSeconds 60
```

如果指定了 `-SentruxPath backend`，Repowise 会默认跟随同一 scope，避免把根目录里的外部轮子、临时文件、研究仓库一起吃进去。

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal -SentruxPath backend
```

如果想生成 Repowise wiki 文档：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal -RepowiseDocs
```

如果 provider 限流，报告会显示 `provider_quota`。这不是本地脚本坏了。

## Understand Anything 图谱

如果报告里出现：

```text
graph_missing: understand graph
```

在支持该技能的 Agent 中运行：

```text
/understand C:\path\to\repo --language zh
```

完整重建：

```text
/understand C:\path\to\repo --language zh --full
```

然后重新运行 pipeline。

## 规则文件

把模板复制到你的项目 scope：

```powershell
New-Item -ItemType Directory -Force C:\path\to\repo\backend\.sentrux
Copy-Item .\templates\sentrux-rules.example.toml C:\path\to\repo\backend\.sentrux\rules.toml
```

示例：

```toml
[constraints]
max_cycles = 0
max_coupling = "B"
max_cc = 25
no_god_files = true

[[layers]]
name = "core"
paths = ["src/core/*"]
order = 0

[[layers]]
name = "app"
paths = ["src/app/*"]
order = 2

[[boundaries]]
from = "src/app/*"
to = "src/core/internal/*"
reason = "App 不应依赖 core 内部实现"
```

保存 baseline：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal -SentruxPath backend -SaveSentruxBaseline
```

不要用新 baseline 掩盖真实退化。

## 大仓库怎么扫

根目录可以扫，但不总是该扫。

默认会把这些目录隔离出治理图：

```text
node_modules
dist
build
target
vendor
third_party
external
tools
```

如果你要治理核心模块，直接指定 scope：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo -SentruxPath backend -Mode normal
```

如果你要分析 `tools/` 里的某个外部轮子，把 scope 指到那个轮子，而不是让它污染主项目：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo\tools\some-lib -Mode normal
```

## 真实跑通过的路径

本仓库已经验证过这些路径：

```text
本项目完整链路：
test-code-intel-pipeline.ps1 -RepoPath D:\projects\_tools\code-intel-pipeline -Mode normal

GitHub fresh clone：
test-code-intel-pipeline.ps1 -RepoPath C:\tmp\code-intel-pipeline-online-test-20260530 -Mode normal

Katana 大仓库 scoped：
test-code-intel-pipeline.ps1 -RepoPath D:\projects\_quant\k-atana -SentruxPath backend -Mode normal
```

Katana 结果示例：

```text
failed=0
manualRequired=0
sentruxFail=0
localToolError=0
hospital.currentState=surgery_plan
primaryDiagnosis=known modernization debt
primaryTarget=simulate_engine
```

这说明工具链能跑，不等于项目已经出院。它能指出第一刀该落在哪。

## CI

仓库自带 GitHub Actions：

```text
.github/workflows/ci.yml
```

每次 push / PR 会跑：

```text
install -> doctor -> smoke
```

CI 使用 Sentrux lite core 保底，所以 runner 没装真实 `sentrux.exe` 时也不会直接断链。

## 常见问题

### `sentrux pro status` 不是 Pro

重新运行安装器：

```powershell
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo
```

然后开一个新 PowerShell。

### `Understand graph missing`

运行：

```text
/understand C:\path\to\repo --language zh
```

再重跑 pipeline。

### Repowise 很慢

先 scoped：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo -SentruxPath backend -Mode normal
```

再缩短超时：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo -SentruxPath backend -Mode normal -RepowiseTimeoutSeconds 60
```

### 报告显示 `surgery_plan`

这不是失败。这表示：

- 工具链能理解项目。
- 规则和门禁没有退化。
- 但 what-if 发现结构债。
- 应该读 `surgery-plan.md`，先修第一热点。

### 可以让 Agent 自动修吗

可以，但建议先这样：

1. 读 `surgery-plan.md`。
2. `session_start`。
3. 让 Agent 只处理第一手术目标。
4. 跑测试。
5. `session_end`。
6. 重跑 pipeline。

不要一上来让 Agent 全仓库乱修。那不是手术，是拿吸尘器进 ICU。

## 给 Agent 的一句话

先跑安装器，再跑 doctor，再跑 normal。读 `summary.md`，失败看 `report.json`，交接看 `understanding.md`，治理看 `hospital.md`，动刀看 `surgery-plan.md`。不要跳过 Sentrux baseline 和 rules，不然 Agent 只是换了个速度更快的方式堆债。

## License

MIT
