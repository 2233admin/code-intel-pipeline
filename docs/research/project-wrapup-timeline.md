# When can code-intel-pipeline "收口" (wrap up)?

Investigation date: 2026-08-21. All data pulled live from primary sources listed inline;
counts are reproducible by re-running the cited command/URL.

## 0. Which tracker is actually the SSOT? (verified, not assumed)

The repo has two remotes:

- `gitea` → `https://git.xart.top:8418/Curry/code-intel-pipeline.git`
- `origin` → `https://github.com/2233admin/code-intel-pipeline.git`

Global convention says "delivery SSOT = the git host that owns the code," and Gitea is the
user's default self-hosted host. **That default does not hold for this repo.** Checked both,
authenticated (Gitea API requires auth even for anonymous browsing — used a credential already
cached by Git Credential Manager for `git.xart.top:8418`):

- Gitea: `GET https://git.xart.top:8418/api/v1/repos/Curry/code-intel-pipeline` →
  `open_issues_count: 1`, `updated_at: 2026-08-19T22:25:13+08:00`. `state=all` on the issues
  endpoint returns exactly **one issue ever, #2**, opened 2026-08-19, still open, zero closed
  issues, zero milestones. This is not an active tracker — it looks like a single test/handoff
  issue.
- GitHub: `gh api repos/2233admin/code-intel-pipeline` → `open_issues_count: 85` (issues + PRs),
  `pushed_at: 2026-08-21T05:36:32Z` (same day as this investigation). `gh api
  repos/2233admin/code-intel-pipeline/issues --paginate` (excluding PRs) → **82 open, 58 closed,
  140 total**, spanning issue numbers up to #313+.

