# T2 launcher classification (issue #47)

Status: historical classification; the compiled Rust authority described below is implemented.

This document preserves measurements and retirement reasoning from `origin/main` at `02f0444`.
Statements phrased as “today”, “current”, or “to port” describe that historical checkpoint, not
the living runtime contract. The current production path is `code-intel run execute`; retained
PowerShell is compatibility/advisory only and does not create repository-run authority.

Working document for retiring `legacy/run-code-intel.ps1` (4742 lines, 90
function definitions, 72 parameters) into `code-intel run` plus a ≤50-line
thin forwarder.

Every claim below is a measurement taken against `origin/main` at
`02f0444`, not an estimate. Uncertain rows are marked `?` with the reason,
following the T1 discipline in [contract-inventory.md](contract-inventory.md).

## 0. The blocker T1 did not name

`run-code-intel.ps1` is not only an entry point. It is also a **function
library that two PowerShell test suites dot-source out of**, via
`Get-ScriptFunctionsSource -Path .../run-code-intel.ps1 -Only @(...)`:

| test file | functions pulled | subject |
|---|---|---|
| `legacy/scripts/tests/test-hospital-trust-contract.ps1` | 18 | hospital state machine, diagnosis, scoring |
| `legacy/scripts/tests/test-regression-fixes.ps1` | 9 | code-evidence symbol extraction (6 languages) |
| `legacy/scripts/tests/test-regression-fixes.ps1` | 3 | sentrux gate metric deltas / no-degradation |
| `legacy/scripts/tests/test-regression-fixes.ps1` | 4 | hospital state machine + surgery target |

34 function references in total. **Shrinking the launcher to a forwarder
deletes the subject of those tests.** No amount of parameter forwarding
avoids this: the tests reach past the parameter surface into the file body.

This is why the ticket's exit criterion (`≤50 lines`) cannot be met by
routing alone, and why T2 necessarily touches test assets that the campaign
map assigns to T7.

### Why retiring them is safe rather than a coverage loss

Both subjects already have native implementations with their own integration
suites on the Rust side:

| PowerShell subject | Rust owner | Rust test suite |
|---|---|---|
| hospital state machine / diagnosis / scoring | `crates/code-intel-cli/src/hospital_diagnosis.rs` (558 lines) | `tests/hospital_diagnosis.rs` — 978 lines, 10 tests |
| code-evidence symbol extraction | `crates/code-intel-cli/src/native_code_evidence.rs` (718 lines) | `tests/native_code_evidence.rs` — 511 lines, 6 tests |
| sentrux gate metric deltas | `crates/code-intel-cli/src/sentrux_gate.rs` (1080 lines) | exercised by the repo's own `sentrux gate` self-scan in CI |

The PowerShell copies are duplicate implementations of logic the Rust DAG
already runs in production — `diagnosis.hospital` and `evidence.native-code`
are live DAG nodes in every `code-intel run execute`. Retiring the
PowerShell copies removes duplication, not coverage.

**Not yet verified, and required before deletion (`?`):** that each of the 16
Rust tests asserts the *same* behaviour as the PowerShell test it would
replace, case for case. A per-function mapping table has to land in the
retirement PR; "a Rust suite exists" is not by itself evidence that a
specific assertion survives. Treat any PowerShell case with no Rust
counterpart as a gap to close before the file is deleted, not after.

## 1. Parameter classification

72 parameters. The three buckets the ticket asks for.

### 1.1 Already covered by `code-intel run execute`

The Rust runner already accepts these concepts; T2 only needs flag aliases
and the config-alias resolution the PowerShell side did.

| PowerShell | Rust today | note |
|---|---|---|
| `-RepoPath` | `--repo` | direct |
| `-Repo` (alias via `pipeline.config.json`) | — | alias resolution itself must port; `code-intel doctor bootstrap` already ports the same `repos`/`sentruxPath` lookup in `doctor_bootstrap/config.rs` and can be reused |
| `-ArtifactRoot` | `--authority-root` / `--out` | staging vs authority split differs; see §3 |
| `-Platform` | implicit from target | no flag needed |
| `-DagCoordinate` | `run dag-coordinate` | already a subcommand |

### 1.2 To port

Mode and the Skip/Allow switches are the substance. The architectural home
is already there: `ExecutionPolicy` in `crates/code-intel-cli/src/execution_policy.rs`
compiles a profile into an immutable `ProviderPolicy`, and `dag_run.rs`
already gates node inclusion on `policy.capability_enabled(...)` and
`policy.provider_diagnosis_enabled()`. Skip flags become policy inputs, not
branches in a script.

