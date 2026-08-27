# Changelog

All notable changes to **code-intel-pipeline** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Dispatch budget primitives**：新增 `Budget` 类型做二维（wall-clock、输入字节）派发成本控制，三级优先链（CLI flag > config > 内置默认），配 `estimate_ok`/`consume`/`exceeded` 查询方法（#308）。
- **Bounded DAG dispatch**：派发前逐节点过闸，保留已完成 artifact，显式发布带 not-dispatched 原因的 budget 结果（#312，closes #305）。
- **超大输入预派发拦截**：新增可配置 `OversizePolicy`（默认 `Budget::bytes_limit` 的 80%），在 `Budget::estimate_ok` 之前拦截，超阈值节点标记 `SkippedOversize`（#307/#311）。
- **单节点超时独立分类**：新增 `node_timeout.rs::run_with_timeout`，超时后杀整个进程树（Windows `taskkill /T`，其余平台 `kill`），不再产生孤儿进程，超时不再误落进 `domain_unknown`（#306/#310）。
- **weco 性能优化通道**：doctor bootstrap 探测新增可选外部工具 weco（AIDE 风格 perf-optimize CLI），支持 BYOK；对已有 benchmark/eval 脚本的目标可发起一次去噪、budget-bounded 的优化 pass 并出报告，不改代码、不开 PR（#300/#303，#301/#313）。
- **advisory.workflow-recommend 接入 run execute DAG**：`-SkipOpenSpec`/`-AutoOpenSpec` 补齐 `ProviderPolicy::open_spec` 门控，是 T2 launcher 最后一个未接入 Rust DAG 的开关（#315）。
- **BMAD 作为第四个 advisory-workflow-recommend 候选**：抽出 `ALTERNATIVE_CANDIDATES` 注册表，新增候选不再需要改 `recommend()`；新增 `orchestration/internalization/bmad.json`（ADR-0010 Internalization Standard）。
- **确定性报告质量诊断**：新增私有 report quality evaluator，对 evidence anchor、surgery verification、snapshot scope 做确定性检查，v1 hospital artifact 形状不变（#317/#318）。
- **知识图谱生成 Rust 化**：新增 `code-intel graph --repo <path> --write` 生成知识图谱（rust-first/manual-fallback，`CODE_INTEL_GRAPH_PROVIDER=rust|manual`）；现存的 `legacy/run-code-intel.ps1` 兼容路径的 `graph.code-intel-understand` 阶段随之接入该 CLI（此前是该 orchestrator 唯一纯文本"生成"步骤）（#321）。
- **`scan.ast-grep-security`：原创 ast-grep 安全规则库 + advisory-only 扫描能力**（#345）。`orchestration/ast-grep-rules/<lang>.yaml` 覆盖本管线自身识别的 7 种语言（python/javascript/typescript/go/rust/java/csharp），每语言一份独立编写的 security 规则文件（SQLi、命令注入、eval/exec、不安全反序列化、硬编码密钥、弱哈希等常见 CWE 类别），规则概念来源于 vitali87/code-graph-rag 的规则分类思路但**不 vendor 其源文件**（`orchestration/internalization/ast-grep-security-rules.json` 记录 reimplement rung 与 MIT 义务）。新 capability 走 CLI 版 `ast-grep scan --rule`，产出 `code-intel-ast-grep-security-findings.v1` artifact，`authority.mode: advisory_only`，从不进 `get_gate_verdict`、从不提议改写；MCP 新增第 7 个工具 `scan_security_findings`。

### Fixed

- `sentrux gate` 在全新 checkout 上因缺基线硬崩溃：missing-baseline 前置检查原本查的是 native baseline 路径，实际 `sentrux gate` 读的是 lite 引擎的 `.sentrux/cache/lite-baseline.json`，现已对齐，缺失时正确落回 `manual_required`（fixes #322）。
- 安装器在显式传入新的仓库或发布根目录时不再被陈旧的 `$env:CODE_INTEL_HOME` 覆盖，避免新二进制读取旧的 Provider manifest（fixes #363）。
- 修复 workflow recommendation 的 Rust 侧 parity 回归（#314）。
- `sentrux_gate` 全量并行测试踩踏两处根因分开修：(1) `tool_path.rs` 的 `path_search_skips_relative_entries` 探测目录名只用 `module_path!()`，在被 `#[path]` 编入多个 `cargo test` 二进制（`run_commit`/`survival_scan` 等）时对同名父模块编译时相同，跨进程共享同一 `target/tool-path-*` 目录并互相踩踏；现补上 `std::process::id()` + 进程内 `AtomicU64` 计数器，与 #175 的 `unique_temp_dir()` 同一套 pid+nonce 写法（fixes #178）。(2) `sentrux_gate.rs`/`boundary_rules_tests.rs` 里 9 处 `run_check(...).expect(...)`/`run_gate(...).expect(...)` 把"引擎子进程没跑起来"和"仓库真的检测到规则违规"报成同一个红灯；新增仅测试用的 `expect_check_ran`/`expect_gate_ran` helper，`Err` 分支 panic 出明确区分于业务断言的文案，并配 `#[should_panic]` regression 测试直接执行 `Err` 路径验证文案本身（fixes #192）。

### Changed

- (a) 类 CI/release PowerShell 调用点归零：atomic-capability 与 project-management-support 契约锁改为 cargo 测试，PS1 脚本保留在磁盘上（PM 走 internalization pin，atomic 保留避免耦合棘轮）；退役 35 个孤儿 legacy PowerShell 文件（#296/#298/#319）。

## [0.7.2-beta.6] — 2026-08-19

### Fixed

- 发布包验证把安装后的 `bin` 加入进程 `PATH`，让 doctor 校验安装拓扑，同时保持 Rust 内置 Sentrux provider 在 Unix 上可用。

## [0.7.2-beta.5] — 2026-08-19

### Fixed

- 发布包的 packaged Sentrux closure 显式把安装后的 `bin` 作为 doctor tool-path prefix，确保 doctor/capability executor 发现同一套安装内置 Sentrux binary。

## [0.7.2-beta.4] — 2026-08-19

### Fixed

- 发布包验收在 bootstrap 安装后继续使用实际 `release_root` 作为 repo，确保 packaged Sentrux closure 和 doctor readiness 验证覆盖真实安装拓扑。

## [0.7.2-beta.3] — 2026-08-19

### Fixed

- 发布包验收现在先通过本地 zip 走 bootstrap 安装拓扑，再使用安装后的 binary 执行 packaged Sentrux closure；修复 release workflow 直接从解压目录运行导致 doctor readiness 误报失败的问题。

## [0.7.2-beta.2] — 2026-08-19

### Added

- **Sentrux capability automation**：能力矩阵、自动发现与执行、lite fallback、统一 artifact/schema 和结果闭环接入报告、影响分析、测试选择、PR gate 与 release gate；发布包增加能力级 smoke 验收。
- **安装拓扑 smoke**：跨平台 CI 通过真实安装后的 graph binary 验证 `doctor bootstrap`，覆盖安装后路径与能力发现边界。

### Fixed

- 刷新合并后受 frozen source set 约束的 E04/E07/E08 compatibility retirement packets，避免 Windows 发布门禁因 stale snapshot 误报。

## [0.7.2-beta.1] — 2026-08-13

### Added

- **语言感知的快速反馈规划（#261）**：Rust 集成测试收窄到所属 `Cargo.toml` 与精确 `--test` 目标；导入图没有测试边时，改用 crate 内 `--lib`/`--bins` 加模块过滤器，并显式提示仍需完整套件收尾。Python 根据 `uv.lock` 选择 `uv run pytest`，JavaScript/TypeScript 根据 `packageManager` 或锁文件选择 bun/pnpm/yarn/npm。Cranelift 保持可选且先测量，不替代稳定 LLVM 完成门禁。

### Fixed

- **安装器子进程钉死 `CODE_INTEL_HOME`（bootstrap.py）**：安装器会把 `CODE_INTEL_HOME` 持久化进用户环境，而它取值来自子进程环境；此前安装器子进程未传入钉好的环境，调用者 shell 里 MSYS 风格 `CODE_INTEL_HOME`（如 `/d/projects/...`，Windows 会解析成 `C:\d\projects\...`）会被原样写进用户注册表并毒化后续运行。现在安装器与 doctor 都使用钉到 release root 的同一环境。
- **测试套件在开发者机器上的 hermeticity 硬化**（#218 修复过程发现，独立提出）：
  - `file_gate/walk.rs`：遍历中目录消失（NotFound）容错为跳过，消除 `sentrux_gate` cycle 检查与 `tool_path` 临时目录删除的并行竞态
  - `internalization_record`：fixture git 清空 `core.excludesFile`，不再被用户全局 ignore（如 `*.bin`）拦截
  - `native_code_evidence`：legacy pwsh facade 清除 PIPELINE_VARS，不再继承 shell 的 `CODE_INTEL_HOME` 指向旧 manifest
  - `snapshot_identity`：用 `.git/shallow` 边界手工构造 shallow 仓库，避开 Windows 8.3 短路径下 `file://` clone 失败

