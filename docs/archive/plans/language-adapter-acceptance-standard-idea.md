# Idea File
> Status: IMPLEMENTED_AND_LOCALLY_VERIFIED
> Created: 2026-07-15
> Source: Pipeline-owned consolidation of existing Code Evidence, capability envelope, parity floor, and internalization evidence

## Abstract
Define one machine-executable acceptance standard for every language adapter. Separate the strength of the adapter's claim from its release stage, then gate contract shape, measured quality, determinism, compatibility, effects, provenance, rollback, and independent verification through one policy.

## Core Insight
Language coverage and production readiness are different axes. A Python or Rust adapter may emit the same structural contract while having very different precision, and a semantically strong adapter may still be research-only because provenance, rollback, or independent verification is incomplete.

## Target Repo
- Path: `<repo>`
- Branch: current working branch
- Current state: large pre-existing dirty worktree; additions must stay isolated

## Success Criteria
- [x] Doctor passes.
- [x] A recent normal Pipeline run has zero effective failures and no Sentrux degradation.
- [x] A versioned policy defines claim levels and research/candidate/production thresholds.
- [x] A strict report contract records evidence for every acceptance dimension.
- [x] One reusable PowerShell gate returns machine-readable pass/fail results.
- [x] The native Code Evidence adapter has a candidate-stage acceptance report.
- [x] Tests prove a valid candidate passes and low precision, hidden network effects, implicit unsupported behavior, stale digests, semantic overclaim, weakened policy, and premature production promotion fail.
- [x] Documentation defines promotion, rollback, and non-overclaim rules.

## Constraints
- Do not add dependencies.
- Do not modify the existing native extractor or workflow files for the first gate.
- Do not turn heuristic structural evidence into a semantic or behavioral claim.
- Threshold changes require reviewed policy changes; reports cannot weaken their own gate.
- Research acceptance never grants production authority.

## Open Questions
1. Which parser-backed adapter should be the first production-stage proving case?
2. Should CI invoke the gate directly or through the future capability facade after the dirty workflow branch is reconciled?

## Implementation Notes
- Minimalism rung: reuse existing JSON evidence plus PowerShell standard-library validation.
- Policy is authoritative for thresholds; reports are observations only.
- The first accepted report targets `candidate` and `structural`, not `production` or `semantic`.
