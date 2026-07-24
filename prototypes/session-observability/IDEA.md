# Idea: session evidence × structural risk

Status: throwaway logic prototype; not a production Code Intel entry point.

## Problem statement

Code Intel currently explains repository structure and risk, but not how an agent actually moved
through the repository during a task. Mindwalk already extracts a useful temporal trace from Codex
and Claude session logs. The question is whether joining those events to Code Intel hotspot evidence
creates a materially better review surface without coupling the core pipeline to Mindwalk.

## Target implementation

- Repository: `<repo-root>`
- Prototype: `prototypes/session-observability`
- Production files touched: none
- Input contracts:
  - Mindwalk trace schema version 1
  - Code Intel `sentrux-hotspots.json`

## Hypothesis

A local, provider-neutral timeline that annotates touched files with existing structural evidence
will make risky agent behavior visible: editing high-complexity files, repeated churn, errors, and
edits after the last verification step.

## Success criteria

1. Parse a trace generated from a real Codex session.
2. Join trace targets to Sentrux hotspot records after path normalization.
3. Navigate events and jump to edits, verification, and errors in a terminal.
4. Keep raw session logs and user-message marks out of Code Intel artifacts.
5. Require no production entry-point or dependency change.

## Non-goals

- Recreating Mindwalk's parser, city map, web UI, or LLM judge.
- Persisting a new authoritative artifact.
- Automatically running on every Code Intel scan.
- Defining a final risk score or policy gate.

## Constraints and invariants

- Mindwalk remains an optional adapter/provider.
- Code Intel owns any future redacted, provider-neutral evidence contract.
- Unknown or unmatched paths remain visibly unknown; they are not treated as safe.
- This prototype reads local files only and writes no session-derived output.
- The Code Intel production tree is currently dirty; this work is isolated in a new directory.

## Provenance

The trace contract and extraction behavior are compatible with cosmtrek/mindwalk at commit
`e208b6b8504138843f671e031f28129b66003a67`, licensed under MIT. No Mindwalk source code is copied
into this prototype.

## Decision after trial

Promote only if a real trace produces useful path joins and the timeline exposes review-relevant
signals. If promoted, internalize the redacted evidence contract and routing policy, while keeping
Mindwalk as a replaceable adapter.
