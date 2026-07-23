# Multi-Agent Merge Queue idea
> Status: ACCEPTED FOR LOCAL IMPLEMENTATION
> Created: 2026-07-15
> Source: 2233admin/claude-code-merge-queue@e7a76958dbd3953b84f12abbc2e6bd755aafce53

## Abstract
Add an optional, fail-closed landing adapter for repositories developed by several local agents in
isolated worktrees. The adapter admits a lane to the external FIFO merge queue only when the queue
is installed locally, a real acceptance command is configured, direct pushes are protected, and
production promotion remains a human action.

## Core Insight
Parallel implementation and serialized integration are different responsibilities. Code Intel and
Pon-derived project conformance decide whether work is acceptable; the merge queue decides when one
accepted lane may rebase, check, and push without racing other lanes.

## Target Repo
- Path: `D:\projects\_tools\code-intel-pipeline`
- Branch: current working branch
- Current state: dirty worktree with pre-existing user changes; add only isolated adapter, policy,
  documentation, registry, record, and tests.

## Success Criteria
- [ ] Orchestration registers an optional landing-coordination stage and adapter.
- [ ] Status is machine-readable and never installs or initializes an external tool.
- [ ] `land` fails closed without local CLI, config, acceptance command, active pre-push hook, or a
  distinct human promotion branch.
- [ ] A ready fixture delegates exactly one `land` invocation to the configured queue command.
- [ ] `promote`, emergency bypass, worktree deletion, and dependency installation remain outside
  the adapter.
- [ ] Project conformance can be used as the upstream queue's `checkCommand` without coupling the
  adapter to Python or Pon.
- [ ] Doctor, orchestration validation, targeted tests, and Sentrux session verification pass.

## Constraints
- Do not add dependencies or vendor upstream code.
- Do not run upstream `init`, `uninstall`, `promote`, `prune`, or emergency-push paths.
- Do not rewrite unrelated code or existing dirty-worktree changes.
- Keep queue availability optional for ordinary code-intel analysis and mandatory only for an
  explicit landing action.

## Open Questions
1. Fleet-wide queues remain out of scope because the selected upstream queue coordinates one
   machine only.
2. A future provider may replace Claude-specific worktree hooks if it preserves the same readiness
   and authority contract.

## Implementation Notes
- Prefer a thin PowerShell compatibility adapter over reimplementing queue locks.
- Resolve only a repository-local package binary by default; never use unpinned `npx` fallback.
- Treat the upstream config as text during status checks; do not execute repository JavaScript just
  to inspect readiness.