### Changed

- Repowise 的精确运行时 pin 从 `0.36.0` 升到当前已评审的 PyPI 最新版 `0.38.0`。安装仍然使用精确版本以保持供应链可复现；版本清单要求运行时 provider 只通过显式审阅的 pin 更新，不在安装时追随浮动 latest。

## [0.7.1] — 2026-08-08

### Fixed

- **安装后 discovery 误把 bin-forwarder manifest 当仓库根（#218）**：安装器会把 `orchestration/integrations.json` 复制到二进制旁边以便脱离 checkout 解析能力清单，但两条 discovery 路径都对这份副本照单全收、把 `<bin>` 当仓库根，导致干净的 v0.7.0 安装报出约 40 条相对入口缺失的假错误。现在 exe 祖先目录候选只是猜测而非配置：命中必须至少解析出一个 manifest 文件入口才算数，否则回落到安装器必写的 `CODE_INTEL_HOME`；`orchestration::resolve_manifest_path` 同享这套探测，并补上它一直缺的两层——`CODE_INTEL_INTEGRATIONS_MANIFEST` 显式覆盖与 `CODE_INTEL_HOME` 回落。README 同步删掉「macOS/Linux 无发布 ZIP、bootstrap.py 仅限 Windows」的过时说法（v0.7.0 起三平台齐发）。

## [0.7.0] — 2026-08-07

首个正式版切割。#14 的发布闸验收条款本身早已做完（自扫门禁、`dag_run`/`execution_kernel` 循环依赖修复，issue 自己的 checklist 勾过）；剩下未勾的 session-intelligence/2D viewer 是另一条独立多版本 roadmap，跟能不能发 GA 无关，留 backlog。#158 复核后没有整条延后：Skill 安装路径升级为先验 GitHub Artifact Attestation 再退回 SHA-256（`gh` 缺失时显式降级，不静默）；跑了一次真实的 supply-chain 自审（`orchestration/audit/reports/audit-report.json`，8.0/10，一条确认发现——发布 tag 未签名也没有 ruleset 保护）；对已发布的 v0.7.0-beta.6 三平台 ZIP 实测 `gh attestation verify` 全过，记录在 `docs/release-provenance-runbook.md`；OpenSSF Best Practices 逐条自评进 `docs/openssf-best-practices-gap.md`。只有「签名 tag + tag 保护 ruleset」明确留给维护者（改仓库设置不该由 agent 单方面做），继续在 #158 追踪。

### Added

- **契约登记表与 schema 发布面对账闸（#206 第一刀）**：`registered_contract()` 接受的每一对 `(schema, type)` 现在都必须有对应的 `orchestration/schemas/<id>.schema.json`，否则单测红灯。判据不另立清单——registry 是 `match` 臂，臂本身就是清单，所以检查从 `artifact_ref.rs` 自己的源码把臂读出来，再把读出的每一对喂回 `registered_contract()` 核对；解析一旦跟真臂漂移，喂回去就不认，测试炸而不是静默放行。家族名单也不手写：从 `registered_contract` 自己的分派体里读出它调用了哪些 `*_family_contract`——新家族要进扫描面，唯一的路径正是进生产的那条路径。三道假绿灯陷阱：臂数少于 40 判定「扫描器瞎了」（第一版实现正是被这条抓到的——按第一个 `#[cfg(test)]` 截断源码，漏掉整个 native code-evidence 家族，只解析出 32 条）；家族数少于 7 判定「分派体丢了」；**解析不了的臂是硬失败，不是跳过**——`(CONST_A, CONST_B) =>` 这种写法如果被静默丢弃，该契约就在总数依然健康的情况下逃出 schema 检查，所以常量臂改为查 `const NAME: &str` 表解析，解析不出直接炸（实测：临时塞一条 `&'static str` 声明的常量臂，门禁报 `uses a form the registry scanner cannot resolve`）。markdown 视图按规则豁免（`<x>-markdown.v1` 只要 `<x>.v1` 也已注册即可），native code-evidence 家族解析伞状 schema 的 `oneOf` 分支 → `$defs` → `properties.schema.const` 来豁免，不是在文件里搜字符串——出现在 title、example 或无关字段里的 id 不算覆盖。`AWAITING_SCHEMA` 豁免表的判据是：表里的 id 一旦有了 schema 文件、或不再被注册，都必须删条目；**表变长本身没有被门禁挡住**（那需要 merge-base 或 CI 持有的基线来比对，记在 #210，这里不声称）。当前基线：41 个已注册契约，24 个已发布 schema，4 个 markdown 视图，8 个走伞状 schema，**5 个真缺口**入 `AWAITING_SCHEMA`。
- **`code-intel serve --mcp`：agent 原生查询面上线，管线在写码中途终于能被问一句话**（#54、#58 提案 3）。stdio MCP server，六个工具：`get_gate_verdict`（权威 run 的门禁结论 + 第一条失败规则 + 最小重跑命令）、`get_facts`（按 artifact type/schema/子串查已验证事实）、`get_evidence`（一条 finding 的证据链：产物、sha256、记录时的 snapshot；查不到明说 `unbacked`，不装真）、`get_audit_status`（各科室结论；没跑过 audit 明说 `unavailable`，不装绿）、`get_change_impact`（默认 stale-advisory——CLI 在这里 fail-closed 正是管线写码时隐形的原因，#58 定为 critical）、`plan_structural_edit`（ast-grep 预览，`repositoryMutation=false`）。**只读，不裁决**：门禁判定照旧只走 CLI 与 CI，查询面被 prompt injection 说服也改不了结论；唯一会执行东西的 `plan_structural_edit` 在跑之前拿注册表核对自己的 capability 声明，一旦出现 `repo_mutation` 直接拒绝（有测试为证）。路径参数复用 `change_impact` / `evidence_query` 的既有请求类型，JSON 进来的路径和 `--changed` 打进来的走同一道越界闸。工具拒答走 `isError` 结果而不是 JSON-RPC error——"还没跑过 run"是答案，不是传输故障。声明的 MCP 版本收窄到 `2025-06-18` 一版：更早的修订要求 JSON-RPC 批量支持，本 transport 一行一条消息，宣称就会让客户端发批量后挂死；走错路的批量收到显式 `-32600` 而不是被静默丢弃。仓库 `.mcp.json` 已注册；README 与 SKILL.md 改为主推查询面，全量扫描降为深检模式，并按工具写清数据来源（四个是已提交 run 的投影，`get_change_impact` 是已提交 import 图 × 当前工作树，`plan_structural_edit` 扫的是当前工作树）。

### Fixed

- **mattpocock/skills 内化记录的上游出处一直是「没查」，而记录里那枚 ADR digest 早就过期、`code-intel repin` 报绿（#133 同因第三例）**：记录声明 `revision: "unverified-upstream; local-adr-sha256:1431a4df…"`，但 `docs/adr/0006-project-management-support-as-agent-intake.md` 的实际 digest 是 `7a0e7da5…`。`code-intel repin` 之所以报 clean，是因为这枚 pin 只活在 `revision` 字符串里，旁边没有 `ownedModifications[].sha256` 的 `{path, sha256}` 声明——`declared_pins` 的模块文档写明了这正是它的盲区，本记录是实例。三处一起修：上游修订钉到 `84fdeffd12f2ee307994d1eb6feb48173b6e0502`，license 实测为 MIT（LICENSE blob `f1dd2c09…`）并把义务从「查清前禁止一切」换成真实的 MIT 保留声明义务，ADR digest 补上 `{path, sha256}` 声明从而进入 `code-intel repin --write` 的可解析面。测试侧同步分家：`assert_research_candidate` 钉的是「完全没查」的形状（`unverified-upstream` + `UNKNOWN-RESEARCH-ONLY` + license/upstream-revision 两条 gap），已核实的记录再套这张断言就等于核实完还继续通过未核实契约，所以新增 `assert_verified_research_candidate`，并断言 mattpocock 记录**只剩** `gap:mattpocock-skills:security-review` 一条——核实出处不等于读过上游 skill 到底干什么，这条 gap 继续挡住它离开 research。

### Notes

- **正式版已知延后项**（发布说明须明写，别让人以为已查过）：性能面 579 条 I/O-in-loop / N+1 静态发现未处理；`artifact_ref.rs` defect score 1.85/10、3807 行，仍是仓内最差且本轮只加了检查没拆；33 个 god file 由 `.sentrux/baseline.json` 按身份豁免（不是消失）；5 个已注册契约无 schema（`code-intel-anchor-verification.v1`、`code-intel-file-inventory.v1`、`code-intel-method-catalog.v1`、`code-intel-sentrux-command-observation.v1`、`code-intel-surgery-plan.v1`）；内化记录里 104 枚本地 digest pin 未用可解析形状声明，其中 37 枚已对不上任何在册文件而 `code-intel repin` 仍报 clean（#133）。

## [0.7.0-beta.6] — 2026-08-05

