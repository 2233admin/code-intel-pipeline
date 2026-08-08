# DR-0004 issue 认领协议

Status: active
Date: 2026-08-08

## Decision

任何 session（人或 agent）对某个 issue 开工前，先在该 issue 打 `claimed` label 并留一条 comment：认领分支名 + 一句话方案。看到已有 `claimed` 的 issue：要么读完认领方案后**加入**（在同一分支/PR 上续），要么**换 issue**。不允许无视认领另起炉灶。

PR 合并或关闭时撤 label。认领超过 48h 无 push 视为过期，可被接管（接管者 comment 说明）。

## Why

2026-08-08 实证：#218 被两个 session 平行修出 #227 与 #228——方案不同（标记检查 vs entrypoint probe）、文件重叠、必然冲突，其中一份工作量注定报废。同日 #208 与 owner 已定口径相反（见 DR-0002）。蜂群的产出带宽是 N 倍，**整合带宽还是 owner 一个人**；没有认领协议，产出带宽的富余全部转化为整合债。

## Enforcement

Convention only（暂无机制强制）。AGENTS.md 收录；违反的后果自然显现——撞车方案二选一时，未认领的一方默认让路。
