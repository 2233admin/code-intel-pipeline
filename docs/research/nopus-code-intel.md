# nopus 对 Code Intel 流程的可借鉴点

日期：2026-08-23

## 结论

nopus 不是代码索引器，而是一个确定性、可配置、单次介入的输出质量检查器。对 Code Intel Pipeline 最有价值的不是其英语词典，而是把 agent 输出质量变成可重复测量、可解释、有限重试的 host-level contract。类似能力应放在 Intel 的 report/diagnosis/plan 产出层，不应污染 evidence provider。

## 上游事实

- response 完成时运行 deterministic prose checks；相同文本和 sensitivity 得到相同决定。[README](https://github.com/2233admin/nopus/blob/main/README.md#how-it-works)
- 检查 uncommon wording、abstract vocabulary/sentences、noun/modifier stacks、phrase load、formulaic cues，并要求多个信号组合后才触发。[README](https://github.com/2233admin/nopus/blob/main/README.md#how-it-works)
- 分析前剔除代码块、inline code、URL、路径和表格，降低计算机术语权重，保护 identifier、command、condition、qualification。[README](https://github.com/2233admin/nopus/blob/main/README.md#how-it-works)
- 自动 rewrite 最多一次；rewrite 请求携带 focus areas 和原文 examples，避免 retry loop。[README](https://github.com/2233admin/nopus/blob/main/README.md#how-it-works)
- 有 low/medium/high 三档 sensitivity，并维护 parity、label review、corpus calibration、rewrite-model evaluation。[README](https://github.com/2233admin/nopus/blob/main/README.md#choose-how-sensitive-it-should-be)、[package.json](https://github.com/2233admin/nopus/blob/main/package.json)

## 与当前 Intel 的对照

当前项目已有 artifact schema/refs、snapshot identity、admissibility、provider provenance、audit report fail-closed validation、anchor verification，以及 change impact / gate verdict。证据见 `CONTEXT.md`、`crates/code-intel-cli/src/artifact_ref.rs`、`admissibility.rs`、`audit_report/`。

差距在上层：agent-facing report、diagnosis、plan 还没有统一的“证据完整性 + 可执行性 + 表达清晰度”终检。nopus 的启发是检查已完成产物、给出定位证据、最多一次 bounded repair，并重新验证 repair 结果。

## 修正后的实施建议

不要先新增顶层 `pass|rewrite_requested|reject` 公共 schema；这会和现有 gate verdict / audit report 状态耦合。第一步应把三个 reason code 作为现有诊断输出的内部规则，用已有 contract fixtures 证明缺口和收益：

1. `missing_evidence_anchor`：结论没有可解析 artifact/anchor。
2. `plan_without_verification`：计划步骤没有 action、target 或 verification。
3. `scope_leak`：产出超出 snapshot/repository scope。

验证目标：

- 合法技术术语、命令、路径和 identifier 不被误伤；
- 既有合法 fixture 行为不变；
- 三类缺陷有稳定、可解释的诊断；
- 不改变底层 provider 事实，也不引入自动重写。

通过后再决定是否扩展公共 schema，或接入最多一次的 repair 流程。事实正确性仍由现有 schema、digest、snapshot、引用解析和 scope validation 负责；表达规则不能替代这些硬闸。

## 不建议照搬

- 不把通用词频/抽象度量直接用于 evidence JSON 或源码。
- 不把 Stop hook 当事实正确性 gate。
- 不新增 Node 生产运行时或第二套长期存储；Rust CLI 与现有 artifact contract 是默认边界。
