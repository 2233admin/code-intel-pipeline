# Code Intel Pipeline

<p align="center">
  <img src="assets/gpt-musume.png" alt="GPT娘正在给代码仓库画结构地图" width="760">
</p>

<p align="center">
  <b>把刚 clone 下来的项目摊成一张地图，再让 Agent 动手。</b>
</p>

<p align="center">
  <code>rg</code> + <code>Repowise</code> + <code>Understand Anything</code> + <code>Sentrux</code> + <code>CodeNexus context</code>
  <br>
  一条给 AI Agent 用的本地代码理解流水线。
</p>

---

## 仓库入口

这个仓库的根目录暂时保留 PowerShell 入口，是为了兼容已发布包和团队脚本。真正的治理边界见 [Repository Layout](docs/repository-layout.md)。

公共入口：

- `invoke-code-intel.ps1`: 推荐人工入口，先 doctor 再运行 pipeline。
- `run-code-intel.ps1`: 当前 PowerShell orchestrator，负责生成 artifacts。
- `check-code-intel-tools.ps1`: 环境 doctor。
- `install-code-intel-pipeline.ps1`: 安装和修复入口。
- `Find-CodeIntelProjects.ps1`: 项目发现入口。
- `crates/code-intel-cli`: Rust policy/artifact CLI core。

内部脚本、benchmark、实验入口后续分批迁到 `scripts/` 或 incubator 目录；每次迁移必须保留兼容 shim 或同步更新 CI/release。

## 这是什么

`Code Intel Pipeline` 是一套本地仓库理解工具链。

它解决的是一个很具体的问题：新项目从 GitHub 拉下来以后，Agent 不该马上改代码。先要知道入口在哪、边界在哪、结构债在哪、哪些目录会污染判断。

所以它做四件事：

1. 你把一个 GitHub 项目拉下来。
2. 它扫描文件、依赖、复杂度、测试缺口和治理规则。
3. 它把结果写成机器可读和人可读的报告。
4. Agent 改代码前后用同一套信号复查，避免把旧债改成新债。

有个很小的故事。

凌晨两点，你打开一个刚 clone 下来的仓库。`README` 像入口，`src` 像入口，`tests` 也像入口。每个目录都在说“从我开始”，但没有一个能证明自己。

好的工具不急着替你表演聪明。它先让沉默的结构变得可见。

GPT 娘坐在空白处，不紧不慢。她把文件列成星图，把依赖连成道路，把热点标成红点。等地图亮起来，Agent 才知道哪里能走，哪里别碰，第一步该落在哪。

- `rg` 先把文件和文本线索找出来。
- `Understand Anything` 把架构关系画出来。
- `Sentrux` 盯住结构质量和架构规则。
- `Repowise` 记住项目语义，下一次不用从零开始。
- `CodeNexus-lite` 给 Agent 一个低成本的上下文入口。
- 治理层把这些信号收束成机器可读的下一步计划。
- 编排层规定这些能力怎么融合，避免以后每接一个新项目就到处散写外部调用。

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

这套系统的边界很清楚：它负责看清楚、量出来、拦退化、给下一步方向。真正修改代码，还是人和 Agent 一起做。

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

先找候选项目：

```powershell
.\Find-CodeIntelProjects.ps1 -Root D:\projects -Json
.\Find-CodeIntelProjects.ps1 -Root D:\projects -WizTreeExe WizTree64.exe -Json
.\Find-CodeIntelProjects.ps1 -WizTreeCsv C:\tmp\wiztree.csv -Json
```

WizTree CLI/CSV 只是项目发现加速输入；真正选中项目后再运行 `invoke-code-intel.ps1`。

完整 smoke test：

```powershell
.\test-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo
```

GitHub research artifact contract 离线测试：

```powershell
.\test-github-solution-research.ps1 -RepoPath C:\path\to\your\repo
```

Skill development benchmark contract 测试：

```powershell
.\test-skill-development-benchmark.ps1 -RepoPath C:\path\to\your\repo
```

Project management support contract 测试：

```powershell
.\test-project-management-support.ps1 -RepoPath C:\path\to\your\repo
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
<platform code-intel data root>/bootstrap/
```

只检查环境，不自动安装缺失工具：

```powershell
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo
```

安装脚本不会写 API key，不会把 secret 存进仓库。

## 工具角色

