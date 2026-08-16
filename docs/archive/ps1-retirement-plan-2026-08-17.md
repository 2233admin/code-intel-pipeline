# PS1 全砍执行清单(code-intel-pipeline)

> 状态:**等待 Codex 落定**(4 个 open PR 合入 + `cargo test` 全绿)后执行。
> 依据:工具仓 AGENTS.md「Do not perform a big-bang PowerShell deletion. Retire each
> compatibility entry point only after its Rust replacement passes the existing
> contract tests.」→ 全砍必须走 retirements 流程,逐入口验证。
> 2026-08-17 建立。基线:分支 `release/v0.7.x-rc`,HEAD `f6b0bf5`。

## 前置门禁(不满足不开砍)

1. `gh pr list --state open` = 0 或仅剩与本任务无关的 PR(当前 4 个:250/249/248/202)
2. `cargo test -p code-intel` 全绿(当前 643/644,失败:
   `registry_toolchain_digests_bind_the_adapter_and_dispatch_sources` =
   `capability_inventory.rs` 的 toolchain digest 未同步,Codex 改了一半)
3. 工具仓工作区干净(`git status` 无 M/?? 于 `crates/`)

## 一、被跟踪 ps1 全量清单

### 1. 根目录(2 个,活入口)
- `invoke-code-intel.ps1` — lite/normal 主入口,调 `legacy/check-code-intel-tools.ps1` + `legacy/update-code-intel-index.ps1`
- `run-code-intel.ps1` — 发布入口(678B)

### 2. legacy/ 顶层(11 个,活入口)
- `code-intel.ps1` / `invoke-code-intel.ps1` / `run-code-intel.ps1`
- `check-code-intel-tools.ps1`(doctor)
- `update-code-intel-index.ps1`(repowise 索引)
- `install-code-intel-pipeline.ps1`(安装器,CI 在用)
- `bootstrap-new-machine.ps1`
- `OpenSpec-Detector.ps1`
- `Invoke-SentruxAgentTool.ps1`(2646 行,health/scan/dsm/session/what_if)
- `Invoke-ScopedRepowise.ps1` / `Invoke-RepowiseProviderProbe.ps1`
- `Invoke-RepomixCodePack.ps1` / `Invoke-ProviderRuntimeInventory.ps1`
- `Invoke-NativeRetrievalBenchmark.ps1` / `Invoke-MultiAgentWorkspacePreflight.ps1`
- `Invoke-MultiAgentMergeQueue.ps1` / `Invoke-ModelChannelDelegate.ps1`
- `Invoke-GreenfieldSpecExtraction.ps1` / `Invoke-GitHubSolutionResearch.ps1`
- `Invoke-CompeteProjectScore.ps1` / `Invoke-CompatibilityFacadeFinalize.ps1`
- `Invoke-CodeNexusLite.ps1` / `Invoke-CodeIntelOrchestrator.ps1`
- `Invoke-CodeIntelAutomaticPullRequest.ps1` / `...PullRequestFlow.ps1`
- `Invoke-CodeIntelAcceptance.ps1` / `Invoke-CodeEvidenceABTest.ps1`
- `Invoke-CccSliceBenchmark.ps1` / `Install-SentruxVlangOverlay.ps1`
- `Install-MultiAgentMergeQueue.ps1` / `Find-CodeIntelProjects.ps1`
- `New-ModelAdapterRequest.ps1` / `New-ModelExecutableHandle.ps1`
- `Invoke-WorkflowRecommendation.ps1`

### 3. legacy/scripts(51 个)
测试/工具脚本,多数被 CI 引用(见下)。

### 4. legacy/tools(37 个 ps1 + 3 个非 ps1)
- 非 ps1 保留:`tools/code-intel-follow-up-automation.psm1`、
  `tools/code-intel-platform.psm1`、`tools/sentrux-shim/sentrux`(Vlang 二进制)
- ps1 全部退役,含 `tools/check-hardcoded-paths.ps1`(AGENTS.md 有引用,需换 Rust 等价或删除该规则)

### 5. legacy/tests(5 个)

### 6. orchestration/retirements/*/rollback-rehearsal/(6 个)
- `e04-codenexus-direct` / `e05-publication` / `e07-native-code` / `e09-doctor-wrapper` / `e10-index` 下的 rollback-rehearsal ps1
- **保留**:它们是退役演练资产,记录回滚路径;若 Rust 等价已覆盖,移到 evidence 或标注 legacy-only

### 7. dist/(未跟踪,构建产物)
`dist/` 未被 git 跟踪(0 文件),无需删除,但构建流程若生成 ps1 需改。

## 二、引用面(删前必须全部改)

