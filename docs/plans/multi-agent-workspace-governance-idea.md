# Multi-Agent Workspace Governance Idea
> Status: IMPLEMENTED
> Created: 2026-07-15
> Source: local code-intel-pipeline

## Abstract
Add a read-only workspace preflight that inventories Git changes before a coding agent starts. The preflight must fail closed for a dirty repository root while still allowing an explicitly declared observation-only session.

## Core Insight
A merge queue cannot recover provenance after several agents have already written into the same dirty root. The cheapest reliable control is therefore a deterministic, machine-readable gate before mutation begins.

## Target Repo
- Path: `<repo-root>`
- Branch: discovered at runtime
- Current state: intentionally dirty; existing tracked and untracked changes are user-owned and must remain untouched

## Success Criteria
- [x] Doctor passes.
- [ ] Pipeline emits `summary.md`, `report.json`, and `understanding.md`.
- [x] Preflight reports tracked and untracked counts without modifying the inspected repository.
- [x] Preflight emits a stable machine-readable manifest and SHA-256 fingerprint.
- [x] Dirty root rejects mutation-oriented agent work by default.
- [x] Explicit observation-only mode remains available and cannot authorize writes.
- [x] Self-contained fixture tests prove clean, dirty, observation-only, and non-repository behavior.

## Constraints
- Do not add dependencies without explicit approval.
- Do not rewrite, clean, stash, reset, commit, or otherwise alter existing user changes.
- Do not modify merge queue, CI, integration registry, or project-conformance files in this slice.
- Use only PowerShell, Git, and .NET standard-library capabilities.
- Keep generated test repositories outside the inspected target and remove them after the test.

## Open Questions
1. A later integration slice may decide where this preflight is called automatically.
2. Multi-machine provenance remains outside this local workspace gate.

## Implementation Notes
- Minimalism rung: platform-native Git porcelain plus the smallest local PowerShell adapter.
- Parse `git status --porcelain=v1 -z --untracked-files=all` so paths with spaces and renames remain unambiguous.
- Hash canonical JSON inventory content, excluding volatile timestamps and absolute repository paths.
- Exit nonzero for mutation intent in a dirty root or for any unverifiable Git state.

## Verification
- Doctor passed on 2026-07-15.
- The self-contained fixture test passed for clean, modified, renamed, untracked, observation-only, subdirectory, non-repository, and invalid-policy cases.
- A live observation of the target root exited `0`; the same inventory with mutation intent exited `20`.
- The lite pipeline attempt was intentionally not retried while sibling agents were writing concurrently; snapshot identity returned exit `74`. The coordinating agent owns the stable post-integration normal run.
