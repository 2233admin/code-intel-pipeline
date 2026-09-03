---
title: Evidence-Bound Design Proposal Capability
status: draft
stage: design
reader_action: review the architecture and approve the implementation boundary
artifact_shape: design doc
authority: advisory_only
---

# Evidence-Bound Design Proposal Capability

## Decision card

- Driver: improve the project's ability to turn a bounded development request into an auditable design proposal.
- Approver: Code Intel Pipeline maintainer.
- Contributors: Rust capability owners, Agent workflow owners.
- Informed: maintainers of provider adapters and orchestration contracts.
- Impact: adds a proposal-oriented consumer of existing repository evidence; does not change authoritative scan semantics.
- Current decision: use a two-stage flow. Rust prepares and validates evidence-bound context; an Agent writes candidate options and recommendation.

## Problem and context

The project already produces repository snapshots, change impact, provider evidence, typed ArtifactRefs, and transactional run commits. It also has an Engineering Method Catalog, but the catalog is currently reference-only. A development request can therefore be analyzed structurally without a stable contract for comparing design options, recording rejected alternatives, or proving that a recommendation is grounded in the current repository.

Atlas-Archify-Coding provides useful patterns for evidence anchors, baseline/delta comparison, graph-to-ledger reconciliation, and fail-closed gates. Its Node CLI and Atlas sidecar are not appropriate as a second runtime or second source of truth for this project.

The first capability targets requirements-to-design for existing projects. It is not a greenfield scaffold generator, automatic code writer, or distributed architecture planner.

## Goals

1. Produce a versioned, advisory-only design proposal artifact.
2. Compare two or three viable design options using common dimensions.
3. Bind design claims and recommendation reasons to verified evidence references.
4. Reuse the existing snapshot, change-impact, method-catalog, ArtifactRef, authority, and Run Commit boundaries.
5. Keep Agent-authored prose untrusted until Rust validates its shape, references, authority, and consistency.
6. Make stale, missing, contradictory, or insufficient evidence explicit.
7. Add three initial method cards: characterization test, seam extraction, and small-step refactoring.

## Non-goals

- No Node runtime or Atlas sidecar import.
- No second state ledger beside the existing artifact and workflow state.
- No automatic code modification, commit, issue close, or plan approval.
- No runtime dependency on a particular LLM or model provider.
- No automatic method selection in the first version.
- No visual diagram renderer requirement in the first version.
- No distributed deployment or multi-tenant control plane.
- No claim that an advisory proposal is an Engineering Fact or Committed Engineering Plan.

## Options considered

### Option A: evidence-bound advisory proposal

Rust prepares a Design Context from verified artifacts. An Agent returns two or three candidate options and a recommendation. Rust validates and publishes a proposal artifact with `authority=advisory_only`.

- Benefits: preserves existing Rust ownership, supports human-quality tradeoff prose, avoids model runtime coupling, and makes evidence validation deterministic.
- Costs: requires a clear handoff envelope and an Agent-side generation step.
- Risks: candidate prose may be weak or incomplete; validator must reject structural and evidence defects.
- Reversibility: high. The capability can remain a consumer-only artifact producer.

### Option B: deterministic Rust templates

Rust selects a fixed set of design templates from repository signals and renders the comparison itself.

- Benefits: deterministic output and simple deployment.
- Costs: limited domain expressiveness; poor handling of nuanced tradeoffs and rejected alternatives.
- Risks: template output can appear authoritative while missing important context.
- Reversibility: medium. Later Agent integration would need a second output path or a broad template rewrite.

### Option C: model Provider inside the pipeline

Rust calls a replaceable model Provider to generate the proposal and then validates it.

- Benefits: integrated user experience and room for richer synthesis.
- Costs: adds network, credentials, cost, model version, timeout, privacy, and nondeterminism contracts.
- Risks: model availability and output drift become runtime pipeline concerns.
- Reversibility: medium-low after consumers depend on the Provider path.

### Recommendation

