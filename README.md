# Code Intel Pipeline

这是一个本地代码仓库理解流水线。目标很简单：

1. 拉一个 GitHub 项目下来。
2. 快速知道这个项目有多少文件、结构干不干净、哪里复杂、哪里缺测试。
3. 给 AI Agent 一个能复用的结构反馈回路，而不是每次都从零瞎猜。

它把几个工具串起来：

- `rg`：精确文件清单和文本搜索。
- `repowise`：语义索引和长期项目记忆。
- `Understand Anything`：项目图谱快照。
- `sentrux`：架构规则、结构质量门禁、Agent 会话回归检查。

## 一键部署

Windows PowerShell 里运行：

```powershell
git clone https://github.com/2233admin/code-intel-pipeline.git
cd code-intel-pipeline
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo -RepairSkillLinks -InstallMissing
```

更傻瓜的一条命令：

```powershell
.\bootstrap-new-machine.ps1 -RepoPath C:\path\to\your\repo
```

它会连续跑安装、doctor、smoke test，并把结果写到：

```text
%LOCALAPPDATA%\code-intel\bootstrap\
```

如果你只想检查环境，不想自动装缺失工具：

```powershell
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo
```

如果要给 MiniMax / repowise 文档生成用模型服务，先在用户环境变量里配：

```text
ANTHROPIC_BASE_URL=https://api.minimaxi.com/anthropic
REPOWISE_PROVIDER=anthropic
ANTHROPIC_API_KEY=<你的 key>
ANTHROPIC_AUTH_TOKEN=<你的 key>
```

安装脚本不会写入 API key，也不会把 secret 存进仓库。

## Sentrux 自动激活

`sentrux` 是 MIT/开源项目，本仓库会自动安装一个很薄的 shim：

```text
%LOCALAPPDATA%\code-intel\bin\sentrux.cmd
%LOCALAPPDATA%\code-intel\bin\sentrux-shim.ps1
```

它做三件事：

- 第一次运行自动写本地 Pro license。
- `sentrux pro status / activate / deactivate` 可以直接用。
- 其他命令优先转发给真实的 `sentrux.exe`。
- 如果机器上没有真实 core，会自动启用仓库内置的 `sentrux-lite-core.ps1`，保底支持 `scan / health / check / gate`。

也就是说，新机器部署后应该能直接看到：

```powershell
sentrux pro status
```

输出类似：

```text
Tier: pro
Status: active
Features: dsm_export, file_detail_panel, evolution_details, what_if_analysis, agent_mcp, rule_gates, nine_color_modes
```

如果你明确想关掉自动 Pro：

```powershell
sentrux pro deactivate
```

重新打开：

```powershell
sentrux pro activate OSS-LOCAL-PRO
```

如果某台机器不想自动激活，设置：

```powershell
$env:SENTRUX_AUTO_PRO = "0"
```

真实 Sentrux core 存在时，shim 会自动用真实 core；不存在时用 lite core。lite core 的作用是保证部署闭环不断，不是替代完整产品。

## Sentrux V 插件覆盖包

Sentrux 0.5.7 自带的 Windows `vlang` 插件包是坏的：缺 `[grammar]`，也缺 `windows-x86_64.dll`。安装脚本会自动把仓库里的覆盖包装到：

```text
%USERPROFILE%\.sentrux\plugins\vlang
```

覆盖包在 `overlays\sentrux\vlang`，包含：

- `plugin.toml`
- `queries\tags.scm`
- `grammars\windows-x86_64.dll`

单独重装：

```powershell
.\Install-SentruxVlangOverlay.ps1
```

验证：

```powershell
sentrux plugin validate $env:USERPROFILE\.sentrux\plugins\vlang
sentrux plugin list
.\Test-SentruxVlangOverlay.ps1
```

正常结果里应该能看到：

```text
vlang v0.2.0 [v] — V
```

如果你明确不想安装这个覆盖包：

```powershell
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo -SkipSentruxVlangOverlay
```

## 最常用命令

先跑 doctor：

```powershell
.\check-code-intel-tools.ps1 -RepoPath C:\path\to\your\repo
```

跑一次正常理解：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\your\repo -Mode normal
```

Repowise 是可选语义记忆层，默认单步超时 `180` 秒；超时会跳过 Repowise 并继续跑 Understand / Sentrux / CodeNexus，不会拖死整轮：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\your\repo -Mode normal -RepowiseTimeoutSeconds 60
```

稳定入口：

```powershell
.\invoke-code-intel.ps1 -RepoPath C:\path\to\your\repo -Mode normal
```

跑完整 smoke test：

```powershell
.\test-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo
```

