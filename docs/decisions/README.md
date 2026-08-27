# Decision records

活语义决策的落账处。规则只有一条：**改变多个 session / 多个 agent 行为口径的决策，必须落在这里，不能只存在于某一次对话里。**

为什么强制：2026-08-08 的实证——「版本 pin 是下限还是精确锁」这个口径在一个 session 里被 owner 定调为下限，而平行 session 看不见，产出了论证精确锁的反向 PR（#208）。工具自己给别的仓库布道 decision records（`get_why`、standing decisions），自己的活口径却散在聊天里。这个目录终结这件事。

## 格式

一条决策一个文件，`DR-NNNN-<slug>.md`，字段：

```
Status: active | superseded by DR-XXXX
Date: YYYY-MM-DD
Decision: 一句话
Why: 证据与反事实（不写这条会发生什么，最好引已发生的实证）
Enforcement: 谁在什么时机强制它（gate / 测试 / 评审规约），没有强制手段的决策写明 "convention only"
```

## 现有决策

| # | 决策 | 状态 |
|---|---|---|
| [DR-0001](DR-0001-install-topology-gate.md) | 安装类 bug 的复现必须同 PR 进 install-smoke 闸 | active |
| [DR-0002](DR-0002-version-pins-are-floors.md) | 工具版本 pin 是下限，不是精确锁 | active |
| [DR-0003](DR-0003-manifest-discovery-precedence.md) | manifest 发现优先级与 probe 语义 | active |
| [DR-0004](DR-0004-issue-claim-protocol.md) | issue 认领协议：开工先打 claimed label | active |
| [DR-0005](DR-0005-integration-debt-ceiling.md) | 整合债上限：修复 PR 积压时停产新 feature | active |
| [DR-0006](DR-0006-solo-operator-retirement-gate.md) | 已证死代码分支（E02/E03）可用静态不可达证明替代 30 天遥测窗口 | active |
| [DR-0007](DR-0007-github-issues-delivery-ssot.md) | GitHub Issues 是本仓库自身工作的交付任务状态权威 | active |
| [DR-0008](DR-0008-evolution-degraded-classification.md) | sentrux.evolution 与 sentrux.what_if 同型病灶，evolution 分类改判 automatic_degraded | active |
| [DR-0009](DR-0009-sentrux-scan-stub-field-honesty.md) | sentrux.scan/rescan 的伪造 stub 字段必须诚实化（null+status，非假 0），scan/rescan 提升为 authoritative_automatic | active |

平行 session 开工前先扫本目录（一次 `ls docs/decisions/` + 读 README 表格，30 秒）。与已有决策相悖的工作，先开 issue 挑战决策本身，不要直接实现相反语义。