Choose Option A. Rust remains authoritative for facts, evidence, contract validation, and publication. Agent output remains a candidate until accepted by the Rust validator. Option C can be added later as an external Provider without changing the proposal artifact contract.

## Proposed architecture

```text
request
  -> Rust Design Context preparation
       -> snapshot-bound facts
       -> change impact
       -> architecture and history evidence
       -> explicit method cards
  -> Agent Proposal Candidate
       -> 2-3 options
       -> tradeoffs and assumptions
       -> recommendation and rejected alternatives
  -> Rust Proposal Validation
       -> schema and exact keys
       -> snapshot and ArtifactRef checks
       -> method applicability
       -> option/recommendation references
       -> advisory authority boundary
  -> staged ArtifactRef publication
       -> manifest and SHA-256
       -> downstream plan or review consumer
```

The public command surface should remain small. The implementation may begin as a capability routed through the existing command catalog and adapter seams rather than adding several top-level commands. A future public command can expose the same contract only after the capability has an adoption baseline and contract tests.

## Design Context

Rust prepares a context envelope containing:

- the current repository snapshot identity;
- Project Orientation and relevant architecture facts;
- current Change Impact and candidate tests;
- verified ArtifactRefs for source, configuration, tests, decisions, and provider evidence;
- explicit constraints and known unknowns;
- selected method card IDs and their required evidence;
- freshness and authority metadata.

The context is read-only from the Agent's perspective. The Agent does not receive authority to mutate the repository or promote facts.

## Proposal contract

Request shape:

```json
{
  "schema": "code-intel-design-proposal-request.v1",
  "repo": "target-repository",
  "snapshot": {},
  "request": {
    "goal": "bounded development request",
    "constraints": [],
    "methodIds": [
      "legacy-characterization-test",
      "legacy-seam-extraction",
      "refactor-small-step"
    ]
  },
  "inputs": []
}
```

Validated result shape:

```json
{
  "schema": "code-intel-design-proposal.v1",
  "kind": "proposal",
  "authority": "advisory_only",
  "snapshot": {},
  "request": {},
  "baseline": {
    "facts": [],
    "evidenceRefs": []
  },
  "delta": {
    "affectedModules": [],
    "affectedInterfaces": [],
    "newBoundaries": [],
    "removedBoundaries": []
  },
  "methods": [],
  "options": [],
  "recommendation": {},
  "risks": [],
  "validationPlan": [],
  "limitations": []
}
```

Each option must include an ID, title, summary, boundary changes, benefits, costs, risks, assumptions, evidence references, validation conditions, and reversibility. The result must contain two or three options. `recommendation.optionId` must resolve to an option in the same result. Recommendation reasons and rejected alternatives must carry evidence references or be explicitly marked as assumptions.

The result is advisory-only even when all validation checks pass. Validation proves contract and provenance properties; it does not approve the design.

## Method catalog additions

Add three cards under `orchestration/methods/cards/` using the existing `code-intel-method-card.v1` shape and register them in `orchestration/methods/catalog.v1.json`:

1. `legacy-characterization-test`: capture observable legacy behavior before changing implementation.
2. `legacy-seam-extraction`: isolate a real dependency or side-effect boundary while preserving the old path.
3. `refactor-small-step`: perform one behavior-preserving transformation at a time under an executable baseline.

The first version accepts explicit `methodIds`. `problemSignals`, `confidenceRules`, and `cost` remain present for future signal-based recommendation, but the catalog policy remains `catalog_only_no_selection_or_execution` until automatic selection has its own evidence and evaluation contract.

## Validation and gate

Rust rejects the candidate when any of these conditions holds:

- fewer than two or more than three options;
- unknown method ID or missing required method evidence;
- malformed or extra contract fields;
- snapshot mismatch between request, context, and evidence references;
- missing, stale, drifted, or digest-invalid evidence reference;
- recommendation references a missing option;
- baseline or delta is structurally incomplete;
- authority is anything other than `advisory_only`;
- a claimed fact has no supporting evidence or is only an unmarked assumption;
- an option lacks a validation condition or boundary statement.

