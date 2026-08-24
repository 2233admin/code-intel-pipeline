# DR-0006 已证死代码分支的遥测窗口静态证明替代路径

Status: active
Date: 2026-08-25

## Decision

`check_rollback_and_usage`/`check_compatibility_window`（`compatibility_retirement_gate.rs`）原逻辑只接受运行时遥测计数 + 30 天窗口来满足 `usage_observation`/`compatibility_window`。新增替代路径：**静态不可达性证明**——如果 CI 里已有跑绿的断言"该调用点在源码中不存在"（如 #323 investigation 记录的 E02/E03：`test-workflow-recommendation-brief.ps1:59-61` 断言 inline 函数不存在，`test-repowise-adapter-contract.ps1:9-13` 断言直接调用不存在），视为 `totalInvocations`/`legacyInvocations` 结构性恒为零，等价于遥测窗口的最强形式（永远通过），不需要再等 30 个日历日去观测一个数学上已经是零的量。此路径**只适用于代码路径已被证明完全移除**的分支（当前已知：E02、E03），不适用于 E04/E07/E08（代码仍在、替代能力未完工或未接默认路径）。

## Why

2026-08-25 实证，来自本仓库自己的调查记录（issue #323 前两条评论）：

- E02（recommender）：inline 函数在 `legacy/run-code-intel.ps1` 里**已经不存在**，默认路径 100% 走 Rust capability，`totalInvocations` 结构性为零，不是"暂未观测到"。
- E03（provider-preflight）：同型，`test-repowise-adapter-contract.ps1` 主动 fail 若走回旧路径。

对这两条分支要求"遥测 30 天"是在测量一个已经被静态证明为零的量——闸门设计假设的前提（"需要运行时观测才能知道用量"）在这两条分支上不成立，等窗口不会产生新信息。

## 已撤回的错误结论（留痕，不删除）

本 DR 最初起草时（同一 session）还主张 `independent_approval` 对单人仓库结构性死锁，理由是"`reviewer != legacy.owner` 在只有一个 owner 身份的仓库里永远无法为真"。**这个结论是错的**，被同一 session 后续读代码推翻，证据：

- `legacy["owner"]` 在真实 manifest 里是能力角色 id（如 `"executor-recommender"`），不是人类身份；测试 fixture 的 owner 是 `"owner-team"`，reviewer 通过的是 `"code-intel-maintainers"`——两者本来就是不同的字符串,不是同一个人扮演两个角色的冲突。
- 信任锚点 `authority.rs::TRUSTED_APPROVERS` 本身就是仓库自签的角色白名单（`[("code-intel-maintainers","repository_governance")]`），文档注释明写"trust comes only from the checked-in id/role allow-list"——设计上就是**仓库治理自证**，不要求第二个独立人类。
- 现有测试 `self_reported_independence_cannot_override_owner_or_authority_policy` 显式验证的是"owner 角色 id 和 reviewer 角色 id 相同时才拦"，不是"同一个人不能签"。
- E02 当前的 `evidence/independent-approval.json` 是纯占位 stub（`reviewer: "independent-verifier-required"`, `authorityEvent: {}`），从未真正尝试构造过签名事件——不是"试了签不了"，是"没人真的走过这个流程"。

**结论**：`independent_approval` 对 E02/E03 不需要任何闸门代码改动，只需要真的生成一份合法的 `code-intel-maintainers`/`repository_governance` 签名 evidence（sha256 摘要 + attestation digest 按 `authority.rs::authority_event_digest` 规则算）。这是数据生成工作，不是决策工作。

## Enforcement

Convention only，静态证明路径尚未写进 `compatibility_retirement_gate.rs`，跟踪实现见 issue #339。E02/E03 的 `independent_approval` 证据本身不受本 DR 约束——按现有闸门逻辑真实生成即可，属于 #323 下的执行工作，不是决策工作。
