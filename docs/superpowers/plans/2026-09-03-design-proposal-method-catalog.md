# Evidence-Bound Design Proposal Capability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Rust-owned, advisory-only capability that prepares snapshot-bound design context, accepts Agent-authored two- or three-option proposals, validates their evidence and authority, and publishes a verified design-proposal artifact.

**Architecture:** Extend the existing capability-execution seam rather than adding a second CLI runtime or Atlas sidecar. Rust creates a read-only Design Context and validates a separate Agent Proposal Candidate; only the validated result is published through existing ArtifactRef and staged publication rules. The existing method catalog receives three reference-only cards and remains `catalog_only_no_selection_or_execution`.

**Tech Stack:** Rust, `serde_json::Value`, existing `AdapterOutput`/`VerifiedArtifact` contracts, JSON artifact manifests, Cargo integration tests, repository fixtures, `code-intel` repin and Sentrux gates.

**Spec:** `docs/superpowers/specs/2026-09-03-design-proposal-method-catalog.md`

## Global Constraints

- Keep the proposal result `authority=advisory_only`; it is not an Engineering Fact or a Committed Engineering Plan.
- Rust owns snapshot binding, evidence validation, contract validation, and staged artifact publication; the Agent only authors the candidate options and recommendation.
- Do not add a Node runtime, Atlas sidecar, model Provider, automatic code mutation, automatic commit, automatic plan approval, or automatic issue close.
- Keep method catalog policy `catalog_only_no_selection_or_execution`; the first version accepts explicit `methodIds` and does not auto-select methods.
- A valid proposal contains two or three options, and `recommendation.optionId` must resolve to one of them.
- Allowed first-version effects are repository read, committed-artifact read, method-catalog read, and staged-artifact write; repository mutation and network model calls remain disallowed.
- Use existing `Snapshot Identity`, `ArtifactRef`, `AdapterOutput`, authority, staged writer, and Run Commit boundaries; do not create a parallel source of truth.
- New production behavior belongs in Rust; do not add PowerShell product behavior.
- After all source/config edits in a batch, run `target/debug/code-intel repin --repo . --write` once and verify no stale pins remain.
- Run focused Rust contract tests, relevant integration tests, `target/debug/code-intel sentrux gate .`, `target/debug/code-intel lint hardcoded-paths`, and `git diff --check` before claiming completion.

## File and module map

- Create `orchestration/methods/cards/legacy-characterization-test.v1.json` for the Feathers characterization-test method card.
- Create `orchestration/methods/cards/legacy-seam-extraction.v1.json` for the Feathers seam-extraction method card.
- Create `orchestration/methods/cards/refactor-small-step.v1.json` for the Fowler/Kerievsky small-step refactoring method card.
- Modify `orchestration/methods/catalog.v1.json` to register the three cards in stable lexical order.
- Modify `crates/code-intel-cli/tests/method_catalog.rs` to assert twelve registered cards and the three new method-specific preconditions.
- Create `crates/code-intel-cli/src/design_proposal.rs` containing the context assembler, candidate validator, result builder, and proposal-specific diagnostics. Keep the public entry point `execute(request, verified_inputs, out) -> Result<AdapterOutput, AdapterError>` consistent with other capability modules.
- Modify `crates/code-intel-cli/src/capability_inventory.rs` to dispatch `advisory.design-proposal.compat` to `design_proposal::execute`.
- Modify `crates/code-intel-cli/src/artifact_ref.rs` to register and validate `code-intel-design-context.v1`, `code-intel-design-proposal-candidate.v1`, and `code-intel-design-proposal.v1` artifact shapes.
- Create `crates/code-intel-cli/tests/fixtures/design-proposal/` fixtures for valid two-option, valid three-option, invalid option count, invalid recommendation, stale snapshot, drifted evidence, missing method evidence, and authority escalation cases.
- Create `crates/code-intel-cli/tests/design_proposal.rs` for CLI-level capability execution tests using the existing `tests/common.rs` and temporary repository helpers.
- Modify `orchestration/capability-contract.v1.json`, `orchestration/integrations.json`, and create `orchestration/internalization/design-proposal.json` to declare the capability, artifact identities, effects, implementation, and conformance test.
- Modify `crates/code-intel-cli/tests/capability_contract.rs` or the nearest existing capability declaration test to assert the new capability and artifact identities are registered without introducing a new top-level command.