Diagnostics must distinguish missing, stale, unknown, invalid, and authority-escalation cases. A proposal with insufficient evidence may be returned as a failed validation result with explicit unknowns; it must not be promoted as a successful proposal.

Allowed effects for the first version:

```text
read_repository
read_committed_artifacts
read_method_catalog
write_staged_artifact
```

Disallowed effects:

```text
repository_mutation
automatic_commit
automatic_plan_approval
automatic_issue_close
network_model_call
```

## Verification plan

Contract tests use fixed Agent Proposal Candidate fixtures and do not call a model or network service.

Required cases:

- valid two-option proposal publishes an advisory artifact;
- valid three-option proposal preserves comparison and recommendation references;
- too few and too many options fail;
- unknown method and missing required evidence fail;
- snapshot mismatch, digest mismatch, and drifted evidence fail;
- invalid recommendation reference fails;
- missing baseline/delta fields fail;
- authority escalation fails;
- unsupported claims are reported as unknown or rejected;
- staged output cleanup occurs after validation failure;
- published ArtifactRef and manifest are snapshot-bound and digest-valid.

The method-card tests validate schema shape, applicability boundary, required evidence, deterministic steps, outputs, contraindications, and related methods. Golden fixtures keep the proposal envelope and diagnostic rules stable.

## Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Agent presents opinion as fact | Require evidenceRefs for claims and explicit assumption markers; keep authority advisory-only |
| Evidence becomes stale during design | Bind every input to snapshot identity and reject mismatched refs |
| Method cards become a second workflow engine | Keep catalog policy reference-only; do not execute cards in the first version |
| Option comparison becomes decorative prose | Require common dimensions, validation conditions, and rejected-alternative reasons |
| Command surface grows without adoption | Start behind existing capability seams and apply the current method/command budget |
| External model becomes a hidden dependency | Keep Agent generation outside Rust runtime; add a Provider only through a separate contract later |

## Touched assets and boundaries

| Asset | Relation | Change | Risk | Verify | Rollback |
| --- | --- | --- | --- | --- | --- |
| `orchestration/methods/catalog.v1.json` | method registry | register three cards | stale catalog pin | catalog contract tests | remove card registrations |
| `orchestration/methods/cards/*.json` | design-method references | add three method cards | incorrect applicability | method-card validation | delete unadopted cards |
| `crates/code-intel-cli` capability and contract modules | runtime owner | add context/proposal validation and artifact publication | authority or snapshot leak | focused Rust contract/integration tests | disable route and remove artifact registration |
| `orchestration/capability-contract.v1.json` | capability contract | register request/result/ref effects | contract drift | schema and pin gates | revert registration |
| `orchestration/acceptance/` | acceptance evidence | add proposal fixtures and gates | false green | adversarial and golden tests | remove acceptance entry |

## Open questions

1. Whether the first implementation exposes one public command or only an internal capability route.
2. Whether the Agent handoff is stdout JSON, a staged file, or an existing agent-session artifact reference.
3. Which existing committed evidence types are sufficient for `Design Context` v1.
4. Whether option comparison dimensions remain fixed or are request-extensible under a bounded whitelist.
5. What adoption baseline is required before enabling automatic method recommendation.

## References

- Atlas-Archify-Coding repository: <https://github.com/agnitum2009/atlas-archify-coding>
- Atlas ADD specification: <https://github.com/agnitum2009/atlas-archify-coding/blob/main/specs/ADD-SPEC.md>
- Atlas command contract: <https://github.com/agnitum2009/atlas-archify-coding/blob/main/specs/command-contract.md>
- Michael Feathers, *Working Effectively with Legacy Code* (2004).
- Martin Fowler, *Refactoring*.
- Joshua Kerievsky, *Refactoring to Patterns* (2004).
- John Ousterhout, *A Philosophy of Software Design*.
- Existing project context: `CONTEXT.md`.
- Existing method registry: `orchestration/methods/catalog.v1.json`.
- Existing advisory implementation: `crates/code-intel-cli/src/workflow_recommendation.rs`.
