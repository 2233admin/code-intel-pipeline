# Non-goal: a CI-consolidation PR for `code-intel verify`

## Non-goal

Opening a PR that replaces separately-run `lint hardcoded-paths` / `sentrux gate` /
`repin --repo` steps in this repo's GitHub Actions workflows with a single
`code-intel verify . --json` step, on the premise that `verify` (#367/#368) was
introduced to deduplicate existing CI steps. No such duplication exists in this
repository today, so there is nothing to consolidate.

**Investigated:** 2026-08-27, scope `.github/workflows/*.yml` on `origin/main`
(post-#370 merge, commit `9c7ed00b`). No branch or PR was opened; this is the
recorded negative result.

## Why

Read every workflow file (`ci.yml`, `release.yml`, `skill-check.yml`,
`parity-observe.yml`, `pr-gate.yml`) and grepped the whole repo for the three
commands `verify` aggregates. None of the five workflows run `lint
hardcoded-paths`, `sentrux gate` (or `sentrux --operation gate`), and `repin
--repo` (check-only) as separate steps in the same job — the precondition the
idea was framed around does not hold:

| Workflow / job | Verdict | Reason |
| --- | --- | --- |
| `ci.yml` / `cross-platform-smoke` (`Hardcoded path scan`, lines 843-847) | Not a candidate | Only `lint hardcoded-paths` is present as a lone step; no sibling `sentrux gate` or `repin --repo` step to consolidate with. |
| `ci.yml` / `windows-build-test-package` | Not a candidate | None of the three checks present. |
| `release.yml` (both jobs) | Not a candidate | None of the three checks present as standalone steps. The "Sentrux"-flavored steps there validate full pipeline-run artifacts, not a bare gate. |
| `pr-gate.yml` / `sentrux-capability-gate` | Not a candidate (concrete reason) | Runs the full `code-intel run execute` DAG and validates the committed Run Commit artifact-index / capability-matrix closure against `orchestration/sentrux-capability-matrix.v1.json`. `verify`'s `sentrux gate` sub-check is a single `sentrux_gate::run_gate(repo, false)` call with no run manifest, no artifact index, no capability matrix — it cannot produce the evidence this job asserts on. Swapping the job body for `verify` would silently drop real coverage. |
| `pr-gate.yml` / `change-risk`, `agent-gate` | Not applicable | Unrelated checks (risk scoring, agent-branch label gate). |
| `parity-observe.yml` | Not a candidate | None of the three checks present as workflow steps; only a comment describing what the legacy PS1 parity harness measures internally. |
| `skill-check.yml` | Not applicable | Unrelated (SKILL.md scoring/link-check/frontmatter validation). |

Root cause of the false premise: issue #367 (closed, `verify` merged) scoped the
work purely to adding the CLI command — composing three already-existing,
already-check-only library entry points (`hardcoded_paths::scan`,
`sentrux_gate::run_gate(.., false)`, `repin::run_check`) into one binary-level
verdict for a human or agent to run ad hoc. Neither #367 nor PR #370 proposed
wiring it into CI. Separately, this repo already gates Sentrux structural
health in CI through a much heavier, evidence-chain-verified path
(`sentrux-capability-gate`), and gates hardcoded paths through one lone lint
step with no siblings. The three checks `verify` composes were designed as
pre-push/pre-edit developer tooling (see `AGENTS.md`: "Run `code-intel lint
hardcoded-paths` before pushing" / "Use the compiled Rust gate `code-intel
sentrux gate` before editing"), not three separately-run CI steps that
happened to duplicate work.

## Instead

- If there is separate appetite for *adding* `code-intel verify` to CI as new
  coverage — e.g. so `cross-platform-smoke` also gates on
  `.sentrux/baseline.json` regressions and stale pins on all three OSes, not
  just hardcoded paths — that is a distinct, scope-expanding proposal
  (net-new behavior, not a "replace N duplicate steps with one"
  simplification) and needs its own cost/benefit discussion before landing.
- `pr-gate.yml`'s `sentrux-capability-gate` should not be replaced by
  `verify` under any framing that keeps today's coverage: it validates
  artifact-index and capability-matrix closure that a bare `verify` call
  cannot produce.
- Re-run this same file-by-file check if new workflow steps are added that
  independently call `lint hardcoded-paths`, `sentrux gate`, and `repin
  --repo` in the same job — that is the actual precondition for a
  consolidation PR to be worth opening.