---

### Task 1: Add the three reference-only method cards

**Files:**
- Create: `orchestration/methods/cards/legacy-characterization-test.v1.json`
- Create: `orchestration/methods/cards/legacy-seam-extraction.v1.json`
- Create: `orchestration/methods/cards/refactor-small-step.v1.json`
- Modify: `orchestration/methods/catalog.v1.json:5-15`
- Test: `crates/code-intel-cli/tests/method_catalog.rs:61-114,187-260`

**Interfaces:**
- Consumes: existing `code-intel-method-card.v1` schema and existing card relationships.
- Produces: three registered cards with IDs `legacy-characterization-test`, `legacy-seam-extraction`, and `refactor-small-step`; every card keeps `executionPolicy` equal to `catalog_only_no_selection_or_execution`.

- [ ] **Step 1: Write the failing catalog assertions**

Add the three IDs to the expected stable list in `seed_catalog_loads_all_nine_methods_in_stable_order`, rename the test to `seed_catalog_loads_all_twelve_methods_in_stable_order`, change the distinct-card count from `9` to `12`, and add a focused assertion that each new card has a non-empty `requiredEvidence`, `deterministicSteps`, `contraindications`, and `applicabilityBoundary`.

```rust
let ids = catalog
    .cards()
    .iter()
    .map(|card| card["id"].as_str().unwrap())
    .collect::<Vec<_>>();
assert!(ids.contains(&"legacy-characterization-test"));
assert!(ids.contains(&"legacy-seam-extraction"));
assert!(ids.contains(&"refactor-small-step"));
```

- [ ] **Step 2: Run the method catalog tests and verify they fail**

Run:

```bash
cargo test -q -p code-intel --test method_catalog seed_catalog_loads_all_twelve_methods_in_stable_order -- --exact --nocapture
```

Expected: FAIL because the catalog still contains nine cards and the three files do not exist.

- [ ] **Step 3: Add `legacy-characterization-test.v1.json`**

Use the existing card shape. Define signals for unknown legacy behavior, missing characterization coverage, and high change risk. Require observed inputs/outputs/side effects/failure semantics and a runnable observation path. Define steps `select-observable-entry`, `capture-current-behavior`, and `freeze-characterization-contract`, producing `observable-contract`, `characterization-test`, and `behavior-baseline`. Relate it to `contract-testing` and `strangler-migration`. Mark `executionPolicy` as `catalog_only_no_selection_or_execution`.

- [ ] **Step 4: Add `legacy-seam-extraction.v1.json`**

Define signals for hidden dependencies, global state, mixed responsibilities, and untestable side effects. Require a call/dependency relation, side-effect inventory, and a preserved old-path contract. Define steps `locate-seam`, `choose-boundary`, and `verify-preserved-path`, producing `seam-map`, `boundary-change`, and `rollback-route`. Relate it to `legacy-characterization-test` and `strangler-migration`. Keep implementation ports advisory and deterministic-tool shaped; do not instruct the card to execute a rewrite.

- [ ] **Step 5: Add `refactor-small-step.v1.json`**

Define signals for code smells, excessive complexity, and boundary erosion. Require an executable behavior baseline, bounded changed scope, and before/after structural evidence. Define steps `freeze-behavior`, `apply-one-transform`, and `inspect-delta`, producing `refactor-slice`, `behavior-proof`, and `residual-smells`. Relate it to `legacy-characterization-test` and `contract-testing`. State that no behavior-changing migration is covered by this card.

- [ ] **Step 6: Register cards and run the focused tests**

Add the three entries to `catalog.v1.json` in the same stable ordering used by the current catalog, then run:

```bash
cargo test -q -p code-intel --test method_catalog -- --nocapture
```

Expected: PASS, with all existing cards still reference-only and all three new cards satisfying the catalog validator.

- [ ] **Step 7: Commit the method catalog slice**

```bash
git add orchestration/methods/catalog.v1.json orchestration/methods/cards/legacy-characterization-test.v1.json orchestration/methods/cards/legacy-seam-extraction.v1.json orchestration/methods/cards/refactor-small-step.v1.json crates/code-intel-cli/tests/method_catalog.rs
git commit -m "feat(methods): add legacy refactoring design cards"
```

