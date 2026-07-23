# Idea File
> Status: IMPLEMENTED_AND_LOCALLY_VERIFIED
> Created: 2026-07-15
> Source: can1357/pon at ab9067dbd2899c64c4d67a4bc27b8ad49472b126

## Abstract
Map the language-independent part of pon's frontend/conformance architecture onto the Pipeline's existing Code Evidence contract. Prove that supported source languages emit the same file, symbol, containment, and import shapes without copying pon code or claiming parser-level semantic precision.

## Core Insight
The Pipeline already owns the sufficient intermediate representation: the tuple of `files`, `symbols`, `symbol-chunks`, and `imports`. The missing protection is a multilingual conformance floor that proves language adapters normalize into this shared contract while unsupported semantics remain explicit unknowns.

## Target Repo
- Path: `D:\projects\_tools\code-intel-pipeline`
- Branch: current working branch
- Current state: large pre-existing dirty worktree; new work must stay in isolated files

## Success Criteria
- [x] Doctor passes.
- [x] Pipeline emits `summary.md`, `report.json`, `hospital.md`, and `understanding.md`.
- [x] A committed fixture covers Python, JavaScript, TypeScript, Rust, Go, Java, PowerShell, and an unsupported-language control.
- [x] One Rust integration test proves supported languages normalize into the same Code Evidence v1 field contract.
- [x] The test proves unsupported languages stay explicit and do not fabricate symbols or imports.
- [x] Documentation distinguishes normalized structural facts from AST, type, control-flow, and runtime semantics.
- [x] An internalization record pins source revision, owned artifacts, evidence hashes, rollback, and review gaps.

## Constraints
- Do not add dependencies.
- Do not copy pon implementation code, fixtures, or prose because the upstream repository has no declared license.
- Do not relabel line heuristics as parser, type, call-graph, or runtime semantic precision.
- Do not modify overlapping workflow or native extractor files for this proof.
- Keep the existing Code Evidence artifact contract and runtime behavior unchanged.

## Open Questions
1. Which language should receive the first parser-backed adapter after this structural floor is established?
2. Should Java imports and C# symbols remain explicit gaps or become the next bounded adapter work?

## Implementation Notes
- Minimalism rung: reuse the existing Pipeline contract and Rust test harness.
- The proof is a conformance fixture, not a second IR implementation.
- Production widening requires parser-backed evidence and independent verification.
