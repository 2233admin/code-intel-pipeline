# Handoff: PS1 全砍迁移(code-intel-pipeline)

> 2026-08-17 · Claude → 下个 session。读此文件即可恢复全部上下文。
> 主计划文档:`docs/archive/ps1-retirement-plan-2026-08-17.md`
> 研究报告:`docs/research/codex-ps1-sync.md`

## 一、任务是什么

工具仓 `D:/projects/_tools/code-intel-pipeline`(Rust CLI `code-intel` v0.7.x)的 PS1 全砍:
把全部 262 个 PowerShell 脚本退役,换成 Rust。AGENTS.md 禁 big-bang,必须逐入口验证后退役。

用户决策:
- 方向 = **全砍**(不是保留兼容面)
- 策略 = 等 Codex 落定 + 主动协调,拆 PR 隔离冲突面
- 调研 = 已做(Codex 同步研究报告),**不让 Claude 自己胡诌**

## 二、已完成(3 个可交付)

### 1. PR #277 已开(codenexus 迁移,零冲突)——**可合入**
- 分支 `agent/codenexus-rust-275`,worktree `D:/projects/_tools/code-intel-pipeline-codenexus-275`
- 内容:`codenexus_lite.rs`(进程内生成 codenexus-context.json)+ `builtin_provider_evidence.rs`(去 pwsh spawn)
- 测试:648+263+9 全绿,13 新测试
- 已验证:Codex 未提交 diff 未碰 builtin_provider_evidence.rs,零冲突
- URL: https://github.com/2233admin/code-intel-pipeline/pull/277

### 2. hardcoded-paths 迁移(本地分支,等 Codex)
- 分支 `agent/hardcoded-paths-275`,worktree `D:/projects/_tools/code-intel-pipeline-hcp-275`
- 内容:`hardcoded_paths.rs`(新子命令 `lint hardcoded-paths`)+ 8 个接线文件(ci.yml/AGENTS.md/command_catalog/legacy/main/docs)
- 测试:640+250+9+5+5 全绿,5 新测试
- **冲突**:command_catalog/{mod,routes/mod,tests}.rs、legacy.rs、main.rs 与 Codex 未提交的 task/friction 控制面同区重叠
- **下一步**:Codex 控制面提交后 → `git rebase` → 开 PR

### 3. 研究报告(已完成,证据充分)
- `docs/research/codex-ps1-sync.md`(工具仓 + tdxcli-rs worktree 各一份)
- 结论:Codex 4 PR 全被 agent-gate 卡死(8 天无新提交),工作全未提交,进程活跃

## 三、关键路径

| 项 | 位置 |
|---|---|
| 主仓 | `D:/projects/_tools/code-intel-pipeline`(release/v0.7.x-rc @ f6b0bf5,Codex 21 M + 26 ?? 未提交) |
| PR #277 worktree | `D:/projects/_tools/code-intel-pipeline-codenexus-275` |
| hcp worktree | `D:/projects/_tools/code-intel-pipeline-hcp-275` |
| 主计划 | `docs/archive/ps1-retirement-plan-2026-08-17.md` |
| 研究报告 | `docs/research/codex-ps1-sync.md` |
| session 调研 | `docs/research/session-gate-migration-notes.md` |
| issue | #275(claim 我的)、#274(Codex 方向,已被 claim) |
| PR | #277(我的,可合)、#250/#249/#248/#202(Codex,全被闸卡) |

## 四、下一步(新 session 从这里开始)

1. **合并 #277**(如果有人 approve 或 owner 合)→ 更新 tdxcli-rs 引用?不需要,#277 不碰 CLI 接口
2. **盯 Codex 控制面**:`cd D:/projects/_tools/code-intel-pipeline && git status --short | wc -l` 变化时 → Codex 快提交了
3. **Codex 提交后**:rebase `agent/hardcoded-paths-275` → 开 PR → 同步更新 ci.yml/AGENTS.md
4. **session 门禁迁移**(下一个大项):`Invoke-SentruxAgentTool.ps1 session_start/end` → Rust。调研笔记在 `docs/research/session-gate-migration-notes.md`。**先确认 Codex 是否在做**(他 PR #250 标题涉及 legacy session)
5. **全砍执行**:按主计划 8 步(parity 留档 → 根/legacy 活入口 → 子目录 → CI/hooks/文档 → 跨仓 tdxcli-rs → dist → 残留验证 → retirements 流程)

## 五、环境速查

- 工具仓双远端:`gitea`(git.xart.top,主)+ `origin`(GitHub,issue/PR 流程)
- GitHub issue/PR 用 `gh`(已认证),Gitea API 用 `GITEA_CLAUDEQWQ_TOKEN`
- 跨 agent 同步:在 Codex 的 PR 上留评论(已做过一轮,复评时引用 #277)
- Rust 测试:`cargo test -p code-intel`(注意:主仓当前测试红 = Codex digest 未同步,不是我的错)
- worktree 惯例:`code-intel-pipeline-<名字>`,分支 `agent/<slug>`
- AGENTS.md 门禁:claim issue 后才能写代码;测试绿才能退役入口

## 六、风险提醒

- Codex 8 天无提交但进程活跃,随时可能抢跑改 command_catalog 等 5 文件——**hcp 分支合入前必须 rebase**
- 主仓测试红(capability_inventory.rs digest)是 Codex 的活,别替他修
- `Invoke-SentruxAgentTool.ps1` 的 session_start/end 是 tdxcli-rs 工作流强依赖,迁移前必须实测 Rust 等价
