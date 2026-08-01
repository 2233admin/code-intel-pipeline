# Idea File
> Status: IMPLEMENTED AS OPTIONAL RESEARCH ADAPTER; NOT DEFAULT OR PRODUCTION
> Created: 2026-07-23
> Source: cosmtrek/mindwalk trial plus local Code Intel evidence

## Abstract

Add an optional Rust session-evidence adapter to Code Intel. The adapter consumes Mindwalk trace v1,
removes prompt/summary content, binds the observation to a Code Intel repository snapshot, joins file
touches to Sentrux structural evidence, and emits advisory review signals.

## Core Insight

Temporal agent behavior is useful only when it is joined to repository-owned structural evidence.
The provider parser is replaceable and imperfect; path safety, privacy, observability grading,
snapshot identity, and review semantics must therefore belong to Code Intel.

## Target Repo

- Path: `<repo>`
- Branch: current user working branch
- Current state: heavily modified/untracked; implementation must be additive and avoid rewriting
  unrelated work

## Success Criteria

- [x] `provider session-adapt` consumes a Mindwalk trace v1 without invoking Mindwalk.
- [x] The emitted artifact contains no user-message marks, raw event summaries, or absolute paths.
- [x] Every in-repository target is normalized and every outside/unsafe target is counted, not
  silently accepted.
- [x] The artifact is bound to a Code Intel repository snapshot.
- [x] Optional Sentrux hotspot/DSM input enriches targets and produces advisory signals.
- [x] Missing or estimated provider fields remain visibly partial/unknown.
- [x] The adapter is optional and absent from normal scan execution.
- [x] Focused Rust privacy, path, snapshot, structural-join, and fail-closed contract tests pass.
- [ ] The cited real-session smoke is not independently reproducible from committed raw evidence;
  production/default promotion remains blocked on a fresh privacy-safe representative run,
  hostile-trace coverage, and measured maintenance/latency value.

## Constraints

- Add no dependency; use Rust standard library plus existing `serde_json`.
- Copy no Mindwalk implementation source.
- Preserve Mindwalk MIT provenance and source revision in documentation.
- Do not add production behavior to PowerShell.
- Do not grant session evidence gate, diagnosis, or Engineering Fact authority.
- Keep generated raw traces outside source control.

## Open Questions

1. Whether a future native Codex adapter should replace Mindwalk for exact structured tool events.
2. Which advisory thresholds should become policy-configurable after real usage data exists.

## Implementation Notes

- Minimalism rung: reuse existing snapshot identity and Sentrux artifact contracts, then add the
  smallest local Rust normalizer.
- Public command:
  `code-intel provider session-adapt --repo <repo> --trace <trace.json> [--hotspots <json>] [--out <json>]`.
- Default working-tree policy is `explicit_overlay`, because session review normally describes the
  files actually edited during the task.
- Mindwalk extraction remains a separate optional preceding command. Code Intel can consume a
  previously generated trace even when Mindwalk is later unavailable.
