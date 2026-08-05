<h1 align="center">Code Intel Pipeline</h1>

<p align="center">
  <a href="README.en.md">English</a> · 简体中文
</p>

<p align="center">
  <b>Hand your coding agent a map of the repo before it edits.</b>
</p>

<p align="center">
  Local code-intelligence pipeline for AI coding agents — architecture maps, hotspots,
  change impact, and structural regression gates, produced entirely on your machine.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <a href="https://github.com/2233admin/code-intel-pipeline/releases"><img src="https://img.shields.io/github/v/release/2233admin/code-intel-pipeline?include_prereleases" alt="Latest release"></a>
</p>

<p align="center">
  <a href="#30-秒开始">Quick start</a> ·
  <a href="https://2233admin.github.io/code-intel-pipeline/demo/">Live demo</a> ·
  <a href="#这是什么">What is this</a> ·
  <a href="#适合谁">Who it's for</a> ·
  <a href="docs/public-beta.md">Public beta guide</a>
</p>

<p align="center">
  <img src="assets/gpt-musume.png" alt="GPT娘正在给代码仓库画结构地图" width="760">
</p>

<p align="center">
  <b>把刚 clone 下来的项目摊成一张地图，再让 Agent 动手。</b>
</p>

<p align="center">
  <code>rg</code> + <code>Repowise</code> + <code>Understand Anything</code> + <code>Sentrux</code> + <code>CodeNexus context</code>
  <br>
  一条给 AI Agent 用的本地代码理解管线。
</p>

---

## 30 秒开始