---

### Task 2: Define proposal fixtures and failing contract tests

**Files:**
- Create: `crates/code-intel-cli/tests/fixtures/design-proposal/valid-two-option.json`
- Create: `crates/code-intel-cli/tests/fixtures/design-proposal/valid-three-option.json`
- Create: `crates/code-intel-cli/tests/fixtures/design-proposal/invalid-option-count.json`
- Create: `crates/code-intel-cli/tests/fixtures/design-proposal/invalid-recommendation.json`
- Create: `crates/code-intel-cli/tests/fixtures/design-proposal/stale-snapshot.json`
- Create: `crates/code-intel-cli/tests/fixtures/design-proposal/drifted-evidence.json`
- Create: `crates/code-intel-cli/tests/fixtures/design-proposal/missing-method-evidence.json`
- Create: `crates/code-intel-cli/tests/fixtures/design-proposal/authority-escalation.json`
- Create: `crates/code-intel-cli/tests/design_proposal.rs`

**Interfaces:**
- Consumes: capability execution entry `capability exec advisory.design-proposal.compat --request <file> --out <dir>` and the Task 1 method cards.
- Produces: failing tests that define exact request modes, input artifact identities, result shape, diagnostic rules, and staged-output behavior for the implementation task.

- [ ] **Step 1: Create a temporary repository and snapshot-bound request helper**

In `design_proposal.rs`, use the existing `tests/common.rs` CLI helper and a `TempTree` helper. Add helpers with these signatures:

```rust
fn temp_repo(label: &str) -> TempTree;
fn snapshot_for(repo: &Path) -> Value;
fn context_request(repo: &Path, out: &Path) -> Value;
fn validate_request(repo: &Path, context: &Path, candidate: &Path, out: &Path) -> Value;
fn run_capability(request: &Value, path: &Path, out: &Path) -> Output;
```

The request must set `capability` to `advisory.design-proposal`, `contractVersion` to `1`, and use `options.mode` values `context` or `validate`.

- [ ] **Step 2: Add the valid two-option and three-option fixtures**

Each candidate fixture must contain exact top-level keys `schema`, `kind`, `authority`, `snapshot`, `request`, `baseline`, `delta`, `methods`, `options`, `recommendation`, `risks`, `validationPlan`, and `limitations`. Use two options in the first fixture and three in the second. Each option must contain `id`, `title`, `summary`, `boundaryChanges`, `tradeoffs`, `assumptions`, `evidenceRefs`, `validationPlan`, and `reversibility`. The recommendation must reference an existing option.

- [ ] **Step 3: Add invalid fixtures and assertions**

Encode one failure per fixture: option count outside 2–3, missing recommendation target, context snapshot mismatch, evidence digest mismatch/drift, a selected method with absent required evidence, and `authority` set to `committed` instead of `advisory_only`. Assert the JSON result contains the exact proposal diagnostic rule and exits nonzero.

- [ ] **Step 4: Add the publication and cleanup assertions**

For a valid two-option request, assert the output directory contains the proposal payload and an ArtifactRef with `code-intel-design-proposal.v1`. For every failed validation, assert the proposal payload is absent and staged output is cleaned according to the existing capability executor contract.

- [ ] **Step 5: Run the new tests and verify they fail for the missing capability**

Run:

```bash
cargo test -q -p code-intel --test design_proposal -- --nocapture
```

Expected: FAIL because `advisory.design-proposal.compat` is not yet dispatched and its artifact contracts do not exist.

- [ ] **Step 6: Commit the contract fixtures and red tests**

```bash
git add crates/code-intel-cli/tests/design_proposal.rs crates/code-intel-cli/tests/fixtures/design-proposal
git commit -m "test(design): define proposal contract fixtures"
```

---

### Task 3: Implement the Design Context and Proposal Candidate validator

**Files:**
- Create: `crates/code-intel-cli/src/design_proposal.rs`
- Test: `crates/code-intel-cli/tests/design_proposal.rs`

