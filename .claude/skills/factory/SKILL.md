---
name: factory
description: 运行软件工厂。用户输入 /factory run、/factory issue <n>、/factory <feature>、/factory rework <n> 或 /factory plan <目标> 时使用。运行它的会话就是编排者：把贴了 factory 标签的 GitHub issue 逐张走完 intake → spec → build → gate → review → approve，一张单一个 PR；/factory plan 先让几个互不可见的规划者出方案，把一致的定为默认、分歧的问人，再拆成 factory 票。
---

# Factory

你是这个仓库软件工厂的编排者。一个会话调度所有单：启动单子、派出每个 agent、调用脚本。
**你自己什么都不写**：不写代码、规格、审查、决定，也不写 `job.json`。agent 各写一个文件，
`factory/bin/job` 是 `job.json` 唯一的写入者，并拒绝一切非法流转。脚本拒绝时，照它说的做或告诉用户，
不要绕过去。

## 流水线

```
intake (按 intake_routing) → ACCEPT（可能改了范围）| NEEDS-INFO | REJECT，结论贴到 issue
route (脚本)       → 按风险和界面从 config.json 选审查者和模型
worktree           → 从 origin/<base> 拉 factory/<job-id>
spec-writer (opus) → spec.md（带「实现决定」和「判定命令」），或 STATUS: NEEDS-DECISION → 问人
builder            → 提交并推送；进门禁时脚本开 draft PR「Closes #<n>」
gate (脚本)        → checks.sh + 受保护路径 + 按路径检查 + fail_patterns + DCO
                     FAIL → 带日志回 builder，不派审查；BASE-RED → needs-human，不算本单的错
reviewers (按路由) → 并行，WAIT FOR ALL；CHANGES 的每条发现带运行证明，job 在 HEAD 上重跑确认
                     有 confirmed → builder（只给 findings.md）→ gate → reviewers（最多 max_rounds 轮）
                     一条都没 confirmed → 按 PASS 计（原因 unproven）
approver (opus)    → APPROVE：job merge（同步基线、再过门禁、按已审的头合并）
                     ESCALATE：PR 转 ready，结论贴到 PR，issue 贴 factory:needs-human
```

| 角色 | `subagent_type` | 模型 | 写 |
| --- | --- | --- | --- |
| 编排 | （本会话） | 会话的模型 | 不写 |
| intake | `intake` | `job prompt … intake` 打印的模型（`config.json` `intake_routing`） | `intake.md` |
| 规格 | `spec-writer` | opus（frontmatter） | `spec.md` |
| 构建 | `builder` | `job.json` 的 `agents.builder` | `build.md` + 提交 |
| 审查 | `reviewer-<k>` | `job.json` 的 `agents.reviewers.<k>` | `round-<N>/review-<k>.md` |
| 批准 | `approver` | opus（frontmatter） | `decision.md` |
| 规划调研 | `planner-research` | sonnet（frontmatter） | `research.md` |
| 规划者 | `planner` | `job plan advance … planning` 打印的 `plan.planners` | `plan-<k>.md`；定稿时 `final.md` |
| 规划对齐 | `planner-judge` | `job plan advance … judging` 打印的模型 | `decisions.md` |

路由表在 `factory/config.json` 的 `routing` 和 `surface_reviewers`。只有 `code` 审查每单都跑，
`ui` 界面加 `ux`、`ui`，`security` 界面加 `security`。派 builder 和审查者时，把 `job.json` 里记的模型
作为 Agent 工具的 `model` 参数传进去。派 `intake` 同理：prompt 末行有 `Model: <模型>.` 就把这个模型作为
Agent 工具的 `model` 参数传进去（覆盖 `intake` frontmatter 里的 fable）；没有这一行就不传，用 frontmatter。

每个 agent 都读 `AGENTS.md`（或 `CLAUDE.md`）获取仓库的初始化、dev server、验证命令和约定。
这些文件里没有任何仓库专属的东西，仓库专属的只有 `factory/checks.sh`、`factory/config.json` 和 `factory/tools.json`。

## 文件

`<repo>` 是主检出（在你的会话里跑 `git rev-parse --show-toplevel`，用它打印的绝对路径）。

