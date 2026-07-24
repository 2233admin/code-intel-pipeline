# Automatic PR One-Command Orchestration
> Status: ACCEPTED FOR IMPLEMENTATION
> Created: 2026-07-16
> Source: local code-intel-pipeline

## Abstract
Add one PowerShell entrypoint that prepares an exact draft-PR proposal, asks for a structured user
decision, records the approved answer through C07, proves replay validity, and then delegates to the
existing fail-closed executor. The entrypoint must stop without repository or network effects when
the answer is missing, declined, stale, malformed, or no longer matches the repository snapshot.

## Core Insight
The orchestration layer must not invent a second authority model. It should compose the existing
Decision Port, Decision Record store, proposal evidence binding, and executor while preserving their
separate validation boundaries.

## Target Repo
- Path: `<repo>`
- Branch: current local branch
- Current state: dirty shared worktree; preserve unrelated changes

## Success Criteria
- [ ] One command generates a canonical proposal and decision request outside the target repository.
- [ ] Interactive mode asks the user; automation mode accepts only a structured response file.
- [ ] Decline, missing response, malformed response, expiry, or drift invokes neither `gh` nor the executor.
- [ ] Approval is validated by Decision Port, committed by C07, replayed, and bound to the exact proposal.
- [ ] Execution still requires both explicit repository-mutation and network switches.
- [ ] One Decision Record can create at most one draft PR.
- [ ] Tests use fake `gh`; implementation work creates no real PR and performs no remote push.

## Constraints
- Do not add dependencies.
- Do not weaken the executor or duplicate its one-time receipt semantics.
- Do not write generated decision artifacts into the scanned repository by default.
- C07 content binding is not cryptographic human identity; keep that limitation explicit.

## Open Questions
1. A trusted native UI/signature provider remains a later integration; console and supplied structured
   responses identify provenance but do not cryptographically authenticate the human.
2. Remote push/PR publication remains explicitly effect-gated even after approval.

## Implementation Notes
- Add a thin root facade and a bounded implementation under `tools/`.
- Resolve the packaged `bin/code-intel.exe` first and source-tree debug binary only as a development fallback.
- Emit a machine-readable flow result and retain the proposal/request/response/record/replay artifacts.
- Reuse `Invoke-CodeIntelAutomaticPullRequest.ps1` for all `gh` effects.
