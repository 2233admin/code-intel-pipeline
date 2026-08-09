## Context

See `proposal.md` and `specs/spec-workflow-adapters/spec.md`. Today `advisory.workflow-recommend` returns v1 through A01 but invokes `legacy/Invoke-WorkflowRecommendation.ps1`, which reaches duplicated selection logic in PowerShell. V1 has free-form `entrySkills`, hard-codes the PowerShell implementation name, and cannot distinguish configured, active, recommended, adopted, or callable actions.

Current candidate records pin OpenSpec 1.7.0 and leave spec-kit revision and license unverified. Upstream verification now establishes OpenSpec v1.8.0 commit `d57889664cab4f2f061d236ec3ff82a5578701bb` and spec-kit v0.16.1 commit `ad4104b56c219b0a27bac06547d1a3c7d6a0dbd6`, both MIT. OpenSpec action availability is profile-dependent; version support alone does not mean a skill was generated. spec-kit supports brownfield work and a workflow engine, invalidating the greenfield-only rule.

## Goals / Non-Goals

**Goals:**

- Add a precise v2 proposal contract without breaking active v1 consumers.
- Make semantic intent, installation state, native invocation, and potential action effects machine-distinct.
- Move one deterministic recommendation policy into Rust and keep candidate facts as governed data.
- Make profile or integration availability visible without executing either candidate runtime.

**Non-Goals:**

- Adopting, initializing, upgrading, invoking, or vendoring OpenSpec or spec-kit.
- Parsing arbitrary natural language inside the kernel; hosts supply normalized intents.
- Building the shipping control loop defined by the separate change.
- Changing A05 authority vocabulary, Hospital, scores outside this advisory, or the default DAG.

## Decisions

### 1. Add v2 beside v1 and make projection explicit

Register a Rust-native `advisory.workflow-recommend.v2` capability and `code-intel-advisory-workflow-recommendation.v2`. Keep the existing `advisory.workflow-recommend` id and v1 schema during migration. Both are produced by one Rust evaluator; v1 is a deterministic projection that drops v2-only distinctions.

V2 keeps the top-level proposal envelope and empty effects. A candidate adds:

- adapter id, fit verdict, score, and reasons;
- presence state and active-artifact evidence;
- adoption state plus an optional supplied authority-event reference;
- `entryActions`, each with semantic intent, action id, availability, invocation map, prerequisites, and action effects;
- separate setup and maintenance actions;
- exact source revision, license, profile or integration observations, and evidence refs.

Creating a new capability id avoids changing an existing A01 output contract. Replacing v1 in place was rejected because strict consumers and historical artifacts name the current schema and implementation.

### 2. Normalize requested intent at the host boundary

The v2 request accepts a closed set of semantic intents and required capabilities. Hosts map phrases such as "定案", "开始做", "做完了吗", "开 PR", and "复盘" to those values. The Rust kernel does not embed a multilingual keyword parser.

This keeps activation wording testable and lets hosts evolve language aliases without changing recommendation semantics. Literal command strings remain output metadata, not routing authority.

### 3. Replace age classification with deterministic capability rules

The evaluator uses this precedence:

1. An approved adoption reference supplied by the caller is reported but never created.
2. Active artifacts for the same change prefer continuation unless competing active roots create a conflict.
3. Required capabilities determine fit: delta/archive/sync governance for OpenSpec; constitution/clarify/checklist/analyze/converge/composed workflow for spec-kit; bounded local work for lightweight.
4. An explicit user preference is recorded as a manual override of fit, still proposal-only.
5. Repository age, size, test presence, and CI maturity remain explanatory evidence only.

First-directory-wins and the `files > 5 && repoAgeDays > 90` split are deleted from production semantics. A configurable weighting engine was rejected because no measured outcome evidence supports learned weights.

### 4. Govern candidate facts in one validated data file

Add a small pipeline-owned candidate catalog under `orchestration/` containing exact provenance, license, capability tags, semantic action ids, profile or integration constraints, invocation templates, and setup classifications. Rust validates the catalog against a closed schema and performs all selection.