这一批的主线是「门禁自己说的话得能被核对」：扫描面、锚点、巨石身份、退出码、schema 五处都从体感口径换成可计算口径——覆盖率不再靠感觉、锚点不再靠猜、巨石不再靠数数、gate finding 不再伪装成崩溃、产物不再违反自家 schema。另一条线是写路径的第一块落地（span 寻址补丁）和文档语言首选项。

### Added

- **`code-intel edit apply` + `edit.span-apply` 能力：span 寻址补丁，终结"改一个字重写整行"**（#96 item 1、charter gate G4 #139）。`--span <startLine:startColumn-endLine:endColumn> --expect-sha256 <该 span 当前字节的 sha256> --replacement <text>`，行列 1-based、结束列开区间；同一文件可带多个互不重叠的 span，全部对改动前字节定位、写临时同级文件后 rename，整文件原子替换。写之前逐 span 比对 digest：不一致即拒绝（退出码 10、`applied:false`），产物给出 `expectedSha256` / `foundSha256` / 有界原文，且信封 `observedEffects` 不含 `repo_mutation`——"没写"是机器可校验的，不是自报的。写路径全程走既有 capability envelope（`edit.span-apply.compat`，`allowedEffects` 含 `repo_mutation`），能力自身在动手前先检查请求的 effectPolicy，把事后审计变成事前门。不含 ast-grep 接线（下一档）。
- **锚点验证闸：三态 `verified` / `approximate` / `dropped` + 计数出账（#151）**。`run execute` 发布前新增一道锚点验证：文件锚点（路径是否还在）、行区间锚点、符号锚点（声称的名字能否在声称的 `file:startLine` 重新解析出来）——只在声称的那一个文件内找，不做全仓搜，解不出就是解不出。三态而非两态：`approximate` 表示同名符号还在同一文件、只是行漂了，带纠正后的行号；`dropped` 表示文件不在了或文件里再也找不到这个名字，带原因。状态类型仿 G1 的 `EvidenceOutcome`（#141）：`Approximate` 造不出来除非真带纠正行号，`Dropped` 造不出来除非真带 reason，`from_json` 对每个状态做穷尽 key 校验，把「只改 state 字段、留下另一状态残留字段」挡在反序列化边界上。
- **扫描面具名排除闸门：覆盖率从体感变计算，38 个失踪文件归位（#152）**。`included_files: 315` 只列 2 条排除、38 个文件消失且无法归因，根因不是精度问题，是 `scan` 与 `dsm` 各自维护一套排除规则会悄悄漂移。用 `file_gate::evaluate` 实测定位到两个真实 bug：`dsm` 的 `excluded_reason` 只把 `tools`/`vendor`/`third_party`/`external` 当 `parts.first()` 检查，`legacy/tools/*` 这类嵌套路径对它不可见（本仓自扫一次贡献 39 个文件的分叉）；`dsm` 的 `SOURCE_EXTENSIONS` 漏了 `cpp`/`c`/`h`/`hpp`，命中文件在 dsm 里连「被排除」都算不上。排除判据统一到单一模块并从磁盘发现，每条排除具名出账。
- **文档语言首选项：优先级链 + locale 默认 + 只在交互式才问（#155）**。新增 `crates/code-intel-cli/src/language_pref.rs`：`--language` 标志 > 项目配置 > 用户配置 > 系统 locale > `en`，链条声明为纯函数（`resolve_from`）并逐档单测，不读用户对话历史或其它文件去猜语言。项目级配置落在 `<repo>/.code-intel/config.json`（和 `.sentrux/`、`.understand-anything/` 同级的仓根点目录，不塞进 `.sentrux/` 内部——语言是管线级设置，不是质量闸门子系统私有的）；用户级落在平台 data root 下的 `config.json`，复用 `doctor_bootstrap::paths` 的三平台 OS switch。

### Fixed

