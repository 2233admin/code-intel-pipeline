# Task 6 fix report — design proposal verification regressions

## Result

The two reported regressions were repaired, and the additional failures exposed after the integration targets compiled were addressed:

- ArtifactRef fixtures now use the current `capability_inventory.rs` implementation digest.
- Design-proposal validation is behind a shared module owned by `artifact_ref`, so `artifact_ref -> capability_inventory` is no longer an import edge. The same strict validators are used by ArtifactRef registration and the design adapter.
- The shared validator module re-includes `method_catalog.rs` only under `cfg(test)` through the existing `#[path]` architecture. Production uses the single crate-root catalog module.
- The three registered design-proposal ArtifactRef identities now have concrete schemas under `orchestration/schemas/`.

Fix commit: `b2b09a3 fix(cli): close design proposal verification regressions`

## Changes

- `crates/code-intel-cli/src/design_proposal_contract.rs`
  - New shared payload parser, shape validators, catalog binding, and error formatting extracted from the adapter. No validation rule was relaxed.
- `crates/code-intel-cli/src/artifact_ref.rs`
  - Owns the shared validator module and uses it for the three design-proposal ArtifactRef contracts. This removes the cycle through `capability_inventory`.
- `crates/code-intel-cli/src/design_proposal.rs`
  - Retains execution/publication behavior and delegates all proposal validation to the shared module.
- `crates/code-intel-cli/tests/artifact_ref.rs`
  - Updates `IMPLEMENTATION_DIGEST` from stale `5090efd13c07531c249637d8e5857f0d13f3ecb8f0d02fb6e858747ea7d8c3d8` to current `264ed4390fbf70e6d1eaf0365f318b8587e4d2d88aa38dd344e9a0a9fbcc35cc`.
- `orchestration/schemas/code-intel-design-context.v1.schema.json`
- `orchestration/schemas/code-intel-design-proposal-candidate.v1.schema.json`
- `orchestration/schemas/code-intel-design-proposal.v1.schema.json`
  - Add closed-object JSON Schemas matching the registered artifact identities and strict Rust contracts.
- `orchestration/integrations.json` and `orchestration/internalization/design-proposal.json`
  - Updated only by the compiled repin command.

## Verification

All commands ran from the repository root.

| Command | Exit | Output summary |
|---|---:|---|
| `cargo test -q -p code-intel --test method_catalog -- --nocapture` | 0 | 9 passed |
| `cargo test -q -p code-intel --test artifact_ref -- --nocapture` | 0 | 10 passed |
| `cargo test -q -p code-intel --test capability_contract -- --nocapture` | 0 | 6 passed |
| `cargo test -q -p code-intel --test design_proposal -- --nocapture` | 0 | 10 passed |
| `cargo test -q -p code-intel --test capability_exec -- --nocapture` | 0 | 39 passed |
| `cargo test -q -p code-intel --test declared_pins repin_reports_a_consistent_tree -- --exact --nocapture` | 0 | 1 passed |
| `cargo test -q -p code-intel --test graph_adapter -- --nocapture` | 0 | 421 passed; no import-cycle failure |
| `target/debug/code-intel.exe lint hardcoded-paths` | 0 | `Hardcoded path scan: OK (292 files)` |
| `git diff --check` | 0 | no whitespace errors |
| placeholder/bypass search in changed Rust sources | 0 | no `TODO: implement`, `test.skip`, `test.only`, `unimplemented`, `no-op`, `noop`, or `stub` matches |

The compiled repin command was run after the final source/schema edits:

```text
target/debug/code-intel.exe repin --repo . --write
```

Final repin output included:

```text
repin: orchestration/internalization/design-proposal.json declared 09d171f5ba29 -> af1110b8b612 (crates/code-intel-cli/src/design_proposal.rs)
orchestration/integrations.json ccc36c4a166e...2a36b707 -> be50219e3762...f53da781 (3x, source: crates/code-intel-cli/src/artifact_ref.rs)
repin: rewrote 3 substitution(s) across 1 file(s) in 2 pass(es); 0 orphaned pin(s), 0 ambiguous digest(s)
```

## Workspace and external gates

`cargo test --workspace` now compiles all targets and no longer reports the missing `crate::method_catalog` module, missing design schemas, or the import-cycle failures. The final workspace run still exits 101 because nine unrelated `method_select` tests fail with:

```text
SelectError("rule table must contain exactly one rule for every C01 method card")
```

Those failures are outside the files changed by this repair and arise from the existing method-selection rule/card inventory mismatch. They are not concealed or baselined here.

The compiled Sentrux gate remains blocked by the repository's absent baseline, with exit 1:

```text
Sentrux baseline missing at ...\\crates\\code-intel-cli\\.sentrux\\baseline.json
Run code-intel sentrux --operation save_baseline --repo ...\\crates\\code-intel-cli
```

No baseline was generated because that would be an unrequested repository mutation. The compiled hardcoded-path gate passes.

## Pin chronology

A preliminary repin was run after the initially reported two-file repair. The subsequent workspace failures required the shared-module/schema fixes, followed by the final repin shown above. The final pin state passes the exact `declared_pins` test and has no stale pins.

## Final status

```text
## agent/issue-337-codenexus-rust...origin/issue-337-codenexus-rust [ahead 18]
```

The fix commit is `b2b09a3`; the working tree was clean immediately after that commit. Residual risks are the missing Sentrux baseline and the pre-existing method-selection rule/card mismatch noted above.