**Conclusion: GitHub (`origin`, https://github.com/2233admin/code-intel-pipeline) is the live
issue tracker for this repo in practice, not Gitea.** All numbers below use GitHub. This is a
deviation from the stated global SSOT convention worth flagging back to the user — either the
convention should be corrected for this repo, or issue tracking should be migrated to Gitea to
match it. Not resolved here; just verified and reported.

## 1. Stated definition of "done" / scope boundary

No single canonical "definition of done" doc exists (no `ROADMAP.md`, no v1.0 milestone). The
closest primary sources:

- **Issue #55 — "[map] Evidence Kernel 北极星 + PS1 巨石退役战役"**
  (https://github.com/2233admin/code-intel-pipeline/issues/55, opened 2026-07-26). This is the
  literal "north star" (北极星) issue. It states the project's positioning ("agentic engineering's
  evidence kernel," not "another code scanner") on four hard boundaries LLMs structurally can't
  cover (determinism/replay, cryptographic attestation, whole-repo fact store vs. context window,
  adversarial-robust gates), and defines a three-plane architecture (fact / policy / judgment
  plane + a provenance spine). It carries an explicit **"顶层验收" (top-level acceptance) checklist**
  for the PS1-retirement campaign specifically:
  - `run-code-intel.ps1` and `Invoke-SentruxAgentTool.ps1` deleted or ≤50-line shims
  - `rules.toml` tightened at least two notches toward `B/70/0`, with manifest evidence
  - ubuntu/macos CI runners green on full self-scan
  - no-change warm self-scan wall-clock time down ≥60%
  - every PR: contract parity green + self-scan green, never green-by-loosening-baseline

  It also has an explicit **"不做" (not doing) list**: no big-bang PS1 deletion, no MoonBit in the
  production path, no new daemon/production dependency without quantified proof, no fact-plane v2
  (datalog engine, distributed cache), and v0.6.0 session trace / v0.7.0 viewer explicitly deferred
  to issue #14.

- **Issue #14 — "roadmap: enforce self-dogfood release gates and add session intelligence"**
  (https://github.com/2233admin/code-intel-pipeline/issues/14, opened 2026-07-24, still open).
  Defines version-tagged delivery slices: v0.5.1 (self-dogfood release integrity), v0.6.0 (session
  trace/behavior intelligence), v0.7.0 (minimum viewer/comparison). A 2026-07-27 comment on this
  issue explicitly **defers** "enforce self-dogfood release gates" full closure past v0.7.0 GA:
  > "明确延后决定（不是关闭）：不作为 v0.7.0 GA 的阻塞项 ... 继续在本 issue 追踪，post-GA 排期。"

- **CHANGELOG.md `[0.7.0-beta.3]` entry** (repo path `CHANGELOG.md`, line ~164) names a
  measurable **quality gate, "北极星闸门 G0"**: an eval benchmark (issues #107/#109) requiring the
  artifact-guided answer arm to solve ≥3/4 "why"-class questions at ≤2× the byte cost of a naive
  read. The most recent measured result (line ~156, tied to issue #142) is **1/4 "why" questions,
  11.5× the byte budget** — nowhere near the stated bar. No later CHANGELOG entry reports this gate
  passing.

- `Cargo.toml` / `crates/code-intel-cli/Cargo.toml` current version: **`0.7.2-beta.6`**. No
  `v1.0`, "GA", "final", or "feature freeze" string appears anywhere in `README.md`, `CHANGELOG.md`,
  `AGENTS.md`, or `CONTEXT.md` (checked via grep). The project has never declared a version that
  means "done."

**Bottom line on scope**: there is a real, falsifiable north star (issue #55) and a real,
falsifiable quality gate (G0, currently failing by a wide margin), but no document states a
version or date that constitutes project completion. "Done" is currently defined only at the
sub-campaign level (PS1 retirement acceptance checklist), not at the project level.

## 2. Open issue inventory (GitHub, `2233admin/code-intel-pipeline`, pulled 2026-08-21)

- **82 open issues**, **0 have a GitHub milestone attached** (`gh api
  repos/2233admin/code-intel-pipeline/milestones` returns an empty list — no milestones exist in
  the repo at all).
- Label breakdown on the 82 open issues: `enhancement` 71, `backlog` 65, `bug` 24,
  `wayfinder:*` 7 (grilling 4, map 2, prototype 1), `claimed` 3, `documentation` 2,
  `ready-for-agent` 1.
- **10 oldest open issues**, all from the north-star campaign, opened 2026-07-24 to 2026-07-28:
  #14 (roadmap), #47–#53 (ps1-exit T2–T8), #55 (north star itself), #58 (write-path gap). Full
  list: https://github.com/2233admin/code-intel-pipeline/issues?q=is%3Aissue+is%3Aopen+sort%3Acreated-asc
- **Largest open issues by body length** (proxy for scope): #14 (7504 chars, roadmap), #265
  (6411, native SymbolView/RelationView), #58 (5330, write-path gap), #78 (4194, CI PS1 removal —
  45 call sites), #264 (3737, multi-view query index).
- **North-star campaign sub-ticket status** (#46–#54, the T1–T9 series under issue #55):
  - #46 (T1, contract freeze) — **closed** 2026-07-27
  - #47 (T2, launcher), #48 (T3, doctor), #49 (T4, model channel), #50 (T5, PS1 wrapper
    decomposition), #51 (T6, fact plane v1), #52 (T7, test asset migration), #53 (T8, retirement
    gate + ratchet) — **all still open**, none have a `closedAt` timestamp, i.e. **zero progress
    since #46 closed on 2026-07-27** (25 days as of 2026-08-21).
  - #54 (T9, MCP query surface — explicitly marked "二期，不阻塞主线" / phase-2, non-blocking in
    #55's own text) — **closed** 2026-08-19, ahead of the T2–T8 items it was declared not to block.

## 3. Velocity (last 8-10 weeks, commits on `origin/main`, weeks are Monday-start ISO weeks)

`git log origin/main --since="10 weeks ago" --pretty=format:"%ad" --date=iso-strict`, grouped:

| week starting | commits |
|---|---|
| 2026-06-22 | 4 |
| 2026-06-29 | 26 |
| 2026-07-06 | 5 |
| 2026-07-13 | 11 |
| 2026-07-20 | 54 |
| 2026-07-27 | 47 |
| 2026-08-03 | 56 |
| 2026-08-10 | 5 |
| 2026-08-17 | 21 (partial — through 2026-08-21, a Friday) |

Total commits on `origin/main`: 270 (`git rev-list --count HEAD` from a checkout tracking main).
Last commit: `2026-08-21T03:29:19+08:00`, `feat(perf): invoke weco for a denoised, budget-bounded
optimization pass (#313)` — same day as this investigation. **No slowdown trend**: commit velocity
is sustained at a high rate (20-56/week) with only one low week (08-10, a plausible short break),
right up to today.

Issue open/close by week (created_at / closed_at from the full GitHub issue set, 140 issues total,
82 open + 58 closed):

| week starting | opened | closed | net | cumulative net backlog growth |
|---|---|---|---|---|
| 2026-07-20 | 23 | 9 | +14 | 14 |
| 2026-07-27 | 30 | 10 | +20 | 34 |
| 2026-08-03 | 56 | 18 | +38 | 72 |
| 2026-08-10 | 17 | 3 | +14 | 86 |
| 2026-08-17 | 14 | 18 | -4 | 82 (partial week, through 08-21) |

Open PRs right now: 3 (`gh pr list --state open`) — #314, #298, #296 — well under the DR-0005
"5 open fix PRs" threshold that would force queue-draining over new features (`AGENTS.md` lines
12-14).

## 4. Explicit milestone/version target mapping to "done"

**None exists.** `gh api repos/2233admin/code-intel-pipeline/milestones` returns an empty array —
no milestones have ever been created on this repo. No issue is labeled or titled as "final cleanup"
or "last item before freeze." The closest thing to a completion signal is the sub-campaign
acceptance checklist in issue #55 (section 1 above), which is scoped to the PS1-retirement
campaign only, not the whole project, and is 25 days stalled at 2/9 sub-tickets closed.

## 5. Scope creep vs. convergence signal

Mixed, tilting toward creep as of this snapshot:

- **Backlog grew net +86 issues** over the 2026-07-20 → 2026-08-17 span before the first (and
  only, so far) net-negative week (-4, and that week is still in progress).
- **Recently closed issues (last ~48h, 2026-08-19 to 2026-08-21)** are dominated by two *new*
  initiatives, not the declared north-star backlog: a "bounds" (run-budget / timeout
  classification) feature cluster (#304, #305, #306, #307, #157) and a "weco" perf-optimization
  loop (#300, #301) tracked separately under issue #299 (opened 2026-08-19,
  "feat(perf): benchmark 驱动的迭代性能优化闭环，对标 weco-skill"). Neither of these was named in
  issue #55's or #14's original scope.
- **Issue #267 — "Wayfinder: Open-source skill-aware Agent fleet control plane"**
  (https://github.com/2233admin/code-intel-pipeline/issues/267, opened 2026-08-15) proposes a
  materially new, large-scope deliverable — a multi-agent fleet controller — inside this repo's
  tracker, currently at the spec/"map" stage (labels `wayfinder:map`, plus 3 sibling issues tagged
  `wayfinder:grilling`/`wayfinder:prototype`). This is scope broader than "Evidence Kernel for one
  repo" as framed in #55.
- Meanwhile the actually-declared north-star sub-tickets (T2-T8, #47-#53) have had **zero closes
  in 25 days** while newer, un-scoped-by-#55 work ships daily.
- The one measurable project-level quality bar that does exist — G0 (section 1) — is still failing
  by roughly 4x (1/4 vs required ≥3/4 "why" answers) and 5.75x (11.5x vs required ≤2x byte cost),
  with no CHANGELOG entry since showing a re-measurement or improvement.

## 6. Realistic wrap-up estimate

**No firm date is supportable from the data, and a range would be manufactured, not grounded.**
Reasons, all traceable to the sections above:

1. **No project-level "done" exists to count down to.** The only board with an explicit
   acceptance checklist (issue #55) is a sub-campaign (PS1 retirement), not the whole project, and
   even that checklist has no target date — only ordering rules ("T1 先行...T8 收口").
2. **No version or milestone marks completion.** Zero milestones on the repo; `Cargo.toml` is at
   `0.7.2-beta.<n>` with beta increments issued almost daily and no v1.0/GA signal anywhere in
   docs.
3. **Backlog is still net-growing** (+86 over 4 weeks) with only one partial week of net shrink —
   one data point is not a trend.
4. **The team's own declared "first war" (PS1 retirement, #55) has stalled** (2/9 sub-tickets
   done, 25 days no movement) while attention visibly shifted to at least two un-scoped new
   initiatives (perf/weco loop, Wayfinder fleet controller) — this is the opposite of the
   convergence you'd need to project a close date.
5. **The one hard, falsifiable quality gate that exists (G0)** is failing by ~4-6x against its own
   bar, with no recent remeasurement showing progress toward it.

If forced to name what would have to be true before a date could even be estimated: (a) a
milestone or explicit version target gets created and populated with the remaining scope, (b) the
issue net-close trend holds net-negative for at least 3-4 consecutive weeks (not one partial week),
and (c) the G0 gate gets re-measured and shows a trend line toward ≥3/4 / ≤2x. None of those three
preconditions are currently met.

## Sources cited

- Gitea API: `https://git.xart.top:8418/api/v1/repos/Curry/code-intel-pipeline` and
  `.../issues?state=all` (authenticated via Git Credential Manager-cached token for
  `git.xart.top:8418`, queried 2026-08-21)
- GitHub API/CLI: `gh api repos/2233admin/code-intel-pipeline`,
  `gh api repos/2233admin/code-intel-pipeline/issues --paginate`,
  `gh api repos/2233admin/code-intel-pipeline/issues?state=closed --paginate`,
  `gh api repos/2233admin/code-intel-pipeline/milestones`,
  `gh pr list --repo 2233admin/code-intel-pipeline --state open` (queried 2026-08-21)
- Issues: #14, #46, #47, #48, #49, #50, #51, #52, #53, #54, #55, #58, #78, #142, #148, #157, #191,
  #260, #264, #265, #267, #297, #299, #300, #301, #304, #305, #306, #307
  (`https://github.com/2233admin/code-intel-pipeline/issues/<n>`)
- `CHANGELOG.md` (repo root), `[0.7.0-beta.3]` section, G0 gate definition and #142 negative
  result
- `Cargo.toml`, `crates/code-intel-cli/Cargo.toml` — current version string
- `AGENTS.md` (repo root) — DR-0005 open-PR threshold (lines 12-14)
- `CONTEXT.md` (repo root) — domain vocabulary, no stated completion criterion found
- `git log origin/main --since="10 weeks ago"` — commit velocity (local checkout,
  `issue-296-omp` worktree, queried 2026-08-21)