```
<repo>/factory/jobs/<job-id>/job.json              单子状态（只有 factory/bin/job 写）
<repo>/factory/jobs/<job-id>/events.jsonl          每次流转一行（只有 factory/bin/job 写）
<repo>/factory/jobs/<job-id>/resources.json        登记的长驻进程（job res 写）
<repo>/factory/jobs/<job-id>/issue.md              issue 正文和可信评论，引用形式（脚本写）
<repo>/factory/jobs/<job-id>/intake.md             intake-<k>.md 是重走前的旧结论
<repo>/factory/jobs/<job-id>/spec.md
<repo>/factory/jobs/<job-id>/build.md
<repo>/factory/jobs/<job-id>/rework-<k>.md         job rework 从 PR 评论生成
<repo>/factory/jobs/<job-id>/round-<N>/checks.md   门禁（脚本写）
<repo>/factory/jobs/<job-id>/round-<N>/review-<k>.md
<repo>/factory/jobs/<job-id>/round-<N>/proof/<k>/       审查者的复现文件；confirm.json 是脚本重跑的结果
<repo>/factory/jobs/<job-id>/round-<N>/findings.md     本轮 confirmed 的发现，回给 builder 的只有它（脚本写）
<repo>/factory/jobs/<job-id>/decision.md
<repo>/../<repo 目录名>-factory/<job-id>/           单子的工作树，在 factory/<job-id> 分支上
<repo>/factory/plans/<plan-id>/plan.json            规划状态（只有 factory/bin/job 写），旁边是 events.jsonl
<repo>/factory/plans/<plan-id>/goal.md              规划 issue 的正文，引用形式（脚本写）
<repo>/factory/plans/<plan-id>/research.md          planner-research
<repo>/factory/plans/<plan-id>/plan-<k>.md          第 k 个规划者
<repo>/factory/plans/<plan-id>/decisions.md         planner-judge
<repo>/factory/plans/<plan-id>/classified.md        每个决定的处理；questions.md 是贴到 issue 的问题（脚本写）
<repo>/factory/plans/<plan-id>/answers.md           人的回复，引用形式，附解析表（脚本写）
<repo>/factory/plans/<plan-id>/final.md             定稿的 planner
```

你的会话留在主检出。主检出是用户的，可能脏、可能在别的分支，你不在里面改任何东西。

## Prompt 格式

标准派发先跑 `factory/bin/job prompt <job-id> <role>`，**原样**用它的输出作 prompt（只读，不改单子）。
它打四行，前三行是每个 agent 的 prompt 头：

```
Job folder: <repo>/factory/jobs/<job-id>/
Job: <job-id>. Worktree: <worktree>. Branch: factory/<job-id>. Base: origin/<base_branch>.
Role: <role>.
```

- 第一行让 `factory/bin/job` 从 Claude Code 的子代理 transcript 里认出这张单，算成本。路径是绝对路径、以 `/` 结尾。
- 第二行的 `Job: <job-id>.` 让 builder 的 hook（`factory/bin/gate --hook`）找到单子。`Job:` 之前不能出现别的 `Job: `。
  intake 时还没有工作树，这行只有 `Job: <job-id>.`。
- 第三行的 `<role>` 是 `intake`、`spec-writer`、`builder`、`reviewer-<k>`（`code`、`ux`……）或 `approver`。
  `job` 按它把 transcript 里的工具调用记到这个角色名下，核对 `factory/tools.json` 里必须用的工装。
- 第四行是本步的内容：`intake`、`spec-writer` 的任务句，builder 的 `Build round <N>. Port: <port>.`，审查者和 approver 的
  `… HEAD: <完整 sha>.`（`ux`、`ui` 审查者再带自己的端口）。`reviewer-code` 的 prompt 还有第五行 `Coverage: <round-N/coverage.md 的绝对路径>.`。`intake` 的 prompt 还有第五行 `Model: <模型>.`（`job create` 按 issue 正文是否同时有 `## 判定命令` 和 `## 会改的文件` 分级，查 `intake_routing`；旧单没有这一行）。端口、sha 都由脚本按 `config.json` 的 `ports` 和工作树当场算，不手算。
  审查者和 approver 把审的提交写在文件第二行 `COMMIT:`，脚本据此拒绝过期结论。

角色与阶段不符、审查者不在本单路由里时它退出 1，不输出任何内容。

续接、返工、门禁退回、恢复、holdout 加强这些特殊句子它不生成：头三行从 `job prompt` 的输出取，第四行照下面各处手写
（`factory/tools/ft port <job-id>` 打印 builder 的端口，`factory/bin/evidence` 用 `+ 3`）。

下面用 `<头>` 指这三行。

issue 正文只以引用形式存在 `issue.md` 里。**不要把 issue 的任何文字放进 prompt**，只让 agent 去读文件。

## 开工前