**Interfaces:**
- Consumes: `request: &Value`, `verified_inputs: &[VerifiedArtifact]`, and `out: &Path` through the standard capability adapter signature.
- Produces: `pub(crate) fn execute(request, verified_inputs, out) -> Result<AdapterOutput, AdapterError>`; `context` mode emits `design.context`; `validate` mode emits `design.proposal`.

- [ ] **Step 1: Add the module skeleton and mode parser**

Create the standard entry point:

```rust
pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    match options.get("mode").and_then(Value::as_str) {
        Some("context") => build_context(request, verified_inputs, out),
        Some("validate") => validate_and_publish(request, verified_inputs, out),
        Some(other) => Err(AdapterError::InvalidOptions(format!("unsupported design proposal mode: {other}"))),
        None => Err(AdapterError::InvalidOptions("options.mode is required".into())),
    }
}
```

Reject unknown option keys and reject input artifact counts that do not match the selected mode.

- [ ] **Step 2: Implement `build_context` as a read-only evidence assembler**

Implement:

```rust
fn build_context(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError>;
```

Require the request snapshot and at least the declared current evidence inputs. Copy only verified snapshot-bound facts, artifact metadata, explicit constraints, known unknowns, selected method IDs, and required method evidence into `code-intel-design-context.v1`. Do not copy arbitrary Agent text into the context. Set the adapter domain verdict to `Pass` only when the context contract is complete; use `Unknown` with a domain failure when the requested evidence is unavailable without fabricating a completed context.

- [ ] **Step 3: Implement candidate and option validation**

Implement private validators with these signatures:

```rust
fn validate_candidate_shape(candidate: &Value) -> Result<(), AdapterError>;
fn validate_options(options: &[Value]) -> Result<(), AdapterError>;
fn validate_recommendation(recommendation: &Value, options: &[Value]) -> Result<(), AdapterError>;
fn validate_methods(candidate: &Value, context: &Value) -> Result<(), AdapterError>;
fn validate_evidence_refs(candidate: &Value, context: &Value) -> Result<(), AdapterError>;
fn validate_snapshot(candidate: &Value, context: &Value) -> Result<(), AdapterError>;
```

Enforce exact keys, 2–3 options, common comparison dimensions, option ID uniqueness, recommendation target existence, `authority=advisory_only`, explicit assumption markers, evidence reference membership, and same snapshot identity. Emit the proposal-specific rules from the spec: `proposal_invalid_shape`, `proposal_evidence_missing`, `proposal_evidence_drifted`, `proposal_snapshot_mismatch`, `proposal_method_not_applicable`, `proposal_option_reference_invalid`, `proposal_authority_escalation`, and `proposal_validation_unknown`.

- [ ] **Step 4: Implement `validate_and_publish`**

Require exactly one verified Design Context input and one verified Proposal Candidate input. Validate both before writing any result. Construct the validated result by preserving candidate comparison content while adding the verified context snapshot and normalized evidence metadata. Publish only through the existing `AdapterArtifact` return path and existing staged writer helper. Use `domain_verdict=Pass` for a fully validated proposal and `domain_verdict=Unknown` or `Fail` for the corresponding explicit validation outcome; never convert a failed validation into a successful artifact.

- [ ] **Step 5: Run the focused proposal tests and make them pass**

Run:

```bash
cargo test -q -p code-intel --test design_proposal -- --nocapture
```

Expected: the tests still fail only on dispatch or artifact registration until Task 4 is complete; direct module tests should pass once the validators are wired into the test harness.

- [ ] **Step 6: Commit the validator implementation**

```bash
git add crates/code-intel-cli/src/design_proposal.rs crates/code-intel-cli/tests/design_proposal.rs
git commit -m "feat(design): validate evidence-bound proposal candidates"
```

---

### Task 4: Register artifact contracts and dispatch the capability

**Files:**
- Modify: `crates/code-intel-cli/src/artifact_ref.rs` near `advisory_family_contract`
- Modify: `crates/code-intel-cli/src/capability_inventory.rs:14-52,86-130`
- Test: `crates/code-intel-cli/tests/artifact_ref.rs`
- Test: `crates/code-intel-cli/tests/capability_exec.rs`

**Interfaces:**
- Consumes: Task 3 `design_proposal::execute` and existing `ArtifactContract`/`AdapterOutput` interfaces.
- Produces: registered input/output contracts and dispatch identity `advisory.design-proposal.compat`.

