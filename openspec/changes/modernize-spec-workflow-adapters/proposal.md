## Why

The current workflow recommender reduces OpenSpec versus spec-kit to repository age and file count, describes spec-kit as greenfield-only, treats setup commands as entry skills, and runs its production recommendation logic through duplicated PowerShell. OpenSpec 1.8.0 and spec-kit 0.16.1 invalidate those assumptions, so recommendations and activation wording no longer reflect the tools being evaluated.

## What Changes

- Add `code-intel-advisory-workflow-recommendation.v2` while retaining v1 compatibility. V2 separates candidate presence, recommendation, explicit adoption, semantic intent, agent-native invocation, setup commands, prerequisites, and effects.
- Replace free-form `entrySkills` as the normative entry contract with structured `entryActions`; retain a deterministic v1 projection only for active compatibility consumers.
- Select OpenSpec, spec-kit, or a bounded lightweight path from active artifacts and requested capabilities rather than repository age alone. Existing configuration is evidence of presence, not adoption authority; simultaneous active roots produce an explicit conflict instead of first-match adoption.
- Describe OpenSpec 1.8.0 as delta-first change governance with proposal/specs/design/tasks plus profile-dependent update, apply, verify, sync, and archive actions.
- Describe spec-kit 0.16.1 as constitution/specify/clarify/plan/checklist/tasks/analyze/implement/converge plus composable workflows, including brownfield use.
- Pin exact upstream revisions and MIT license evidence for both candidates, while preserving their research-only, no-runtime-dependency boundary.
- Move the deterministic producer and selection policy into Rust. Keep PowerShell entry points as thin compatibility forwarders; do not add product behavior to `.ps1` files.
- Keep all outputs proposal-only with empty effects. Recommendation, trigger wording, directory detection, or user phrasing cannot emit an Adoption Decision or initialize either tool.

## Capabilities

### New Capabilities

- `spec-workflow-adapters`: Additive v2 workflow recommendation contract, structured entry actions, current OpenSpec/spec-kit candidate semantics, and Rust-native deterministic selection.

### Modified Capabilities

None.

## Impact

- Affected surfaces: Rust capability implementation and CLI adapter, additive schema and lifecycle registration, A01/A03 validation, orchestration integration metadata, candidate internalization records, compatibility projections, documentation, and contract tests.
- Existing `code-intel-advisory-workflow-recommendation.v1` consumers remain valid during migration; no existing authority names or protected transitions change.
- No OpenSpec/spec-kit runtime dependency, automatic initialization, external lookup at execution time, repository mutation, network effect, Hospital verdict, default-DAG change, learning weight, or workflow adoption authority.