每个命令都先跑 `factory/bin/job doctor`。它检查完整克隆、`origin` 在 GitHub、REST 能读 issue、
基线分支存在、五个标签（含 `factory:plan`）存在、`checks.sh` 可执行、`tools.json` 合法。失败就停，把它的输出原样告诉用户，不领任何单。
它报 `warn transcripts` 时不停，但告诉用户：`job doctor` 那一行 `inactive here:` 后面列出的机制（单张成本、必须用的工装、留出验收盲核对、规划盲核对）这次都核对不了。

## /factory run

把队列跑空：每张开着的 `factory` 单，同时最多 `concurrency` 张（`config.json`，默认 3）。也负责恢复。

开始时先给用户打印本次的完成条件，一句可检查的判断式：「`job queue` 里没有 `new`、`resume`、`stalled`，手上没有进行中的单」。
然后推进到它成立为止（pstack `autonomous-run`）：中途不向用户确认可逆的步骤，只在单子进 `needs-info` / `needs-human`
或完成条件成立时汇报；要人回答的问题攒成一次给。

1. 跑 `factory/bin/job queue`。每行用制表符分隔：`动作 issue 单号 阶段 说明`。
   新票正文有 `## 会改的文件` 时，和在途的单（或排在它前面的新票）有交集它就报 `wait` 并点名冲突的单；
   正文里有 `<!-- factory:blocked-by … -->` 而挡它的 issue 还开着，也是 `wait`。
   - `new`：还没有单子。有空位时 `factory/bin/job create <issue>`，它打印单号。它拒绝（比如已有别人开着的、
     关闭这张 issue 的 PR）时，转告它打印的内容，跳过这张。
   - `resume`：从它的阶段接着推进（见 **推进一张单**）。
   - `stalled`：单子在这个阶段太久没动静（`config.json` 的 `stall_minutes`），脚本已记一条 `stalled` 事件。
     说明里写了等谁的哪个文件。按「恢复时 agent 已经不在」处理：用全新上下文重派同一角色，prompt 照它原来的写
     （intake、spec-writer、builder、approver 各自的原 prompt；review 只补说明里点名的审查者，`job prompt` 重新取 HEAD）；
     说明写文件已写好、等编排者推进时，不派 agent，直接按那个阶段的推进步骤往下走。
     同一阶段再卡住一次，脚本自己把单子转 `needs-human`（`REASON: env`），那一行是 `wait`，不再派：
     照 `needs-human` 处理并汇报（intake/spec 阶段的单要删掉单子目录重来）。
   - `wait`、`skip`：不动，在总结里列出说明。
2. 每张进行中的单独立推进。所有 agent 都在后台派出，同一条消息里能派的一起派。
   Claude Code 默认每个会话最多 20 个并发子代理，3 张单各 4 个审查者也够。
3. 一张单到了 `merged`、`rejected`、`needs-info` 或 `needs-human` 就让出空位，再跑一次 `job queue` 补位。
4. 队列里没有 `new`、`resume` 和 `stalled`、手上也没有进行中的单时结束，依次跑：
   - `factory/bin/job watch`：在已合入、还没看过的提交上跑 `checks.sh`（每次最多 10 个）。合入提交红、合入前的基线（按合并方式找）绿，
     它就开一张贴 `factory` 和 `factory:escaped` 的 issue；把它打印的 `escaped → issue #<n>` 各行原样列给用户。
     这些新 issue 带 `factory` 标签，下一次 `/factory run` 才会领，这一次不要再跑 `job queue` 去领。
   - `factory/bin/job tune`：按指标写建议到 `factory/jobs/.tune.md`，打印条数。把条数告诉用户，请他去读。
     **不应用任何建议**：不改 `factory/config.json`、`factory/tools.json`、`.claude/agents/` 里的任何文件，也不派 agent 去改。
   - `factory/bin/job summary` 和 `factory/bin/job metrics`，把两张表给用户。

## /factory issue <n>

只跑这一张。没有单子就 `factory/bin/job create <n>`，有就从它的阶段接着推进。

## /factory <feature>

0. 先看值不值得进工厂。一两个文件、低风险的小改动（措辞、配置项、明确的一行修复）不进工厂：告诉用户直接改、开 PR 更快，
   用户仍要走工厂再继续。工厂一张单最少要接单、规格、构建、门禁、审查、批准六步，小改动的大部分时间花在流程上。
1. 用一句短标题概括这个请求。先跑 `factory/bin/job search "<标题>"`。
   它列出疑似重复的 issue 或 PR 时，**停下来**把列表给用户，问是用已有的、还是确实要新建。
2. 把用户的请求原文写进仓库外的临时文件（如 `mktemp` 得到的路径），再建 issue：
   `factory/bin/job new-issue "<标题>" <临时文件>`。它只走 REST、贴上 `factory` 标签，打印 `<编号> <链接>`。