Load-bearing property to preserve: `with_doctor_overrides` refuses to weaken
`Strict` or re-enable `Offline`. Any new Skip flag must inherit that
guarantee, or a compatibility flag becomes a way to silently disarm a strict
run.

| behaviour | current PS1 anchor | target |
|---|---|---|
| `-Mode lite\|normal\|full` | mode gates the sentrux stage and repowise docs | a `RunProfile`-adjacent input, not a fourth profile — see §2 |
| `-SkipRepowise` / `-SkipSentrux` / `-SkipSentruxCheck` / `-SkipSentruxGate` | per-stage gates | `ProviderRequirement::Disabled` on the matching capability |
| `-SkipOpenSpec` / `-AutoOpenSpec` | OpenSpec stage | **ported.** `advisory.workflow-recommend` is a standalone `run execute`/`dag-coordinate` DAG node gated by `ProviderPolicy::open_spec` (Optional under Default/Offline-disabled, Required under Strict/Compatibility); `-SkipOpenSpec` maps to `--skip-open-spec` narrowing that requirement, `-AutoOpenSpec` maps to `--auto-open-spec`, a passthrough option independent of the requirement (`ExecutionPolicy::with_open_spec_auto`) |
| `-SkipRepomix` / `-RepomixStyle` / `-RepomixCompress` | external `repomix` invocation | `?` — no Rust node; may belong in the dead bucket if unconsumed |
| `-RequireUnderstandGraph` | missing graph is fatal vs advisory | maps onto `providers.understand` requirement |
| `-SaveSentruxBaseline` / `-AutoSaveMissingSentruxBaseline` | writes `.sentrux/baseline.json` inside the scanned repo | needs an explicit effect declaration; writing into the scanned tree is a declared effect, not a side effect |
| `-InventoryExclude` | feeds the `rg` inventory | `inventory.rg` node options |
| `-ProactiveSkillSuggestions` / `-AutomaticPullRequests` / `-BugSkill` | follow-up automation block in the hospital report | `?` — consumer is the hospital report renderer |
| multi-artifact publishing (`report.json`, `summary.md`, `understanding.md`, `hospital.md`) | `:4179-4182` area | Rust publishes a run manifest; the markdown views are PS1-only today |

### 1.3 Declare dead

| item | evidence | why dead |
|---|---|---|
| `-SkipGitHubResearch` | T1 §1.1 and §7.1: zero other references to `$SkipGitHubResearch`; `$githubResearch` is unconditionally the `New-GitHubSolutionResearchNotApplicable` stub regardless of the flag | the switch has no effect at all — five test files pass it and none assert on it, because there is nothing to assert |
| `Test-GitHubSolutionResearchRequired` | T1 §1.1: defined, zero call sites | dead function reachable only by dot-sourcing |

Both were recorded by T1 as findings and deliberately left unfixed there
("record, don't fix"). T2 is the ticket that removes them.

**Status (2026-08-07): removed.** `$SkipGitHubResearch` and
`Test-GitHubSolutionResearchRequired` are deleted from `run-code-intel.ps1`.
Every real caller that threaded the no-op flag through — `invoke-code-intel.ps1`'s
own dead passthrough param, `test-code-intel-pipeline.ps1`'s param and
forwarding block, `test-ps1-rust-parity.ps1`, `test-transactional-publication.ps1`,
`test-runtime-ci-hospital-pet.ps1`, `test-code-evidence-layer.ps1`,
`Invoke-NativeRetrievalBenchmark.ps1`, `Invoke-CccSliceBenchmark.ps1`,
`Invoke-CodeEvidenceABTest.ps1`, `ci.yml` (both self-scan jobs),
`release.yml`, and `orchestration/code-intel-project-conformance-policy.v1.json`
— had the argument dropped too. `legacy/scripts/tests/test-github-solution-research.ps1`
was **not** touched: it exercises `Invoke-GitHubSolutionResearch.ps1`'s own,
unrelated `-SkipGitHubResearch` switch, a same-name flag on a different
script. Verified: pwsh AST parse on every edited `.ps1`, YAML/JSON parse on
the workflow and policy files, `cargo test -p code-intel --test
native_code_evidence` (6 passed), and every directly-touched PowerShell
contract test run standalone (`test-ps1-rust-parity.ps1`,
`test-transactional-publication.ps1`, `test-runtime-ci-hospital-pet.ps1`,
`test-code-evidence-layer.ps1`, `test-code-intel-pipeline.ps1`,
`test-github-solution-research.ps1`) — all exit 0. The T1 parity harness's
overall `ok:false` verdict is pre-existing (documented node-coverage and
hospital-shape drift this same document already catalogs in §0/§3, plus a
worktree-local `doctor`/`sentrux` self-scan false-negative unrelated to this
change), not something this removal introduced.

