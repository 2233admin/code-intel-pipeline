# Model-independent Code Intel Pipeline completion
> Status: IMPLEMENTED AND LOCALLY VERIFIED — EXTERNAL ASSURANCE GAPS EXPLICIT
> Created: 2026-07-23
> Previous completion claim: 2026-07-23 (retracted after requirement-by-requirement audit)
> Source: user goal + ADR 0010 + Pipeline self-run evidence

The previous completion claim was incorrect. Green compatibility tests and a committed core run did
not prove that the default `normal` path was one manifest-bound production spine, that failed DAG
runs could not enter the index, or that provider/graph/Sentrux/diagnosis evidence participated in
the authoritative run. The renewed implementation and evidence below close those local product
gaps. Independent clean-machine assurance, an official clean-worktree package, and promotion of the
optional Mindwalk/session adapter remain explicitly outside the locally verified claim.

## Abstract

Complete Code Intel Pipeline as a deterministic, model-independent evidence pipeline. The Pipeline
collects, normalizes, snapshot-binds, validates, composes, publishes, indexes, and replays code
evidence; models remain replaceable consumers or optional providers and never own Pipeline facts.

This is not a workbench, IDE, dashboard, autonomous PR product, or another RAG stack. Completion
means the already-defined atomic capabilities are connected into the production run path, missing
query/change contracts are added at the smallest sufficient rung, and the Pipeline proves itself on
representative repositories.

## Core Insight

The repository already contained most of the required atoms. The completed work connected them into
one manifest-bound production spine, made non-completed runs ineligible for A08 authority, and exposed
committed provider evidence through bounded query/change interfaces. The continuing risk is now
assurance drift: digest-bound records, cross-contract tests, real wrapper E2E, and representative runs
must remain release gates as the model or provider set changes.

## Target Repo

- Path: `<repo-root>`
- Branch: `agent/sentrux-rust-dsm-kernel`
- Current state: heavily dirty shared worktree; preserve existing changes and avoid broad rewrites.
- Baseline self-run: `20260723-035204`; orchestration passes after the session adapter stage repair,
  Sentrux gate blocks on `complex_functions 20 -> 21`, and committed-only index reports zero repos.

## Pipeline Boundary

Pipeline owns:

- deterministic collection and provider-neutral adapters;
- snapshot identity, freshness, provenance, coverage, confidence, and authority labels;
- artifact validation, staging, atomic publication, committed-run indexing, and replay;
- bounded evidence query, change impact, test candidates, and verification observations;
- conformance, benchmark, rollback, and schema compatibility evidence.

Pipeline does not own:

- a user-facing workbench or IDE;
- general chat, planning, code generation, or model reasoning;
- automatic PR/issue/wiki publication without explicit authority;
- provider databases, UIs, prompts, summaries, or internal algorithms beyond survival boundaries.

## Success Criteria

- [x] Doctor and integration orchestration validation pass.
- [x] Production normal runs use one manifest-bound staging/commit/publication spine.
- [x] At least one representative green run enters the committed-only artifact index.
- [x] Legacy runs remain explicit diagnostics or read-only compatibility; they are never silently promoted.
- [x] Graph/provider freshness binds snapshot identity, producer version, and explicit age policy.
- [x] `query` returns bounded path/symbol/finding evidence with provenance, freshness, coverage,
      confidence, authority, and unknowns.
- [x] `impact` maps a snapshot-bound change set to affected symbols/files and test candidates without
      inventing semantic certainty.
- [x] Language observations expose inventory/structural/semantic/behavioral claim level and pass the
      existing language-adapter acceptance gate.
- [x] Session evidence is optional verification input and never required for normal scans.
- [x] Cross-contract CI rejects unknown stages, command drift, missing schemas, incompatible schema
      changes, and operation-trace drift.
- [x] Representative benchmarks measure correctness, determinism, latency, artifact size, and
      unresolved/unsupported coverage.
- [x] Pipeline self-run has no unexplained effective failures; remaining debt is explicit and bounded.

## Constraints

- No new production dependency without explicit approval.
- Reuse A01-A09, B01-B10, C01-C07, D01-D04, existing adapters, schemas, and tests before adding code.
- Keep production behavior in Rust where the existing atomic core owns it; PowerShell remains a facade.
- No baseline rewrite to hide structural regression.
- No raw prompts, secrets, absolute outside paths, or provider-private payloads in normalized evidence.
- Do not commit, delete, or overwrite unrelated shared-worktree changes.