3. 照 `/factory issue <新编号>` 跑。

## /factory rework <n>

单子必须在 `needs-human`。跑 `factory/bin/job rework <n>`：它把操作者在 PR 上的新评论写成 `rework-<k>.md`
并把单子转回 `build`；没有新评论时它会拒绝，转告用户先去 PR 上写意见。然后**新起**一个名为 `builder-<job-id>`
的 builder，prompt：`<头>` + `Rework: read rework-<k>.md and address it, then update build.md for round <N>. Port: <port>.`
（`N` 是 `job.json` 的当前 `round`）。之后照 **构建检查** 继续。
操作者要授权改 spec 没列的已有测试，就在评论里写一节 `## 允许修改的测试`（格式同 spec.md），`job rework` 会把其中的路径追加进 `job.json` 的 `allowed_tests`。

## /factory plan <目标>

规划层：把一个大目标拆成工厂票之前，先测量哪些决定是「谁来做都会这样定」的。AI 分不清什么时候不该定细节，
所以这里不让任何 agent 判断「该不该定」：几个互相看不见的规划者各出一份方案，**一致的作为默认，分歧的问人，
只有一方提出的默认丢掉，纯实现层面的不进规格**，最后由人拍板边界。你不判断任何决定，也不替人回答。

规划的 prompt 头三行（下面叫 `<规划头>`），一字不差：

```
Plan folder: <repo>/factory/plans/<plan-id>/
Plan: <plan-id>.
Role: <role>.
```

`<role>` 是 `planner-research`、`planner`、`planner-judge` 或 `planner-final`。盲规划的每个规划者，`Role: planner.` 之后
再加一行 `Planner: <k>.`（`<k>` 是它的序号）：脚本靠这一行知道哪份记录是第几个规划者，进 judging 时核对有没有谁读了别人的方案。
规划的 agent 不算在任何单子的成本里。

1. 起点：
   - `/factory plan <目标文字>`：先 `factory/bin/job search "<标题>"` 查重（同 `/factory <feature>`），再把用户的原话写进仓库外的
     临时文件，`factory/bin/job new-issue --plan "<标题>" <临时文件>`：它建一张贴 `factory:plan`（不是 `factory`）的 issue，
     打印 `<编号> <链接>`。
   - `/factory plan <n>`：用已有的 issue，它必须贴着 `factory:plan`。它的规划没做完就从那个阶段接着做（见下面；
     `factory/bin/job plan list` 列出每张规划 issue 该做什么：`new`、`resume`、`wait`、`skip`）。
   没有规划，或上一轮已经 `done`（T1 合入后规划下一轮：新一轮的 goal.md 带着上一轮写进 issue 正文的决定和拆票）：
   `factory/bin/job plan new <n>`，打印规划号。
2. **research**：派 `planner-research`，prompt `<规划头>` + `Research: read goal.md and the repo, write research.md.`。
   完成后 `factory/bin/job plan advance <plan-id> planning`。research.md 有条目没写 `来源:` 时它拒绝：把原话续接给同一个
   planner-research 补一次。成功时它打印 `planners 1=<模型> 2=<模型> …`（来自 `config.json` 的 `plan.planners`）。
3. **planning**：**在一条消息里**后台派出每个规划者，`subagent_type` 是 `planner`，`model` 是它打印的模型，
   prompt `<规划头>`（`Role: planner.`）+ `Planner: <k>.` + `Plan <k>: read only goal.md and research.md, write plan-<k>.md.`。
   每个都是全新的子代理，不续接、不互相转告，不把一个的输出给另一个看。**WAIT FOR ALL。**
   然后 `factory/bin/job plan advance <plan-id> judging --parent <你这个会话的模型>`。缺 `plan-<k>.md` 或格式不对时它拒绝：
   格式问题把原话续接给那个规划者改一次。它还用子代理记录核对「盲」：有规划者读了别人的 `plan-<j>.md` 就拒绝并点名谁读了谁，
   这时该方案作废，重新派那个规划者（全新上下文）重写，不要绕过。拿不到记录时放行，`plan.json` 记 `blind_verified: false`，
   `classified.md` 顶部写明，汇报时照说。它打印 `judge <模型> independent true|false`。
