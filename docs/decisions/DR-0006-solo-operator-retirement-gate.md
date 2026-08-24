# DR-0006 单人仓库下退休闸的独立审批与遥测替代路径

Status: active
Date: 2026-08-25

## Decision

`compatibility_retirement_gate.rs` 的两条检查在单人仓库（本仓库：单一 owner，无独立评审池）下按以下方式满足，不再要求原始形态：

1. **`independent_approval`**：原逻辑要求 `reviewer != legacy.owner` 且有第二身份的签名 authority event（`check_independent_approval`, `compatibility_retirement_gate.rs:504-523`）。单人仓库里这个身份不存在，是结构性死锁，不是"还没找人签"。替代路径：作者本人签署，但必须是**两次相隔 ≥7 个自然日的独立签名事件**（cooldown 自证，不是同一 session 内一次签两次）。`independent-approval.json` 的 evidence schema 增加 `mode: "solo-operator-cooldown"` 字段区分两种满足路径；`reviewer == legacy.owner` 允许，但 `secondSignatureAt - firstSignatureAt >= 7*86400` 强制检查。

2. **`unproven_usage_observation` / `unproven_compatibility_window`**：原逻辑只接受运行时遥测计数 + 30 天窗口。新增替代路径：**静态不可达性证明**——如果测试套件里已有断言"该函数/调用点在源码中不存在"（如 #323 investigation 记录的 E02/E03：`scripts/tests/test-workflow-recommendation-brief.ps1:59-61` 断言 inline 函数不存在，`test-repowise-adapter-contract.ps1:9-13` 断言直接调用不存在），且该断言在 CI 中跑绿，视为 `totalInvocations`/`legacyInvocations` 结构性恒为零，等价于遥测窗口的最强形式（永远通过），不需要再等 30 个日历日去观测一个数学上已经是零的东西。此路径**只适用于代码路径已被证明完全移除**的分支（当前已知：E02、E03），不适用于代码仍然存在只是"看起来没人调用"的分支（E04/E07/E08 不适用，它们的替代能力本身还在开发或未接入默认路径）。

## Why

2026-08-25 实证，来自本仓库自己的调查记录（issue #323 前两条评论）：

- E02（recommender）：inline 函数在 `legacy/run-code-intel.ps1` 里**已经不存在**，默认路径 100% 走 Rust capability，`totalInvocations` 结构性为零，不是"暂未观测到"。
- E03（provider-preflight）：同型，`test-repowise-adapter-contract.ps1` 主动 fail 若走回旧路径。

对这两条分支要求"遥测 30 天"是在测量一个已经被静态证明为零的量——闸门设计假设的前提（"需要运行时观测才能知道用量"）在这两条分支上不成立，等窗口不会产生新信息。

`independent_approval` 的问题更基础：`reviewer != legacy["owner"]` 这个检查（`compatibility_retirement_gate.rs:514`）在只有一个 owner 身份的仓库里**永远无法为真**，不管等多久、遥测多完整。这不是"证据不够"，是闸门设计时假设了一个多人评审团队（issue #14 的 self-dogfood 意图是防"agent 自己批自己"，这个意图本身是对的），但没有为单人 + AI-agent 协作这种拓扑留退路，导致这条闸永久锁死，与产品是否真的安全无关。

本项目非量化交易类高风险系统（对比：仓库里 sextant/katana-data-doctor 类项目才是有真实资金后果的场景），删除已证明死掉的 PowerShell 分支的下行风险是"删错了要 revert"，不是资金损失或数据损坏，7 天冷静期 + 静态证明的组合已经对称覆盖原闸门想防的"仓促自批"风险。

## Enforcement

Convention only，本 DR 落账口径；**代码尚未改**。`compatibility_retirement_gate.rs::check_independent_approval` 和 `check_compatibility_and_usage` 仍是原始行为，本 DR 生效前任何 session 想真的让 E02/E03 的 `decision` 从 `blocked` 变成非 blocked，必须先把这条 DR 描述的替代路径实现进闸代码 + 更新对应 packet 的 evidence json，并跑 `test-retirement-packets.ps1` 证明其余未满足替代条件的分支（E04/E07/E08）行为不变。跟踪实现的 issue：待开（本 session 未开，下一 session 开工前先查 `gh issue list` 是否已存在再开新的，避免重复）。
