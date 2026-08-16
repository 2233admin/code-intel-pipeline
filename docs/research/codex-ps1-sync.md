# Codex 同步研究报告(2026-08-17)

> 只读研究:不修改任何仓库文件;证据来自 gh CLI 输出与 git 工作树状态。仓库 main worktree 位于 D:\projects\_tools\code-intel-pipeline(分支 release/v0.7.x-rc, HEAD f6b0bf5)。报告路径:本文件。

## 结论

- **等 + 协调(需主动协调,不宜干等)**:Codex 当前正处于活跃开发中(codex.exe PID 68716 在跑,主 worktree + 94ec/fd06 三个工作树都堆着大量未提交改动,全部基于 f6b0bf5,分支上零新提交),但其 4 个 open PR 全部被 `agent-gate` 红闸卡死(2 个 CONFLICTING、2 个 BLOCKED),无任何人 approve,近期不会自动落定。
- **#276(Claude)与 Codex 未提交工作在 5 个文件上同区重叠**(command_catalog/mod.rs、routes/mod.rs、tests.rs、legacy.rs、main.rs),冲突风险高;若 Codex 先提交 task_control/friction_control 控制面,再合 #276 必然产生直接 merge 冲突。
- **#276 的 ps1 迁移核心(builtin_provider_evidence.rs 的 codenexus_admission、codenexus_lite.rs、hardcoded_paths.rs)与 Codex 未提交改动完全不相交**——Codex 未提交 diff 根本没碰 builtin_provider_evidence.rs(空 diff),该部分可独立推进、无冲突。
- **issue 274 同方向(去 PS 化)但范围独立**,已被 2233admin claim,尚未关联 PR。#275/#276 自身注释也认定 274 与其"同方向不重叠"。
- **建议**:等 Codex 先把 command-catalog 控制面(含 #276 要动的那 5 个文件)提交落定再推 #276 的 plumbing 部分;或现在就与 Codex 协调,让 Codex 的控制面提交避开 #276 的路由/帮助/模块声明区。ps1 迁移的纯 Rust 部分(证据生成、codenexus_lite、hardcoded_paths 子命令)可立即并行推进。

## 1. PR 状态表(4 个 Codex PR,均 target `main`,author=2233admin)

| PR | 标题 | head 分支 | 状态 | mergeable | mergeStateStatus | reviewDecision | agent-gate(必需闸) | head commit | 最近更新 |
|----|------|-----------|------|-----------|------------------|----------------|--------------------|-------------|----------|
| #250 | test: report terminated legacy session processes clearly | codex/issue-213-pwsh-signal-report | OPEN | CONFLICTING | DIRTY | (空) | **fail** | 2026-08-08T18:35Z | 2026-08-16T21:31Z |
| #249 | fix: rust use imports | codex/issue-223-rust-use-imports | OPEN | CONFLICTING | DIRTY | (空) | **fail** | 2026-08-08T21:01Z | 2026-08-16(更新) |
| #248 | fix: release install | codex/issue-232-release-install | OPEN | MERGEABLE | BLOCKED | (空) | **fail** | 2026-08-08T17:13Z | 2026-08-16(更新) |
| #202 | fix(gate): change-risk 闸判绝对分数,不判会漂的排名 (#201) | agent/change-risk-absolute-201 | OPEN | MERGEABLE | BLOCKED | (空) | **fail** | 2026-08-08T20:09Z | 2026-08-16T21:32Z |

- **无任何人 approve**:4 个 PR 的 reviewDecision 全空;reviews 里只有 CodeRabbit(机器人)评论(249/248 各 1 条,#202 CodeRabbit 状态 "Review completed"),无人类 review。
- **全被 agent-gate 卡死**:`gh pr checks` 对 4 个 PR 均返回 `agent-gate  fail`(其他 cross-platform-smoke / parity-observe / windows-build-test-package / GitGuardian / CodeRabbit 均 pass)。PR 250/249 因 base 漂移为 CONFLICTING(需 rebase),248/202 为 MERGEABLE 但被 agent-gate 红闸 BLOCKED。
- **head commit 全部停在 08-08**:4 个分支最近 commit 时间戳均来自 08-08;updatedAt 到 08-16 只是 CI/check 重跑,非新提交。

## 2. 文件重叠分析(#276 2 个 commit f6b0bf5..HEAD vs Codex 主 worktree 未提交 diff)

#276(PR #276, branch agent/ps1-migration-275, base `main`,48 文件,加 3580 / 删 4070,CONFLICTING)自身 2 个 commit(3c4a84c 迁移 + e73ce3e 文档)相对 f6b0bf5 改了 12 个文件;Codex 未提交改动在同样基于 f6b0bf5 的主 worktree 上改了 21 M + 26 ??。

| 文件 | Codex 未提交改了什么 | #276 改了什么 | 冲突风险 |
|------|----------------------|---------------|----------|
| cli/command_catalog/mod.rs | import 块加入 task_control/friction_control;CompatibilityRoute 枚举加 Task/Friction;execute_compatibility 加两路 dispatch | import 块加入 hardcoded_paths;枚举加 LintHardcodedPaths;dispatch 加一路 | **高**——同一 import 列表、同一枚举、同一 dispatch 函数 |
| cli/command_catalog/routes/mod.rs | 加 task+friction 两个 raw_route;把 resolve_command_route/resolve_legacy_route/resolve_raw_route 抽到新 resolution.rs(删约 40 行),`mod resolution` + route_macros | 在 COMMAND_ROUTES 加 `lint hardcoded-paths` raw_route(+15 行,约 L133-147) | **高**——都在 COMMAND_ROUTES 插 raw_route,且 Codex 会动 resolve 函数区 |
| cli/command_catalog/tests.rs | raw 路由计数断言 33→35 | 同一条断言 33→34 | **中**——同一断言行,目标值不同,直接行冲突 |
| cli/legacy.rs | Commands: 帮助块加 task/friction 两行(约 L1170) | Commands: 帮助块加 `lint hardcoded-paths [<repo-path>] [--json]`(约 L1144) | **中**——同一帮助块,相邻行 |
| main.rs | mod 声明加 friction_control + task_control | mod 声明加 hardcoded_paths | **中**——同一 mod 声明区 |
| builtin_provider_evidence.rs | **未改**(git diff 为空) | 重写 codenexus_admission:去掉 spawn `pwsh -File legacy/Invoke-CodeNexusLite.ps1`,改为 codenexus_lite::build_context 进程内生成;implementation id 改 codenexus_lite::IMPLEMENTATION_ID | **无**——Codex 未提交 diff 完全不碰该文件(回答了问题:Codex 未改 codenexus_admission) |
| capability_inventory.rs | EXCLUDES 12 项加 `!**/.code-intel/**`;加 `modernize_extract_rules_lite` module + dispatch | #276 自身 2 commit 不碰它(PR 文件清单含它只是 release-vs-main 血统差异,f6b0bf5..HEAD diff 为空) | **无直接冲突**——仅 Codex 一侧改动 |
| tests/fixtures/cli-head-parity.v2.json | M(未提交) | #276 2 commit 不碰(仅血统差异) | 无 #276 意图冲突 |

**重叠结论**:真正需要协调的冲突面是 command_catalog/mod.rs、routes/mod.rs、tests.rs、legacy.rs、main.rs 共 5 个文件——Codex 的控制面(task/friction)与 #276 的 hardcoded_paths 子命令都要改路由表、帮助文本、模块声明。ps1 迁移本体的证据生成/codenexus_lite/hardcoded_paths 逻辑与 Codex 未提交改动零交集。

## 3. issue 274

- **状态**:OPEN;title=`fix(review): remove PS orchestration drift and cover relocated shim in install smoke`;labels:`backlog, bug, claimed, enhancement`;assignee/author=`2233admin`;comments=2;无关联 PR 显示(gh issue view 无 Linked pull requests 段)。
- **同方向**:是——body 是 review follow-up,要求 1) 根 `invoke-code-intel.ps1` 去掉生产编排行为(编排归 Rust,PS 仅作兼容面) 2) 补 #216 install-class 路径迁移的 install-smoke 复现。与 #275/#276 的去 PS 化同向。
- **范围**:与 #276 不重叠——#276 处理 issue #275(迁移 CodeNexusLite facade + check-hardcoded-paths),issue #275 body 明确写"274 ... 同方向不重叠"。274 已被 claim(标签 claimed + assignee),但还没有对应 PR。