## Delivery Order

1. Production spine: orchestration drift guard, structure regression, A09/A06/A07/A08 integration,
   and real freshness.
2. Evidence interface: query/explain over admitted artifacts and native code evidence.
3. Change interface: snapshot-bound diff, impact relationships, and conservative test candidates.
4. Optional verification: session evidence composition and runtime/CI evidence.
5. Evaluation: a real representative repository plus Pipeline self-analysis, and a deterministic
   nine-fixture corpus covering correctness, replay, calibration, latency, size, unresolved, and
   unsupported behavior.
6. Final self-run, documentation reconciliation, and explicit residual-risk report.

## Stop Condition

Stop only when the success criteria have fresh evidence or a remaining item is blocked by an
irreversible/external decision. A feature count, document count, or model-produced explanation is
not completion evidence.

## Completion Evidence

- Authoritative self-run: `code-intel-pipeline/20260723-112701-891-core` completed through the stable
  wrapper. A09 staged one snapshot-bound manifest, A07 committed it atomically, and A08 admitted it
  into the committed-only index. Graph and real Sentrux `gate`/`check` evidence fed Hospital in the
  same run.
- Read/change closure: `artifact query` returned committed graph/Sentrux/Hospital evidence with equal
  recorded/current snapshot identity `b496f0c5...71a1` and freshness `current`. `change impact` for
  `crates/code-intel-cli/src/dag_run.rs` used that same run, found the file in inventory, and emitted
  explicit heuristic limitations and advisory-only test candidates.
- Real representative repository: Mindwalk run `mindwalk/20260723-105742-733-core` completed, and
  current query/impact reads resolved its real `internal/adapter/codex/adapter.go` path.
- Failure authority: `domain_failed` and `domain_unknown` retain verified diagnostic artifacts at A07,
  but A08 classifies every non-completed commit as `non_completed`. Process/contract failures remain
  distinct, and invalid UTF-8 failure injection proves failed runs are retained for audit but excluded
  from current authority.
- Benchmark: 9 fixtures × 3 cold/warm repetitions passed with deterministic replay, field correctness,
  provenance completeness, unknown precision, unresolved coverage, and unsupported coverage all
  `1.0`; the largest measured artifact was 5,195 bytes. The report truthfully remains
  `cleanMachine: false`.
- Contracts and E2E: the complete Rust test matrix passed from the beginning after digest
  reconciliation; internalization records passed 40/40 twice under hardened parallel execution.
  The stable wrapper E2E executed real Sentrux, committed/indexed a completed run, queried current
  evidence, and verified failure exclusion. PowerShell parser checks and integration toolchain digest
  checks passed.
- Adapter status: the seven-language native adapter passed all acceptance gates only at
  `candidate + structural`; semantic, behavioral, independent, and production claims remain false.
  Mindwalk/session is implemented as an optional research adapter and is not invoked by default.
- Deliverable: Rust CLI version `0.3.0`, root MIT license, lockfile, changelog, and Windows beta package
  were generated. Development-package verification covered 752 files, 12 locked dependencies,
  checksums, traversal safety, PowerShell parsing, CLI help, and wrapper smoke without Cargo or
  Repowise.

## Bounded Residuals

- The historical clean-machine attestation is source-stale and now states
  `externalVerificationComplete: false`. A fresh independent disposable-machine run is required
  before claiming current clean-machine assurance.
- The shared worktree is intentionally dirty, so the verified ZIP is a development beta build, not an
  official clean-worktree release. The verifier rejects it without the explicit `-AllowDirty` flag.
- Mindwalk/session promotion is blocked on independently reproducible privacy-safe real-session raw
  evidence, hostile-trace testing, and latency/maintenance measurements. It remains research-only.
- The authoritative Rust core exposes committed query/Hospital evidence but does not pretend to emit
  the legacy `summary.md`/`understanding.md` report pack; those remain compatibility UX rather than
  current-run authority.
- The index reports 647 historical diagnostic rows from prior legacy/invalid runs. Current entries are
  completed-only; the diagnostics are retained audit history, not silently deleted debt.
- The Rust test architecture still emits known `dead_code` warnings because integration suites import
  production modules directly, plus one removable `unused_mut`. Formatting, compilation, contracts,
  and behavior are green; warning cleanup is maintenance debt rather than a correctness blocker.