4. **judging**：派 `planner-judge`，`model` 是上一步打印的模型（它和你不是同一个模型家族；做不到时脚本已记 `independent: false`，
   在汇报里照说；默认配置下四个别名同属 `anthropic` 家族，所以默认就是 `false`，贴到 issue 的问题顶部会写「方案来自同一模型家族，
   一致不代表独立确认」，汇报里也照说），prompt `<规划头>` + `Judge: read every plan-<k>.md, write decisions.md.`。
   完成后 `factory/bin/job plan advance <plan-id> deciding`。它核对表是否把每份方案的每条决定都归进了一行，
   不对就拒绝：把原话续接给 planner-judge 改一次。通过后它按规则分类、写 `classified.md`，把**一条**评论
   （分歧逐条成问、每题带推荐答案，一致的默认和只有一方提出的清单）贴到规划 issue，贴 `factory:needs-info`，
   打印各类条数。把条数和 issue 链接告诉用户，**停下等人回复**。你不回答、不派 agent 去回答 `ask` 类问题。
5. **deciding**：`job plan list` 对这张报 `resume`（作者或操作者回复了）时：`factory/bin/job plan advance <plan-id> publishing`。
   脚本把回复引用进 `answers.md` 并解析每条 `M<n>: <答案>`。然后**新起**一个 `planner`（不是 planner-judge，也不续接盲规划时的任何一个），
   prompt `<规划头>`（`Role: planner-final.`）+ `Final: read classified.md, answers.md and the plans' 拆票, write final.md.`。
6. **publishing**：`factory/bin/job plan publish <plan-id>`。它核对 `final.md` 的每个决定都来自 `default` 或人的回复、
   没回答的分歧只写在 `## 未定`；不符就拒绝：把原话续接给定稿的 planner 改一次，还不对就停下告诉用户。
   通过后它用 REST 把定稿的决定写进规划 issue 的正文、按拆票建子 issue（贴 `factory`，正文有 `## 实现决定`、`## 判定命令`、
   `## 会改的文件`）、加原生 blocked-by，打印一行。重跑不会重复建票。它打印 `blocked-by text` 时告诉用户：依赖写成了正文里的
   `依赖: #n`，`job doctor` 会报。
7. 完成后把子 issue 列给用户：它们贴着 `factory`，`/factory run` 会领；被挡着的票在 `job queue` 里是 `wait`。
   **不关闭规划 issue。**

## 推进一张单

单子在 `build`、`gate`、`review`、`approve` 时，每次 agent 回来后跑 `factory/bin/job drive <job-id>`，照它的输出做：

- `DISPATCH <角色> <模型>` 加下面几行提示词（可能有几块，每块一个 agent）：在一条消息里全部后台派出，提示词原样用。
  `builder` 那块是续接（第四行是门禁退回或审查发现的续接句）：`SendMessage` 给 `builder-<job-id>`，内容是第四行；
  它不在了就用同样的名字新起，prompt 用整块。
- `WAIT <阶段> <角色…>`：已派出的这些 agent 还没写完自己的文件。什么都不做，等它们回来再跑一次 `job drive`。
- `STOP <阶段> <原因>`：脚本做不了的那一步，或某一步被拒（原因是拒绝的原话）。到下面对应阶段，照那里写的处理。
- `ERROR <命令> <输出>`（退出 1）：某一步没能跑起来。停下，把这一行告诉用户。

它只做没有判断的那几步（`advance`、`gate`、`evidence`、`sync`、`merge`），步骤和下面各阶段写的一样；
其他阶段、以及它 `STOP` 下来的情况，照下面手工做。

每一步都先读 `job.json` 的 `stage`，按阶段做下面的事。每个脚本命令只打印一行；拒绝时退出非零并说明原因。
退回、挂起和升级都要一个原因：`intake.md`（NEEDS-INFO、REJECT）、`spec.md`（NEEDS-DECISION）、`build.md`（BLOCKED）、
审查（CHANGES）、`decision.md`（ESCALATE）的文件头要有 `REASON: <值>`，取值是 `config.json` 的 `reasons`。
脚本因缺它拒绝时，把原话续接给写那个文件的 agent 补一次；还被拒就停下告诉用户。脚本自己写的原因（门禁、合并）不用管。

### intake

`intake.md` 不存在：派 `intake`，prompt 是 `factory/bin/job prompt <job-id> intake` 的输出。
完成后读 `intake.md` 第一行：

- `DECISION: ACCEPT`：`factory/bin/route <job-id>`，再 `factory/bin/job advance <job-id> spec`
  （脚本把结论贴到 issue，并从 `origin/<base>` 建工作树）。
- `DECISION: NEEDS-INFO`：`factory/bin/job advance <job-id> needs-info`（贴问题、加标签，单子挂起）。
- `DECISION: REJECT`：`factory/bin/job advance <job-id> rejected`。

### needs-info

`job queue` 报 `resume`（作者或操作者回复了）时：`factory/bin/job advance <job-id> intake`。
脚本归档旧的 `intake.md`、按新评论重写 `issue.md`，然后照 **intake** 重走。

