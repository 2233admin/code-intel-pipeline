## Purpose

Defines current, proposal-only OpenSpec and spec-kit workflow recommendations with structured activation actions, exact provenance, and no hidden adoption authority.

## ADDED Requirements

### Requirement: V2 is additive and proposal-only
The system SHALL emit `code-intel-advisory-workflow-recommendation.v2` as an additive contract while preserving valid v1 consumers. V2 MUST retain `kind: proposal`, top-level `effects: []`, evidence, confidence, alternatives, and provenance, and MUST NOT emit an Adoption Decision or committed plan.

#### Scenario: V2 request returns no effects
- **WHEN** a caller requests a v2 workflow recommendation with valid repository evidence
- **THEN** the result validates as v2, reports proposal authority only, and has an empty top-level effects list

#### Scenario: Existing v1 consumer remains valid
- **WHEN** an active compatibility consumer requests or receives the v1 projection
- **THEN** the system emits a deterministic v1 document accepted by the existing v1 schema without requiring v2 fields

### Requirement: Entry actions are structured and setup is separate
Each v2 candidate MUST expose structured `entryActions`. Every entry action MUST identify semantic intent, adapter action id, availability, agent-native and generic invocation forms when known, prerequisites, and the effects of invoking that action. Installation, initialization, profile configuration, and upgrade commands MUST appear only in a separate setup or maintenance section and MUST NOT be represented as entry skills.

#### Scenario: Codex activation is represented without setup confusion
- **WHEN** a candidate has an installed Codex skill for the requested semantic intent
- **THEN** its entry action identifies the Codex-native invocation and does not substitute `openspec init`, `openspec update`, or `specify init` as the activation

#### Scenario: Unavailable profile action is not advertised as installed
- **WHEN** a product version supports an action but the active profile or integration did not install its skill
- **THEN** the action is marked unavailable or conditional and the result does not claim its native invocation is callable

### Requirement: Presence, recommendation, and adoption remain distinct
The system MUST report tool-root presence and active artifacts separately from recommendation and approved adoption. A directory or generated skill proves configuration only. An approved adoption reference MAY be reported when supplied as evidence, but the recommender MUST NOT create or infer it.

#### Scenario: Directory presence is not already adopted
- **WHEN** `openspec/` or `.specify/` exists without an approved authority event bound to the current change
- **THEN** the candidate is reported as configured or active-artifact-present, not adopted

#### Scenario: Both active roots produce conflict
- **WHEN** active OpenSpec and spec-kit artifacts both claim the same change and no approved selection identifies one normative source
- **THEN** the recommendation reports a normative-source conflict and does not choose by detector order

### Requirement: Selection uses requested capability and active state
The recommender SHALL rank candidates from active change artifacts, requested semantic intents, governance needs, and adapter capabilities. Repository age and file count MAY appear as descriptive evidence but MUST NOT decide OpenSpec versus spec-kit by themselves.

#### Scenario: Delta governance favors OpenSpec
- **WHEN** the request requires change deltas, explicit artifact dependencies, cross-change archive or sync, and replayable governance history
- **THEN** OpenSpec fit reasons name those capabilities instead of repository age

#### Scenario: Constitution and convergence favor spec-kit
- **WHEN** the request requires constitution governance, user-story specification, clarification, checklists, cross-artifact analysis, convergence, or composed multi-step workflows
- **THEN** spec-kit fit reasons name those capabilities even for an existing brownfield repository

#### Scenario: Bounded change can use lightweight path
- **WHEN** the change is local, has no multi-artifact governance need, and existing project policy permits a lightweight plan
- **THEN** the recommender can select the lightweight candidate rather than forcing either external workflow

#### Scenario: Explicit preference is recorded as override evidence
- **WHEN** a user explicitly prefers one candidate contrary to the computed fit
- **THEN** the recommender records the manual override and its reason while keeping the output proposal-only until A05 approval exists

### Requirement: OpenSpec candidate reflects version 1.8.0 and active profile
The OpenSpec candidate MUST cite upstream tag v1.8.0 at commit `d57889664cab4f2f061d236ec3ff82a5578701bb` with MIT license evidence. It MUST describe the default spec-driven artifact dependency chain and distinguish core versus custom or expanded workflow availability instead of assuming every upstream action is installed.