This follows the architecture map's direction to extract registered advisory candidates as data while keeping behavior in Rust. Loading upstream manifests or calling candidate CLIs at recommendation time was rejected because it adds availability and supply-chain effects to an offline advisory.

### 5. Detect availability from repository evidence, not version assumption

OpenSpec evidence includes project root, schema, generated skill names, and when available the configured profile or workflow selection. The known v1.8.0 action catalog distinguishes core and optional workflows; the result advertises only observed callable invocations and marks supported-but-uninstalled actions conditional.

spec-kit evidence similarly distinguishes `.specify/`, installed agent skills or commands, presets, extensions, bundles, and workflow definitions. Presence never means adoption. Setup commands are placed in `setupActions`, never projected into v1 `entrySkills` unless an existing compatibility test proves an active consumer requires the old string; any such exception is documented and scheduled for retirement.

### 6. Retire duplicated PowerShell logic after parity

Implementation order:

1. Add Rust fixtures reproducing current v1 outputs and edge cases.
2. Implement v2 evaluation and v1 projection in Rust.
3. Point A01 capability registration at the native adapter.
4. Convert `legacy/Invoke-WorkflowRecommendation.ps1` to a thin compiled-CLI forwarder.
5. Remove the independent `legacy/OpenSpec-Detector.ps1` policy and duplicated inline detector only after integration-contract parity proves no active consumer needs them.

No new product behavior is added to PowerShell. Keeping the old detector as a fallback was rejected because two policy implementations caused the drift this change fixes.

### 7. Update provenance without granting runtime adoption

Update both internalization records with exact tag commits, MIT evidence, release dates, maintenance check evidence, and corrected capability descriptions. Their rung remains `reimplement`, lifecycle remains research unless a separate authority event changes it, and owned boundaries continue to prohibit candidate-runtime invocation or initialization.

Exact provenance supports reproducible wording; it is not an adoption decision.

### 8. Keep shipping and outcomes out of adapter recommendation

V2 can return semantic handoffs for `ship` and `observe`, but their adapter is the tool-neutral control loop or outcome ledger, not OpenSpec or spec-kit. Until the separate control-loop change is delivered, these handoffs are reported unavailable with the missing capability named.

This prevents a planning adapter from claiming pull-request, merge, or improvement authority.

## Risks / Trade-offs

- **Two schema versions increase migration surface.** Mitigation: one Rust model, explicit projection, usage evidence, and a retirement criterion for v1.
- **Installed actions can drift from global profile state.** Mitigation: report repository-observed callability and provenance separately; never infer from version alone.
- **A static catalog can become stale.** Mitigation: exact revision pins, expiry dates, upstream fixture checks, and outcome-ledger evaluation; no runtime network lookup.
- **Removing PowerShell duplicates may expose hidden consumers.** Mitigation: search tracked invocation sites, retain one thin facade, run orchestration parity and packaging smoke before deletion.
- **Manual intent normalization can differ across hosts.** Mitigation: closed intent vocabulary, conformance fixtures, and host aliases tested outside the Rust policy.

## Migration Plan

1. Add v2 schema, candidate-catalog schema and data, lifecycle entries, and exact provenance fixtures.
2. Add Rust request normalization, selection, v2 rendering, and v1 projection with focused tests.
3. Register the new v2 capability and keep the v1 capability unchanged at its public boundary.
4. Switch v1 production execution to the Rust evaluator and prove A01/A03 plus standalone parity.
5. Thin the retained PowerShell facade, remove duplicate policy only after invocation audit, and update pinned digests by literal replacement.
6. Update activation wording, profile-dependent availability docs, and hardcoded-path-safe examples.
7. Use the tool-neutral control loop, when available, to ship this change and record its outcome against the prior recommender baseline.

Rollback disables the v2 capability and restores the v1 route to its Rust projection. Candidate facts and historical v2 artifacts remain readable; no tool installation or repository adoption needs reversal.