如果项目很脏、很多 vendor/research/tools 目录，只检查核心目录：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\your\repo -Mode normal -SentruxPath backend
```

根目录可以扫，但治理指标会默认排除顶层 `tools/`、`vendor/`、`third_party/`、`external/`。这些目录通常是外部轮子、镜像源码或参考实现；如果要分析它们，把 scope 直接指过去，不要让它们污染主项目分数。

## Agent 工作流

Agent 开始改代码前：

```powershell
.\Invoke-SentruxAgentTool.ps1 session_start C:\path\to\your\repo\backend
```

Agent 改完代码后：

```powershell
.\Invoke-SentruxAgentTool.ps1 session_end C:\path\to\your\repo\backend
```

如果结构质量下降，`session_end` 会返回失败，告诉你分数前后变化。

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

也支持 Agent/MCP 风格别名：

```text
sentrux_scan
sentrux_health
sentrux_dsm
sentrux_git_stats
sentrux_test_gaps
```

常看三个：

```powershell
.\Invoke-SentruxAgentTool.ps1 dsm C:\path\to\repo\backend
.\Invoke-SentruxAgentTool.ps1 git_stats C:\path\to\repo\backend
.\Invoke-SentruxAgentTool.ps1 evolution C:\path\to\repo\backend
.\Invoke-SentruxAgentTool.ps1 what_if C:\path\to\repo\backend
```

它们分别回答：

- `dsm`：结构地图、9 种颜色模式、文件详情、函数复杂度。
- `git_stats`：churn、age、dirty/untracked、作者分布、bus factor 风险。
- `evolution`：热点、耦合、bus factor、历史趋势。
- `what_if`：如果规则收紧，会卡住哪些模块/文件/函数。

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
```

检查规则：

```powershell
sentrux check C:\path\to\repo\backend
```

保存第一次 baseline：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal -SentruxPath backend -SaveSentruxBaseline
```

## 输出在哪里

每次运行会生成一个目录：

```text
%LOCALAPPDATA%\code-intel\artifacts\<repo>\<timestamp>\
```

先看：

```text
summary.md
```

机器读：

```text
report.json
```

给人/Agent 交接：

```text
understanding.md
```

医院报告：

```text
hospital.md
hospital-report.json
```

Sentrux 结构产物：

```text
sentrux-dsm.json
sentrux-file-details.json
sentrux-hotspots.json
sentrux-evolution.json
sentrux-what-if.json
codenexus-context.json
```

`codenexus-context.json` 是自动生成的 CodeNexus-lite 上下文：热点文件、近期提交、引用搜索、下一步查询建议。它解决的是“Agent 下一步该看哪里”，不是只在报告里写一句空建议。

`hospital-report.json` 是 Code Intel Pipeline 的医院层。它把工具输出重新组织成检查分型、报告质量、诊断、治疗方案和复查协议：

- `xray`：`rg` 文件清单，快，看项目骨架。
- `anatomy`：Understand 图谱，看系统解剖结构。
- `ct`：Sentrux DSM、热点、文件/函数详情，看结构切片。
- `mri`：CodeNexus-lite 上下文，看影响面和下一步定位。
- `pet`：Sentrux evolution / what-if / test gap 代理真实执行风险。
- `chart`：Repowise 项目病历和长期语义记忆。
- `governance`：Sentrux rules + gate，执行架构院规。

`hospital.md` 给人读，`hospital-report.json` 给 Agent/CI 读。核心字段是 `triage.disposition`、`triage.primary_diagnosis`、`triage.overall_score` 和 `triage.next_protocol`。

`triage.disposition` 是总处置：

- `admit`：住院。还有结构病灶、缺图谱、缺规则、门禁失败或待排期手术。
- `observe`：留观。可接受但需要复查。
- `discharge_ready`：可出院。规则、门禁、诊断和术后复查都满足放行标准。

完整协议见 `docs/hospital-mode.md`。

## 模式

- `lite`：只做清单和环境检查。
- `normal`：常规理解，推荐默认。
- `full`：要求重新生成完整 Understand 图谱命令。

如果需要 repowise 生成 wiki 文档：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo -Mode normal -RepowiseDocs
```

如果 provider 限流，报告里会显示 `provider_quota`，这不是本地脚本坏了。

## 常见问题

### `sentrux pro status` 不是 Pro

重新运行安装：

```powershell
.\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo
```

然后开一个新 PowerShell。

### `Understand graph missing`

在 Claude / 支持技能的 Agent 里运行：

```text
/understand C:\path\to\repo --language zh --full
```

再重新跑 pipeline。

### repo 太大、vendor 太多

根目录可以扫。Sentrux 会自动把 `node_modules`、`dist`、`build`、`static/assets` 这类依赖或构建产物隔离出治理图；只有要给核心模块单独打 baseline 时，才指定 scope：

```powershell
.\run-code-intel.ps1 -RepoPath C:\path\to\repo -SentruxPath backend -Mode normal
```

## 给 Agent 的一句话

先跑安装器，再跑 doctor，再跑 normal。读 `summary.md`，如果失败看 `report.json`，如果要交接看 `understanding.md`。不要跳过 Sentrux baseline 和 rules，否则 Agent 只是在高速制造结构债。

## CI

仓库自带 GitHub Actions：

```text
.github/workflows/ci.yml
```

每次 push / PR 会在 Windows runner 上跑：

```text
install -> doctor -> smoke
```

CI 使用 Sentrux lite core 保底，所以不会因为 runner 没装真实 `sentrux.exe` 直接完蛋。