The rest of §1.2/§1.3/§1.4 (flag porting, remaining dead-item removal, the
route-away items) and the launcher shrink itself (blocked on the §0
dot-source test migration) are still open.

### 1.4 Route away rather than port

The eight early-exit facades (T1 §1.8) are already thin forwarders into the
Rust CLI. Their retirement work is *stop routing through the launcher*, not
*port logic*, because the logic already lives in Rust:

`-ModelInventoryResult`, `-ModelAdapterRequest`, `-RepowiseAdapterRequest`,
`-GraphAdapterRequest`, `-SentruxAdapterRequest`, `-CodeNexusAdapterRequest`,
`-SurvivalScanRequest`, `-RunCommitManifestRef` (plus their companion
`*ArtifactRoot`/`*EvaluatedAt`/`*MaxAgeSeconds` parameters).

Each already resolves `target/debug/<exe>`, invokes a subcommand, and
`exit $LASTEXITCODE`. Callers should invoke `code-intel` directly; the
forwarder does not need to reproduce them.

`-SentruxAdapterArtifactRoot`, `-SentruxAdapterEvaluatedAt` and
`-SentruxAdapterMaxAgeSeconds` are additionally never passed by any live
caller — measured across every non-frozen `.ps1`/`.yml` in the tree.

## 2. Open design question: what `-Mode` becomes

`RunProfile` already has four values (`Default`, `Strict`, `Offline`,
`Compatibility`) that describe *provider requirement policy*. `-Mode`
(`lite`/`normal`/`full`) describes *how much work to do*. These are
orthogonal — a strict lite run is meaningful — so `-Mode` must not be
flattened into `RunProfile`.

Recommended: a separate `--mode` input that adjusts which optional nodes
enter the DAG, composed with, never overriding, the profile's requirement
policy. Deciding this before writing code matters, because collapsing the
two axes is the kind of mistake that is expensive to reverse once the flag
is public.

## 3. Contract divergences recorded for the port

The following were carried from T1 §5 and §7 at `02f0444`. Current-state annotations distinguish
resolved behavior from historical evidence:

1. **Exit codes.** `run execute` distinguishes `10` (architecture/domain
   gate) from `70` (process failure). The launcher only ever exits `0` or
   `1`. The forwarder must decide whether to propagate the richer code or
   collapse it; propagating is preferable but is a behaviour change for
   anything parsing the old binary outcome.
2. **Publication authority (resolved).** At the measured checkpoint, both paths wrote
   `run-complete.json` with unrelated shapes. That is no longer current behavior: only the
   compiled Run Commit path writes canonical `code-intel-run-commit.v1`. The PowerShell report
   path atomically promotes an advisory compatibility directory and writes no canonical marker.
3. **Hospital shape drift.** Both producers claim schema
   `code-intel-hospital.v1`; PS1 nests its verdict under `triage`, Rust under
   `diagnosis` + `domainVerdict`. The schema string is not currently a
   guarantee of shape compatibility.
4. **Two paths to "sentrux skipped".** `-Mode lite` and `-SkipSentrux`
   produce the same user-visible outcome with different `error` text. The
   port has to pick one.

## 4. Sequencing

1. Land this classification. *(this commit)*
2. Resolve §2 (`--mode` axis) and §3.1 (exit-code contract) — both are
   public-surface decisions that are cheap now and expensive later.
3. Port §1.2 into `ExecutionPolicy` + `dag_run`, with the strict/offline
   non-weakening guarantee extended to every new switch.
4. Build the per-function mapping table for §0 and close any assertion gaps
   on the Rust side.
5. Delete the dead items in §1.3, retire the duplicated PowerShell functions
   and their tests, shrink the launcher to a forwarder.
6. Repoint `ci.yml`, `release.yml`, `README.md` and the skill docs to the
   binary; keep the forwarder as a documented compatibility entry point.
7. T1 parity harness green across the forwarder and the direct path.
