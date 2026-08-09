## 1. Admission and compatibility baseline

- [x] 1.1 Read active decision records, count open fix PRs, obtain the DR-0004 issue claim for this branch, audit all tracked workflow-recommender call sites, and record which v1 and PowerShell surfaces have active consumers.
- [x] 1.2 Run the mutation preflight in the dedicated worktree and start a Sentrux session before the first production edit.
- [x] 1.3 Add failing Rust and integration fixtures for current v1 parity, v2 empty effects, configured-versus-adopted state, competing roots, manual override, capability-driven selection, profile-dependent action availability, brownfield spec-kit, setup separation, and offline determinism.

## 2. V2 contracts and governed candidate data

- [x] 2.1 Add closed request, `code-intel-advisory-workflow-recommendation.v2`, candidate-catalog, and structured `entryActions` schemas; keep setup and maintenance actions separate and retain existing authority names.
- [x] 2.2 Add a validated pipeline-owned candidate catalog with OpenSpec v1.8.0 commit `d57889664cab4f2f061d236ec3ff82a5578701bb`, spec-kit v0.16.1 commit `ad4104b56c219b0a27bac06547d1a3c7d6a0dbd6`, MIT evidence, capability tags, profile or integration constraints, invocation templates, and no-runtime-dependency boundaries.
- [x] 2.3 Register v2 schema lifecycle, A01/A03 Artifact Ref validation, integration metadata, staged artifact, and run-commit coverage without changing the default DAG, Hospital, or protected authority transitions.

## 3. Rust-native recommendation

- [x] 3.1 Implement closed semantic intents and required-capability inputs; keep multilingual phrase aliases at host boundaries rather than embedding a natural-language parser in Rust.
- [x] 3.2 Implement deterministic presence and active-artifact detection that reports directories as configuration evidence, supplied approved authority refs as adoption evidence, and competing active roots as a normative-source conflict.
- [x] 3.3 Replace repository-age selection with ordered capability rules for continuation, OpenSpec delta governance, spec-kit constitution and convergence, lightweight bounded work, and explicit manual override evidence.
- [x] 3.4 Resolve entry-action availability from observed generated skills, profile or integration evidence, and candidate catalog constraints; never advertise supported-but-uninstalled actions as callable.
- [x] 3.5 Render proposal-only v2 and a deterministic v1 projection from one Rust model; preserve v1 schema validity and exclude setup commands from normative entry actions.
- [x] 3.6 Register `advisory.workflow-recommend.v2` as a Rust-native A01 capability and switch the existing v1 capability to the same Rust evaluator only after focused parity passes.

## 4. Compatibility retirement and documentation

- [x] 4.1 Convert the retained PowerShell workflow-recommendation entry point to a thin compiled-CLI forwarder; add no selection, activation, candidate, or adoption behavior to `.ps1`.
- [x] 4.2 Remove `legacy/OpenSpec-Detector.ps1` and duplicated inline policy only after invocation audit, normalized v1 parity, standalone compatibility, orchestration, and packaging evidence prove the Rust path covers active consumers.
- [x] 4.3 Update OpenSpec and spec-kit internalization records with exact revision, MIT, release, maintenance, capability, owned-boundary, rollback, and expiry evidence while retaining research/reimplement status and no adoption event.
- [x] 4.4 Update recommender and architecture docs: capability-based wording, brownfield support for both candidates, profile-dependent OpenSpec actions, structured activation intent, setup separation, conflict handling, manual override, and shipping/outcome handoff.
- [x] 4.5 Query `code-intel change impact` for final changed paths and update focused test selection; resync any orchestration pins once by literal replacement after all other edits.

## 5. Verification and shipping handoff

- [x] 5.1 Run focused Rust tests for the new evaluator plus `artifact_ref`, `capability_exec`, `internalization_record`, `run_commit`, `schema_lifecycle`, and `staged_artifact`; run v1/v2 and PowerShell compatibility parity fixtures with exact counts.
- [x] 5.2 Run relevant A01/A03 integration-contract checks, CLI binary tests, `cargo fmt --check`, metadata, workspace check/build, and stable-MSVC clippy; distinguish repository-existing warnings from regressions.
- [x] 5.3 Run orchestration parity, primary-entry and packaging smoke, declared-pin validation, and `legacy/tools/check-hardcoded-paths.ps1`; leave unrelated EOL or pin failures explicit instead of overwriting evidence.
- [x] 5.4 End the Sentrux session and require no structural regression; do not refresh a baseline to hide a failure.
- [x] 5.5 Run `openspec validate modernize-spec-workflow-adapters --strict`, check tasks only when their evidence exists, and produce a verified pre-checkpoint handoff for the tool-neutral shipping control loop; if that capability is not yet delivered, record the missing dependency and do not claim shipping completion.