## Apply Evidence

The 2026-08-09 admission audit found three active compatibility consumers. `legacy/run-code-intel.ps1` invokes the v1 A01 capability and reads `workflow-recommendation.json`; `legacy/Invoke-WorkflowRecommendation.ps1` is the retained standalone facade; and `legacy/scripts/tests/test-workflow-recommendation-brief.ps1` exercises both the facade and the detector directly. The Rust `capability_exec` integration test also requires normalized facade parity. Documentation, internalization records, and retirement packets are descriptive or historical consumers and do not justify a second production evaluator.

Issue `#255` is claimed by branch `codex/modernize-spec-workflow-adapters`. The mutation preflight was clean and Sentrux session `20260809-162502` started before the first production edit (quality 4265, coupling 65.67, cycles 0, god files 31).

The first A06/A07 aggregate run exposed one change-local contract omission: the newly registered `code-intel-authority-event.v1` A03 input had no standalone published schema. The minimal correction publishes a closed schema, registers it in schema lifecycle, and validates the real approved-event fixture against that schema before A03 consumption. This does not add an authority name or change A05 transitions; it makes the already reused authority event independently publishable and fail-closed.

Focused verification after that correction passed: `artifact_ref` 9/9; `capability_exec` 44/44 (the previous 38 plus six v2 scenarios); `internalization_record` 58/58; `run_commit` 296/296 including the real v2 A06/A07 replay path; schema lifecycle 2/2; staged artifact 16/16; and the CLI binary 692/692. The standalone PowerShell facade/v1 projection test, compatibility-facade audit, explicit current-manifest orchestration validation, parity fixtures, integration smoke, primary launchers, Rust primary-entry 8/8, and the full six-step pipeline smoke all passed. The pipeline smoke had zero effective failures and one explicitly allowed graph-missing manual step.

Release packaging was exercised without changing the real Git index: an isolated temporary index represented the final deletion/addition set, then the beta packager and verifier checked 996 files and 80 dependencies, with zero PowerShell parser errors and passing CLI and packaged-wrapper smoke. Declared-pin validation passed 1/1 and a final repin report was clean with no stale or orphaned pins. The hardcoded-path scan passed over the final legacy file set (128 files). `cargo fmt --check`, metadata, workspace check/build, and stable MSVC Rust 1.97.1 clippy passed; clippy emitted only the repository's existing warning class.

The final change-impact query covered 37 changed paths and returned 23 impacted files plus six test-file candidates. Its committed evidence snapshot `613695fc50982518d847bb2f4a101c523fe6e63c8f9d39197a61ee6b1c1d5926` differs from current snapshot `97857b417965d3cec7d5a6d5981249e1db917136a0f665048d05d284bba659e3`, so the result is recorded only as `stale-advisory` with `advisoryOnly:true`; it did not gate or replace the executed test set.

The first Sentrux end correctly rejected the implementation at quality 4264 and coupling 65.79 versus the original session's quality 4265 and coupling 65.67; cycles and god files stayed at 0 and 31. The fix removed duplicate import edges inside the new adapter boundary and routed authority-event canonical hashing through the existing content-contract primitive. Repair session `20260809-173011` then passed at quality 4267, coupling 65.41, cycles 0, and god files 31. These absolute final values improve on the original session baseline; no repository baseline was changed to conceal the initial failure.

The broader historical retirement-packet suite was inspected but is not used as adoption or active-consumer evidence here. E02 still freezes the deleted detector path, while E03/E04/E07/E08 freeze shared-source snapshot identities changed by this work. Those attestations are intentionally excluded from ordinary repin and were not bulk re-signed to manufacture a green result. The active compatibility facade audit itself passes with the retained-surface count corrected from eleven to ten.

Shipping remains outside this change. `adopt-tool-neutral-agent-shipping-control-loop` is the required consumer for checkpoint, independent review, hosted checks, PR, human merge, and outcome-ledger observation; until it is delivered this apply result is a verified pre-checkpoint implementation, not a shipped or completed adoption.
