# DR-0007 GitHub Issues 是本仓库自身工作的交付任务状态权威

Status: active
Date: 2026-08-27

## Decision

对本仓库自身的 agent 工作（不是被扫描的目标仓库），GitHub Issues 是交付任务状态（delivery task state）的唯一权威：谁在做什么、是否被认领、是否阻塞、是否关闭，都以 `gh issue`/`gh pr` 里的状态为准。Linear、Gitea Projects 或其他托管 tracker 只能作为链接/镜像，不得成为并行的可写状态源；只有某个 initiative 明确授权后才允许把它们提升为该 initiative 的权威（沿用 ADR-0008 的 local-first 发现顺序——本 DR 是该顺序对本仓库的具体落地结果，不是替换它）。

## Why

三组证据，全部来自本仓库自己的历史，不是偏好声明：

1. **60+ 天、约 60 次提交/合并的实际流转，0 次经过 Linear。** `git log --oneline -60` 里几乎每条记录都带 `(#NNN)` 形式的 GitHub issue/PR 编号（如 #367→#368、#361→#362、#352→#353/#354/#356、#349→#355、#178/#192→#350 等），认领与整合协议本身（DR-0004）就是在 GitHub issue 上打 `claimed` label + comment，PR 合并/关闭时撤 label。没有一次是通过 Linear 状态变更驱动的。
2. **`orchestration/internalization/linear.json` 自己记录了用量为零。** 该 internalization record 的 `economics.benefit` 字段：`{"metric":"current scanner operations requiring Linear","value":0,"unit":"operations"}`，`necessityEvidence` 的 evidence id 是 `local:r23:no-current-use`。这不是本 DR 的主张，是仓库既有机制早就落过账的数字。
3. **ADR-0008 已经把 Linear 从"preferred default"降级为"optional projection"，但 `docs/agents/issue-tracker.md` 当时的文案（"multi-tracker precedence，第 3 级是 hosted tracker，第 4 级需显式授权新建"）仍然把 GitHub 和 Linear/Gitea 摆在同一层级的候选位置，没有反映出上面两条实证——本仓库的"发现"步骤（ADR-0008 步骤 2："discover and reuse an existing … repo-native task graph"）在本仓库里实际上一直、且唯一地解析到 GitHub Issues。文档口径落后于已经稳定发生的行为超过一个月，属于该被纠正的漂移，不是新政策。

## Enforcement

- 机制强制：DR-0004 的 issue 认领协议已经把 GitHub Issues 当作事实上的任务状态机在跑（claim/comment/close 全部是 `gh` 命令）；本 DR 只是把这个既成事实写成决策记录，不需要新增闸门代码。
- 文档强制：`docs/agents/issue-tracker.md`、`docs/agents/domain.md`、`docs/agents/triage-labels.md`、`AGENTS.md` 本 PR 内同步改写，直接体现本决策；这些文档是 agent 开工前必读（`domain.md` 的 "Before Work"/"Before exploring, read these" 一节），文档本身就是强制面。
- Convention only 的部分：不存在自动阻止某个 initiative 显式选择 Linear 作为其权威的技术闸门——需要时仍可按 ADR-0008 的显式授权路径走，只是默认值和本仓库实际观测到的行为改成 GitHub。