- **execution-result schema declared `additionalProperties: false` yet omitted the `failures` field it always emits (#168)**: every `code-intel run execute` output was invalid by its own schema; nothing caught it because the existing schema validation step does not enforce the closed-world bit. The schema now declares `failures` (`{process: [{node, diagnostic}], domain: [{node, verdict}]}`, shapes taken from `to_execution_json()` / `execution_kernel::failures()`, not invented) and lists it as required. A strict in-process key check now runs against a real `code-intel run execute` output in `dag_run.rs`, with a negative control proving an undeclared extra field turns it red. The 88-schema emitter sweep from #168 remains open.
- **Exit-code semantics conflated gate findings with process failure (#130)**: `legacy/run-code-intel.ps1` exited `1` whenever `sentrux gate` reported architecture/quality debt (e.g. `god_files 23 -> 25`) in the target repo — identical to a genuine tool crash, even though every artifact was produced intact. Exit codes are now layered: `0` clean, `2` pipeline completed with Sentrux gate findings (see new `report.summary.gateFindings`), `1` the pipeline genuinely did not complete. Scoped to `sentrux gate` only; `sentrux check` keeps its prior behavior. See `docs/artifact-data-contract.md#exit-code-contract-issue-130`.
- **God-file ratchet compared counts against a stale baseline — three real branches each shipped a new god file with every self-scan green (#165)**: the ratchet now compares file *identities*. `.sentrux/baseline.json` moves to schema v5 and records every tolerated god file by path with its measured loc/functions and the rule branch it trips; any god file the baseline does not list fails `god_files_increased`, and the violation names the file, the branch (`loc>800` or `functions>25&&loc>400`), and the measured values — files, not counts (#148 C1). Count slack and fix+regress swaps can no longer hide a new monolith. Baselines without the identity list fail closed as `baseline_engine_mismatch`. A green gate now also reports reclaimable slack ("N god file(s) no longer over threshold") since the gate itself never rewrites repository state. Decision recorded in `.sentrux/rules.toml`: test functions count toward the functions limit (option B) — the convention that tests live in a module directory's own `tests.rs` is what keeps production files under the limit; the cfg-aware alternative is explicitly rebutted there. The repository's 33 standing god files are grandfathered by identity, not amnestied.
- **测试不对称信号不认本仓自己的 `tests.rs` 约定——96 百分位假阻断（#170）**：`is_test_file` 的三条判据全都漏掉内联测试模块约定（`change_risk/tests.rs`、`change_agenda/tests.rs`、`file_gate/tests.rs`：最后一段是 `tests.rs` 不是 `tests` 目录，文件名不以 `test_` 开头，词干 `tests` 不以 `_test` 结尾）。后果不是理论上的：PR #166 加了 229 行 `file_gate/tests.rs`，门禁却报「source changed, no tests touched」并以 96 百分位挡下它——而测试不对称是公式里权重最大的单项（`WEIGHT_TEST_ASYMMETRY = 25`）。
- **sentrux shim 的 lite 门与权威 baseline 分居（#182）**：lite 门的缓存迁入 `.sentrux/cache`，撞上 baseline schema v5 不再崩、不再覆写权威 baseline、不再静默回填。session 门锁的 lite 语义直调 lite-core，`SENTRUX_CORE_EXE` 作为显式注入缝。
- **测试临时目录按进程归属，不再只按时钟命名（#175）**：`switching_documentation_language_changes_only_the_language_field` 在 windows-latest 上偶发红——一条断言确定性的测试自己不确定，训练出「重跑一下就好」的假绿灯文化。机制是 `graph` 模块被 `#[path]` 编进 12 个集成测试二进制、由 cargo 当独立进程并行跑，而 `unique_temp_dir()` 的目录名只由 `SystemTime::now()` 决定（无 pid、无计数器）：两个进程观测到同一瞬间就拿到同一路径，先跑完的 `remove_dir_all` 掉另一个正在读的树。修法是结构性的——目录名带进程 id，跨进程碰撞从「不太可能」变成「不可能」。

### Notes

- `scoring.rs` 补数值单测，26 个存活变异体归零（#179）：8 条测试钉住离散取值，变异测试 caught 35 / missed 0。
- agent 分支的 PR 须带 `agent-approved` label 才放行（#189）：draft 阶段的口头约定升级为 pr-gate 机制。
- OpenSpec 内化记录核实入账（#156）：MIT + v1.7.0 证据落到文档。

## [0.7.0-beta.5] — 2026-08-04

两件都关于「把 review 该做什么交出去」：管线第一次能回答「这件事我答不了，谁能答」，也第一次把变更排成有序议程而不是一个标量分。之前 gap 只存在于操作者脑子里，靠现场想起某个 plugin 存在；现在它是一次可执行、可复现、只出提案的查询。

### Added

- **`code-intel change agenda <revspec>`（PR #150）**：纯 git、免索引、免网络、免 LLM 的 review 议程。变更文件按 co-change 历史并查集聚成 review unit，每个 unit 走 `change_risk::score_subset`——和 `change risk` 完全同一把尺子、同一次 history walk 的子集，不新造第二套风险公式——最坏优先排序。Schema `code-intel-change-agenda.v1`，authority 为 WorkspaceAdvisory/GitHistory。`testSelection` 与 `structuralRules` 需要已提交 run 的证据，一律报 `status: "unavailable"` 并给出产出它的命令，绝不近似；单次碰超过 50 个文件的 commit 判为 sweep，从耦合证据剔除并计入 `wideCommitsSkipped`，不静默丢弃。时间窗锚在被评 commit 自己的 committer date，不读墙上时钟。
- **`assistance.discovery` 上线（PR #160）**：gap 进，dossier 出，`proposalOnly=true`、零 effect、零 authority event、零 adoption decision。实现是激活仓里早就写好、有测试、却从没接进 `main.rs` 的 `assistance_discovery.rs`——决策核保持不碰 adapter 和文件系统类型，好让它自己的测试能单独 `#[path]` include；adapter 单独成 `assistance_adapter.rs`，不塞进已经 1266 行的 `capability_inventory.rs`。
- **`orchestration/agent-assistance-catalog.v1.json`**：6 个官方 plugin（`code-review` / `pr-review-toolkit` / `code-simplifier` / `feature-dev` / `code-modernization` / `claude-security`）按引用绑定，一个文件都不 vendor。每条候选带一次性评审过的 fit / license / security / integration / reversibility 与证据引用。候选**只能**从目录解析——call time 现编评级等于把评审本身废掉。目录进了 registry 的 `toolchainDigestEvidence.inputs`，改一条评级就动 digest，逼一次 repin。
- **doctor 探测路由目标**：`checks.assistancePlugins` 与 `assistance:<candidate-id>` provider 行。缺失是观察不是失败——未安装的路由目标是操作者该看见的事实，不是坏掉的安装，所以 `--require-provider-conformance` 不会因此挂掉。
- **`SKILL.md` 路由表**：管线证据 → plugin 入口的直连映射，走 `change risk` / `change impact` / `diagnosis.hospital-view` / `code_evidence.agent_slice` 这些已有信号。

### Notes

- `claude-security` 的 LICENSE 不是 Apache-2.0，是 Anthropic 专有授权，明文禁止分发 plugin 或其修改版。它的 dossier 报 `license: review_required`：「管线路由式使用是否还在 internal-use 授权内」是操作者的判断，不在代码里替他拍板。这也是整条路线「只引用不搬运」的硬理由，不是风格偏好。
- 加候选 = 加一条目录条目，不加运行时分支。

## [0.7.0-beta.4] — 2026-08-04

诚实批次：把 CLI 收敛成单一权威迭代，把基准从「代码一重构就烂」修成内容锚定，并第一次让「我没查」在类型层面无法伪装成「我查过了」。判据本身写进了纲领 issue #139。

### Added

- **G1 诚实位（#141）**：`EvidenceOutcome::Complete(EvidenceScope) / Partial{reason,scope} / NotComputed{reason}`。`Complete` 没有跳过 scope 的构造路径——不给出扫描面就构造不出「查过了」。首个落地面是 `repin` 的扫描覆盖声明。验收是反向测试：把 `partial` 只改 `status` 字段伪装成 `complete`（scope 仍然真实合法）必须被拒绝；关掉校验该测试会红，证明它不是空的。
- **架构收敛（#134）**：`main.rs` 收到 73 行的进程壳；`cli/` 统一 parse → dispatch → render；`authoritative_run::{execution_kernel, completion}` 成为单一权威迭代；`CommittedEvidenceController`（已提交证据、可门禁）与 `WorkspaceAdvisoryController`（工作树建议、不可门禁）分权。
- **golden 内容锚（#137）**：`eval/golden_anchors.py` 按符号名或字面片段在评分时解析 golden span，不再钉死行号。锚解析不了的题标记为 **broken**——两臂都不评、不进任何覆盖率与胜负统计，单独列出。以前这类题被静默记成「A 臂答不出」。

### Fixed

- **`--require-understand` 是 fail-open 空转（#144）**：`doctor bootstrap --require-understand --json` 在 `understandAnything` 的 skill 与 plugin 皆为 false 时仍返回 `ok:true, missing:[]`。
- **doctor 看不见 ast-grep（#143）**：`edit.ast-grep-plan` 会 shell out 到 ast-grep，但 doctor 从不探测它，于是在跑不动该能力的机器上照样报 `ok:true`。
- **发布名撞车报错不可用（#145）**：重跑同名自扫时 exit 65、stdout 零字节、stderr 只有一条本地化 OS 错误（os error 183）。现在给出独立退出码与可解析的 stdout 信封。
- **墙上时钟进了摘要（#147）**：`observedAt` 被哈希进 evidence payload，导致未变更的树每次运行都重新发布 22 个对象中的 6 个。时间戳保留在外层信封供新鲜度判断，不再进 `payload.sha256`。
- **static 锚永远解析不了（#137）**：`_strip_modifiers` 无条件剥掉 `"static "` 前缀，而 `symbol_kind="static"` 要匹配的正是它——文档声称支持的能力静默失效。
- **repin 边界安全（#91）**、**silent git/child death 不再产出空诊断（#131）**、**CI 死引用清理（#136）**、**曝光面与 agent 地图排序（#126）**。

### Changed

- **Arm A 读取窗口 10 → 25（#146）**：实测只翻转一题（q12，why 类），每覆盖题边际成本约 1,296 字节。与之捆绑的另一条杠杆（Rust 符号抽取扩到 const/static/struct）单独收益为零、语料 +17.6%，**未合入**——理由见 #142。

### 明确的负结果（不合并，保留为证据）

- **symbol-bounded chunks（#140）**：把 `code_evidence.chunks` 从整文件 chunk 改成符号级行区间。实现者动手前写下预测「q03 不会动」，理由是 `arm_a_answer` 在 `symbols → imports → chunks` 中命中第一个即 break，而目标关键词在 symbols 层就命中——chunks 层永远走不到。跑完与预测完全一致：零题状态变化，而 chunks 产物 ×3.56，Arm A 每覆盖题字节 +24.7%。三个独立验伪全部复现该结论。
- 由此得到 #142：**Arm A 的开销 99.8% 是产物全量扫描**，源码窗口仅占 775 字节；让产物更精确只会让它更贵。北极星闸门 G0 的真实距离是 why 1/4（需 ≥3/4）与 11.5× Arm B 字节（需 ≤2×）。

### 已知问题

`sentrux --help` / `doctor --help` 静默绿灯、巨石门禁只报数量不报文件名、`sentrux dsm` 边矩阵结构性为空、149 条 rustc 警告直通发布、`fallbackChunkRate: 1.0`、`ranking.json` 按字母序而非分数——完整清单见 #148。

## [0.7.0-beta.3] — 2026-08-02

北极星落地批次：占 chokepoint（PR 门禁）、立裁判（eval 基准）、砍概念债（docs 生命周期首扫）、出自举圈（第二仓冷启动）。工作流第一性原理与全部方向决策见 issue #55 及其评论。

### Added

- **`code-intel change risk <revspec>` + PR 门禁 workflow**（#102、#108）。纯 git、免索引、免网络、免 LLM 的确定性缺陷风险分（diff 形状 / 测试不对称 / bug 磁铁 / churn 四信号定权重），对照最近 50 个非 merge 提交出百分位；`pr-gate.yml` 给每张 PR 打分并 sticky 评论，`risk_percentile >= 90` 且无 `risk-accepted` 标签即红检查、阻断 auto-merge。输出 machine-first JSON（`code-intel-change-risk.v1`）。上线当天狗粮自拦（82 分 high，走标签放行完成首次完整拦截+放行演练）；CodeRabbit 审出的基线自污染、`A...B` 三点语义、warning 路径炸门禁、非 ASCII 路径 quotePath 漏匹配均已修复并带回归测试。
- **eval 北极星双臂基准 v1**（#107、#109）。12 题 artifact-guided（A 臂）vs naive 裸读（B 臂），零 LLM、逐字节可复现。首个数字如实入库：A 臂覆盖 6/12、胜 2 负 10，`why` 类 0/4 全灭——当前工件不是为回答问题设计的（#58/#105 的最硬证据）。修复基准自指污染（两臂搜索空间排除 `eval/`、`docs/archive/`、`.out-of-scope/`），基线轮换至可复现态。
- **第二仓冷启动实录**（#110）。首次对非自举仓（tdxcli-rs）跑全链路：13 步 8 顺 / 5 改造 / 2 失败、181 秒，目标仓零写入双重验证；机读 runbook `docs/runbooks/second-repo-cold-start.json` + 摩擦五条全部立案（#111–#115，含 `run execute` 写入 `artifact query` 永远索引不到的死端真 bug）。
- **`.out-of-scope/` 非目标注册表**（#109）。显式非目标当一等文件：无鉴权本地 HTTP 端口 / 人读产物当真源 / 斜杠调用产品形态。

### Changed

- **docs 生命周期首扫**（#109、#116）。123 篇盘点、17 篇一次性文档归档至 `docs/archive/`、机读判例表 `docs/INVENTORY.md`；死代码清扫为诚实零删除（全部候选均有活引用，负结果入档）。#116 回滚恢复 E09 证据文档并立判例：retirement packet 按字面路径钉死证据物，`retired=true` 也不许归档。

### Added

- **`code-intel --version`** (also `-V`, and `--version --json` emitting
  `code-intel-version.v1`). The installed binary could not report what it was:
  the installed `bin/repo.json` records where the installer ran from, not what
  it produced, so a machine on a stale build was indistinguishable from a
  current one. Observed in practice — an installed v0.6.0 binary answered
  `snapshot identity` in 10.9s while the same command on a build containing the
  `git cat-file --batch` fix took 0.28s, and nothing on the machine could tell
  the two apart. Answered ahead of route dispatch, because a leading `-` flag is
  neither a primary invocation nor a raw route and previously fell through to
  `unknown command: --version`.

  This is a self-declared build identity, not provenance: the value is
  `CARGO_PKG_VERSION` from the binary being questioned. It separates stale from
  current; it does not separate genuine from substituted. The installer's
  recorded `sha256=` remains the provenance signal.

### Fixed

- **The installer's version pin is now enforced, not merely declared.**
  `Install-MissingTool` returned on presence alone: `Get-Command` finding the
  tool on PATH produced `already_present` and the installer scriptblock never
  ran. For `repowise` — the one external tool carrying a pinned version
  (`repowise==0.36.0`, supply-chain-003) — this meant the pin could never fire
  on a machine that had installed repowise once, and a box sitting four minor
  releases behind reported the same status as a correct one. `Get-InstallMetadata`
  now carries `pinnedVersion`, a new `Get-ToolVersion` reads the tool's own
  `--version`, and a mismatch reports `version_drift` (or `upgraded` /
  `upgrade_failed` under `-InstallMissing`). An unreadable version reports as
  `unknown` and never as a match — the state the gate exists to surface. Tools
  without a pin keep their previous behaviour exactly.

  The probe follows the launch rule `crates/code-intel-cli/src/tool_path.rs`
  states for the rest of the project — "only ever launches by absolute path",
  "relative PATH entries are skipped outright". `Test-ToolVersionProbeAllowed`
  refuses any source that is not a rooted, existing, non-script file, so a
  `repowise.ps1` planted on PATH cannot be dot-run inside the installer's own
  process. A refused probe is not a failed probe: the tool keeps its previous
  presence-only reporting, so an unverifiable source can never induce a
  reinstall. The child is launched with `Start-Process -PassThru -Wait` and its
  own exit code is read, rather than the ambient `$LASTEXITCODE`, which is only
  set by native commands and would otherwise carry a stale value — or, under
  `Set-StrictMode`, abort the installer outright. Only stdout is parsed, and the
  match is anchored to the tool's own name when it is known, so a deprecation
  banner carrying its own version number cannot forge a pass or a drift.

  **Action required on already-provisioned machines.** Confirmed drift now
  fails the install. `ok` is computed from `checks` alone, so a status that only
  reached `installActions` was invisible to every consumer —
  `bootstrap-new-machine.ps1` reads `installResult.ok` and nothing else, and
  would have reported `Install OK: True` on a drifted box.
  `Add-VersionComplianceChecks` derives a `version:<tool>` check from the
  actions, so a machine whose `repowise` is off-pin now reports `ok: false` with
  `version:repowise` in `missingRequired`. Resolve it with `-InstallMissing`, or
  move the pin to the version you intend to run. A refused or unreadable probe
  emits the check as **not required**: uncertainty is surfaced, but an install
  is never failed over a version that could not be measured.

  One more behaviour change: the default (doctor) mode now executes `--version`
  on a pinned tool that is already present, where it previously ran no external
  command for it — today that is `repowise` only.

### Changed

- **Sentrux `coupling_score` now divides by import-modelled files only**
  (sentrux-native 2.1.0). The score is import lines per file; the scanner reads
  `import` / `from` / `use` / `mod` / `require(` / `#include` / `using`, none of
  which is how PowerShell declares a dependency. Counting `.ps1`/`.psm1` files
  in the denominator therefore made the number track the PowerShell share of a
  tree: it fell when PowerShell grew and rose when PowerShell shrank. Under the
  PS1 retirement campaign (issue #78) that inverted the gate — deleting a
  PowerShell test and landing the Rust test that replaces it improved
  `quality_signal` and still tripped `coupling_increased`. Numerator and
  denominator now both cover only languages whose imports the scanner models,
  so a repository with none of the unmodelled languages scores exactly as
  before. `metrics.import_modeled_files` and a `[coupling_basis]` line report
  the denominator instead of leaving it to be derived.

  This tree measures 74.41 (1064 import edges over 143 modelled files) where
  the diluted formula reported 45.48. Consequences, all in this same commit:

  - `.sentrux/baseline.json` re-saved; schema moved to
    `code-intel-sentrux-baseline.v3`. A v2 baseline holds a number this engine
    cannot produce, so it now fails closed as `baseline_engine_mismatch` with
    the re-baseline instruction rather than reporting a fabricated ~30-point
    coupling regression. **Any repository with a v2 baseline must re-save it.**
  - `max_coupling` accepts a bare number as well as an `A`..`D` grade. The
    ladder tops out at D = 6 imports per file, which no idiomatic Rust tree
    stays under, so `.sentrux/rules.toml` records the measured ceiling (76.0)
    as a ratchet; tightening it is tracked with the other threshold debt in
    issue #14.
  - `legacy/tools/sentrux-shim/sentrux-lite-core.ps1` mirrors the same
    denominator and numeric limit, so a shim injected through
    `options.toolPathPrefix` cannot compare an old-formula score against a
    new-formula baseline.

- **The doctor bootstrap probe is native Rust** (issue #48, T3 of the PS1
  retirement campaign). `code-intel doctor bootstrap` now computes the
  tool/runtime health inventory that `archive/check-code-intel-tools.ps1` used
  to implement in 409 lines of PowerShell; the script is a ~35-line thin
  forwarder retained for the installer and rollback paths. The observation it
  emits keeps the `code-intel-doctor-bootstrap-observation.v1` schema, the
  `observation_only` authority, and the same `ok`/`missing` pair, so installer
  and CI consumers are unchanged.

  ```bash
  code-intel doctor bootstrap --repo-path . --no-require-repowise --json
  ```

  CI and release workflows call the binary directly. Running the forwarder on a
  machine without the binary is now reported as a `code-intel binary` entry in
  `missing` (exit 1) rather than as a crash.

### Fixed

- **An ungoverned repository no longer reads as an architecture gate failure.**
  A repository that never ran `save_baseline` has no prior measurement, so the
  built-in Sentrux gate cannot detect a regression against one. That absence of
  governance was being published as a failing `sentrux_gate` rule, which made
  `diagnosis.hospital` diagnose `architecture gate failure` and
  `code-intel run dag-coordinate` exit 10 on *any* never-baselined repository —
  including a fixture holding a single `README.md`. The gate now reports the
  ungoverned case as `pass`, matching how `check` has always treated a missing
  `.sentrux/rules.toml` ("Quality: not gated").

  `code-intel sentrux --operation gate` still exits 1 with the save-baseline
  instruction, and the raw exit code and stdout stay verbatim in
  `sentrux-command-observation.json`, so the ungoverned state remains auditable.
  A baseline that exists but cannot be read by this engine
  (`baseline_engine_mismatch`) is unchanged: re-baselining stays a deliberate
  decision. A baselined repository that genuinely regresses still fails the
  gate and still exits 10.

- **The E05 publication retirement packet can be regenerated and verified from
  a clean checkout.** Two defects kept it pinned as the sole known-blocked
  retirement lane. Regeneration ran `test-dag-facade.ps1`, which failed on the
  gate false positive above; and the rollback rehearsal was written to an
  ephemeral `archive/work/<name>-<timestamp>/` directory outside the packet, so
  its frozen evidence pointed at a path a clean checkout never has. The
  rehearsal now lives inside the packet at `rollback-rehearsal/`, as it already
  did for E04/E07/E09, and the verifier resolves it against `$PacketRoot`.

  Carrying the rehearsal in-tree adds a 4742-line frozen copy of
  `run-code-intel.ps1`, which Sentrux counts as a god file, so
  `.sentrux/baseline.json` moves `god_file_count` 32 → 33 and `quality_signal`
  3603 → 3484. The delta is entirely that one evidence artifact — with it moved
  aside the gate reports `No degradation detected` — and matches how the
  existing E04/E07 rehearsal copies are already carried in the baseline. Anyone
  adding a future retirement packet should expect the same one-file step.

- **The doctor capability no longer answers from a stub when `pwsh` is
  absent.** The adapter previously shelled out to the PowerShell probe and, on
  failure to launch it, fell back to an in-process approximation that reported
  `graphProvider` presence as hardcoded `true` — masking exactly the drift the
  doctor exists to surface. With one native probe there is no fallback path and
  no `pwsh` dependency on the kernel path.

### Added

- `archive/scripts/tests/test-dag-facade.ps1` runs in CI. It asserts DAG facade
  artifact routing and explicit/default inventory parity against a repository
  name containing spaces, `&`, and non-ASCII characters, and had never been
  wired into a workflow.

## [0.7.0-beta.2] — 2026-07-30

### Changed — action required

- **Every PowerShell entry point moved under `archive/`.** Installation still
  runs through PowerShell — the compiled `code-intel` CLI cannot install the
  binary that provides it — so the documented command moves with it:

  ```powershell
  pwsh ./archive/install-code-intel-pipeline.ps1 -RepoPath <repo> -InstallMissing
  ```

  The same shift applies to `code-intel.ps1`, `check-code-intel-tools.ps1`,
  `run-code-intel.ps1`, the contract test suites under
  `archive/scripts/tests/`, and every `$env:CODE_INTEL_HOME/*.ps1` invocation
  the bundled skill documents. Release archives carry the same layout, so a
  packaged `code-intel.ps1` now lives at `archive/code-intel.ps1`.

  Nothing was deleted: this is a relocation, not the entry-point retirement
  tracked in the PS1-exit campaign. The 132 relocated files keep their
  relative structure, so script-to-script references are unchanged.

- The skill bootstrap installs both layouts. `find_payload_root` probes
  `archive/` first and falls back to the payload root, so releases published
  before this one stay installable.

### Fixed

- Path resolution in the relocated scripts, wherever a single root variable
  had been carrying two meanings — "where the PowerShell lives" and "where the
  repository is". Those diverged under `archive/`, and the split had to be made
  explicit in the doctor (its graph-provider probe and `CODE_INTEL_HOME`
  default), the installer (binary install, integrations manifest, vlang
  overlay, bundled skill, reported root), the beta packager and its verifier,
  the orchestrator's manifest validation, the compatibility retirement
  packets, and the archived contract suites.
- `cargo test --release` no longer fails on the inventory fault-injection
  contract test. The `CODE_INTEL_TEST_RG_EXTRA_PATH` hook it drives is
  deliberately `#[cfg(debug_assertions)]`, so the shipped binary carries no
  inventory fault injection; the test is now gated on the same cfg. CI only ran
  the debug profile, so the failure had never surfaced there.
- Removed a stale duplicate of the `resume` contract left in `main.rs` when that
  logic moved to `artifacts.rs`: the `ResumeSummary` struct, four JSON helpers,
  `next_read`, and a verbatim copy of two contract tests that
  `artifacts_tests.rs` already owns. `cmd_resume` has delegated to
  `artifacts::resume` throughout, so nothing was serving the dead copy.
- Dropped three unused imports and moved `MAX_JSON_DEPTH` to the only test that
  uses it, instead of re-exporting it crate-wide.
- `skill:codex` and `skill:claude` verify whose skill occupies the path instead
  of only that some `SKILL.md` exists there. Agent hosts share those directories
  with other skill managers, so an unrelated manager's junction served a stale
  skill while the installer reported `OK skill:claude` on every run. A drifted
  path is now reported, and `-RepairSkillLinks` moves the previous occupant
  aside — unlinking a reparse point, renaming a real directory — rather than
  deleting it.
- Installing the bundled skill no longer copies `__pycache__` / `*.pyc` into an
  agent host's skill directory. One stray `bootstrap.cpython-313.pyc` left by a
  local `bootstrap.py` run made the byte-parity check report `skill:source`
  outdated permanently.
- The `repowise-thinking-patch` overlay distinguishes "obsolete" from "broken".
  Upstream repowise 0.32.0 walks `response.content` and skips non-text blocks
  itself, so a healthy machine reported `install_failed` on every install run.
  The installer now reports `not_needed` for the upstream-fixed shape and keeps
  `install_failed` for a genuinely unrecognised layout.

## [0.7.0-beta.1] — 2026-07-28

This release moves Code Intel into the write path and makes the official
release installable on Windows, macOS, and Linux. It also closes the security,
audit, and release-gate findings discovered by the repository's own audit and
adversarial verification passes.

### Added

- Write-time workflow support: the Skill now triggers for implementation,
  refactoring, and fixes; `change impact --staleness advisory` provides
  explicitly non-gating guidance while a working tree is changing.
- Deterministic, non-mutating `edit.ast-grep-plan` previews, backed by an
  internalized and CI-pinned ast-grep toolchain.
- Official Windows, macOS, and Linux ZIP assets with matching SHA-256
  sidecars, manifests, provenance attestations, and platform-aware Skill
  bootstrap installation.
- A PowerShell-versus-Rust parity observation harness and a fast structural
  cycle regression test for the self-dogfood release gate.

### Changed

- macOS/Linux installs now persist `PATH` and `CODE_INTEL_HOME` through a
  generated POSIX environment file, install the integrations manifest beside
  the binary, and report platform-correct doctor guidance.
- Release publishing is split from unprivileged platform build jobs. Only the
  final publisher receives release and attestation permissions, and it
  validates the complete nine-asset inventory before publishing.
- Audit rendering, CI, release self-scan, and packaged-payload validation now
  share the same fail-closed audit validation path.

### Fixed

- Repository snapshots and inventory exclude nested linked worktree markers
  instead of treating them as ordinary files.
- `run commit` preserves the caller's manifest bytes and digest when artifact
  references do not change.
- A 29-defect hardening pass repaired Git/path handling, CJK path support,
  capability and routing dead paths, baseline metric validation, index
  replacement safety, CI gate coverage, and documentation drift.
- Tool discovery now resolves executables absolutely, internalization evidence
  binds the relevant digests, and the release/self-scan snapshot identity
  checks remain enforced.

### Security

- Audit department prompts define an explicit untrusted-content boundary;
  model-process output is bounded; adversarial audit fixtures cover fabricated
  evidence, reversed ranges, and false coverage claims.
- GitHub Actions expressions are removed from executable script bodies,
  actions and package inputs are pinned, Cargo release commands use
  `--locked`, and release assets can no longer be silently clobbered.

## [0.6.0] — 2026-07-26

This release adds the **audit layer**: audit dimensions run as hospital
departments over the modality evidence the pipeline already gathers, and emit
findings under one fail-closed contract. Three departments ship enabled
(`security`, `ai-safety`, `supply-chain`), reports render as Markdown or as a
self-contained HTML document, and an audit can be scoped to a git diff for
pull-request review.

The layer was pointed at this repository before shipping. It found the
unpinned toolchain and the mutable CI action tags fixed below; the git config
hardening came out of the same pass. Methodology adapted from
[Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain)
(MIT).

### Fixed

- Git no longer runs inside a scanned repository with that repository's own
  program-executing config keys live. `core.fsmonitor` names a program Git
  executes on ordinary read commands like `git status`, so a repository that
  arrived with its `.git` intact (an archive, a backup, a copied directory)
  got command execution before any gate looked at it. New
  `crates/code-intel-cli/src/hardened_git.rs` pins `core.fsmonitor`,
  `core.hooksPath`, `core.sshCommand`, `diff.external`, and `core.pager` empty
  via `-c` and sets `GIT_CONFIG_NOSYSTEM`; all five production call sites and
  both `archive/run-code-intel.ps1` invocations route through it. The pipeline already
  stripped `RIPGREP_CONFIG_PATH` at every ripgrep call site — this applies the
  same standard to the tool that can start a process.
- Audit evidence is now grounded in the tree it cites.
  `AuditReport::validate()` is filesystem-free, so a `confirmed` finding's
  evidence `path` was only checked for being non-empty — a department is an
  agent, so a fabricated or drifted citation validated green. New
  `validate_evidence_grounding(repo_root)`, run by `audit --operation validate
  --repo <root>`, resolves every `file` evidence entry under the repository,
  requires it to exist, and requires any line range to be ordered and within
  that file.
- The department registry no longer accepts path strings that escape the
  repository. It is read from `--repo` — a scanned repository's own file — but
  its rubric and prompt paths were joined onto the repo root and only
  existence-checked, so an absolute path replaced the base and `..` escaped
  it, letting a target repo satisfy the kernel's fail-closed existence
  invariant with host files and redirect a department's `prompt` (the
  instruction source an audit agent reads). Both path classes now parse under
  the portable repo-relative contract `artifact_ref.rs` already enforces.
- `archive/Invoke-WorkflowRecommendation.ps1 -Json` pins stdout to UTF-8 on Windows.
  pwsh encodes redirected stdout with the system codepage, so on a zh-CN host
  the proposal arrived as GBK bytes and the capability adapter's parse failed
  with `invalid unicode code point`; CI runners are UTF-8, so the suite only
  failed on real zh hosts.

### Changed

- Pinned the Rust toolchain in `rust-toolchain.toml` (1.95.0, `rustfmt`,
  minimal profile). CI and release jobs now install from that file instead of
  running `rustup default stable`, and the release manifest records the
  resolved `rustc --version`. Released binaries were previously built by
  whatever `stable` happened to be current, so the shipped `.sha256` proved
  transport integrity but nobody could rebuild a tag's bytes to check them.

### Added

- Audit report HTML rendering and incremental (diff-scoped) audits.
  `code-intel audit --operation render --report <path> --format html` prints
  one self-contained HTML document — inline styles only, no external CSS/JS/
  fonts/images or other network reference, generated directly from the
  parsed, validated `AuditReport` model so there is no placeholder-
  substitution failure mode for a separate linter to catch; `--format
  markdown` stays the default and existing invocations are unchanged.
  Separately, the report contract gains an optional top-level `scope` block
  (`{"kind": "full" | "diff", "since", "files"}`); a new fail-closed rule in
  `validate()` requires a `"diff"` scope's `since`/`files` to be present and
  bounds every finding's file evidence to a path in `scope.files` (normalising
  `\`/`/` before comparing) — a finding outside the declared diff is a
  contract violation. `code-intel audit --operation scope --repo <root>
  --since <git-ref>` computes that file set (`git diff --name-only
  <since>...HEAD`, filtered to files still on disk) and prints a
  ready-to-embed scope block. See `docs/audit-report.md`.
- Audit departments `ai-safety` (T3) and `supply-chain` (T4): prompts under
  `orchestration/audit/prompts/`, adapted from
  [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain)
  (MIT) and rewritten to run on pipeline modality evidence. `ai-safety` gates
  on an AI/LLM surface being present and reports `not_assessed` with evidence
  when there is none; `supply-chain` parses manifests, lockfiles, and CI
  workflow permissions as structured facts before judging. Both are flipped to
  `enabled: true` in `orchestration/audit/departments.v1.json`; the kernel and
  the `code-intel audit` CLI needed no change.

- Audit kernel (T1): a shared contract for audit departments that run as
  hospital departments over existing modality evidence. Adds the
  `code-intel-audit-report.v1` schema, `orchestration/audit/rubrics/`
  (severity, confidence, evidence, coverage, scoring — adapted from
  [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain),
  MIT), and `crates/code-intel-cli/src/audit_report.rs` with a fail-closed
  `validate()` and a `departments.v1.json` registry loader. `validate()` is
  registry-authoritative: a report's `departments` must exactly match the
  registered department ids, and each department run's `status` must agree
  with the registry's `enabled` flag. `hospital-report.json`
  gains an optional `audit` summary block and `hospital.md` gains an optional
  `## Audit` section; both are additive and render nothing when no audit ran.
  See `docs/audit-report.md`.
- Security audit department (T2): `orchestration/audit/prompts/security.md`
  (adapted from
  [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain),
  MIT) and `orchestration/audit/departments.v1.json`'s `security` entry flips
  to `enabled: true`; `ai-safety` and `supply-chain` landed after it as T3/T4
  (see the entry above). New
  `code-intel audit --operation validate|render` CLI parses a report,
  validates it against the department registry (`--operation validate
  --repo <root> --report <path>`), or prints its `## Audit` markdown section
  (`--operation render --report <path>`, no registry needed). The audit
  report is now a first-class artifact
  (`code-intel-audit-report.v1` / `diagnosis.audit`) in `artifact_ref.rs`.

## [0.5.1-beta.1] — 2026-07-25

### Added

- A deterministic paired tool-effectiveness scorer for frozen `C0`, `C1`, and
  `Cfull` Agent runs, with externally attested outcomes, exact experiment
  profile matching, and atomic benchmark reports.
- ADRs that freeze canonical EntityRef identity and the first real-world
  dogfood evaluation baseline before expanding provider-specific ablations.
- Built-in `sentrux-native` structural gate engine (`code-intel sentrux --operation
  scan|health|check|gate|save_baseline`): deterministic metrics, structured
  violations with target files, and real resolved-import cycle detection for
  Rust modules (sentrux-lite hardcoded `cycle_count = 0`). The `evidence.sentrux`
  DAG node now runs this engine in-process by default; an external Sentrux is
  used only when `options.toolPathPrefix` is given.
- Baseline v2 (`code-intel-sentrux-baseline.v2`) carries engine identity,
  version, source commit, and scope. The gate fail-closes on engine mismatch
  instead of comparing numbers produced by different engines and scales.
- Structural rules may carry `details.violations` (rule, message, targets)
  through the adapter/admission chain; the Hospital report and surgery plan now
  name the first failing rule, its target files, and the smallest rerun
  command, and plan a bounded surgery whenever an actionable target exists.
- Execution results expose a `failures` block that separates process failures
  from domain failures so a tooling error can no longer visually swallow an
  architecture-gate verdict.
- CI and the release workflow gain a release-blocking authoritative self-scan
  (`run execute` on the exact candidate tree, no skip flags), landed in the
  companion `ci(release)` workflow commit on this branch.
- Stable release zips are packaged from `git archive HEAD`, ship with a
  `.sha256` and a `release-manifest.json` (tag, commit, zip digest), and the
  packaged binary must pass its own `check` and `gate` against the packaged
  payload before publication (same companion workflow commit).

### Fixed

- Removed the `dag_run` <-> `execution_kernel` module cycle by moving the run
  CLI front-end to `run_cli.rs` and `RunError` to `run_error.rs`, and the
  previously undetected `artifact_ref` <-> `capability` cycle by extracting the
  shared content-contract primitives into `content_contract.rs`.
- Kernel runs no longer process-fail outside wrapper environments: the doctor
  adapter falls back to a native tool observation when the PowerShell bootstrap
  is unavailable or nonconforming, and the built-in gate engine satisfies the
  Sentrux requirement when no external Sentrux is installed (a present but
  nonconforming external overlay still fails conformance on purpose).
- Repository inventory no longer treats the `.git` pointer file of a linked
  git worktree as an unexplained extra path.

## [0.4.0] — 2026-07-24

### Added

- One authoritative typed execution kernel that owns DAG execution, structured outcomes,
  atomic publication, and the stable process exit contract.
- Default, strict, offline, and compatibility execution profiles compiled into one immutable
  policy, with optional provider unavailability represented explicitly.

### Changed

- The PowerShell entrypoint is now a thin adapter over `run execute` and validates the
  versioned execution-result schema before returning.
- The internal graph remains authoritative by default; Sentrux acceleration and external
  enrichment stay optional, while strict mode requires every enabled provider.

## [0.3.0] — 2026-07-24

### Added

- One manifest-bound normal-run spine for snapshot, doctor, inventory, native code evidence,
  architecture graph, real Sentrux `gate`/`check` observations, Hospital diagnosis, atomic
  publication, committed-only indexing, evidence query, freshness, and conservative change impact.
- Optional, research-stage Mindwalk trace normalization for privacy-reduced session review; it is
  advisory-only and absent from default scans.
- Representative benchmark gates for deterministic replay, artifact size, unresolved coverage, and
  unsupported-file coverage.

- One-command automatic draft-PR orchestration from exact proposal through structured user decision,
  C07 record/replay, and the existing fail-closed executor.
- Zero-effect proactive `/investigate` suggestions for actionable Pipeline failures,
  plus a branch-local user decision request before any automatic draft-PR path.
- Public-beta package verification, including clean extracted-ZIP smoke coverage
  and release checksums.
- Runtime/CI and file-boundary evidence providers, transactional artifact
  contracts, model request synthesis, executable handles, and compatibility
  retirement approval evidence.
- An installable Codex Skill whose stable bootstrap is pinned to the verified
  `v0.3.0` GitHub Release and validates the published SHA-256 digest before
  extraction.

### Changed

- Non-completed runs are retained as audit diagnostics and can never replace the latest completed
  authority. Domain-failed nodes retain their verified evidence without becoming authoritative.
- The native seven-language adapter is explicitly graded `candidate + structural`; semantic,
  behavioral, and production claims remain unsupported.
- Project license metadata, README, and root license text are now consistently MIT.

- Repowise semantic memory remains in the default orchestration plan but is now
  explicitly optional and non-blocking for the beta core.
- CodeNexus context remains an optional compatibility adapter; generated
  `work/` paths are excluded from repository evidence.
- Sentrux debt normalization treats an improving quality signal as
  informational while structural metric increases remain blocking.
- The stable wrapper resolves the packaged `bin/code-intel.exe` before any
  development-tree or Cargo fallback.
- PowerShell contract tests now live under `scripts/tests/`; the seven public
  compatibility entry points remain at the repository root.

### Security

- Production model delegation uses synthesized requests and validated
  executable handles; legacy raw CLI execution is rejected by default.

### Known limits

- The public beta package is Windows-only.
- The incubated `crates/code-nexus-lite` Rust worker is not a shipped workspace
  binary; CodeNexus indexing is not a beta-core dependency.

## [0.3.0-beta.1] — 2026-07-16

Pre-release for the Rust-first Code Intel control plane. This build is intended
for integration testing before the `0.3.0` stable release.

### Added

- **Rust Sentrux DSM analysis kernel** — repository inventory, dependency
  structure, complexity, health, rules, gaps, and evolution analysis now run in
  the Rust CLI, with the PowerShell path retained as a compatibility fallback.
- **Atomic capability contract v1** — defines the execution envelope used to
  coordinate capability ownership, effects, dependencies, and artifacts.
- **Trust-boundary hardening** — Hospital and scoped Repowise paths fail closed
  at repository and artifact boundaries.

### Changed

- Rust DSM executable discovery is cross-platform and accepts an explicit
  `CODE_INTEL_RUST_CLI` override.
- File inventory is self-contained when Git metadata is absent, and symlinked
  directories are not followed during recursive traversal.
- Concurrent DSM integration fixtures are isolated to prevent cross-test
  interference on Windows.

### Verified

- Rust unit and integration suites pass locally.
- Windows package/build and Windows, Ubuntu, and macOS smoke jobs pass in CI.
- Rust and PowerShell DSM providers produce matching core repository and module
  metrics on the release candidate repository.

### Beta limitations

- GitHub Release packaging currently publishes a Windows ZIP only.
- The PowerShell DSM provider remains the automatic fallback when the Rust CLI
  cannot be located or executed.
- Complexity scoring is intentionally heuristic and may count keywords inside
  trailing comments; naive comment stripping was rejected because it corrupts
  strings and URLs.

## [0.2.0] — 2026-07-02

The "understand any repo, cheaply" release. Docs generation now runs on any
LLM (MiniMax, local Ollama, custom OpenAI-compatible endpoints), the installed
toolchain self-heals, and the pipeline finishes with a three-stack workflow
recommendation telling you how to start working on the repo it just mapped.

### Added

- **Three-stack workflow recommender** — replaces the OpenSpec-only detector.
  Each pipeline run emits a `workflows` array in `report.json` (legacy
  `openSpec` block kept for compatibility) with layered, complementary
  verdicts: *matt-flow* (idea→ship: `/grill-with-docs`, `/to-prd`,
  `/to-issues`, `/triage`), *gstack* (delivery/quality: `/qa`,
  `/design-review`, `/ship`, `/canary`, `/review`), and *spec-driven*
  (picks OpenSpec OPSX for brownfield repos vs github/spec-kit for
  greenfield; detects `openspec/` / `.specify/` as already adopted).
- **Regression suite + fail-open lint** (`test-regression-fixes.ps1`) — 24
  cases locking down the fail-open/false-green fixes, plus an AST-based lint
  that flags `catch { return $true }` patterns across all `.ps1` files
  (`# lint-allow: fail-open` marker supported).
- **Self-healing repowise patch** — `archive/install-code-intel-pipeline.ps1` now
  idempotently re-applies the ThinkingBlock fix to the installed repowise
  venv on every run (reasoning models behind Anthropic-compatible endpoints
  return thinking blocks first; upstream reads `content[0].text`). Survives
  `uv tool upgrade repowise`; documented in `overlays/repowise/README.md`.

### Changed

- **Docs LLM provider generalized: local models + custom APIs** — provider
  selected via `CODE_INTEL_PROVIDER` (default `anthropic`) with generic
  `CODE_INTEL_MODEL` / `CODE_INTEL_API_KEY` / `CODE_INTEL_BASE_URL`, reusing
  repowise's own provider registry. Keyless providers (ollama) work without
  credentials; `CODE_INTEL_ANTHROPIC_*` remains as backward-compatible
  fallback. Preflight covers anthropic / openai / ollama and runs on the
  repowise uv venv python (system-python dependency dropped).
- **Thin-forwarder install** — `Install-SentruxShim` generates forwarders
  into the user-local Code Intel bin directory instead of copying script bodies;
  repo edits take effect immediately via PATH, and a moved repo fails loudly.
- **Fail-closed hardening** — session_end no longer backfills baselines on
  zero parseable metrics; the surgery_plan→post_op guard evaluates real
  data; doctor survives malformed config JSON; overlay compare and global
  index refresh fail closed instead of open; baselines are backed up to
  `baseline.prev.json` before overwrite.
- **Detector accuracy** — code-size scan is now repo-root recursive (was a
  5-dir/7-extension whitelist that measured some repos as 1 file);
  repo age uses first-commit date (was last-commit, which judged every
  active old repo "greenfield"); multiple StrictMode crashes fixed.
- Local toolchain verified against **repowise 0.25** (upgraded from 0.21).

### Verified

- End-to-end on AIGX: 7/7 steps green, `workflows[3]` + legacy block emitted.
- Cold-start on an unfamiliar clone (fastapi/typer, 747 files): 15.7 s index,
  full understanding pack, and a sane three-stack verdict (108 contributors →
  PRD breakdown; deploy indicators → ship/canary; 2385-day brownfield →
  OpenSpec OPSX, score 65).
- Regression suite 24/24; provider preflight ok for MiniMax-M2.7, MiniMax-M3,
  and local Ollama; scoped docs generated 9-18 pages via MiniMax.

## [0.1.2] — 2026-06-10

First public release of code-intel-pipeline. Headline addition is
the Rust + iii worker binary `code-nexus-lite` that replaces the
Windows-only PowerShell surface with a cross-platform Agent-callable
HTTP API. Also adds a PR-time skill-check quality gate.

### Added

- **`crates/code-nexus-lite/`** — Rust + iii worker binary, wraps Repowise + Sentrux for Agent-friendly code-understanding context. 5.2 MB stripped + LTO. Cross-platform replacement for the Windows-only `archive/Invoke-CodeNexusLite.ps1`. Apache-2.0 license (matches iii SDK).
  - 3 iii functions: `codenexus::scan` / `codenexus::lite` / `codenexus::doctor`
  - 3 HTTP triggers: `POST /scan` / `POST /lite` / `POST /doctor`
  - Depends on `iii-sdk = "0.11"` (crates.io, Apache-2.0) + `repowise` 0.10.0 (Python) + `sqlite3` CLI
  - See `crates/code-nexus-lite/README.md` for the full design

- **`.github/workflows/skill-check.yml`** — PR-time quality gate. Runs a heuristic 8-dim darwin-skill scoring on every changed SKILL.md, validates YAML frontmatter, checks for broken internal links. Threshold 70/100 to pass. Triggers on PRs that touch `crates/code-nexus-lite/`, `.claude/skills/`, or `skills/`.

- **`.gitignore` updates** — Added `target/` (Rust build artifacts), IDE files (`.idea/`, `.vscode/`), OS files (`.DS_Store`, `Thumbs.db`), PowerShell artifacts (`*.ps1.xml`).

- **`crates/code-nexus-lite/.gitignore`** — Same as above, scoped to the sub-crate.

### Changed

- `archive/Invoke-SentruxAgentTool.ps1` — minor edits
- `templates/sentrux-rules.example.toml` — minor edits

### Verified

- ✅ `cargo build --release` succeeds (52 s first build, ~5 s incremental)
- ✅ Smoke test: binary starts, registers 3 functions + 3 HTTP triggers, attempts engine connection (engine not running locally — expected)
- ✅ Doctor: `repowise --version` reports v0.10.0, all 4 required tools (rg / git / repowise / sentrux) found

## v0.1.1 - 2026-05-30

Release infrastructure patch.

- GitHub Actions now exports the installed Code Intel tool bin directory through `GITHUB_PATH`, so later CI steps can find the Sentrux shim.
- CI smoke tests can explicitly allow the expected `graph_missing` manual step while still failing on local tool errors and Sentrux regressions.
- GitHub-hosted smoke tests skip the historical Sentrux baseline gate when running on the lite fallback, because lite metrics are not compatible with a real-core baseline.
- Release workflow is idempotent: if a GitHub Release already exists for a tag, it uploads or replaces the zip asset instead of failing.
- Release package avoids bundling local `pipeline.config.json`; it ships `pipeline.config.example.json` instead.

## v0.1.0 - 2026-05-30

Code Intel Pipeline 的第一个公开版本。

这一版把本地代码理解工具链整理成一条可重复的流程：刚从 GitHub clone 下来的项目，先摊成地图，再交给 Agent 动手。

- 便携安装器、doctor、自检脚本和一条命令入口。
- 串起 `rg`、Repowise、Understand Anything、Sentrux、CodeNexus-lite。
- 大仓库支持 scoped Repowise，避免根目录里的外部轮子污染判断。
- Governance 状态机输出 `hospital-report.json` 和 `surgery-plan.md`。
- Sentrux Agent 工具：`scan`、`health`、`session_start`、`session_end`、`rescan`、`check_rules`、`evolution`、`dsm`、`git_stats`、`test_gaps`、`what_if`。
- Sentrux lite fallback 和开源部署下的本地 auto-Pro 激活。
- Windows 下的 Sentrux V language 插件覆盖包。
- GitHub Actions Windows smoke test。
- 中文 README、GPT娘横幅、实际部署说明。

已知边界：

- Understand Anything 图谱生成仍依赖宿主 Agent skill。如果缺 `.understand-anything/knowledge-graph.json`，先运行 `/understand <repo> --language zh`，再重跑 pipeline。
