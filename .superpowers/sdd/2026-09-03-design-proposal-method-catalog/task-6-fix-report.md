# Task 6 fix report — design proposal verification regressions

## Result

The reported ArtifactRef and design-proposal regressions were repaired, and the feature-induced method-selection failures exposed by the workspace run were fixed:

- ArtifactRef fixtures now use the current capability_inventory.rs implementation digest.
- Design-proposal validation is behind a shared module owned by artifact_ref, removing the artifact_ref -> capability_inventory import edge while preserving strict validation.
- Production uses the single crate-root method catalog; test targets retain the existing #[path] inclusion pattern.
- The three registered design-proposal ArtifactRef identities have concrete schemas under orchestration/schemas/.
- The method-selection rule table now contains one validated rule for each of the twelve catalog cards, including the three cards added by Task 1.

Original fix commit: `b2b09a3 fix(cli): close design proposal verification regressions`
Follow-up fix commit: `aa628e1 fix(cli): complete method selection and proposal schemas`

## Changes

- crates/code-intel-cli/src/design_proposal_contract.rs
  - Shared payload parser, shape validators, catalog binding, and error formatting. No validation rule was relaxed.
- crates/code-intel-cli/tests/artifact_ref.rs
  - Updates the stale implementation digest 5090efd13c07531c249637d8e5857f0d13f3ecb8f0d02fb6e858747ea7d8c3d8 to 264ed4390fbf70e6d1eaf0365f318b8587e4d2d88aa38dd344e9a0a9fbcc35cc.
- orchestration/method-selection-rules.v1.json
  - Adds rules for legacy-characterization-test, legacy-seam-extraction, and refactor-small-step using each card's declared signals and contraindications.
- orchestration/schemas/code-intel-design-proposal-candidate.v1.schema.json
- orchestration/schemas/code-intel-design-proposal.v1.schema.json
  - Match the shared Rust validator: option boundaryChanges and validationPlan require non-empty arrays, while evidenceIds remains a non-empty array whose string items may be empty.
- crates/code-intel-cli/tests/design_proposal.rs
  - Adds a regression check for the proposal schema array contract.
- orchestration/internalization/design-proposal.json
  - Updated by the compiled repin command after the final test edit.

## Verification

All commands ran from the repository root.

| Command | Exit | Output summary |
|---|---:|---|
| `cargo test -q -p code-intel --test method_select -- --nocapture` | 0 | 425 passed |
| `cargo test -q -p code-intel --test method_catalog -- --nocapture` | 0 | 9 passed |
| `cargo test -q -p code-intel --test artifact_ref -- --nocapture` | 0 | 10 passed |
| `cargo test -q -p code-intel --test capability_contract -- --nocapture` | 0 | 6 passed |
| `cargo test -q -p code-intel --test design_proposal -- --nocapture` | 0 | 11 passed |
| `cargo test -q -p code-intel --test capability_exec -- --nocapture` | 0 | 39 passed |
| `cargo test -q -p code-intel --test graph_adapter -- --nocapture` | 0 | 421 passed |
| `cargo test --workspace` | 101 | One unrelated native_code_evidence test failed; method-selection and proposal/artifact/capability suites passed |
| `target/debug/code-intel.exe repin --repo . --write` | 0 | Rewrote 3 substitutions; no orphaned or ambiguous pins |
| `git diff --check` | 0 | No whitespace errors |

The new schema regression test passed as part of the 11 design_proposal tests. The exact declared-pins test passed after the final repin, and the compiled hardcoded-path lint gate passed in the original repair.

## Workspace and external gates

The method-selection failures were feature-induced: Task 1 added three catalog cards while the rule table still contained nine entries. The repaired table restores the one-rule-per-card invariant without weakening count validation.

The workspace run still exits 101 because native_code_evidence::a01_a09_artifacts_match_the_real_legacy_producer_on_the_same_fixture fails with `request implementation differs from declaration`. This failure is unrelated to the method-rule/schema changes; no orchestration semantics were changed to mask it.

The compiled Sentrux gate remains blocked by the repository's absent baseline, with exit 1:

```text
Sentrux baseline missing at ...\\crates\\code-intel-cli\\.sentrux\\baseline.json
Run code-intel sentrux --operation save_baseline --repo ...\\crates\\code-intel-cli
```

No baseline was generated because that would be an unrequested repository mutation.

## Pin chronology

A preliminary repin followed the initially reported two-file repair. After the follow-up method-rule, schema, and schema-test edits, the compiled repin was run once and updated the design_proposal.rs test pin. No further repin was run.

## Final status

```text
## agent/issue-337-codenexus-rust...origin/issue-337-codenexus-rust [ahead 21]
```

Residual gates are the missing Sentrux baseline and the unrelated legacy producer declaration mismatch reported by cargo test --workspace.