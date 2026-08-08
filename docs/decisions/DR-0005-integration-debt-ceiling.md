# DR-0005 整合债上限

Status: active
Date: 2026-08-08

## Decision

未合并的**修复类** PR（`fix:` / bug label）数量 ≥ 5 时，停止开新 feature PR，直到修复队列消化到 5 以下。清偿动作优先级：可合的合（CLEAN 且绿）→ 冲突的 rebase → 被取代的关闭。

## Why

2026-08-08 队列快照，病灶不是缺解药是解药不被采纳：

- #214（install-smoke 闸——本可拦下当天全部安装 bug 的那道闸）`CLEAN` 可合，GA 当天开的，**GA 没带它就切了**
- #202（change-risk 绝对分闸）烂到 `DIRTY` 3 天，期间被它取代的百分位闸继续误伤无辜 PR
- #216（真实安装 blocker 修复）分支落后 main，CI 陈旧红，没人管

meta 修永远排不过 feature，蜂群还在放大失衡：产出带宽 N 倍，整合带宽 1 倍。上限是唯一能逆转优先级的机制——**先吃药，再产新药**。

## Enforcement

Convention only。AGENTS.md 收录：session 开工前 `gh pr list --state open` 数一下 fix PR；≥5 时本 session 的产出必须是清偿队列，不是新增。