### spec

1. 工作树是新的：在工作树里按 AGENTS.md 做全新检出的初始化（装依赖等）。失败就停，告诉用户。
2. `spec.md` 不存在，或第一行是 `STATUS: NEEDS-DECISION`（重走 intake 之后）：派 `spec-writer`，
   prompt 是 `factory/bin/job prompt <job-id> spec-writer` 的输出。
3. 读 `spec.md` 第一行：
   - `STATUS: NEEDS-DECISION`：`factory/bin/job advance <job-id> needs-info`（问题贴到 issue，单子挂起）。
   - 否则 `factory/bin/job advance <job-id> build`。脚本在缺 `## 实现决定`（第一条 `来源:`）或 `## 判定命令` 时拒绝：
     用它的原话续接同一个 spec-writer 改一次；还被拒就停下告诉用户。`holdout/manifest.json` 不对、`config.json` 的
     `holdout_required` 为 `true` 却没有 `holdout/`、`## 稳定性` 缺键或 `runs` 超过 `satisfaction.max_runs` 时它也拒绝，同样处理。
     通过时脚本把 `holdout/` 每个文件的 sha256、`## 允许修改的测试`、`## 稳定性` 记进 `job.json`。
   - 单子是从 gate 退回来的（门禁 `HOLDOUT: WEAK`，见 **gate**）：派 `spec-writer`，prompt `<头>` +
     `Spec: the holdout passes on the base too (round-<N>/checks.md). Strengthen holdout/ so it fails on the base.`，
     完成后照上面 `factory/bin/job advance <job-id> build`。轮次不变；再 `SendMessage` 给 `builder-<job-id>`：
     `The spec was revised (spec.md); the round stays <N>. Check your work against it, commit and push if anything changes, and update build.md.`
4. 进入 `build` 后派 builder，名字 **`builder-<job-id>`**，prompt 是 `factory/bin/job prompt <job-id> builder` 的输出（从 gate 退回来的单不新派，照上面续接）。

### build

builder 完成后（第一次、续接或返工都一样），做 **构建检查**：读 `build.md` 第一行。

- `STATUS: READY`：`factory/bin/job advance <job-id> gate`。脚本核对工作树干净、分支有提交且已推送、
  `tools.json` 里 builder 必须用的工装用过且证据在，第一次还会开 draft PR，并回收 builder 用 `job res` 登记的进程。
  拒绝时（没推送、有未提交改动、`required tools missing`）把原话用 `SendMessage` 发给 `builder-<job-id>` 让它修，再检查一次。
  它在 stderr 打 `warning: … required tools weren't checked` 时照常继续，在汇报里带上这一句。
  单子有留出验收时，脚本还从 transcript 核对 builder 没碰过单子目录的 `holdout/`、`holdout-logs/`：拒绝
  （`the builder touched the holdout acceptance`）时**不要**把原话转给 builder，留出验收已经不再保密：停下告诉用户，由人决定
  （让 spec-writer 重写一份留出验收，或者不要它）。拿不到 transcript 时它放行，stderr 打
  `warning: … holdout blindness not verified`，`job.json` 记 `holdout_blind_verified: false`，在汇报里带上。
- `STATUS: BLOCKED`：`factory/bin/job advance <job-id> needs-human`。`build.md` 第二行缺 `REASON:` 时脚本拒绝，
  把原话发给 builder 补上。`REASON: spec-conflict` 是 builder 发现测试或验收与规格矛盾，告诉用户矛盾在哪。

恢复时 builder 已经不在（会话重启过，`SendMessage` 找不到 `builder-<job-id>`）：用同样的名字新起一个，
prompt `<头>` + `Build round <N>: read build.md and the round folders, then continue. Port: <port>.`。

### gate

跑 `factory/bin/gate <job-id>`，然后读 `round-<N>/checks.md` 第一行：

- `GATE: PASS`：先跑 `factory/bin/evidence <job-id>`，再 `factory/bin/job advance <job-id> review`，然后派审查者（见 **review**）。
  `evidence` 只对 `SURFACES` 含 `ui` 的单采证据：起 dev server、按 spec 的 `## 验收路径` 逐条截图，写
  `round-<N>/evidence/index.md`，用完按 pid 停掉服务。别的单、或 `config.json` 没配 `dev_server.cmd` 和 `ft.shot` 时它说跳过，
  不写文件，`advance` 照常放行（没配时 stderr 打一行警告、事件里记一条 `warning`，在汇报里带上）。
  它退出非零（截图失败或 dev server 没起来）时 `advance` 会拒绝（`evidence.allow_failed` 放行的除外）：把它那一行原样告诉用户，这张单停在 gate。