- [ ] **Step 1: Add failing contract-registration assertions**

Add tests that call `registered_contract` with ArtifactRefs for:

```text
code-intel-design-context.v1 / design.context
code-intel-design-proposal-candidate.v1 / design.proposal-candidate
code-intel-design-proposal.v1 / design.proposal
```

Assert valid payloads pass and unknown keys, wrong authority, wrong option count, and malformed evidence references fail.

- [ ] **Step 2: Register the three artifact families**

Add an `advisory_family_contract` match for the three schema/type pairs. Use explicit size ceilings no larger than the existing advisory 8 MiB ceiling. Route payload validation to proposal-specific functions exposed from `design_proposal.rs` without duplicating the validator logic in `artifact_ref.rs`.

- [ ] **Step 3: Dispatch the capability**

Add:

```rust
#[path = "design_proposal.rs"]
mod design_proposal;
```

and the dispatch arm:

```rust
"advisory.design-proposal.compat" => {
    design_proposal::execute(request, verified_inputs, out)
}
```

Do not add a new top-level CLI command. The generic `capability exec` route remains the public execution seam for this first version.

- [ ] **Step 4: Run capability and artifact tests**

Run:

```bash
cargo test -q -p code-intel --test artifact_ref -- --nocapture
cargo test -q -p code-intel --test capability_exec design_proposal -- --nocapture
cargo test -q -p code-intel --test design_proposal -- --nocapture
```

Expected: PASS for contract registration, dispatch, valid publication, and all invalid candidate diagnostics.

- [ ] **Step 5: Commit runtime registration**

```bash
git add crates/code-intel-cli/src/artifact_ref.rs crates/code-intel-cli/src/capability_inventory.rs crates/code-intel-cli/tests/artifact_ref.rs crates/code-intel-cli/tests/capability_exec.rs
git commit -m "feat(capability): register design proposal artifacts"
```

---

### Task 5: Add orchestration and internalization declarations

**Files:**
- Modify: `orchestration/capability-contract.v1.json`
- Modify: `orchestration/integrations.json`
- Create: `orchestration/internalization/design-proposal.json`
- Modify: `crates/code-intel-cli/tests/capability_contract.rs`
- Modify: `crates/code-intel-cli/tests/capability_exec.rs`

**Interfaces:**
- Consumes: runtime dispatch identity, three artifact identities, and existing integration registry format.
- Produces: a pinned capability declaration with request/result schemas, explicit effects, implementation identity, and conformance test references.

- [ ] **Step 1: Add the failing declaration test**

Extend the capability contract tests to require one declaration for `advisory.design-proposal.compat`, with effects limited to repository read, committed-artifact read, method-catalog read, and local staged write. Assert no network or repository-mutation effect is declared.

- [ ] **Step 2: Declare the request and result contracts**

Add the request/result/ref envelope entries to `capability-contract.v1.json` using the exact schemas:

```text
code-intel-design-proposal-request.v1
code-intel-design-context.v1
code-intel-design-proposal-candidate.v1
code-intel-design-proposal.v1
```

Declare `authority=advisory_only` and the `design.context`, `design.proposal-candidate`, and `design.proposal` artifact types.

- [ ] **Step 3: Add the internalization record**

Create `orchestration/internalization/design-proposal.json` with implementation ID `design-proposal.rust.v1`, source path `crates/code-intel-cli/src/design_proposal.rs`, conformance path `crates/code-intel-cli/tests/design_proposal.rs`, explicit no-network/no-repository-mutation boundary, and exit criteria stating that Agent prose remains non-authoritative.

- [ ] **Step 4: Register the production integration**

Add the capability to the production integration registry with its command form:

```text
code-intel capability exec advisory.design-proposal.compat --request <request.json|-> --out <staging-dir> --artifact-root <run-root>
```

Declare both operation modes and all three artifact identities without adding a new command family.

- [ ] **Step 5: Repin and run declaration tests**

Run:

```bash
target/debug/code-intel repin --repo . --write
cargo test -q -p code-intel --test capability_contract -- --nocapture
cargo test -q -p code-intel --test declared_pins repin_reports_a_consistent_tree -- --exact --nocapture
```