| 引用点 | 内容 | 处置 |
|---|---|---|
| `.github/workflows/ci.yml:127-151` | PowerShell parser checks 列表(28 个 ps1) | 删除整个 step,或改为 rust parser 验证 |
| `.github/workflows/parity-observe.yml` | parity 观察:legacy ps1 vs Rust | 全砍后无 legacy 可对比 → 整个 workflow 退役 |
| `.github/workflows/release.yml:142-211` | install-code-intel-pipeline.ps1 + 4 个测试 ps1 + payload 打包 | 换 Rust 安装/测试,payload 不含 ps1 |
| `.github/workflows/skill-check.yml` | (需查) | — |
| `.githooks/pre-push:17` | `legacy/Install-MultiAgentMergeQueue.ps1` | 换 Rust 等价或删除提示 |
| 工具仓 `AGENTS.md` | 23/52/53/63/64/70 行引用 ps1 | 换 `code-intel` CLI 命令 |
| 工具仓 `docs/decisions/README.md` | (需查) | — |
| tdxcli-rs `docs/agents/work-pipeline.md:57-179` | `check-code-intel-tools.ps1` / `invoke-code-intel.ps1` / `Invoke-SentruxAgentTool.ps1` / `run-code-intel.ps1` | 换 `code-intel` 二进制命令(`code-intel doctor`、`code-intel run`、`code-intel sentrux ...`) |
| tdxcli-rs `AGENTS.md` | (需查) | — |

## 三、Rust 等价命令(退役替代)

```bash
code-intel run lite|normal <repo>          # 替代 invoke-code-intel.ps1
code-intel doctor <repo>                    # 替代 check-code-intel-tools.ps1
code-intel sentrux scan|health|dsm|session_start|session_end  # 替代 Invoke-SentruxAgentTool.ps1
code-intel run execute                      # 替代 run-code-intel.ps1
# repowise 索引:repowise_adapter / ProviderRepowiseAdapt 路由
```

验证方式:对同一 repo 跑 ps1 与 Rust 命令,`parity-observe` 已有对比框架
(`legacy/scripts/tests/test-ps1-rust-parity.ps1`),全砍前需跑最后一次 parity 并留档。

## 四、执行顺序(Codex 落定后)

1. `cargo test -p code-intel` 全绿 → 记录 parity 留档
2. 删除根目录 2 个 ps1 + legacy/ 顶层 11 个活入口(先验证 Rust 等价)
3. legacy/scripts、tools、tests 全部 ps1 → 退役(测试脚本迁 Rust 测试或删除)
4. 改 CI 4 个 workflow + .githooks/pre-push + 工具仓 AGENTS.md/docs
5. 跨仓:tdxcli-rs work-pipeline.md + AGENTS.md 换 Rust 命令
6. 构建产物 dist/ 移除 ps1 生成
7. 最终验证:`git grep -l "\.ps1"` 于工具仓 = 0(或仅剩 rollback-rehearsal 标注资产)
8. 提交:工具仓走 `orchestration/retirements/` 流程逐入口退役(AGENTS.md 门禁),不 one-shot 大删除提交

## 四-A、硬依赖:Rust 显式引用 ps1(全砍阻断项)

2026-08-17 实测 `code-intel provider --action List` + grep 确认,**3 个 ps1 被 Rust 源码显式引用为适配器,不是死代码**:

| ps1 | Rust 引用点 | 职责 |
|---|---|---|
| `legacy/Invoke-CodeNexusLite.ps1` (424 行) | `builtin_provider_evidence.rs:276`、`providers.rs:163`、`orchestration.rs:278` | codenexus 上下文生成:选 hotspot 文件 + git log + 产出 `codenexus-context.json`(schema `code-intel-codenexus-evidence.v1`, implementationId `invoke-codenexus-lite.ps1`) |
| `legacy/tools/check-hardcoded-paths.ps1` (77 行) | `.github/workflows/ci.yml:697`(CI 强制 step)+ `AGENTS.md:52`(push 前门禁) | 扫描 tracked `*.ps1/*.psm1/*.md/*.yml`,查 5 个字面模式(`C:\Users\Administrator`、`powershell.exe`、`LOCALAPPDATA`、`USERPROFILE`、`APPDATA`)+ `X:\...\code-intel-pipeline` 路径,`$env:VAR` 豁免;exit 1 失败。**命令面无 Rust 等价**(2026-08-17 实测 `--help --all` 无 path 检查命令) |

**处置**:`check-hardcoded-paths.ps1` 逻辑简单(77 行、正则扫描),全砍时在 Rust 侧加一个 `lint hardcoded-paths` 子命令(或并入 `audit`)替代,再删 ps1、改 ci.yml:697。

**处置**:全砍前必须先验证 `crates/code-intel-cli/src/codenexus_scratch.rs` / `codenexus_adapter.rs` 是否已实现同等输出。若未实现 → 这是**迁移任务**(把 424 行 ps1 逻辑搬 Rust),不是删除任务;若已实现 → 改 `providers.rs:163` 的 command_template 指向 Rust,再删 ps1。
另有 `repowise:lite` 路由同样指向该 ps1,一并处理。

## 五、风险

- Codex 的 21 个未提交 Rust 文件可能已实现部分 ps1 路由(legacy.rs/command_catalog),
  落定后需重新核对清单,可能有 ps1 引用已删除
- **`Invoke-CodeNexusLite.ps1` 是 Rust 的活适配器(2026-08-17 实测)**,直接删会断
  `provider codenexus-lite` / `repowise-lite` 两条路由 —— 必须先迁 Rust 或改路由
- `Invoke-SentruxAgentTool.ps1` 的 session_start/session_end 是 tdxcli-rs 工作流强依赖,
  Rust 等价(如果存在)必须先实测,再改文档
- 工具仓 CI 的 PowerShell parser checks 是全量语法门禁,删除后需确认 Rust 侧
  有等价解析验证(或接受删除)