- `GATE: FAIL` 且文件头有 `HOLDOUT: WEAK`（`REASON: test-weak`：spec-writer 写的留出验收在基线上也通过）：
  `factory/bin/job advance <job-id> spec`，照 **spec** 里「从 gate 退回来的」做。不派回 builder，不占轮次。
- `GATE: FAIL`（含 `REASON: test-weak`：builder 的新测试在基线上就通过；`REASON: flaky`：`## 稳定性` 的命令在 HEAD 上
  通过次数不够；`HOLDOUT: FAIL`：留出验收在 HEAD 上失败，`checks.md` 里只有用例名和断言摘要）：
  `factory/bin/job advance <job-id> build`。脚本说这是最后一轮时，改用
  `factory/bin/job advance <job-id> needs-human`。转回 build 后轮次加一，`SendMessage` 给 `builder-<job-id>`：
  `The gate failed in round <N>: read round-<N>/checks.md and checks.log, fix it, commit, push, and update build.md for round <N+1>.`
  **门禁没过时不派任何审查者。**
- `GATE: BASE-RED`：基线本身就红，不是本单的错。`factory/bin/job advance <job-id> needs-human`，
  不派回 builder。告诉用户基线哪里红（`checks.md` 里有基线的日志）。

### review

1. 把 `job.json` 里 `agents.reviewers` 的每个审查者**在一条消息里**全部后台派出，
   `subagent_type` 是 `reviewer-<k>`，`model` 是记录的模型，prompt 是 `factory/bin/job prompt <job-id> reviewer-<k>` 的输出
   （它带 HEAD，`ux`、`ui` 还带端口）。
2. **WAIT FOR ALL。** 每个审查者完成时跑 `factory/bin/job sync <job-id>`，让看板的标记随结论落下。
   `VERDICT: CHANGES` 的审查，`sync` 会在工作树里按审查时的 HEAD 重跑每条发现的证明命令（每条最多 300 秒，占门禁名额），
   结果写进 `round-<N>/proof/<k>/confirm.json`，`sync` 那一行里记 `CHANGES`（至少一条 confirmed）或 `unproven`
   （一条都没有，按 PASS 计）。同一份审查文件只重跑一次。
   一轮只有在所有路由到的 `review-<k>.md` 都存在后才算完。**轮次中途绝不续接 builder。**
   审查者没写文件就结束、或第一行不是结论（`job sync` 显示 `invalid`）：停下告诉用户。
   `invalid` 也可能是 `VERDICT: CHANGES` 缺第三行 `REASON:`、取值不在 `config.json` 的 `reasons` 里或是 `style`，
   缺 `## 发现` 或某条发现没有证明，或者证明命令跑完改了工作树：`advance` 会说明，把它的原话用 `SendMessage`
   发给那个审查者改一次（改了工作树的，由它把工作树恢复原样、改好证明），还不对再停下。
   工作树没恢复前，所有审查的证明都不会跑，单子不流转。
3. 判：
   - 离开 review（到 `approve` 或 `build`）时，脚本核对本轮每个审查者在 `tools.json` 里必须用的工装；
     拒绝（`required tools missing in review: reviewer-<k> …`）时把原话 `SendMessage` 给点名的审查者，
     让它照 `run` 用过、按需改写结论后再判。
   - 离开 review 时脚本还核对只读：工作树干净、HEAD 还是派出时记的、单子目录里没有审查者自己文件以外的新增、改动或删除。
     拒绝（`read-only check failed in review …`，点名每个路径或 sha）时把原话 `SendMessage` 给点名的审查者，
     让它恢复原样后再判；HEAD 被别人改了（不是审查者动的）的转 `needs-human`。
   - 全部 `PASS` 或 `unproven`：`factory/bin/job advance <job-id> approve`。
   - 有 `CHANGES`：`factory/bin/job advance <job-id> build`；脚本说轮次用完时改用 `needs-human`。
     脚本把本轮 confirmed 的发现写进 `round-<N>/findings.md`；转告 builder 的只有它，不转 unconfirmed 的发现和建议。
     转回 build 后 `SendMessage` 给 `builder-<job-id>`：
     `Round <N> confirmed findings are in round-<N>/findings.md. Fix each one, commit, push, and update build.md for round <N+1> with what each finding's proof command gives now.`
   - 有 `stale`（审查的 `COMMIT` 不是当前 HEAD）：先 `git -C <worktree> rev-parse HEAD` 对照 `checks.md` 的 `COMMIT`。
     相同，说明是审查者写错或审错了提交：只重派那个审查者，带当前 HEAD。不同，说明门禁之后分支被别人改了：
     不要凑结论，`factory/bin/job advance <job-id> needs-human`，把 `job sync` 的输出和两个 sha 告诉用户。