> **macOS / Linux 用户**：当前只支持源码构建安装——没有非 Windows 的 Release ZIP，`bootstrap.py` 也不支持非 Windows。请直接看 [macOS / Linux 快速开始](#macos--linux-快速开始)；本节以下命令默认 Windows。

在要分析的仓库目录中运行稳定入口；不传参数时默认分析当前目录：

```powershell
code-intel .
```

或显式指定仓库：

```powershell
code-intel C:\path\to\your\repo
```

首次安装、Skill 安装和依赖说明见[完整上手](#安装与完整上手)。

## macOS / Linux 快速开始

**支持等级**：对已发布的版本（v0.6.0 及更早），macOS / Linux 只支持源码构建安装——这些版本的 Release ZIP 和 `bootstrap.py` 引导只覆盖 Windows。从下一个 release 起，每个版本会同时发布 windows / macos / linux 三个 Release ZIP，`bootstrap.py` 引导在 macOS / Linux 上同样可用（详见 [Public beta guide](docs/public-beta.md)）。所有入口脚本都要求 PowerShell 7.2+（`pwsh`），README 里其余 `.ps1` 命令在 macOS / Linux 上同样用 `pwsh` 运行。

前置依赖（macOS，Homebrew）：

```bash
brew install --cask powershell
brew install ripgrep git rustup
rustup-init -y
```

前置依赖（Debian / Ubuntu）：

```bash
# PowerShell：Microsoft 官方仓库（或 sudo snap install powershell --classic）
wget -q "https://packages.microsoft.com/config/ubuntu/$(lsb_release -rs)/packages-microsoft-prod.deb"
sudo dpkg -i packages-microsoft-prod.deb
sudo apt-get update && sudo apt-get install -y powershell ripgrep git
# Rust toolchain（安装器不会代装 rustup/cargo）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

注意：`-InstallMissing` 只会通过 brew/apt/dnf/pacman 补装 `rg`、`git`、`python`；`rustup`/`cargo` 必须自己先装好——macOS / Linux 没有预编译二进制，安装器要现场 `cargo build`。

安装：

```bash
git clone https://github.com/2233admin/code-intel-pipeline.git
cd code-intel-pipeline
pwsh ./legacy/install-code-intel-pipeline.ps1 -RepoPath ~/src/your-repo -InstallMissing
```

安装器把编译好的 `code-intel` 复制到平台 bin 目录：

```text
macOS: ~/Library/Application Support/code-intel/bin
Linux: ~/.local/share/code-intel/bin   （设置了 XDG_DATA_HOME 时优先用它）
```

并写出 POSIX 环境文件 `~/.config/code-intel/env.sh`。按安装摘要末尾打印的那一行，把 `source` 追加进 shell 配置（zsh 用 `~/.zshrc`，bash 用 `~/.bashrc`），或手动把上面的 bin 目录加进 `PATH`：

```bash
echo 'source "$HOME/.config/code-intel/env.sh"' >> ~/.zshrc
```

重开 shell 后第一次运行：

```bash
code-intel ~/src/your-repo
```

## 语言方向与治理

> **语言方向：** 停止新增 PowerShell。现有 `.ps1` 只作为兼容入口维护，不再承载新产品逻辑；生产能力默认使用 Rust。MoonBit 可用于隔离实验，只有通过 artifact 契约一致性、跨平台构建和测试后才进入正式路径。迁移不做一次性重写，按入口逐个替换和退休。Agent 规则见 [AGENTS.md](AGENTS.md)。

Workflow-stack guidance is emitted by the read-only `advisory.workflow-recommend` atom. See [docs/advisory-workflow-recommendation.md](docs/advisory-workflow-recommendation.md); recommendations are proposals with zero effects and never authorize tool initialization or adoption.

Follow-up automation can proactively propose `/investigate` for actionable scan failures and can ask whether to enter the exact draft-PR flow. The one-command orchestrator composes proposal → user decision → C07 record/replay → fail-closed executor. It defaults to suggestion-on and PR-consent-required; neither path silently executes a skill or creates a PR. See [docs/follow-up-automation.md](docs/follow-up-automation.md).

## 仓库入口

编译后的 `code-intel` 是唯一正式入口：所有 Pipeline 语义都在它和 `crates/code-intel-cli` 里。真正的治理边界见 [Repository Layout](docs/repository-layout.md)。

**`legacy/` 是什么**：PowerShell 面的统一存放目录，**不等于「已退役」**。它混着两类东西——仍是唯一活路径的安装/恢复/门禁入口，和正在按 [#55](https://github.com/2233admin/code-intel-pipeline/issues/55) 逐个退役的兼容 facade。下面逐条标注，不要按目录名推断状态。

正式主入口：

- `code-intel`: 人工和 Agent 的正式主入口。新文档和命令示例一律以它开头。
- `crates/code-intel-cli`: Rust policy/artifact CLI core。

`legacy/` 中**仍然活跃、目前没有替代品**的入口：

- `legacy/install-code-intel-pipeline.ps1`: 安装和修复入口。源码安装（含 macOS / Linux）目前只有这一条路径。
- `legacy/code-intel.ps1`: PowerShell 7.2+ 恢复启动器；健康安装只转发，`-Update` 才显式更新。
- `legacy/Invoke-SentruxAgentTool.ps1`: 编码会话门禁入口（`session_start` / `session_end`），AGENTS.md 强制要求，退役前不可绕过。拆解见 [#50](https://github.com/2233admin/code-intel-pipeline/issues/50)。
- `legacy/Find-CodeIntelProjects.ps1`: 项目发现入口。
- `legacy/bootstrap-new-machine.ps1`: 新机器自举入口。

`legacy/` 中**已被取代或正在退役**的兼容 facade，新调用一律不要用：

- `legacy/check-code-intel-tools.ps1`: 已被原生 Rust doctor 取代（[#48](https://github.com/2233admin/code-intel-pipeline/issues/48) 已完成），保留仅为历史参考。请用 `code-intel doctor`。
- `legacy/run-code-intel.ps1`: 兼容 facade；不传开关时默认走旧扫描器分支，只有显式传 `-DagCoordinate` 才调用 Rust DAG 协调。收编进 `code-intel run` 见 [#47](https://github.com/2233admin/code-intel-pipeline/issues/47)。
- `legacy/invoke-code-intel.ps1`: v0.x 兼容转发器，不再推荐新调用。

PowerShell 合同测试在 `legacy/scripts/tests/`。`legacy/` 下的兼容 facade 仍被安装器、集成清单和 Rust 合同绑定：再次移动它们必须同时更新这些契约、重新冻结受影响的退役 packet 并重算 digest pin，不能只做文件移动。

## Public beta 范围

当前已发布兼容面仍是 **Windows PowerShell public beta**，但后续功能开发已经转向 Rust/MoonBit 路线，不再扩大 PowerShell surface。测试版承诺的是：稳定入口可运行、核心报告可事务化落盘、结构退化不会被伪装成成功、发布 ZIP 可在没有源码树和 Rust toolchain 的干净目录中启动。

0.4.0 核心路径：

- `code-intel` Primary Operator Entry
- `code-intel.exe` 的 A01-A09 capability/DAG/policy/artifact core
- `rg` inventory、native code evidence、内部 graph provider、真实 Sentrux `gate`/`check` 命令证据和 Hospital diagnosis
- snapshot-bound staging、A07 原子提交、A08 completed-only 索引、query/impact/freshness

默认包含但不阻塞测试版的增强能力：

- Repowise 语义索引与文档生成
- Understand Anything 图谱
- CodeNexus context、Repomix、模型辅助通道
- runtime/CI 证据和 file-boundary provider

缺少这些增强工具时，流水线必须明确记录 skipped、manual-required 或 provider failure，不能把它们冒充成功，也不能因此阻断只依赖核心路径的 public beta。详细支持矩阵和已知边界见 [Public beta guide](docs/public-beta.md)。

0.5.1 beta 还提供可重放的工具效果基线评分器。它不会把工具“能运行”误当成“对 Agent 有帮助”；只有冻结任务、配对条件和外部证明完整时才记录基线：

```powershell
code-intel benchmark tools --corpus <corpus.json> --runs <runs.json> --artifact-root <authority-root> --out <new-directory>
```

实验边界和当前可证明结论见 [Tool Effectiveness Baseline](docs/tool-effectiveness-baseline.md)。

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

## 安装与完整上手

Codex 可以先安装官方结构的 Skill 包，再由 Skill 下载并校验稳定版 Release：

```text
请使用 $skill-installer 安装：
https://github.com/2233admin/code-intel-pipeline/tree/main/skills/code-intel-pipeline

然后使用 $code-intel-pipeline 为 C:\path\to\your\repo 安装并运行稳定版。
```

Skill 默认只解析稳定版，校验 GitHub Release 提供的 SHA-256 后才解压。预发布版本和第三方依赖安装都需要显式选择。

人工用户从 GitHub Release 下载并解压安装包；Agent 用户通过 Skill 安装。macOS / Linux 没有 Release ZIP，走[macOS / Linux 快速开始](#macos--linux-快速开始)的源码构建路径。Windows 源码安装仍可使用：

```powershell
git clone https://github.com/2233admin/code-intel-pipeline.git
cd code-intel-pipeline
.\legacy\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo -RepairSkillLinks -InstallMissing
code-intel C:\path\to\your\repo
```

PowerShell 恢复入口：

```powershell
.\legacy\code-intel.ps1 C:\path\to\your\repo
.\legacy\code-intel.ps1 -Update
```

先找候选项目：

```powershell
.\legacy\Find-CodeIntelProjects.ps1 -Root D:\projects -Json
.\legacy\Find-CodeIntelProjects.ps1 -Root D:\projects -WizTreeExe WizTree64.exe -Json
.\legacy\Find-CodeIntelProjects.ps1 -WizTreeCsv C:\tmp\wiztree.csv -Json
```

WizTree CLI/CSV 只是项目发现加速输入；真正选中项目后再运行 `code-intel <path>`。

完整 smoke test：

```powershell
.\legacy/scripts/tests/test-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo
```

单元级回归测试（覆盖 fail-open / 假绿类修复 + fail-open lint，不依赖真实 repo，跑在临时目录里）：

```powershell
.\legacy/scripts/tests/test-regression-fixes.ps1 -VerboseOutput
```

GitHub research artifact contract 离线测试：

```powershell
.\legacy/scripts/tests/test-github-solution-research.ps1 -RepoPath C:\path\to\your\repo
```

Skill development benchmark contract 测试：

```powershell
.\legacy/scripts/tests/test-skill-development-benchmark.ps1 -RepoPath C:\path\to\your\repo
```

Project management support contract 测试：

```powershell
.\legacy/scripts/tests/test-project-management-support.ps1 -RepoPath C:\path\to\your\repo
```

从 GitHub Release ZIP 运行时，安装后直接使用编译入口；不需要 Cargo，也不依赖仓库里的 `target/`：

```powershell
code-intel C:\path\to\your\repo
```

只使用离线核心能力：

```powershell
code-intel C:\path\to\your\repo --mode lite
```

Greenfield 行为规格适配器测试：

```powershell
.\legacy/scripts/tests/test-greenfield-integration.ps1
```

普通用户直接运行主入口；兼容 runner 只保留给维护测试：

```powershell
code-intel C:\path\to\your\repo --mode normal
```

## 新机器部署

最省心：

```powershell
.\legacy\bootstrap-new-machine.ps1 -RepoPath C:\path\to\your\repo
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
.\legacy\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\your\repo
```

安装脚本不会写 API key，不会把 secret 存进仓库。

## 工具角色

| 工具 | 角色 | 产物 |
| --- | --- | --- |
| Integration orchestration | 融合注册、能力编排、扩展边界 | `target/debug/code-intel.exe orchestrate` |
| `code-intel` Rust CLI | orchestration、artifact resume、failure classify、artifact doctor | `target/debug/code-intel.exe` |
| CodeNexus compatibility adapter | 可选热点定位、引用搜索、下一步上下文；失败不阻塞 beta core | `codenexus-context.json` |
| `rg` | 快速文件清单、文本搜索 | `files.txt` |
| `Repowise` | 语义索引、长期记忆、项目上下文 | `.repowise/` 或 scoped shadow |
| `Repomix` | 把本地或远程仓库打包成 AI 友好的单文件上下文 | `repomix-output.md`、`repomix-summary.json` |
| `Understand Anything` | 架构图谱快照 | `.understand-anything/knowledge-graph.json` |
| `Sentrux` | 结构质量、规则门禁、Agent 会话回归 | DSM、hotspots、what-if、evolution |
| `CodeNexus-lite` | 热点定位、引用搜索、下一步上下文 | `codenexus-context.json` |
| `Greenfield` | 从源码、文档、SDK、运行时和二进制证据抽取干净行为规格 | `greenfield-manifest.json`、`greenfield-plan.md`、`greenfield-workspace/output/` |
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

每次运行会在 artifact 根目录下创建一个已提交的运行目录：

```text
<platform code-intel data root>/artifacts/<repo-name>/<run-id>/
```

不管是主入口自动落盘的路径，还是 `run execute --authority-root/--final-name` 指到的路径，已提交（committed）的运行目录本身都是扁平、content-addressed 的，不含任何按节点分的子目录，只有两样东西：

```text
run-complete.json          # 提交标记；manifest.sha256 指向下面的 manifest blob
objects/sha256/<hash>      # 每个产物的原始字节，文件名就是它自己的 sha256
```

`diagnosis.hospital/hospital.md`、`evidence.native-code/code-evidence/...` 这类路径不是磁盘上的文件。manifest blob 里每个 artifact 的 `type` 字段（如 `diagnosis.hospital-view`）才是逻辑身份；它的 `path` / `sha256` 在发布时已经被改写成物理 blob 位置，原始逻辑路径不会留在已提交的 run 里（发布机制见 [docs/run-commit.md](docs/run-commit.md)）。`run execute --out <dir>` 显式指定的 staging 目录是例外——提交前那里仍是人读的真实文件树（`<node>.request.json` / `<node>.result.json` 信封也只在这里）；一旦提交，权威副本就只剩 blob。

从 `run-complete.json` 走到某个具体产物（已在本仓验证，Windows Git Bash + `jq`；`<run-dir>` 用命令行 `--json` 输出里的 `publication.path`）：

```bash
cd <run-dir>
manifest_sha=$(jq -r '.manifest.sha256' run-complete.json)
view_sha=$(jq -r '.nodes["diagnosis.hospital"].artifacts[] | select(.type=="diagnosis.hospital-view") | .sha256' "objects/sha256/$manifest_sha")
cat "objects/sha256/$view_sha"   # 就是 hospital.md 的 markdown 正文
```

或者跳过手工 jq，直接用上面「全链路命令」里的 `artifact query --type diagnosis.hospital-view`——两条路径读到的是同一份已验证字节，`artifact query` 只是替你做了这趟 manifest 解引用。artifact 根目录的 `index.json` 只有主入口会在提交后自动重建；单独跑 `run execute` 不会顺带写它，要索引就显式 `artifact index --operation rebuild`。

`run-complete.json` 是最后写入的事务提交标记；主入口的标记绑定 manifest blob 的 sha256（即上面 `manifest.sha256` 字段），索引只接受标记存在且校验一致的运行目录。
旧兼容 runner 的标记绑定的是 `report.json` 的 `reportSha256`。

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

下列报告与结构产物目前只由旧兼容 runner（`legacy/run-code-intel.ps1` / `legacy/scripts/tests/test-code-intel-pipeline.ps1`）生成，主入口不产出：

```text
summary.md
report.json
understanding.md
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
greenfield-manifest.json
greenfield-plan.md
```

读报告顺序（主入口 `code-intel .` / `run execute`；已提交 run 里没有这些文件名，按 `type` 查，方法见上）：

1. 先看命令行汇总（或 `--json` 输出）确认整轮 outcome；失败细节看 manifest blob 里的失败节点。
2. 查 `--type code_evidence.agent_slice` 做 ranked 文件 / 符号导航。
3. 做治理判断查 `--type diagnosis.hospital-view`（人读）或 `diagnosis.hospital`（机器读 JSON）。
4. 要开工修结构查 `--type diagnosis.surgery-plan-view`。

读报告顺序（旧兼容 runner）：

1. Repomix 成功时先看 `repomix-output.*`，它是给人和 Agent 快速理解陌生仓库的整仓包。
2. 再看 `summary.md`，它是整轮运行状态、失败分类、关键 artifact 的入口页。
3. 交接给人或 Agent 前看 `understanding.md`。
4. 治理与行动计划同主入口：`hospital.md`、`surgery-plan.md`。

## Portable Snapshot Identity

仓库输入身份可以独立计算，不依赖时间戳目录或机器绝对路径：

```powershell
target/debug/code-intel.exe snapshot identity --repo <repo-root> --working-tree-policy explicit_overlay --scope .
```

它绑定 Git lineage、HEAD、工作树策略、规范化 scope 与实际输入字节，并逐类报告 dirty overlay。shallow、unborn、无 Git、ignored、symlink、submodule、LFS 与并发变化规则见 `docs/repository-snapshot-identity.md`。

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

## Audit 层

0.6.0 起，audit 维度作为 hospital 科室跑在流水线已有的 modality 证据上。`security`、`ai-safety`、`supply-chain` 三个科室默认启用，报告遵循 fail-closed 的 `code-intel-audit-report.v1` 契约：

```powershell
code-intel audit --operation validate --repo C:\path\to\repo --report C:\path\to\audit-report.json
code-intel audit --operation render --repo C:\path\to\repo --report C:\path\to\audit-report.json --format html
code-intel audit --operation scope --repo C:\path\to\repo --since <git-ref>
```

`validate` 做结构、注册表和证据落地校验；`render` 先跑同一套校验，成功后才输出 Markdown 或自包含 HTML；`scope` 计算 diff 范围的 scope 块，用于 PR 级增量 audit。有 audit 时 `hospital-report.json` 增加可选 `audit` 块，`hospital.md` 增加 `## Audit` 段。完整契约见 [docs/audit-report.md](docs/audit-report.md)。

## Agent 工作流

### 先接查询面，全量扫描是深检模式

Agent 平时问单点问题走 MCP，不必为一个问题跑一整轮 pipeline：

```powershell
code-intel serve --mcp --repo <name>
```

stdio MCP server，按需起、随 session 生灭。注册进 `.mcp.json` 后 Claude Code / Codex 直连可调。**数据来源逐个工具不同**，读答案时按这一列判它有多新：

| 工具 | 回答什么 | 数据来源 |
|---|---|---|
| `get_gate_verdict` | 权威 run 的门禁结论、第一条失败规则、最小重跑命令 | 已提交 run；附 freshness |
| `get_facts` | 按 artifact type / schema / 子串查已验证事实（热点、import 边、scorecard、符号） | 已提交 run（digest 已校验） |
| `get_evidence` | 一条 finding 的证据链：哪些产物提到它、各自 sha256、记录时的 snapshot | 已提交 run |
| `get_audit_status` | 各科室审计结论、评分、覆盖；没跑过 audit 会明说"不可用"而不是装绿 | 已提交 run |
| `get_change_impact` | 改这些文件会波及谁、该先跑哪些测试 | **已提交 import 图 × 当前 `--repo-path`**；默认标 stale-advisory 并同时给出 recorded/current 两个 snapshot identity |
| `plan_structural_edit` | ast-grep 结构改写预览，只出匹配清单，不落盘 | **当前工作树**（不是已提交 run） |

**这个面只读，不裁决。** 门禁判定照旧只走 CLI 与 CI 路径——查询面被 prompt injection 说服也改不了结论。唯一会执行东西的工具是 `plan_structural_edit`，它在跑之前拿注册表核对自己的 capability 声明，一旦声明里出现 `repo_mutation` 就直接拒绝。

`--repo` 建议显式给：worktree 的目录名不是 `run commit` 发布时用的仓名，不给就会去查错仓的 run。`--repo-path` 默认取工作目录。

全量 `code-intel <path> --mode normal` 留给深检和出证据，不是日常问答的入口。

### 结构门禁

Agent 开始改代码前：

```powershell
.\legacy\Invoke-SentruxAgentTool.ps1 session_start C:\path\to\repo\backend
```

Agent 改完代码后：

```powershell
.\legacy\Invoke-SentruxAgentTool.ps1 session_end C:\path\to\repo\backend
```

如果结构质量下降，`session_end` 会失败，并返回前后分数。

Agent 改代码过程中，不必等整轮 pipeline，直接问增量问题：

```powershell
code-intel change impact --artifact-root <root> --repo <name> --repo-path C:\path\to\repo --changed src\module\file.rs --staleness advisory
```

它从最近一次已提交的 run 回答“改这些文件会波及谁、该先跑哪些测试”。`--staleness advisory` 表示答案只作建议、永不门禁，working tree 脏了也能问。

机械化批量改写先出预览计划：

```powershell
code-intel capability exec edit.ast-grep-plan --request <request.json> --out <staging-dir>
```

计划只预览（`repositoryMutation=false`），不会改文件。

改一个标识符不必重写整行——按 span 下指令，工具执行：

```powershell
code-intel edit apply --repo-path C:\path\to\repo --file src/module/file.rs `
  --span 12:21-12:33 --expect-sha256 <该 span 当前字节的 sha256> --replacement retryBudget
```

`--expect-sha256` 由调用方给出（来自索引或先前读到的字节），工具在写之前拿它跟 span 现在的字节比对：不一致就拒绝，退出码 10，并回报"期望什么 / 实际是什么"（含 `foundSha256` 与一段有界原文），文件一个字节不动。这是索引漂移时不误伤的唯一机械保证。同一次调用可带多个 `--span`（同一文件，互不重叠），全部按改动前的字节定位、整文件原子替换，要么全落要么全不落。

改完 `session_end` 收门禁。

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
.\legacy\Invoke-SentruxAgentTool.ps1 health C:\path\to\repo\backend
.\legacy\Invoke-SentruxAgentTool.ps1 dsm C:\path\to\repo\backend
.\legacy\Invoke-SentruxAgentTool.ps1 evolution C:\path\to\repo\backend
.\legacy\Invoke-SentruxAgentTool.ps1 what_if C:\path\to\repo\backend
```

别让 Agent 裸奔。没有 `session_start/session_end`，它改完代码以后自己也不知道有没有把结构弄坏。

## 全链路命令

上面这些是编辑时的轻量问答，答案来自上一次已提交（committed）的 run，不重跑管线。CI 级的权威扫描、直接查证据、PR 门禁是另外四个命令——默认 `code-intel --help` 只列 6 个，这几个要 `code-intel --help --all` 才看得到。呼应 [#92](https://github.com/2233admin/code-intel-pipeline/issues/92) 的接入分层：

| 命令 | 档位 | 一句话 |
| --- | --- | --- |
| `artifact query` | 直查 | 从已提交 run 里按 type / schema / contains 直接读证据，不重跑管线 |
| `run execute` | 跑管线 | 权威全量扫描 + 发布到 content-addressed authority root；CI 自扫描步骤用它 |
| `change risk` | 门禁 | 只用 git 历史给 PR 打缺陷风险分，不需要索引 / 网络 / LLM；驱动 `pr-gate.yml` |
| `repin` | 维护 | 扫描（可选改写）全仓过期的 sha256 pin，一趟收敛到不动点 |

```powershell
code-intel run execute --repo . --out <run-staging-dir> --authority-root <publication-root>\<repo-name> --final-name <run-id> --manifest orchestration/integrations.json --doctor-require-repowise false
```

`.github/workflows/ci.yml` / `release.yml` 的自扫描步骤就是这行去掉 `\<repo-name>`（`--final-name ci-self-scan` / `release-self-scan`）——CI 只认退出码和 `publication.status == "committed"`，从不回查自己，所以省得了这层嵌套。要接下面的 `artifact query`，`--authority-root` 得把仓库名折进去，如上。

```powershell
code-intel artifact query --artifact-root <publication-root> --repo <repo-name> --type diagnosis.hospital-view --limit 1
```

字段契约见 [docs/evidence-query.md](docs/evidence-query.md)。

```powershell
code-intel change risk HEAD~5..HEAD --format json
```

没有 `--repo` 参数，从目标仓库内部（CWD）跑；`pr-gate.yml` 用它给 PR 打分：`code-intel change risk "origin/<base-branch>..HEAD" --format json`。

```powershell
code-intel repin --write --json
```

干净仓库上是安全的空操作；有过期 pin 时内部最多迭代 25 轮收敛到不动点。

完整参数表见 `code-intel --help --all`，不在这里重复。

## Sentrux 自动 Pro

`sentrux` 是 MIT/开源项目，本仓库会安装一个很薄的 shim：

```text
<platform code-intel data root>/bin/sentrux
<platform code-intel data root>/bin/sentrux-shim.ps1
```

它做几件事：

- 默认**不**自动激活 Pro（opt-in）：只有显式设置 `SENTRUX_AUTO_PRO=1`（或 `true`）时，第一次运行才会写本地 Pro license。上游没有任何 license 证据支持默认自动激活（supply-chain-009），所以默认保持 free tier。
- `sentrux pro status / activate / deactivate` 可直接用。
- 优先转发给真实 `sentrux.exe`。
- 没有真实 core 时，使用仓库内置 `sentrux-lite-core.ps1` 保底，覆盖 `scan`、`health`、`check`、`gate` 和 `plugin list/validate`。

`bin\` 里的 `sentrux-shim.ps1` / `sentrux-lite-core.ps1` 只是薄转发器（thin forwarder），不是脚本正文的拷贝：它们在安装时把仓库路径写死进去，运行时转发到 `tools\sentrux-shim\` 下的真身并透传参数和退出码。改仓库里的 `tools\sentrux-shim\*.ps1` 立即生效，PATH 调用不需要重跑 install。只有仓库整体挪了目录才需要重跑 `legacy/install-code-intel-pipeline.ps1`——挪了目录之后转发器会报清晰错误（`repo not found at <path>`），不会静默失败或跑到旧代码。

检查：

```powershell
sentrux pro status
```

预期（默认，未 opt-in）：

```text
Tier: free
Status: inactive
License: <本地 license 路径>
Features: check, gate, scan, mcp, plugin, analytics
```

开启自动 Pro（opt-in）：

```powershell
$env:SENTRUX_AUTO_PRO = "1"
sentrux pro status
```

之后输出变为：

```text
Tier: pro
Status: active
Features: dsm_export, file_detail_panel, evolution_details, what_if_analysis, agent_mcp, rule_gates, nine_color_modes
```

也可以不设环境变量，手动激活：

```powershell
sentrux pro activate OSS-LOCAL-PRO
```

停用：

```powershell
sentrux pro deactivate
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
.\legacy\Install-SentruxVlangOverlay.ps1
```

验证：

```powershell
sentrux plugin validate ~/.sentrux/plugins/vlang
sentrux plugin list
.\legacy/scripts/tests/Test-SentruxVlangOverlay.ps1
```

不安装覆盖包：

```powershell
.\legacy\install-code-intel-pipeline.ps1 -RepoPath C:\path\to\repo -SkipSentruxVlangOverlay
```

## Repowise 语义记忆

Repowise 是可选语义记忆层。`normal` 在可用时使用，`lite` 不依赖它，`full` 才要求所有可选 provider 就绪：

```powershell
code-intel C:\path\to\repo --mode normal
```

要缩小 scope，直接把目标路径指向相应子目录，避免把根目录里的外部轮子、临时文件、研究仓库一起吃进去。

```powershell
code-intel C:\path\to\repo\backend --mode normal
```

Repowise wiki 文档属于兼容适配器维护能力，不是正式主入口参数。

如果 provider 限流，报告会显示 `provider_quota`。这不是本地脚本坏了。

### Docs LLM provider 配置

Repowise docs 生成走 repowise 自带的 provider 注册表，通过 User 级(或进程级)环境变量选择:

| 变量 | 作用 | 默认 |
| --- | --- | --- |
| `CODE_INTEL_PROVIDER` | provider 名(`anthropic` / `openai` / `ollama` / 其他 registry 支持的名字) | `anthropic` |
| `CODE_INTEL_MODEL` | 模型名 | anthropic 时为 `MiniMax-M2.7` |
| `CODE_INTEL_API_KEY` | 通用凭证(ollama 不需要) | — |
| `CODE_INTEL_BASE_URL` | 通用端点 | 各 provider 官方端点 |
| `CODE_INTEL_ANTHROPIC_API_KEY` / `CODE_INTEL_ANTHROPIC_BASE_URL` | 旧变量,provider=anthropic 且通用变量缺失时回落 | — |

注意:不要设置全局(User/Machine)的 `ANTHROPIC_API_KEY` / `ANTHROPIC_BASE_URL`——那可能属于本机 Claude Code 代理链;脚本只在进程级临时注入。

示例:

```powershell
# MiniMax(Anthropic 兼容端点,现状默认,无需 CODE_INTEL_PROVIDER)
# User env: CODE_INTEL_ANTHROPIC_API_KEY=<key>
#           CODE_INTEL_ANTHROPIC_BASE_URL=https://api.minimaxi.com/anthropic

# 本地 Ollama(无需 key)
$env:CODE_INTEL_PROVIDER = "ollama"
$env:CODE_INTEL_MODEL = "qwen3:4b"          # 默认端点 http://localhost:11434

# 任意 OpenAI 兼容端点
$env:CODE_INTEL_PROVIDER = "openai"
$env:CODE_INTEL_MODEL = "your-model"
$env:CODE_INTEL_API_KEY = "<key>"
$env:CODE_INTEL_BASE_URL = "https://your-endpoint/v1"
```

跑 docs 前可先做 preflight:

```powershell
.\legacy/scripts/tests/test-code-intel-provider.ps1 -Json                              # 按 env 选 provider
.\legacy/scripts/tests/test-code-intel-provider.ps1 -Provider ollama -Model qwen3:4b   # 显式指定
```

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
sentrux gate --save C:\path\to\repo\backend
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
code-intel C:\path\to\repo\backend --mode normal
```

如果你要分析 `tools/` 里的某个外部轮子，把 scope 指到那个轮子，而不是让它污染主项目：

```powershell
code-intel C:\path\to\repo\tools\some-lib --mode normal
```

## 真实跑通过的路径

本仓库已经验证过这些路径：

```text
本项目完整链路：
legacy/scripts/tests/test-code-intel-pipeline.ps1 -RepoPath $env:CODE_INTEL_HOME -Mode normal

GitHub fresh clone：
legacy/scripts/tests/test-code-intel-pipeline.ps1 -RepoPath <tmp>/code-intel-pipeline-online-test -Mode normal

Katana 大仓库 scoped：
legacy/scripts/tests/test-code-intel-pipeline.ps1 -RepoPath <k-atana-path> -SentruxPath backend -Mode normal
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

这是默认行为：自动 Pro 是 opt-in（supply-chain-009），不会因为跑了安装器就激活。要开启，显式设置环境变量：

```powershell
$env:SENTRUX_AUTO_PRO = "1"
sentrux pro status
```

或手动激活：`sentrux pro activate OSS-LOCAL-PRO`。详见上文「Sentrux 自动 Pro」一节。

### `Understand graph missing`

运行：

```powershell
.\target\debug\code-intel.exe graph --repo C:\path\to\repo --language zh --write --json
```

再重跑 pipeline。

### Repowise 很慢

先 scoped：

```powershell
code-intel C:\path\to\repo\backend --mode normal
```

仍然过慢时切到 `--mode lite`；provider 超时只在兼容适配器配置中维护。

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

先跑安装器，再跑 doctor，再跑 `code-intel .`。改代码前先读证据：整轮 outcome 看命令行汇总，失败看 `run-manifest.json` 里的失败节点，导航用 `evidence.native-code/code-evidence/merged/agent/index.md`，治理看 `diagnosis.hospital/hospital.md`，行动计划看 `diagnosis.hospital/surgery-plan.md`。改代码时继续问管线：`session_start` 起基线，`change impact --staleness advisory` 查波及面和该跑的测试，机械改写先 `capability exec edit.ast-grep-plan` 出预览计划，改完 `session_end` 收门禁。不要跳过 Sentrux baseline 和 rules，不然 Agent 只是换了个速度更快的方式堆债。

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

The Rust CLI owns the default normal production spine, integration orchestration,
snapshot-bound evidence, atomic run publication, committed-only indexing, and
cross-session query/impact reads. PowerShell scripts remain thin Windows
compatibility and installation facades; legacy report generation is not an
alternative authority path.