## 4. Codex 活跃度

- **进程**:`codex.exe` PID 68716(≈390MB)与 `codex-code-mode-host.exe` PID 134752 当前正在运行 → Codex 此刻活跃。
- **三个工作树全部停在 f6b0bf5、分支零新提交,工作全未提交**:
  - 主 worktree(D:/projects/_tools/code-intel-pipeline,release/v0.7.x-rc):21 M + 26 ??(注:任务假设 4 个 ?? 不准确,实为 26 个未跟踪文件)。改动集中在 task_control/friction_control/dag_run_recovery/dag_run_registry/modernize_extract_rules_lite 控制面 + command_catalog/legacy/main/capability_inventory 接线。
  - 94ec worktree(C:/Users/Administrator/.codex/worktrees/94ec/code-intel-pipeline, codex/issue-268-delegation-contract):同样控制面 + delegation-contract 专属文件(CONTEXT.md、.github/workflows/ci.yml、invoke-code-intel.ps1、legacy/scripts/tests/test-primary-launchers.ps1、DR-0007-delegation-contract-and-skill-use-attestation.md)。
  - fd06 worktree(C:/Users/Administrator/.codex/worktrees/fd06/code-intel-pipeline, codex/issue-58-project-understanding-loop):同样控制面 + project-understanding-loop 专属文件(understanding_loop.rs、routes/understanding_routes.rs、edit_apply.rs、edit_impact.rs、tests/span_apply.rs、project-understanding-loop.md、DR-0007-project-understanding-loop.md)。