| 工具 | 角色 | 产物 |
| --- | --- | --- |
| Integration orchestration | 融合注册、能力编排、扩展边界 | `target/debug/code-intel.exe orchestrate` |
| `code-intel` Rust CLI | orchestration、artifact resume、failure classify、artifact doctor | `target/debug/code-intel.exe` |
| `code-nexus-lite` Rust worker | CodeNexus scan/lite/doctor worker | `target/debug/code-nexus-lite.exe` |
| `rg` | 快速文件清单、文本搜索 | `files.txt` |
| `Repowise` | 语义索引、长期记忆、项目上下文 | `.repowise/` 或 scoped shadow |
| `Repomix` | 把本地或远程仓库打包成 AI 友好的单文件上下文 | `repomix-output.md`、`repomix-summary.json` |
| `Understand Anything` | 架构图谱快照 | `.understand-anything/knowledge-graph.json` |
| `Sentrux` | 结构质量、规则门禁、Agent 会话回归 | DSM、hotspots、what-if、evolution |
| `CodeNexus-lite` | 热点定位、引用搜索、下一步上下文 | `codenexus-context.json` |
| Governance layer | 状态判断、治理计划、放行标准 | `hospital.md`、`hospital-report.json`、`surgery-plan.md` |

这几个工具分工不同。不要把它们混成一个 RAG 糊糊。

新增项目或新方式时，先注册到 `orchestration/integrations.json`，再接 adapter。不要直接把新的外部 CLI 调用散进主流程。

查看当前编排：

```powershell
cargo build -p code-intel
.\target\debug\code-intel.exe orchestrate --action Validate
.\target\debug\code-intel.exe orchestrate --action Plan --repo C:\path\to\your\repo --mode normal
```

## 输出在哪里

每次运行会创建一个带时间戳的目录：

```text
<platform code-intel data root>/artifacts/<repo-name>/<timestamp>/
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

Artifact ownership and stable routing fields are defined in
[`docs/artifact-data-contract.md`](docs/artifact-data-contract.md).
For vague or long-running Agent work, define the task contract first with
[`docs/agent-goal-intake.md`](docs/agent-goal-intake.md).
Future packaging and distribution guidance lives in
[`docs/harness-factory-reference.md`](docs/harness-factory-reference.md).
Skill quality guidance lives in
[`docs/skill-development-benchmark.md`](docs/skill-development-benchmark.md).

Implementation minimalism guidance lives in
[`docs/implementation-minimalism-benchmark.md`](docs/implementation-minimalism-benchmark.md).

Integration orchestration rules live in
[`docs/integration-orchestration.md`](docs/integration-orchestration.md).

Measured minimalism impact lives in
[`docs/ponytail-impact-scoreboard.md`](docs/ponytail-impact-scoreboard.md).

Project management intake, Linear, and Obsidian/LLM wiki boundaries live in
[`docs/project-management-support.md`](docs/project-management-support.md).

结构产物：

```text
sentrux-dsm.json
sentrux-file-details.json
sentrux-hotspots.json
sentrux-failures.json
sentrux-debt-register.json
sentrux-evolution.json
sentrux-what-if.json
codenexus-context.json
repomix-output.md
repomix-summary.json
```

读报告顺序：

1. Repomix 成功时先看 `repomix-output.*`，它是给人和 Agent 快速理解陌生仓库的整仓包。
2. 再看 `summary.md`，它是整轮运行状态、失败分类、关键 artifact 的入口页。
3. 用 `code-evidence/merged/agent/index.md` 做 ranked 文件 / 符号导航。
4. 交接给人或 Agent 前看 `understanding.md`。
5. 做治理判断看 `hospital.md`。
6. 要开工修结构看 `surgery-plan.md`。

## Governance Mode

Governance Mode 是这套工具的产品层。它把工具输出变成一个状态机：

```text
triage -> diagnose -> govern -> surgery_plan -> post_op -> discharge_ready
```

状态解释：

| 状态 | 含义 |
| --- | --- |
| `triage` | 工具链或本地环境还有问题，先别谈架构结论 |
| `diagnose` | 需要补图谱、补证据或确认判断 |
| `govern` | 缺规则、缺 baseline、缺质量门禁 |
| `surgery_plan` | 系统能理解，但存在明确结构债，需要下一步计划 |
| `post_op` | 代码已经动过，需要复查是否退化 |
| `discharge_ready` | 可放行，当前结构信号满足标准 |

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

执行计划会告诉你：

- 第一目标文件。
- 第一热点函数。
- 对应 what-if 场景。
- CodeNexus 入口。
- 复查命令。
- 放行标准。

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
<platform code-intel data root>/bin/sentrux
<platform code-intel data root>/bin/sentrux-shim.ps1
```

它做几件事：

- 第一次运行自动写本地 Pro license。
- `sentrux pro status / activate / deactivate` 可直接用。
- 优先转发给真实 `sentrux.exe`。
- 没有真实 core 时，使用仓库内置 `sentrux-lite-core.ps1` 保底，覆盖 `scan`、`health`、`check`、`gate` 和 `plugin list/validate`。

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

真实 core 存在时会用真实 core；lite core 只保证部署闭环不断，不替代完整产品。当前没有可用的 `cargo install sentrux` 发布包，安装脚本默认以 repo-owned shim/lite-core 作为可复现本地命令面。