#### Scenario: Custom profile reports only installed actions
- **WHEN** repository evidence shows a custom OpenSpec profile with propose, explore, apply, and archive only
- **THEN** those actions are reported available and update, sync, verify, or other profile-dependent actions are not reported as installed

#### Scenario: OpenSpec wording is capability-based
- **WHEN** OpenSpec is evaluated for an existing or new repository
- **THEN** the description emphasizes delta-first change governance and artifact dependencies, not a brownfield-only age rule

### Requirement: spec-kit candidate reflects version 0.16.1 and brownfield support
The spec-kit candidate MUST cite upstream tag v0.16.1 at commit `ad4104b56c219b0a27bac06547d1a3c7d6a0dbd6` with MIT license evidence. It MUST describe constitution, specify, clarify, plan, checklist, tasks, analyze, implement, converge, and composable workflow capabilities, including documented brownfield use.

#### Scenario: Brownfield repository is eligible for spec-kit
- **WHEN** an existing repository requests modernization with constitution, requirement-quality, staged implementation, or convergence needs
- **THEN** spec-kit remains eligible and is not rejected as greenfield-only

#### Scenario: Missing integration is explicit
- **WHEN** spec-kit is a fit but its requested agent integration is not installed
- **THEN** the result separates fit from availability and lists setup guidance outside `entryActions`

### Requirement: Activation wording maps intent before tool spelling
The system MUST normalize activation wording into semantic intents before selecting an adapter action. Exploration, proposal, update, implementation, verification or convergence, shipping, and outcome observation MUST remain distinct intents. Literal activation phrases and command names MUST NOT grant adoption or effects.

#### Scenario: Implementation phrase selects an action, not a tool globally
- **WHEN** the user says "开始做", "继续实现", or an equivalent implementation phrase after one normative source is adopted
- **THEN** the host can resolve the adapter's apply or implement entry action without changing pipeline-wide adoption

#### Scenario: Completion question selects verification or convergence
- **WHEN** the user asks "做完了吗", "按规范验收", or "找遗漏"
- **THEN** the result identifies a verification or convergence intent and does not infer completion from checked tasks

#### Scenario: Shipping and outcome remain outside planning adapters
- **WHEN** the user asks to commit, open a pull request, review to clean, merge, or determine whether the change improved results
- **THEN** the result routes to the tool-neutral shipping or outcome capability rather than claiming OpenSpec or spec-kit owns those effects

### Requirement: Rust owns production recommendation semantics
The production v2 recommendation and v1 compatibility projection MUST be deterministic Rust behavior over supplied repository evidence and governed candidate data. PowerShell compatibility entry points MUST contain only argument forwarding, invocation, and output adaptation needed for active consumers; they MUST NOT retain an independent selection policy.

#### Scenario: Rust and compatibility paths agree
- **WHEN** the same fixture is evaluated through the compiled capability and a retained PowerShell compatibility entry point
- **THEN** their normalized v1 outputs and v2-to-v1 projection are contract-equivalent

#### Scenario: Recommendation remains offline
- **WHEN** the capability evaluates a valid request without network access or installed OpenSpec/spec-kit runtimes
- **THEN** it returns a deterministic recommendation from local evidence and performs no repository mutation or external lookup

### Requirement: Artifact validators enforce the published closed schemas
The A03 validators for workflow recommendation and supplied adoption authority MUST reject payloads that violate their published nested schemas. Recommendation provenance MUST contain the exact governed OpenSpec 1.8.0 and spec-kit 0.16.1 source entries; a substituted URI, version, or revision MUST NOT satisfy provenance.

#### Scenario: Malformed nested recommendation is rejected
- **WHEN** a v2 payload contains a scalar recommendation, malformed evidence, non-integer score, invalid action invocation, or non-array capability set
- **THEN** the artifact validator rejects it before staging or consumption

#### Scenario: Malformed adoption authority is rejected
- **WHEN** a supplied authority event has duplicate or empty evidence ids, extra nested fields, or non-integer timestamps even with a recomputed attestation
- **THEN** A03 rejects it and the recommender does not report adoption

#### Scenario: Substituted provenance is rejected
- **WHEN** a v2 payload omits or substitutes either governed OpenSpec 1.8.0 or spec-kit 0.16.1 source identity
- **THEN** both the public JSON Schema and the Rust artifact validator reject the payload