- **落定预期**:4 个 PR 分支 08-08 后无新提交;当前活工作是三个工作树里未提交的控制面;agent-gate 必过才能合并,而该闸现全 fail。因此 Codex 近期不会自动合入;未提交的控制面(与 #276 重叠)何时 commit/push 不可预期,存在被 Codex 抢先改动同一批文件的风险。

## 证据(命令 + 输出摘录)

### 1a. PR 状态与 checks(gh pr view/checks,在 D:/projects/_tools/code-intel-pipeline 内执行)
```
gh pr view 250 --json state,reviewDecision,mergeable,mergeStateStatus,headRefName,headRefOid,isDraft,createdAt,updatedAt,author,reviews
{"state":"OPEN","mergeStateStatus":"DIRTY","mergeable":"CONFLICTING","headRefName":"codex/issue-213-pwsh-signal-report","headRefOid":"e376641...","reviewDecision":"","reviews":[],...}
gh pr checks 250 → agent-gate fail | cross-platform-smoke/parity-observe/windows-build-test-package/GitGuardian/CodeRabbit pass
gh pr view 249 ... "mergeable":"CONFLICTING","mergeStateStatus":"DIRTY","reviews":[CodeRabbit 1 comment] 
gh pr view 248 ... "mergeable":"MERGEABLE","mergeStateStatus":"BLOCKED","reviews":[CodeRabbit nitpick] ; checks → agent-gate fail
gh pr view 202 ... "mergeable":"MERGEABLE","mergeStateStatus":"BLOCKED","reviews":[] ; checks → agent-gate fail
gh pr view 250/249/248/202 --json baseRefName → 全部 "main"
```
### 1b. head commit 时间(gh api .../commits/<sha> --jq .commit.committer.date)
```
PR250 e376641 → 2026-08-08T18:35:06Z ; PR249 efb4ebc → 2026-08-08T21:01:27Z
PR248 c9565c3 → 2026-08-08T17:13:23Z ; PR202 6267c6c → 2026-08-08T20:09:24Z
```

### 2a. 主 worktree 未提交(在 D:/projects/_tools/code-intel-pipeline 执行 git status --short / git diff)
```
21 M + 26 ?? 文件。M: execution_kernel.rs, capability_inventory.rs, command_catalog/mod.rs, routes/mod.rs, tests.rs, legacy.rs, dag_coordinator.rs, dag_run.rs, main.rs, run_cli.rs, snapshot.rs, cli-head-parity.v2.json, docs/…, orchestration/…, skills/code-intel-pipeline/SKILL.md
?? : routes/resolution.rs, routes/route_macros.rs, dag_run_recovery.rs, dag_run_registry.rs, friction_control.rs, friction_control_state.rs, friction_control_tests.rs, modernize_extract_rules_lite.rs, task_control.rs, task_control_state.rs, task_control_tests.rs, DR-0006-…, …
git diff -- crates/code-intel-cli/src/builtin_provider_evidence.rs → (空,未改动;故 codenexus_admission 未被 Codex 未提交工作触碰)
```
### 2b. Codex 未提交 diff 摘录(对重叠文件)
```
command_catalog/mod.rs:  import 加 friction_control/task_control;CompatibilityRoute 加 Task/Friction;dispatch 加 task_control::run_raw/friction_control::run_raw
routes/mod.rs:  加 task/friction raw_route;resolve_* 三函数抽到 resolution.rs(pub(super) use resolution::{…})
tests.rs:  raw count 33→35
legacy.rs:  Commands: 加 task/friction 两行
main.rs:  mod friction_control; mod task_control
capability_inventory.rs:  EXCLUDES 加 !**/.code-intel/**;加 modernize_extract_rules_lite module+dispatch
```
### 2c. #276 自身 2 commit(f6b0bf5..HEAD,在 D:/projects/_tools/code-intel-pipeline-issues-275 执行 git diff)
```
git rev-parse --abbrev-ref HEAD → agent/ps1-migration-275 ; git log --oneline -3 → e73ce3e docs: add PS1 retirement plan / 3c4a84c fix(rust): replace CodeNexusLite facade and hardcoded-paths ps1 with Rust implementations / f6b0bf5
builtin_provider_evidence.rs: 删 pwsh Command::new("pwsh") spawn;改 codenexus_lite::build_context(repo,repo,None,None,8,12,0);implementation id → codenexus_lite::IMPLEMENTATION_ID
command_catalog/mod.rs: import 加 hardcoded_paths;CompatibilityRoute 加 LintHardcodedPaths;dispatch 加 hardcoded_paths::run_raw
routes/mod.rs: +15 行,COMMAND_ROUTES 加 raw_route!(command:"lint", subcommand:Some("hardcoded-paths"), id: CompatibilityRoute::LintHardcodedPaths)
tests.rs: raw count 33→34
legacy.rs: Commands: 加 lint hardcoded-paths [<repo-path>] [--json]
main.rs: mod hardcoded_paths
gh pr view 276 → base main, 48 files, +3580/-4070, mergeable CONFLICTING
```

### 3. issue 274(在 D:/projects/_tools/code-intel-pipeline 内 gh issue view 274)
```
title: fix(review): remove PS orchestration drift and cover relocated shim in install smoke
state: OPEN ; labels: backlog, bug, claimed, enhancement ; assignees: 2233admin ; comments: 2 ; 无 Linked pull requests
body: 两个 P1 —— 1) 根 invoke-code-intel.ps1 去掉生产编排行为; 2) #216 install-class 路径迁移补 install-smoke 复现。
```

### 4. Codex 工作树状态(各工作树内 git log --oneline -3 / git status --short)
```
94ec(codex/issue-268-delegation-contract): HEAD=f6b0bf5(零新提交);控制面 + CONTEXT.md/ci.yml/invoke-code-intel.ps1/test-primary-launchers.ps1/DR-0007-delegation-contract
fd06(codex/issue-58-project-understanding-loop): HEAD=f6b0bf5(零新提交);控制面 + understanding_loop.rs/understanding_routes.rs/edit_apply.rs/edit_impact.rs/span_apply.rs/DR-0007-project-understanding-loop.md
主 worktree: release/v0.7.x-rc @ f6b0bf5
进程: tasklist → codex.exe PID 68716 (≈390MB) + codex-code-mode-host.exe PID 134752 运行中
```