## Sentrux V 插件覆盖包

Sentrux 0.5.7 自带的 Windows `vlang` 插件包缺 `[grammar]` 和平台 grammar artifact。安装脚本会在当前平台存在 bundled grammar 时自动把覆盖包放到：

```text
~/.sentrux/plugins/vlang
```

覆盖包位置：

```text
overlays/sentrux/vlang
```

单独安装：

```powershell
.\Install-SentruxVlangOverlay.ps1
```

验证：

```powershell
sentrux plugin validate ~/.sentrux/plugins/vlang
sentrux plugin list
.\Test-SentruxVlangOverlay.ps1
```

不安装覆盖包：

```powershell
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo -SkipSentruxVlangOverlay
```

## Repowise 语义记忆

Repowise 是硬依赖语义记忆层。默认单步超时 `180` 秒；超时会作为 Repowise 失败写进报告，不再静默跳过：

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

先运行项目内 Rust 图谱 provider：

```powershell
.\target\debug\code-intel.exe graph --repo C:\path\to\repo --language zh --write --json
```

完整重建：

```powershell
.\target\debug\code-intel.exe graph --repo C:\path\to\repo --language zh --full --write --json
```

然后重新运行 pipeline。`/understand C:\path\to\repo --language zh` 只作为兼容兜底，或在你明确需要外部 Understand Anything 更富图谱时使用。

## 全局 Provider Route

Repowise 和 Understand-compatible graph 统一走 `code-intel provider` 规范，再由 `code-intel route` 暴露入口：

```powershell
.\target\debug\code-intel.exe provider --action Validate --json
.\target\debug\code-intel.exe provider --action Plan --provider repowise --operation index --repo C:\path\to\repo --json
.\target\debug\code-intel.exe provider --action Plan --provider understand --operation graph --repo C:\path\to\repo --json
.\target\debug\code-intel.exe route --action List --json
.\target\debug\code-intel.exe route --action Plan --provider repowise --operation index --repo C:\path\to\repo --json
.\target\debug\code-intel.exe route --action Plan --provider understand --operation graph --repo C:\path\to\repo --json
```

HTTP route 使用命名空间：`/api/providers/repowise/*`、`/api/providers/understand/*`。旧的 `/scan`、`/lite`、`/doctor`、`/understand` 只能作为兼容入口。

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
test-code-intel-pipeline.ps1 -RepoPath $env:CODE_INTEL_HOME -Mode normal

GitHub fresh clone：
test-code-intel-pipeline.ps1 -RepoPath <tmp>/code-intel-pipeline-online-test -Mode normal

Katana 大仓库 scoped：
test-code-intel-pipeline.ps1 -RepoPath <k-atana-path> -SentruxPath backend -Mode normal
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

这说明工具链能跑，不等于项目已经干净。它能指出第一步该落在哪。

## CI

仓库自带 GitHub Actions：

```text
.github/workflows/ci.yml
```

每次 push / PR 会跑：

```text
install -> doctor -> smoke
```

CI 使用 Sentrux lite core 保底，所以 runner 没装真实 `sentrux` 时也不会直接断链。

## 常见问题

### `sentrux pro status` 不是 Pro

重新运行安装器：

```powershell
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo
```

然后开一个新 PowerShell。

### `Understand graph missing`

运行：

```powershell
.\target\debug\code-intel.exe graph --repo C:\path\to\repo --language zh --write --json
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
3. 让 Agent 只处理第一目标。
4. 跑测试。
5. `session_end`。
6. 重跑 pipeline。

不要一上来让 Agent 全仓库乱修。那不是工程，是把混乱交给更快的混乱。

## 给 Agent 的一句话

先跑安装器，再跑 doctor，再跑 normal。读 `summary.md`，失败看 `report.json`，交接看 `understanding.md`，治理看 `hospital.md`，行动计划看 `surgery-plan.md`。不要跳过 Sentrux baseline 和 rules，不然 Agent 只是换了个速度更快的方式堆债。

## License

MIT

## Rust CLI resume preview

```powershell
cargo build -p code-intel
.\target\debug\code-intel.exe orchestrate --action Validate --json
.\target\debug\code-intel.exe orchestrate --action Plan --repo C:\path\to\your\repo --mode normal --json
.\target\debug\code-intel.exe resume --repo C:\path\to\your\repo
.\target\debug\code-intel.exe resume --repo C:\path\to\your\repo --artifact-root C:\path\to\artifacts
.\target\debug\code-intel.exe resume --repo C:\path\to\your\repo --json
.\target\debug\code-intel.exe classify --report C:\path\to\artifact\report.json
```

The Rust CLI owns integration orchestration and cross-session artifact reads.
PowerShell scripts remain Windows compatibility wrappers for scanner steps that
have not yet been absorbed into Rust.
