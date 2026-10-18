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

在要分析的仓库目录中运行稳定入口；不传参数时默认分析当前目录：

```bash
code-intel .
```

或显式指定仓库：

```bash
code-intel C:\path\to\your\repo
```

还没有 `code-intel`？见[安装](#安装)——windows / macos / linux 都有 Release ZIP，不需要 PowerShell 或 Rust 工具链。装好后第一次运行：

```bash
code-intel status ~/src/your-repo
code-intel ~/src/your-repo
code-intel query ~/src/your-repo --kind evidence --json
```

`status` 是低摩擦的项目入口：它把仓库身份、最近一次 committed run 的新鲜度和
下一步动作放在一起。未分析时返回 `needs_run`，证据与当前 checkout 一致时返回
`ready`，不一致时返回 `stale`；Agent 可用 `--json` 读取同一个
`code-intel-project-status.v1` envelope。它只组合现有 authority/freshness 契约，
不会把未提交或过期证据伪装成当前事实。

`code-intel run <repo>` 是分析主入口的等价命名形式。`query` 只接收仓库路径，
由 ProjectContext 统一解析仓库键和 artifact root；日常调用不需要再传
`--artifact-root`、`--repo`、run id 或 manifest。低层发布/排障仍可使用
`run execute` 和 `artifact query` 的显式参数接口。

## 语言方向与治理

> **语言方向：** PowerShell 已从产品中删除（DR-0018）：没有 `.ps1` 入口，新代码也不得新增。生产能力使用 Rust。MoonBit 可用于隔离实验，只有通过 artifact 契约一致性、跨平台构建和测试后才进入正式路径。Agent 规则见 [AGENTS.md](AGENTS.md)。

Workflow-stack guidance is emitted by the read-only `advisory.workflow-recommend` atom. See [docs/advisory-workflow-recommendation.md](docs/advisory-workflow-recommendation.md); recommendations are proposals with zero effects and never authorize tool initialization or adoption.

Follow-up automation can proactively propose `/investigate` for actionable scan failures and can ask whether to enter the exact draft-PR flow. The one-command orchestrator composes proposal → user decision → C07 record/replay → fail-closed executor. It defaults to suggestion-on and PR-consent-required; neither path silently executes a skill or creates a PR. See [docs/follow-up-automation.md](docs/follow-up-automation.md).

## 仓库入口

编译后的 `code-intel` 是唯一正式入口：所有 Pipeline 语义都在它和 `crates/code-intel-cli` 里。真正的治理边界见 [Repository Layout](docs/repository-layout.md)。

- `code-intel`: 人工和 Agent 的正式主入口。新文档和命令示例一律以它开头。
- `crates/code-intel-cli`: Rust policy/artifact CLI core。
- 常用子命令：`code-intel setup`（安装收尾）、`code-intel doctor` / `code-intel doctor bootstrap`（环境检查）、`code-intel sentrux gate`（结构门禁）、`code-intel lint hardcoded-paths`（机器路径扫描）。完整列表见 `code-intel --help --all`。

## Public beta 范围

当前为 **public beta**：每个版本发布 windows / macos / linux 三个 Release ZIP，生产逻辑全部在 Rust `code-intel` 中。测试版承诺的是：稳定入口可运行、核心报告可事务化落盘、结构退化不会被伪装成成功、发布 ZIP 可在没有源码树和 Rust toolchain 的干净目录中启动。

0.4.0 核心路径：

- `code-intel` Primary Operator Entry
- `code-intel.exe` 的 A01-A09 capability/DAG/policy/artifact core
- `rg` inventory、native code evidence、内部 graph provider、真实 Sentrux `gate`/`check` 命令证据和 Hospital diagnosis
- snapshot-bound staging、A07 原子提交、A08 completed-only 索引、query/impact/freshness

默认包含但不阻塞测试版的增强能力：

- Repowise 语义索引与文档生成
- Understand Anything 图谱
- CodeNexus context、模型辅助通道
- runtime/CI 证据和 file-boundary provider

缺少这些增强工具时，流水线必须明确记录 skipped、manual-required 或 provider failure，不能把它们冒充成功，也不能因此阻断只依赖核心路径的 public beta。详细支持矩阵和已知边界见 [Public beta guide](docs/public-beta.md)。

0.5.1 beta 还提供可重放的工具效果基线评分器。它不会把工具“能运行”误当成“对 Agent 有帮助”；只有冻结任务、配对条件和外部证明完整时才记录基线：

```bash
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

## 安装

windows / macos / linux 三平台的安装方式相同：**不需要 PowerShell，也不需要 Rust 工具链**。

### 推荐：通过 Agent Skill 安装

Codex 可以先安装官方结构的 Skill 包，再由 Skill 下载并校验稳定版 Release：

```text
请使用 $skill-installer 安装：
https://github.com/2233admin/code-intel-pipeline/tree/main/skills/code-intel-pipeline

然后使用 $code-intel-pipeline 为 /path/to/your/repo 安装并运行稳定版。
```

也可以在 `skills/code-intel-pipeline/` 下直接运行引导脚本：先看计划，再执行：

```bash
python scripts/bootstrap.py --repo-path "<repo>" --dry-run --json
python scripts/bootstrap.py --repo-path "<repo>" --json
```

可选参数：`--version <tag>` 指定版本，`--channel prerelease` 选择预发布。它会下载当前平台的 GitHub Release ZIP，校验 SHA-256 和构建来源，解压后运行该 Release 自带的 `bin/code-intel setup`。

Skill 默认只解析稳定版；`gh` 2.49+ 在场时先验 [GitHub Artifact Attestation](https://cli.github.com/manual/gh_attestation_verify)（证明这份 ZIP 确实产自本仓发布流水线，不只是字节没传坏），再校验 GitHub Release 提供的 SHA-256；`gh` 缺失或过旧会显式打印降级说明而不是静默只查 SHA-256。命令与验证记录见 [docs/release-provenance-runbook.md](docs/release-provenance-runbook.md)。预发布版本和第三方依赖安装都需要显式选择。

### 手动安装

1. 从 [GitHub Releases](https://github.com/2233admin/code-intel-pipeline/releases) 下载 `code-intel-pipeline-<tag>-<platform>.zip` 和对应的 `.sha256`（`<platform>` 为 `windows` / `macos` / `linux`）。
2. 按下文[验证发布来源](#验证发布来源)校验。
3. 解压，运行包内自带的 `setup`（Windows 上是 `code-intel.exe`）：

```bash
<解压目录>/code-intel-pipeline/bin/code-intel setup --repo-path <repo>
```

加 `--language zh|en` 可记录该仓库的报告语言。

### `code-intel setup` 做了什么

- 把二进制复制到数据目录的 `bin`：Windows `$env:LOCALAPPDATA\code-intel\bin`，macOS `~/Library/Application Support/code-intel/bin`，Linux `${XDG_DATA_HOME:-~/.local/share}/code-intel/bin`。
- 持久化 `PATH` 和 `CODE_INTEL_HOME`：Windows 写用户环境变量（新开终端生效）；macOS / Linux 写 `~/.config/code-intel/env.sh`，并在 `~/.profile`（以及存在的 `~/.bashrc` / `~/.bash_profile`，装了 zsh 时还有 `.zshenv`）里加一行 `. "…/env.sh"`。`--no-modify-path` 跳过这些修改。
- 把 Skill 安装到 `~/.agents/skills/code-intel-pipeline`，并链接进 `~/.claude/skills` 和 `~/.codex/skills`（不允许链接时改为复制；已存在且不匹配的目录会被改名为 `<name>.replaced-<hex>`，从不删除）。`--no-skills` 跳过。
- 最后运行 doctor（`--skip-doctor` 跳过）。缺失的外部工具（`rg`、`git`、`python`、`repowise`）会连同安装命令一起报告，`setup` 自己从不安装系统工具。

重复运行是幂等的。其余参数：`--release-root`、`--repo-path`、`--platform`、`--json`。Artifact 默认写到 `<数据目录>/artifacts`（各平台相同）。

### 源码构建（需要 Rust 工具链）

```bash
git clone https://github.com/2233admin/code-intel-pipeline.git
cd code-intel-pipeline
cargo build --release -p code-intel
mkdir -p bin && cp target/release/code-intel bin/    # Windows 为 code-intel.exe
bin/code-intel setup --repo-path ~/src/your-repo
```

流水线需要完整 git 历史来计算 lineage identity；浅克隆请先 `git fetch --unshallow`。

### 验证发布来源

除了上面 Skill 自动做的 attestation + SHA-256 校验，人工也可以逐项验证：

```bash
# 校验二进制确实产自本仓 CI（不是有人改了字节又同步改了 SHA-256）
gh attestation verify code-intel-pipeline-v0.7.0-windows.zip --repo 2233admin/code-intel-pipeline

# 校验 SHA-256 —— Windows
certutil -hashfile code-intel-pipeline-v0.7.0-windows.zip SHA256

# 校验 SHA-256 —— macOS
shasum -a 256 -c code-intel-pipeline-v0.7.0-macos.zip.sha256

# 校验 SHA-256 —— Linux
sha256sum -c code-intel-pipeline-v0.7.0-linux.zip.sha256
```

正式版（不带 `-beta.<n>` 或 `-rc.<n>` 后缀的 tag，例如 `v0.7.0`；`v0.7.0-beta.2` 这种预发布 tag 不算）额外要求 tag 本身经 SSH 签名，签名与验证方式见 [docs/RELEASE_SIGNING.md](docs/RELEASE_SIGNING.md)：

```bash
git verify-tag v0.7.0
```

### 运行

安装后直接使用编译入口；不需要 Cargo，也不依赖仓库里的 `target/`：

```bash
code-intel C:\path\to\your\repo
```

只使用离线核心能力：

```bash
code-intel C:\path\to\your\repo --mode lite
```

完整 normal 模式：

```bash
code-intel C:\path\to\your\repo --mode normal
```

## 新机器部署

按[安装](#安装)装好后，用 doctor 复查环境（只检查，不会自动安装缺失工具）：

```bash
code-intel doctor bootstrap --repo-path C:\path\to\your\repo
```

缺失的外部工具（`rg`、`git`、`python`、`repowise`）会连同对应的 winget / brew / apt / pip 安装命令一起报告，由你自己决定是否执行。`setup` 不会写 API key，不会把 secret 存进仓库。

## 工具角色

| 工具 | 角色 | 产物 |
| --- | --- | --- |
| Integration orchestration | 融合注册、能力编排、扩展边界 | `code-intel orchestrate` |
| `code-intel` Rust CLI | orchestration、artifact resume、failure classify、artifact doctor | `code-intel` |
| CodeNexus compatibility adapter | 可选热点定位、引用搜索、下一步上下文；失败不阻塞 beta core | `codenexus-context.json` |
| `rg` | 快速文件清单、文本搜索 | `files.txt` |
| `Repowise` | 语义索引、长期记忆、项目上下文 | `.repowise/` 或 scoped shadow |
| `Understand Anything` | 架构图谱快照 | `.understand-anything/knowledge-graph.json` |
| `Sentrux` | 结构质量、规则门禁、Agent 会话回归 | DSM、hotspots、what-if、evolution |
| `CodeNexus-lite` | 热点定位、引用搜索、下一步上下文 | `codenexus-context.json` |
| `Greenfield` | 从源码、文档、SDK、运行时和二进制证据抽取干净行为规格 | `greenfield-manifest.json`、`greenfield-plan.md`、`greenfield-workspace/output/` |
| Governance layer | 状态判断、治理计划、放行标准 | `hospital.md`、`hospital-report.json`、`surgery-plan.md` |

这几个工具分工不同。不要把它们混成一个 RAG 糊糊。

新增项目或新方式时，先注册到 `orchestration/integrations.json`，再接 adapter。不要直接把新的外部 CLI 调用散进主流程。

查看当前编排：

```bash
code-intel orchestrate --action Validate
code-intel orchestrate --action Plan --repo C:\path\to\your\repo --mode normal
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

不想手工解引用 blob，直接读最近一次已提交结果：

```bash
code-intel report --repo C:\path\to\your\repo --artifact-root <artifact-root>
```

它校验 `run-complete.json` 和 manifest 的 Artifact Ref，打印 `hospital.md`、机器报告和可选的 Agent 切片排名；`--json` 只输出这些已验证路径。

或者跳过手工 jq，直接用上面「全链路命令」里的 `artifact query --type diagnosis.hospital-view`——两条路径读到的是同一份已验证字节，`artifact query` 只是替你做了这趟 manifest 解引用。artifact 根目录的 `index.json` 只有主入口会在提交后自动重建；单独跑 `run execute` 不会顺带写它，要索引就显式 `artifact index --operation rebuild`。

`run-complete.json` 是最后写入的事务提交标记；主入口的标记绑定 manifest blob 的 sha256（即上面 `manifest.sha256` 字段），索引只接受标记存在且校验一致的运行目录。

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

读报告顺序（主入口 `code-intel .` / `run execute`；已提交 run 里没有这些文件名，按 `type` 查，方法见上）：

1. 先看命令行汇总（或 `--json` 输出）确认整轮 outcome；失败细节看 manifest blob 里的失败节点。
2. 查 `--type code_evidence.agent_slice` 做 ranked 文件 / 符号导航。
3. 做治理判断查 `--type diagnosis.hospital-view`（人读）或 `diagnosis.hospital`（机器读 JSON）。
4. 要开工修结构查 `--type diagnosis.surgery-plan-view`。

## Portable Snapshot Identity

仓库输入身份可以独立计算，不依赖时间戳目录或机器绝对路径：

```bash
code-intel snapshot identity --repo <repo-root> --working-tree-policy explicit_overlay --scope .
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

```bash
code-intel audit --operation validate --repo C:\path\to\repo --report C:\path\to\audit-report.json
code-intel audit --operation render --repo C:\path\to\repo --report C:\path\to\audit-report.json --format html
code-intel audit --operation scope --repo C:\path\to\repo --since <git-ref>
```

`validate` 做结构、注册表和证据落地校验；`render` 先跑同一套校验，成功后才输出 Markdown 或自包含 HTML；`scope` 计算 diff 范围的 scope 块，用于 PR 级增量 audit。有 audit 时 `hospital-report.json` 增加可选 `audit` 块，`hospital.md` 增加 `## Audit` 段。完整契约见 [docs/audit-report.md](docs/audit-report.md)。

## Agent 工作流

### 先看状态，再沿下一步工作

```bash
code-intel status <path>
code-intel status <path> --json
```

状态输出给出仓库绑定、committed run、新鲜度和一组有界的 `nextActions`：首次分析、
上下文查询、证据查询、MCP 追踪或 advisory 影响分析。动作清单是可迭代的产品提示，
不改变 artifact、provider 或 gate 的 authority；研究判断和后续 UX 次序仍可通过
decision/research 文档与 issue 修订。

### 先接查询面，全量扫描是深检模式

Agent 平时问单点问题走 MCP，不必为一个问题跑一整轮 pipeline：

```bash
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
| `get_code_context` | 语义定位、真实源码片段、符号及调用上下文 | **当前 `--repo-path` 的显式 CodeGraph 索引**；校验源码 snapshot 与 provider/database 身份，过期拒绝；partial/advisory，不能裁决门禁 |
| `plan_structural_edit` | ast-grep 结构改写预览，只出匹配清单，不落盘 | **当前工作树**（不是已提交 run） |
| `scan_security_findings` | 跑内置原创 ast-grep 安全规则集（csharp/go/java/javascript/python/rust/typescript），出 finding 清单 | **当前工作树**；advisory-only，findings 是复核提示不是已验证漏洞，从不影响 `get_gate_verdict` |

**这个面不裁决，不改源码。** 门禁判定照旧只走 CLI 与 CI 路径——查询面被 prompt injection 说服也改不了结论。`plan_structural_edit` 和 `scan_security_findings` 执行注册的 preview-only capability，声明出现 `repo_mutation` 就拒绝。可选 `get_code_context` 执行本地 CodeGraph 查询并在仓库外保存证据，不自动建索引、同步、启动 watcher 或修改 Git hooks；它的 partial 结果不会变成权威事实。

`--repo` 建议显式给：worktree 的目录名不是 `run commit` 发布时用的仓名，不给就会去查错仓的 run。`--repo-path` 默认取工作目录。

CodeGraph 接入使用编译后的 `code-intel provider codegraph`；先显式 `index`，源码变化后显式 `sync`。引擎缺席不影响原生 CodeNexus 基线。安装、命令、缓存边界及验证见 [CodeGraph 选择性内化记录](docs/research/codegraph-adoption.md)。

全量 `code-intel <path> --mode normal` 留给深检和出证据，不是日常问答的入口。

### 结构门禁

首次在某个 scope 上保存基线，之后改动前后各跑一次 `gate`：

```bash
code-intel sentrux gate_save C:\path\to\repo\backend   # 首次：保存基线
code-intel sentrux gate C:\path\to\repo\backend        # Agent 改代码前
code-intel sentrux gate C:\path\to\repo\backend        # Agent 改完代码后
```

如果结构质量相对基线下降，`gate` 会失败。

Agent 改代码过程中，不必等整轮 pipeline，直接问增量问题：

```bash
code-intel change impact --artifact-root <root> --repo <name> --repo-path C:\path\to\repo --changed src\module\file.rs --staleness advisory
```

它从最近一次已提交的 run 回答“改这些文件会波及谁、该先跑哪些测试”。`--staleness advisory` 表示答案只作建议、永不门禁，working tree 脏了也能问。

机械化批量改写先出预览计划：

```bash
code-intel capability exec edit.ast-grep-plan --request <request.json> --out <staging-dir>
```

计划只预览（`repositoryMutation=false`），不会改文件。

改一个标识符不必重写整行——按 span 下指令，工具执行：

```bash
code-intel edit apply --repo-path C:\path\to\repo --file src/module/file.rs \
  --span 12:21-12:33 --expect-sha256 <该 span 当前字节的 sha256> --replacement retryBudget
```

`--expect-sha256` 由调用方给出（来自索引或先前读到的字节），工具在写之前拿它跟 span 现在的字节比对：不一致就拒绝，退出码 10，并回报"期望什么 / 实际是什么"（含 `foundSha256` 与一段有界原文），文件一个字节不动。这是索引漂移时不误伤的唯一机械保证。同一次调用可带多个 `--span`（同一文件，互不重叠），全部按改动前的字节定位、整文件原子替换，要么全落要么全不落。

改完再跑 `sentrux gate` 收门禁。

`code-intel sentrux` 支持的子命令：

```text
dsm
scan
health
check
gate
check_rules
gate_save
hotspots
```

常用命令：

```bash
code-intel sentrux health C:\path\to\repo\backend
code-intel sentrux dsm C:\path\to\repo\backend
code-intel sentrux hotspots C:\path\to\repo\backend
```

别让 Agent 裸奔。没有改前改后的 `gate`，它改完代码以后自己也不知道有没有把结构弄坏。

## 全链路命令

上面这些是编辑时的轻量问答，答案来自上一次已提交（committed）的 run，不重跑管线。CI 级的权威扫描、直接查证据、PR 门禁是另外四个命令——默认 `code-intel --help` 只列 6 个，这几个要 `code-intel --help --all` 才看得到。呼应 [#92](https://github.com/2233admin/code-intel-pipeline/issues/92) 的接入分层：

| 命令 | 档位 | 一句话 |
| --- | --- | --- |
| `report` | 结果阅读 | 直接把最近一次已提交的 `hospital.md` 和关键 artifact 路径递到人眼前 |
| `artifact query` | 直查 | 从已提交 run 里按 type / schema / contains 直接读证据，不重跑管线 |
| `run execute` | 跑管线 | 权威全量扫描 + 发布到 content-addressed authority root；CI 自扫描步骤用它 |
| `change risk` | 门禁 | 只用 git 历史给 PR 打缺陷风险分，不需要索引 / 网络 / LLM；驱动 `pr-gate.yml` |
| `repin` | 维护 | 扫描（可选改写）全仓过期的 sha256 pin，一趟收敛到不动点 |

```bash
code-intel run execute --repo . --out <run-staging-dir> --authority-root <publication-root>\<repo-name> --final-name <run-id> --manifest orchestration/integrations.json --doctor-require-repowise false
```

`.github/workflows/ci.yml` / `release.yml` 的自扫描步骤就是这行去掉 `\<repo-name>`（`--final-name ci-self-scan` / `release-self-scan`）——CI 只认退出码和 `publication.status == "committed"`，从不回查自己，所以省得了这层嵌套。要接下面的 `artifact query`，`--authority-root` 得把仓库名折进去，如上。

```bash
code-intel artifact query --artifact-root <publication-root> --repo <repo-name> --type diagnosis.hospital-view --limit 1
```

字段契约见 [docs/evidence-query.md](docs/evidence-query.md)。

```bash
code-intel change risk HEAD~5..HEAD --format json
```

没有 `--repo` 参数，从目标仓库内部（CWD）跑；`pr-gate.yml` 用它给 PR 打分：`code-intel change risk "origin/<base-branch>..HEAD" --format json`。

```bash
code-intel repin --write --json
```

干净仓库上是安全的空操作；有过期 pin 时内部最多迭代 25 轮收敛到不动点。

完整参数表见 `code-intel --help --all`，不在这里重复。

## Repowise 语义记忆

Repowise 是可选语义记忆层。`normal` 在可用时使用，`lite` 不依赖它，`full` 才要求所有可选 provider 就绪：

```bash
code-intel C:\path\to\repo --mode normal
```

要缩小 scope，直接把目标路径指向相应子目录，避免把根目录里的外部轮子、临时文件、研究仓库一起吃进去。

```bash
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

示例（设置环境变量即可，三平台相同）:

```text
# MiniMax(Anthropic 兼容端点,现状默认,无需 CODE_INTEL_PROVIDER)
CODE_INTEL_ANTHROPIC_API_KEY=<key>
CODE_INTEL_ANTHROPIC_BASE_URL=https://api.minimaxi.com/anthropic

# 本地 Ollama(无需 key,默认端点 http://localhost:11434)
CODE_INTEL_PROVIDER=ollama
CODE_INTEL_MODEL=qwen3:4b

# 任意 OpenAI 兼容端点
CODE_INTEL_PROVIDER=openai
CODE_INTEL_MODEL=your-model
CODE_INTEL_API_KEY=<key>
CODE_INTEL_BASE_URL=https://your-endpoint/v1
```

## Understand Anything 图谱

如果报告里出现：

```text
graph_missing: understand graph
```

先运行项目内 Rust 图谱 provider：

```bash
code-intel graph --repo C:\path\to\repo --language zh --write --json
```

完整重建：

```bash
code-intel graph --repo C:\path\to\repo --language zh --full --write --json
```

然后重新运行 pipeline。`/understand C:\path\to\repo --language zh` 只作为兼容兜底，或在你明确需要外部 Understand Anything 更富图谱时使用。

## 全局 Provider Route

Repowise 和 Understand-compatible graph 统一走 `code-intel provider` 规范，再由 `code-intel route` 暴露入口：

```bash
code-intel provider --action Validate --json
code-intel provider --action Plan --provider repowise --operation index --repo C:\path\to\repo --json
code-intel provider --action Plan --provider understand --operation graph --repo C:\path\to\repo --json
code-intel route --action List --json
code-intel route --action Plan --provider repowise --operation index --repo C:\path\to\repo --json
code-intel route --action Plan --provider understand --operation graph --repo C:\path\to\repo --json
```

HTTP route 使用命名空间：`/api/providers/repowise/*`、`/api/providers/understand/*`。旧的 `/scan`、`/lite`、`/doctor`、`/understand` 只能作为兼容入口。

## 规则文件

把模板复制到你的项目 scope：

```bash
mkdir -p /path/to/repo/backend/.sentrux
cp templates/sentrux-rules.example.toml /path/to/repo/backend/.sentrux/rules.toml
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

```bash
code-intel sentrux gate_save C:\path\to\repo\backend
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

```bash
code-intel C:\path\to\repo\backend --mode normal
```

如果你要分析 `tools/` 里的某个外部轮子，把 scope 指到那个轮子，而不是让它污染主项目：

```bash
code-intel C:\path\to\repo\tools\some-lib --mode normal
```

## 真实跑通过的路径

本仓库已经用 `code-intel <path> --mode normal` 验证过：本项目自身、GitHub fresh clone，以及 Katana 大仓库（scope 指到 `backend`）。

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
install-smoke -> doctor -> tests
```

install-smoke 在 windows / macos / linux 三个 runner 上从 Release 形态的 ZIP 完整走一遍安装。

## 常见问题

### `Understand graph missing`

运行：

```bash
code-intel graph --repo C:\path\to\repo --language zh --write --json
```

再重跑 pipeline。

### Repowise 很慢

先 scoped：

```bash
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
2. `code-intel sentrux gate <scope>` 确认基线。
3. 让 Agent 只处理第一目标。
4. 跑测试。
5. 再跑 `code-intel sentrux gate <scope>` 收门禁。
6. 重跑 pipeline。

不要一上来让 Agent 全仓库乱修。那不是工程，是把混乱交给更快的混乱。

## 给 Agent 的一句话

先安装（见[安装](#安装)），再跑 `code-intel doctor`，再跑 `code-intel .`。改代码前先读证据：整轮 outcome 看命令行汇总，失败看 `run-manifest.json` 里的失败节点，导航用 `evidence.native-code/code-evidence/merged/agent/index.md`，治理看 `diagnosis.hospital/hospital.md`，行动计划看 `diagnosis.hospital/surgery-plan.md`。改代码时继续问管线：改前 `code-intel sentrux gate` 确认基线，`change impact --staleness advisory` 查波及面和该跑的测试，机械改写先 `capability exec edit.ast-grep-plan` 出预览计划，改完再跑 `code-intel sentrux gate` 收门禁。不要跳过 Sentrux baseline 和 rules，不然 Agent 只是换了个速度更快的方式堆债。

## License

MIT

## Rust CLI resume preview

```bash
code-intel orchestrate --action Validate --json
code-intel orchestrate --action Plan --repo /path/to/your/repo --mode normal --json
code-intel report --repo /path/to/your/repo
code-intel resume --repo /path/to/your/repo
code-intel resume --repo /path/to/your/repo --artifact-root /path/to/artifacts
code-intel resume --repo /path/to/your/repo --json
code-intel classify --report /path/to/artifact/report.json
```

The Rust CLI owns the default normal production spine, integration orchestration,
snapshot-bound evidence, atomic run publication, committed-only indexing, and
cross-session query/impact reads.
