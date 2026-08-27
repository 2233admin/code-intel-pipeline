# DR-0008 sentrux.evolution 与 sentrux.what_if 分类不对称的裁决

Status: active
Date: 2026-08-27

## Decision

`orchestration/sentrux-capability-matrix.v1.json` 里 `sentrux.evolution` 的
`currentState` 从 `authoritative_automatic` 改判为 `automatic_degraded`，与
`sentrux.what_if`（修复前）同类。原因：两者当时处于**完全相同**的代码路径病灶
——生产 DAG 派发（`builtin_provider_evidence.rs::run_sentrux`）都无条件走
`sentrux_lite_capabilities.rs` 的 lite 简化实现（`evolution_json`/
`what_if_json`），而各自都有一个更完整、PS1 编排器已经在用的"真引擎"
（`sentrux_evolution.rs`/`sentrux_what_if.rs`）未被接入。`sentrux_evolution.rs`
自己的模块文档原文把两个 lite 实现称为同一句话："an intentionally simplified
('lite') degraded fallback, not shape-compatible with what the PS1
orchestrator's artifacts... expect"——这句话对 `evolution_json` 和
`what_if_json` 一视同仁，但矩阵里两者的 `currentState` 却不同。

`sentrux.what_if` 的这一病灶已在 #374 修复（DAG 派发换成
`sentrux_evolution::what_if`，其 `currentState` 已随之改回
`authoritative_automatic`）。`sentrux.evolution` 的同型病灶**未在 #374 修复**
——`evolution` 分支的接线验证需要独立核对其消费方（`evidence.sentrux`、
`diagnosis.hospital`、`report`、`release_gate`）的字段/形状依赖，工作量与
`what_if` 本身相当，不是一行对称改动，因此拆到新 issue #377 单独跟踪，不在
#374 这个 PR 里顺手修。

在 #377 完成接线与消费方验证之前，矩阵必须如实反映当前代码：`evolution` 仍然
只产出 lite 输出，`authoritative_automatic` 标签与实际不符。

## Why

2026-08-27 实证，源自 #374 的调查记录：

- `builtin_provider_evidence.rs::run_sentrux` 的 `"evolution"` 分支硬编码调用
  `sentrux_lite_capabilities::evolution_json`，与修复前的 `"what_if"` 分支
  （调用 `what_if_json`）是同一个模式——生产环境（`toolPathPrefix` 未配置时，
  即永远）总是拿到 lite 结果。
- `sentrux_capability_artifacts.rs::uses_lite_fallback` 把 `"evolution"` 和
  `"what_if"` 并列在同一个 lite-fallback 判定列表里，代码本身不区分二者。
- `currentState` 是纯人工声明字段——`sentrux_capabilities.rs::capability_audit`
  只是聚合读取这个字段，没有任何代码校验它与实际运行路径是否一致（issue
  #373 也明确指出这一点）。矩阵由 #285（commit `b5bb8f04`）一次性整体撰写、
  此后未再编辑过——`evolution` 的 `authoritative_automatic` 标签大概率是撰写
  时的疏漏或过于乐观的假设，而不是基于当时代码路径的真实核实结论；本次调查
  没有找到任何文档、注释或 issue 记录能证明这是一个"evolution 与 what_if
  确有实质区别"的有意决策。
- 反事实：如果放任 `evolution` 继续标 `authoritative_automatic`，#373 描述的
  "4 个 automatic_degraded capability 需要补齐 authoritative_automatic 才能
  放行 complete release" 的完成度台账就会少算一个真实缺口——CI/PR-gate/
  release 三条流水线（`.github/workflows/{ci,pr-gate,release}.yml`）都要求
  `authoritative_automatic` capability 的 `payload.status` 必须是
  `"succeeded"`，而 `automatic_degraded` capability 额外接受 `"degraded"`；
  给一个实际只产出简化数据的 capability 挂上更严格的标签，是把"这个能力值得
  信任"的错误信号焊进发布闸门。

## Enforcement

Convention only（矩阵是手工维护的清单，无运行时校验）。`tests/
test_sentrux_capability_matrix.py` 新增/沿用的回归断言把
`sentrux.evolution`/`sentrux.what_if` 的 `currentState` 钉死为当前值，防止
后续改动在没有配套接线证据的情况下静默漂移；接线完成后修改断言值需要同一
PR 里附带 #377 的接线证据（对应引擎已实际接入 DAG 派发 + 消费方形状核实）。