### approve

派 `approver`，prompt 是 `factory/bin/job prompt <job-id> approver` 的输出。完成后读 `decision.md` 第一行：

- `APPROVE`：`factory/bin/job merge <job-id>`。它要求 `decision.md` 的 `COMMIT` 等于 HEAD，同步基线、再过门禁，
  按刚推送并通过门禁的头合并；改到 `human_review_paths`、有冲突、门禁不过、或合并时头变了，它会转 needs-human 并写明原因。
  照它打印的那一行告诉用户。
- 离开 approve（`merge` 或 `advance needs-human`）时脚本核对只读：工作树干净、HEAD 还是进 approve 时记的、
  单子目录里除 `decision.md` 外没有被动过。拒绝（`read-only check failed in approve …`）时把原话 `SendMessage` 给
  approver，让它恢复原样后再判；HEAD 被别人改了的转 `needs-human`。
- `ESCALATE`：`factory/bin/job advance <job-id> needs-human`。`decision.md` 第三行缺 `REASON:` 时脚本拒绝，
  把原话发给 approver 补上。

### merging

恢复到这个阶段（合并做到一半会话断了）：再跑一次 `factory/bin/job merge <job-id>`。

### needs-human、merged、rejected

不动。`needs-human` 的单由人合并 PR（`job queue` 会发现并记为 merged），或在 PR 上写意见后 `/factory rework <n>`。

例外：因 `GATE: BASE-RED` 进 `needs-human` 的单，用户说基线已修好时，`factory/bin/job advance <job-id> gate`
再照 **gate** 重跑门禁；脚本只允许这类单这样回去。

## 分工（照 Anthropic AI Fluency 的 4D）

- **委派**：人只定只有人知道的：他自己的情况（数字、账户、偏好、能承受什么）、取舍（先后、哪条线不碰）、不可逆的事。
  有成熟做法的「怎么设计」由 agent 带出处定成默认，不推回给人。
- **描述**：issue 写目标和约束就够了，不用写解法；spec-writer 从成熟做法和 `decision_docs` 推出解法。
- **判断**：门禁、跑得出来的证明、留出验收、作者以外的审查，都由脚本核对，不信 agent 的自述。
- **尽责**：定过的决定在 `decision_docs` 里，agent 先查再问；人回答过的项目级决定随那张单的 PR 记进去，不问第二遍。

## 边界

- 只碰贴了 `factory` 标签的开着的 issue。打标签需要 triage 或写权限，标签就是信任边界。
- 只推 `factory/*` 分支，只有 `factory/bin/job merge` 合并，从不部署。你自己不跑 `git push`、`gh pr merge`
  或任何 GraphQL 的 `gh` 子命令（`gh issue`、`gh pr`、`gh repo`）；需要 GitHub 的事交给脚本。
- 同会话子代理的审查不算独立审查：命中 `human_review_paths` 的单，脚本不会合并，交给人。
- 每单最多 `max_rounds` 轮构建，用完交给人。
- 规划里的 `ask` 类决定只由人回答：你和任何 agent 都不替人答，也不关闭规划 issue。
- 控制律只由人改：`job tune` 的建议、`job lessons` 的草稿你都只转告，不照着改配置、工装清单或提示词；
  `factory/bin/job lessons --open` 只由操作者自己运行，你不跑。

## 汇报

流转只记在 `events.jsonl` 里，不逐条报给用户。单子进 `needs-info`、`needs-human` 时报一行，带原因（各文件头的 `REASON:`），
例如 `004-dark-mode-toggle: round 2, code CHANGES (test-missing) → needs-human (max_rounds)`；同一时刻有几张单要人回答，合成一条。
`/factory run` 结束时列出合入的单（附 PR 链接），给出 `job watch` 新开的漏出 issue、
`job tune` 的建议条数（说明都没有应用），以及 `job summary` 的表格（每张单的阶段、PR、成本、耗时）和 `job metrics` 的表
（周期、轮次、升级率、漏出率、原因分布、工装使用率、审查发现的确认率）。
成本是按 API 价格折算的 agent 花费，订阅计划下看相对开销，不是账单。
`/factory run` 结束时，把 `job doctor` 那一行 `inactive here:` 后面列出的失效机制原样告诉用户。
`/factory run` 结束时，把 `job doctor` 输出里 `agentskills lock` 那一行原样告诉用户（没有这一行时不报）。