Expected: no stale pins, declaration tests pass, and the new record has matching source/conformance digests.

- [ ] **Step 6: Commit orchestration declarations**

```bash
git add orchestration/capability-contract.v1.json orchestration/integrations.json orchestration/internalization/design-proposal.json crates/code-intel-cli/tests/capability_contract.rs crates/code-intel-cli/tests/capability_exec.rs
git commit -m "feat(orchestration): declare design proposal capability"
```

---

### Task 6: Complete adversarial verification and repository gates

**Files:**
- Modify: `crates/code-intel-cli/tests/design_proposal.rs` if a discovered contract gap needs a test-only correction
- Modify: `crates/code-intel-cli/tests/method_catalog.rs` if a discovered catalog assertion needs a test-only correction
- Modify: `orchestration/internalization/design-proposal.json` only when a source/conformance pin changes after final edits

**Interfaces:**
- Consumes: all implementation and declaration work from Tasks 1–5.
- Produces: reproducible evidence that the capability is complete, bounded, pinned, and non-authoritative.

- [ ] **Step 1: Run focused Rust suites**

Run:

```bash
cargo test -q -p code-intel --test method_catalog -- --nocapture
cargo test -q -p code-intel --test artifact_ref -- --nocapture
cargo test -q -p code-intel --test capability_contract -- --nocapture
cargo test -q -p code-intel --test design_proposal -- --nocapture
cargo test -q -p code-intel --test capability_exec design_proposal -- --nocapture
```

Expected: all focused suites pass, including valid two- and three-option proposals and every specified failure mode.

- [ ] **Step 2: Run the full workspace suite**

Run:

```bash
cargo test --workspace
```


Expected: all workspace tests pass. If an unrelated timing-sensitive test fails, rerun that exact test once and record the result without changing unrelated runtime code.

- [ ] **Step 3: Run final repository gates**

Run in this order:

```bash
target/debug/code-intel repin --repo . --write
target/debug/code-intel sentrux gate .
target/debug/code-intel lint hardcoded-paths
git diff --check
git status --short --branch
```

Expected: repin reports clean, Sentrux reports no degradation, hardcoded path scan is OK, diff check is clean, and the working tree contains only the intended implementation-plan branch changes.

- [ ] **Step 4: Review the artifact boundary manually**

Inspect one valid proposal artifact and confirm:

```text
authority = advisory_only
recommendation.optionId resolves to an option
all evidence refs use the context snapshot
no repository mutation occurred
no Agent prose was promoted to Engineering Fact
```

- [ ] **Step 5: Commit final pin updates and verification evidence**

```bash
git add crates/code-intel-cli/tests/design_proposal.rs crates/code-intel-cli/tests/method_catalog.rs orchestration/internalization/design-proposal.json orchestration/integrations.json orchestration/capability-contract.v1.json
git commit -m "test(design): verify evidence-bound proposal gates"
```

- [ ] **Step 6: Push and update the existing PR**

```bash
git push origin HEAD:issue-337-codenexus-rust
gh pr view 390 --json state,headRefName,baseRefName,url
```

Expected: PR #390 remains based on `main`, points to `issue-337-codenexus-rust`, and contains the final design capability commits.

## Plan self-review

- Spec coverage: goals, non-goals, Option A recommendation, two-stage Agent/Rust flow, proposal envelope, three method cards, explicit method selection, validation rules, allowed effects, verification cases, risks, touched assets, and rollback boundaries each map to Tasks 1–6.
- Authority coverage: the plan never gives the Agent repository mutation or promotion authority; every result remains advisory-only.
- Reuse coverage: the plan extends existing method catalog, capability inventory, ArtifactRef registration, generic capability execution, integration registry, and Run Commit rather than creating parallel infrastructure.
- Completeness scan: no deferred or unspecified implementation step is used. Open design choices from the spec are intentionally resolved here: generic capability route first, artifact-file Agent handoff, fixed comparison dimensions, and no automatic method selection.
- Type consistency: every runtime task uses the existing `execute(request, verified_inputs, out) -> Result<AdapterOutput, AdapterError>` seam; `context` and `validate` are explicit request modes; artifact schema/type identities are consistent across fixtures, Rust registration, and orchestration declarations.
