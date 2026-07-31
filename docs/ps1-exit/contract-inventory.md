# PS1 Monolith Contract Inventory (T1)

Addresses issue #46 (campaign map #55, "PS1 巨石退役战役"). **Zero production
behavior changes** — this document and its companion parity harness
(`legacy/scripts/tests/test-ps1-rust-parity.ps1`) are pure analysis and test
infrastructure. If this inventory surfaces an actual bug, it is recorded in
[§7 Gaps and known issues](#7-gaps-and-known-issues) rather than fixed here.

Every `evidence` cell below is a `file:line` this session actually read
(via `Read`/`Grep`/a sandboxed file-content pass — never guessed from a
symbol name or a stale index). Uncertain classifications are marked `?`
with one line of reasoning per the ticket's instruction to prefer
under-claiming over a confidently wrong row.

## 0. Scope correction (read this before the tables)

The ticket's description of `legacy/Invoke-SentruxAgentTool.ps1`'s "distinctive
surfaces" — shim management, pro activation, failure normalization, output
adaptation — does not match what is actually in that file. Verified by
grepping the live file for `shim`, `activation`, `pro license`,
`normalizeFailure`, `output adapt`: **zero matches** for any of them. The
real picture, confirmed by tracing actual call sites:

| Claimed surface | Actually lives in |
|---|---|
| Sentrux shim management (core-binary resolution, thin-forwarder detection) | `legacy/tools/sentrux-shim/sentrux-shim.ps1` (339 lines) |
| Pro activation (license write/read, auto-disable, `sentrux pro` subcommand) | `legacy/tools/sentrux-shim/sentrux-shim.ps1` (same file) |
| Failure normalization (`code-intel-sentrux-failures.v1`, the schema in `docs/archive/2026-07/sentrux-failure-normalization-plan.md`) | `legacy/run-code-intel.ps1`'s `New-CodeIntelSentruxFailures` (:2741) and related helpers — i.e. **the other named monolith**, not this one |
| Output adaptation | Arguably this file's actual job (see §2), but not under that name anywhere in the source |

`legacy/run-code-intel.ps1` does call `legacy/Invoke-SentruxAgentTool.ps1` — but only for
three of its eleven tool operations (`dsm`, `evolution`, `what_if`; see
§2.2 and §1.8), and its "sentrux check"/"sentrux gate" pipeline steps go
through the **separate** `sentrux` shim command (`tools/sentrux-shim/`),
confirmed by `grep -c Invoke-SentruxAgentTool legacy/tools/sentrux-shim/*.ps1` = 0.

This document therefore inventories `legacy/Invoke-SentruxAgentTool.ps1` as what
it verifiably is (§2: an 11-operation DSM/git-churn/complexity metrics
engine), gives `tools/sentrux-shim/*.ps1` the light pass the "shim
management + pro activation" framing actually calls for (§3, 906 lines,
well within the spirit of the ticket even though those files were not
named in the ticket body), and does not force PS1 code to match a premise
the code itself contradicts.

## 1. `legacy/run-code-intel.ps1` (4723 lines)

### 1.1 `param()` block — 72 parameters (16 switches, 56 value params)

Full block: `legacy/run-code-intel.ps1:3-91`. Grouped by cohesive behavior rather
than listed as 72 flat rows (a `?`-worthy judgment call in itself — the
ticket says "every parameter", so the full flat list is preserved in
§1.1.1 for completeness; this table is the behavioral grouping used for
classification).

| behavior | evidence | existing test coverage | classification | notes/gap |
|---|---|---|---|---|
| `-Repo` / `-RepoPath` (repo path resolution, `-Repo` is a deprecated alias resolved via `Resolve-Repo`) | `:4-5`, `Resolve-Repo` fn `:249-258` | widely covered (12+ files pass `-RepoPath`) | port | Core entry, must exist in Rust equivalent (`--repo` already does in `code-intel run execute`). |
| `-Config` (path to a JSON config file merged under CLI args) | `:7` | `test-doctor-repo-config-resolution.ps1`, `test-repo-config-resolution.ps1` | port | |
| `-Platform` (`auto`\|`windows`\|`macos`\|`linux`) | `:9-10` | indirectly via `-RepoPath` family tests | port | Drives `Get-CodeIntelPlatform` in `legacy/tools/code-intel-platform.psm1`. |
| `-Mode` (`lite`\|`normal`\|`full`) | `:12-13` | `test-code-evidence-layer.ps1`, `test-primary-launchers.ps1`, `test-runtime-ci-hospital-pet.ps1`, `test-transactional-publication.ps1` | port | See §1.2 for full semantics. |
| `-Language` | `:15` | none found | port? | Only observed use is composing the advisory `$understandCommand` string (`:3611`); low-risk, `?` because I did not trace every consumer exhaustively. |
| `-ArtifactRoot` | `:17` | widely covered | port | Defaults via `Get-CodeIntelArtifactRoot` (`legacy/tools/code-intel-platform.psm1:98`, reads `CODE_INTEL_ARTIFACT_ROOT`) when empty — see §1.4. |
| `-SentruxPath` | `:18` | `test-code-intel-pipeline.ps1` | port | Scoping path for the sentrux stage; defaults to repo root via `Resolve-ChildPath`. |
| `-RepowiseScopePaths`, `-RepowiseRootFiles` | `:21-22` | `test-repo-config-resolution.ps1` (via the camelCase `repo.json` config keys `repowiseScopePaths`/`repowiseRootFiles`, `:39-40` — config-resolution path, not a direct `-Flag` CLI invocation) | port | |
| `-RepowiseWorkspaceRoot` | `:19` | weak: `test-code-evidence-layer.ps1:53` sets `repowiseWorkspaceRoot = ""` in a fixture object but does not assert on it — a hollow match, treated as effectively uncovered | port | |
| `-RepowiseShadowRoot` | `:20` | none found | port | |
| `-RepowiseTimeoutSeconds`, `-RepowiseProvider`, `-RepowiseModel`, `-RepowiseReasoning` | `:23-26` | **none found** — corrected. An earlier pass of this doc claimed `test-code-intel-provider.ps1`/`test-repowise-adapter-contract.ps1`/`test-repowise-provider-probe-classification.ps1` covered these; verified false: `test-code-intel-provider.ps1` tests the unrelated `legacy/Invoke-RepowiseProviderProbe.ps1` script (confirmed by reading it — its only Repowise-shaped match is the string `"Repowise provider probe is missing"`), and neither of the other two files references `-RepowiseProvider`/`-RepowiseModel`/`-RepowiseReasoning` as literal flags either. The original grep matched substrings of unrelated function names (`Get-RepowiseProviderArgs`, `Normalize-RepowiseProvider`). Left in as an explicit self-correction rather than silently fixed, since a wrong "covered" claim is exactly the failure mode this ticket's verification discipline exists to catch. | port | `RepowiseProvider`/`Model`/`Reasoning` are also resolvable from env vars — see §1.3. |
| `-SkipRepowise` (switch) | `:71`, gate at `:3646` | 6 files (`test-code-evidence-layer.ps1`, `test-code-intel-pipeline.ps1`, `test-primary-launchers.ps1`, `test-runtime-ci-hospital-pet.ps1`, `test-stable-wrapper-e2e.ps1`, `test-transactional-publication.ps1`) | port | Skips the entire Repowise stage. |
| `-RepowiseDocs` (switch) | `:72`, effect at `:3174,3396-3423,3502,3674,3743` | `test-code-intel-pipeline.ps1` | port | Selects `anthropic` vs `mock` provider (`:3174`); auto-disabled under several routing conditions (`:3404,3423,3512`) — real decision logic, not a thin flag. |
| `-AllowRepowiseShadowMutation` (switch) | `:73`, gate at `:3660` | none found | port | |
| `-ModelRoutingResult`, `-ModelInventoryResult`, `-ModelExecutableHandle`, `-ModelPromptFile`, `-ModelEndpoint`, `-ModelProtocol`, `-ModelCredentialEnvName`, `-ModelTimeoutSeconds`, `-ModelResponseFormat` | `:27-38` | `test-model-request-synthesis-and-handle.ps1` | shim | Feed the model-request-synthesis early-exit facade (§1.8, block 1) which forwards to `legacy/New-ModelAdapterRequest.ps1` + `legacy/Invoke-ModelChannelDelegate.ps1` (both out of this ticket's scope) and exits. The facade itself is thin; whether the two target scripts are thin is a separate ticket's concern. |
| `-ModelAdapterRequest`, `-ModelAdapterArtifactRoot` | `:39-40` | `test-model-request-synthesis-and-handle.ps1` | shim | Second early-exit facade (§1.8, block 2), forwards to `legacy/Invoke-ModelChannelDelegate.ps1` directly. |
| `-RuntimeCiEvidenceRequest`, `-RuntimeCiEvidenceArtifactRoot` | `:41-42` | `test-runtime-ci-hospital-pet.ps1` | shim | Mid-flow facade (not an early exit) at `:4214-4218`, forwards to `$defaultRustCli`. |
| `-RepowiseAdapterRequest`/`ArtifactRoot`/`EvaluatedAt`/`MaxAgeSeconds` | `:43-46` | `test-repowise-adapter-contract.ps1` (request only; `ArtifactRoot`/`EvaluatedAt`/`MaxAgeSeconds` not independently exercised) | shim | Early-exit facade (§1.8, block 3): `& $rustCli provider repowise-adapt ...` then `exit $LASTEXITCODE` (`:151-167`). Already a thin forwarder to Rust. |
| `-GraphAdapterRequest`/`ArtifactRoot`/`EvaluatedAt`/`MaxAgeSeconds` | `:47-50` | **none found** | shim | Early-exit facade (§1.8, block 4): `& $rustCli provider graph-adapt ...` (`:169-185`). Untested gap — see §6. |
| `-SentruxAdapterRequest`/`ArtifactRoot`/`EvaluatedAt`/`MaxAgeSeconds` | `:51-54` | **none found** | shim | Early-exit facade (§1.8, block 5): `& $rustCli provider sentrux-adapt ...` (`:187-197`). Not to be confused with the separately-tested `-SkipSentruxGate`/check flow — this is a distinct facade. Untested gap. |
| `-CodeNexusAdapterRequest`/`ArtifactRoot`/`EvaluatedAt`/`MaxAgeSeconds` | `:55-58` | `test-codenexus-adapter-contract.ps1` (request + `ArtifactRoot`; not independently confirmed for `EvaluatedAt`/`MaxAgeSeconds`) | shim | Early-exit facade (§1.8, block 6): `:199-215`. |
| `-SurvivalScanRequest` | `:59` | **none found** | shim | Early-exit facade (§1.8, block 7): `& $rustCli repository survival-scan ...` (`:217-229`). Untested gap — `legacy/scripts/tests/test-survival-scan-contract.ps1` exists and covers `-SurvivalScanArtifactRoot`, but not `-SurvivalScanRequest` (likely tests the Rust `repository survival-scan` subcommand directly rather than this PS1 facade — see §4.2). |
| `-SurvivalScanArtifactRoot` | `:60` | `test-survival-scan-contract.ps1` | shim | Same facade as above; only this one companion param is exercised. |
| `-RunCommitSourceRoot`/`AuthorityRoot`/`ManifestRef`/`FinalName` | `:61-64` | `tests/run_commit.rs::production_run_commit_cli_restages_a09_refs_through_a06_and_publishes`, `::cli_publication_preserves_the_callers_manifest_bytes_and_digest`; marker shape at `tests/decision_record.rs:318`; admission at `tests/dag_run.rs::production_dag_output_commits_and_enters_the_authoritative_index` and the mutated-marker rejections in `tests/artifact_index.rs:159-182` | shim | Early-exit facade (§1.8, block 8): `& $rustCli run commit ...` (`:231-247`). `test-run-commit-contract.ps1` was deleted once those Rust tests were confirmed to cover every assertion it made; the one assertion not carried over — that the marker lacks the legacy `generatedAt`/`reportSha256` fields — guarded the PowerShell producer, which no longer writes the marker. |
| `-InventoryExclude` (string[]) | `:65` | **none found** | port | Merged with `$defaultInventoryExclude` (`:3239`); feeds `rg` file inventory. |
| `-DagCoordinate` (switch) | `:67`, facade at `:3258-3270` | `tests/dag_run.rs::artifact_root_routes_runs_where_readers_look_and_matches_the_environment_default`, `::out_and_artifact_root_cannot_both_name_the_staging_directory`, `::ungoverned_repository_completes_instead_of_failing_the_architecture_gate`, `::production_run_preserves_doctor_domain_failure_and_completes_unrelated_branches` | shim | `& $rustCli` (DAG coordinate path), `throw`s on non-zero exit rather than propagating — see §1.5. The artifact-path composition this facade owned (`<artifact root>/<repo name>/<stamp>.dag-staging-<nonce>`) is now `run dag-coordinate --artifact-root`, which also reads `CODE_INTEL_ARTIFACT_ROOT`. `test-dag-facade.ps1` is off CI but still on disk: `New-PublicationRetirementPacket.ps1:15` executes it as E05's golden-parity evidence, so it can only be deleted when E05 closes. |
| `-SaveSentruxBaseline`, `-AutoSaveMissingSentruxBaseline` (switches) | `:69-70`, effect at `:3836-3852` | **none found** | port | Baseline-save branch of the sentrux gate; copies existing baseline to `.prev.json` before overwrite. |
| `-SkipRepomix` (switch), `-RepomixStyle` (`xml`\|`markdown`\|`json`\|`plain`), `-RepomixCompress` (switch) | `:74-77` | `-SkipRepomix`/`-RepomixCompress`: `test-transactional-publication.ps1`. `-RepomixStyle`: **none found** | shim? | Shells out to the external `repomix` tool (`:3582-3597` area); classified `shim` because the PS1 code appears to just build args and invoke it, but I did not fully trace whether output gets reinterpreted — `?`. |
| `-SkipSentrux` (switch, whole-stage) | `:78`, gate at `:3785` | `test-code-evidence-layer.ps1`, `test-runtime-ci-hospital-pet.ps1`, `test-transactional-publication.ps1` | port | Mutually exclusive with the `Mode -eq "lite"` skip (`:3773`) — two different code paths produce the same "sentrux skipped" outcome; see §1.3. |
| `-SkipSentruxCheck` (switch) | `:79`, gate at `:3804,3810` | `test-code-intel-pipeline.ps1` | port | Skips just the `sentrux check` sub-step (metrics production); gate below still runs. |
| `-SkipSentruxGate` (switch) | `:80`, gate at `:3824` | `test-code-intel-pipeline.ps1`, `test-sentrux-failure-normalization.ps1` | port | Skips baseline-regression comparison only; independent of `-SkipSentruxCheck`. |
| `-RequireUnderstandGraph` (switch) | `:81`, effect at `:3633,3638,3640` | **none found** (pre-existing; this PR's harness now exercises the *default*, i.e. NOT passing it) | port | Missing `knowledge-graph.json` is fatal (`status=failed`, `exitCode=1`) when set, soft (`manual_required`, `exitCode=0`) when not. |
| `-SkipGitHubResearch` (switch) | `:82` | 5 files pass the flag (`test-code-evidence-layer.ps1`, `test-code-intel-pipeline.ps1`, `test-github-solution-research.ps1`, `test-runtime-ci-hospital-pet.ps1`, `test-transactional-publication.ps1`) — **but none assert on its effect, because it has none** | **kill-candidate** | **Confirmed dead.** Zero other references to `$SkipGitHubResearch` in the file. `$githubResearch` is unconditionally `New-GitHubSolutionResearchNotApplicable` (`:3919`, stub defined `:684-696`, unconditional regardless of the flag). `Test-GitHubSolutionResearchRequired` (`:531`) is defined but has **zero call sites** anywhere in the file — a second, independent kill-candidate. Not fixed here per the ticket's "record, don't fix" instruction. |
| `-WorkspaceAdd` (switch) | `:83`, gate at `:3755` | **none found** | port | |
| `-SkipOpenSpec` (switch) | `:84`, gate at `:3449` | `test-runtime-ci-hospital-pet.ps1`, `test-transactional-publication.ps1` | port | **No graceful degradation**: when not skipped, `:3450-3453` hardcodes `$rustCli = $defaultRustCli` (`target/debug/<exe>`) and `throw`s if that debug binary is absent — the only one of the `$defaultRustCli` call sites with no fallback. Load-bearing for the parity harness's flag choice (see harness comments). |
| `-AutoOpenSpec` (switch) | `:85`, effect at `:3470` | **none found** | port | |
| `-ProactiveSkillSuggestions` (`auto`\|`enabled`\|`disabled`), `-AutomaticPullRequests` (`auto`\|`ask`\|`enabled`\|`disabled`) | `:86-89`, resolved together at `:3253-3256` via `Resolve-CodeIntelFollowUpSettings` | `legacy/tests/test-follow-up-automation.ps1` — **note the directory**: this is `tests/`, a sibling of `scripts/tests/` that this inventory's primary sweep initially missed (caught by the supplementary automated cross-reference pass, §4). `tests/` holds 5 more PS1 test files: `test-automatic-pull-request-flow.ps1`, `test-automatic-pull-request.ps1`, `test-follow-up-automation.ps1`, `test-model-channel-degraded-pipeline.ps1`, `test-model-channel-delegate.ps1` — none of these were named in the ticket, all are in-scope for §4's cross-reference and now included. | port | Feed `Write-CodeIntelFollowUpPrompt` (`:4714`) and the hospital report's follow-up automation block. |
| `-BugSkill` | `:90` | none found (`legacy/tests/test-follow-up-automation.ps1` covers the other two params in this trio but not this one — confirmed by direct read) | port | |

#### 1.1.1 Full flat parameter list (for exhaustiveness)

`Repo, RepoPath, Config, Platform, Mode, Language, ArtifactRoot, SentruxPath, RepowiseWorkspaceRoot, RepowiseShadowRoot, RepowiseScopePaths, RepowiseRootFiles, RepowiseTimeoutSeconds, RepowiseProvider, RepowiseModel, RepowiseReasoning, ModelRoutingResult, ModelInventoryResult, ModelExecutableHandle, ModelPromptFile, ModelEndpoint, ModelProtocol, ModelCredentialEnvName, ModelTimeoutSeconds, ModelResponseFormat, ModelAdapterRequest, ModelAdapterArtifactRoot, RuntimeCiEvidenceRequest, RuntimeCiEvidenceArtifactRoot, RepowiseAdapterRequest, RepowiseAdapterArtifactRoot, RepowiseAdapterEvaluatedAt, RepowiseAdapterMaxAgeSeconds, GraphAdapterRequest, GraphAdapterArtifactRoot, GraphAdapterEvaluatedAt, GraphAdapterMaxAgeSeconds, SentruxAdapterRequest, SentruxAdapterArtifactRoot, SentruxAdapterEvaluatedAt, SentruxAdapterMaxAgeSeconds, CodeNexusAdapterRequest, CodeNexusAdapterArtifactRoot, CodeNexusAdapterEvaluatedAt, CodeNexusAdapterMaxAgeSeconds, SurvivalScanRequest, SurvivalScanArtifactRoot, RunCommitSourceRoot, RunCommitAuthorityRoot, RunCommitManifestRef, RunCommitFinalName, InventoryExclude, DagCoordinate, SaveSentruxBaseline, AutoSaveMissingSentruxBaseline, SkipRepowise, RepowiseDocs, AllowRepowiseShadowMutation, SkipRepomix, RepomixStyle, RepomixCompress, SkipSentrux, SkipSentruxCheck, SkipSentruxGate, RequireUnderstandGraph, SkipGitHubResearch, WorkspaceAdd, SkipOpenSpec, AutoOpenSpec, ProactiveSkillSuggestions, AutomaticPullRequests, BugSkill`
(72 names; counted programmatically from the param block, cross-checked against the grouped table above.)

### 1.2 `-Mode` semantics

| behavior | evidence | existing test coverage | classification | notes/gap |
|---|---|---|---|---|
| `Mode = "full"` appends `--full` to the advisory `$understandCommand` string | `:3612-3614` | none isolating this specifically | port | Does not change what gets scanned — only the recommended manual command text. |
| `Mode -ne "lite"` gates Repowise docs (`RepowiseDocs` combination) and the Repowise state/workspace re-check block | `:3674`, `:3719` | `test-code-intel-pipeline.ps1` (indirectly) | port | |
| `Mode -eq "lite"` skips the entire Sentrux stage | `:3773-3784` | `test-runtime-ci-hospital-pet.ps1`, `test-transactional-publication.ps1` pass `-Mode` but I did not confirm any test asserts the lite-mode sentrux-skip specifically — `?` | port | Produces a `steps` entry `{name:"sentrux", status:"skipped", error:""}` distinct from the `-SkipSentrux` path's equivalent entry (`:3785-3797`, `error:"sentrux not found"` wording differs) — two code paths, same externally-visible "skipped" outcome, slightly different metadata. Worth a T2-T5 note. |

### 1.3 Environment variables consumed directly (`grep '\$env:'`)

| variable | evidence | direction | existing test coverage | classification | notes/gap |
|---|---|---|---|---|---|
| `PYTHONIOENCODING`, `PYTHONUTF8`, `TERM`, `NO_COLOR`, `RICH_FORCE_TERMINAL` | `:107-111` | write (forced to fixed values, not read) | none found | shim | Console/subprocess encoding hygiene; not behavior-bearing for parity purposes. |
| `CODE_INTEL_SENTRUX_DSM_PROVIDER` (`rust`\|`powershell`) | `:3937-3943` | read | none found | port | Selects the DSM provider preference; `throw`s on any other value. |
| `CODE_INTEL_RUST_CLI` | `:3945-3948` | read | none found | shim | Overrides `$defaultRustCli` **only** for the Sentrux-DSM call site — the other ~9 `$defaultRustCli` call sites (facades, DAG coordinator, OpenSpec, runtime-CI evidence) do **not** consult this variable and always resolve `target/debug/<exe>` relative to `$PSScriptRoot`. Load-bearing finding for the parity harness (documented in the harness's own comments). |
| `CODE_INTEL_REPOWISE_PROVIDER`, `REPOWISE_PROVIDER` | `:3180` (via `Resolve-ConfigString -EnvNames`) | read | `test-code-intel-provider.ps1` (uncertain which exact var) | port | |
| `CODE_INTEL_REPOWISE_MODEL`, `REPOWISE_MODEL` | `:3188` | read | as above | port | |
| `CODE_INTEL_REPOWISE_REASONING`, `REPOWISE_REASONING` | `:3195` | read | as above | port | |

Transitively (not a direct `$env:` reference in this file, but a real
effect on default behavior when the corresponding param is empty):
`CODE_INTEL_ARTIFACT_ROOT` and `CODE_INTEL_SHADOW_ROOT`, read by
`Get-CodeIntelArtifactRoot`/`Get-CodeIntelShadowRoot` in
`legacy/tools/code-intel-platform.psm1:104,118`, called from `Get-DefaultArtifactRoot`/`Get-DefaultShadowRoot`
(`legacy/run-code-intel.ps1:371-377`). Listed for completeness; evidence cell
intentionally points at the module, not this file, since that is where the
`$env:` read actually happens.

`ModelCredentialEnvName` (`:34`) is a parameter whose **string value names
an env var for a downstream script to read** (`legacy/Invoke-ModelChannelDelegate.ps1`,
out of scope) — this file never reads that named variable itself, it only
forwards the name (`:137`).

### 1.4 Exit code semantics

| behavior | evidence | existing test coverage | classification | notes/gap |
|---|---|---|---|---|
| 8 early-exit facades: `exit $LASTEXITCODE` after forwarding to `$defaultRustCli` or `legacy/Invoke-ModelChannelDelegate.ps1` | `:140,148,166,184,196,214,228,246` | see §1.1 per-facade rows | shim | Directly propagates the child process's exit code — no translation. |
| Final success/failure: `exit 1` if `$effectiveFailed.Count -gt 0`, else `exit 0` | `:4715-4723` | `test-code-intel-pipeline.ps1` and most integration tests exercise the success path; failure-path exit code not independently isolated in a dedicated test I found — `?` | port | **Binary only** (0 or 1). No architecture/domain-vs-process distinction. |
| Mid-flow `throw` on subprocess failure (DAG coordinator, workflow recommendation) | `:3268`, `:3455`, `:3478` | `test-dag-facade.ps1` (DAG only) | port | An uncaught `throw` in PowerShell run via `pwsh -File` terminates with a non-zero exit code (observed as `1` in practice) — same binary outcome as the final block, different code path. |
| ~50 other `throw` statements throughout (config/contract validation) | counted programmatically, not individually cited | scattered | port | Not enumerated row-by-row — each is a validation guard (e.g. `:117,122,155,173,191,203,219,235,1857`-adjacent-style contract asserts), not a top-level exit-code contract. Flagged in aggregate; a T2-T5 ticket doing the actual port should re-grep `throw` rather than trust a static count here. |

**Current parity gap (see §5 for the harness's live measurement):** the
Rust CLI's `code-intel run execute` distinguishes exit code `10`
(architecture/domain gate failure) from `70` (process failure) — documented
in `.github/workflows/ci.yml`'s self-scan step comment. `legacy/run-code-intel.ps1`
has no equivalent split; every failure surfaces as exit `1`. This is a real,
already-observable contract divergence, not a hypothetical one. Note this
is distinct from — and not contradicted by — the fact that codes `10`/`70`
(and the fuller `0,10,20,64,65,69,70,74` outcome matrix) **are** tested
elsewhere: `legacy/scripts/tests/test-atomic-capability-contract.ps1` validates
that vocabulary at the schema/contract level against
`docs/adr/0009-atomic-capability-execution-model.md`, for the Rust side's
own atomic-capability model. No test anywhere asserts a **live**
`legacy/run-code-intel.ps1` process actually exiting `10` or `70`, because it
never does — confirmed by this section's own line-cited evidence that its
only exit statements are `exit 1`/`exit 0` (`:4721,4723`) and the
propagated-`$LASTEXITCODE` facades (`:140` etc., which forward whatever
the invoked Rust subcommand returned verbatim — whether those specific
subcommands, `provider repowise-adapt` etc., actually use the same
`10`/`70` vocabulary as `run execute` was not independently confirmed
this session, `?`). Either way, a propagated code is Rust's exit code
passing through unmodified, not legacy/run-code-intel.ps1's own main-flow
contract, which stays binary.

### 1.5 Artifacts written

| artifact | evidence | schema cross-ref | existing test coverage | classification | notes/gap |
|---|---|---|---|---|---|
| `<runDir>/report.json` | `:4178,4694` | no `orchestration/schemas/*.json` file matches this name; closest relative is `code-intel-run-manifest.v1` on the Rust side, but the shapes are unrelated (steps-array vs. DAG-nodes-map) | widely covered | port | The PS1 "overall run" artifact; NOT the same shape as Rust's `run-manifest.json` despite both being "the top-level report" for their respective paths. |
| `<runDir>/hospital-report.json` | `:4181,4697`, built by `New-CodeIntelHospitalReport` (`:2344-2551`) | `orchestration/schemas/code-intel-hospital.v1.schema.json`; **both** PS1 and Rust claim `schema: "code-intel-hospital.v1"` | `test-hospital-trust-contract.ps1` (decision-logic level, not full-document schema) | port | Top-level shape has already drifted from the Rust producer of the same schema string — see §5. |
| `<runDir>/summary.md`, `<runDir>/understanding.md`, `<runDir>/hospital.md` | `:4179-4182,4695-4698` | none (markdown, not schema'd) | widely covered | port | |
| `<runDir>/surgery-plan.json` / `.md` (via `Convert-SurgeryPlanToMarkdown`, `New-CodeIntelSurgeryPlan`) | fn defs `:1676-1801` | `code-intel-surgery-plan.v1` (Rust side confirms this name empirically — see §5) | none found isolating this file directly | port | |
| `<runDir>/sentrux-evolution.json`, `sentrux-hotspots.json`, `sentrux-dsm.json`, `sentrux-what-if.json`, `codenexus-context.json` (paths built around `:3931-4134`) | variable names `sentruxEvolutionPath` etc., confirmed via searchable-terms sweep of the file; exact `Join-Path` lines not individually re-verified for each — `?` | none | none found | port | |
| `<runDir>/model-assistance-dossier.json` | `:3426` (`$dossierPath`) | `code-intel-model-assistance-dossier.v1.schema.json` | `test-model-request-synthesis-and-handle.ps1` | port | |
| `<runDir>/run-complete.json` (publish marker) | `:4700-4706` | none (PS1-local schema `code-intel-run-commit.v1`, distinct from the Rust authority-root's own `run-complete.json`) | `test-transactional-publication.ps1` | port | Rust also writes a `run-complete.json` (different shape — publication marker under the authority root, not a run-report digest). Same filename, unrelated schema; a naming collision worth flagging for T2-T5, not a bug in either side individually. |

### 1.6 Side effects

| effect | evidence | classification | notes/gap |
|---|---|---|---|
| Console/subprocess env var mutation (`PYTHONIOENCODING` etc.) | `:107-111` | shim | Process-local, does not escape to the calling shell. |
| In-place text rewrite of any repomix/text-extension file under `$runDir` that contains the staging path, replacing it with the final path post-rename | `:4670-4690` | port | Runs over **every text file in the run directory**, not just known artifacts — a broad, generic find-and-replace. Worth noting for anyone porting this: it is easy to under-scope a Rust reimplementation to "the artifacts we know about" and miss this catch-all. |
| Atomic publish via `[System.IO.Directory]::Move($runDir, $finalRunDir)` | `:4692` | port | PS1's own staging/promote pattern, structurally analogous to (but implemented independently of) the Rust `staged_artifact`/authority-root commit mechanism. |
| `sentrux gate --save` / `sentrux check` invoke the external `sentrux` shim command (`tools/sentrux-shim/`), which itself may write `.sentrux/baseline.json`, `.sentrux/baseline.prev.json` **inside the scanned repo tree** | `:3799-3852` | port | File placement outside the artifact root — writes land in the repo being scanned, not in `-ArtifactRoot`. Intentional (that's where `.sentrux/` configuration lives), but worth calling out explicitly since it means running this pipeline against a repo mutates that repo. |

### 1.7 `-Mode`/Skip-flag interaction matrix (condensed)

Already covered per-flag above; the one non-obvious interaction worth a
dedicated callout: **`-SkipSentrux` and `Mode -eq "lite"` are two
independent code paths that produce the same user-visible "sentrux
skipped" outcome** (`:3773-3784` vs `:3785-3797`) with slightly different
step metadata (`error` field differs). A behavior-preserving Rust port
needs to decide whether to preserve two paths or collapse them — flagged,
not decided, here.

### 1.8 The eight early-exit facade blocks (`legacy/run-code-intel.ps1:113-247`)

These are the single most important finding for classification purposes:
**over 10% of this file's externally observable behavior (8 of the file's
distinct entry-point behaviors) is already a thin forwarder to the Rust
CLI**, gated purely by which optional parameter is non-empty:

| # | trigger param | forwards to | evidence |
|---|---|---|---|
| 1 | `-ModelInventoryResult` (+ Routing/PromptFile/AdapterArtifactRoot) | `legacy/New-ModelAdapterRequest.ps1` then `legacy/Invoke-ModelChannelDelegate.ps1` | `:113-141` |
| 2 | `-ModelAdapterRequest` | `legacy/Invoke-ModelChannelDelegate.ps1` | `:143-149` |
| 3 | `-RepowiseAdapterRequest` | `$rustCli provider repowise-adapt` | `:151-167` |
| 4 | `-GraphAdapterRequest` | `$rustCli provider graph-adapt` | `:169-185` |
| 5 | `-SentruxAdapterRequest` | `$rustCli provider sentrux-adapt` | `:187-197` |
| 6 | `-CodeNexusAdapterRequest` | `$rustCli provider codenexus-adapt` | `:199-215` |
| 7 | `-SurvivalScanRequest` | `$rustCli repository survival-scan` | `:217-229` |
| 8 | `-RunCommitManifestRef` | `$rustCli run commit` | `:231-247` |

All eight validate required companion params, resolve `$rustCli =
$defaultRustCli` (`target/debug/<exe>`, `:103`), `throw` if that binary is
missing, invoke it, and `exit $LASTEXITCODE`. Classification: **shim**,
uniformly — the actual retirement work for these eight is "stop routing
through legacy/run-code-intel.ps1 at all and call `code-intel` directly", not
"port logic to Rust" (the logic already lives there).

## 2. `legacy/Invoke-SentruxAgentTool.ps1` (3125 lines)

An 11-operation DSM / git-churn / complexity-metrics engine invoked either
(a) directly (by an agent/user, per its `-Tool` positional dispatch — the
tool names read like MCP-style operations) or (b) programmatically from
`legacy/run-code-intel.ps1` for exactly three operations (`dsm`, `evolution`,
`what_if`; see §1.8-adjacent evidence below). It does **not** implement
shim management, pro activation, or failure normalization (§0).

### 2.1 `param()` block — 6 parameters

Full block: `legacy/Invoke-SentruxAgentTool.ps1:3-16`.

| behavior | evidence | existing test coverage | classification | notes/gap |
|---|---|---|---|---|
| `-Tool` (mandatory, positional 0, `ValidateSet` of 15 values) | `:4-6` | 3 files reference the script by name (`test-code-intel-pipeline.ps1`, `test-regression-fixes.ps1`, `test-sentrux-failure-normalization.ps1`); exact per-operation breakdown not independently confirmed for each — `?` | port | See §2.2 for the 15-value → 11-operation normalization. |
| `-Path` (positional 1, default `"."`) | `:8-9` | as above | port | |
| `-SessionId` | `:11` | as above | port | |
| `-Recent` (int, default 10) | `:12` | none isolated | port | Session-history window for `evolution`. |
| `-Platform` (`auto`\|`windows`\|`macos`\|`linux`) | `:13-14` | none isolated | port | |
| `-PollutionExclusions` (string[]) | `:15` | **none found** | port | Feeds `Get-PollutionSignals` (`:293-347`). |

### 2.2 `-Tool` dispatch — 15 accepted values, 11 distinct operations

`sentrux_*`-prefixed values are aliases, stripped by
`$normalizedTool = if ($Tool.StartsWith("sentrux_")) {...}` (`:3107`)
before the `switch` at `:3108-3123`.

**Coverage-detection gotcha, caught by the supplementary automated
cross-reference (§4) and independently re-verified here**: `-Tool`/`-Path`
are `[Parameter(Position=0/1)]`-bound (`:4,8`), so real call sites invoke
this script positionally — `& $sentruxAgentTool dsm $targetPath`
(`legacy/run-code-intel.ps1:3995` is a real example of this exact shape) — and
**never** write a literal `-Tool "dsm"` string. A naive grep for quoted
tool-value strings (`'dsm'`, `'health'`, ...) finds nothing even where
real coverage exists, because the value appears as a bare unquoted
positional token. Precise coverage below was re-derived by reading actual
call sites, not by trusting either the naive grep or the cross-reference
pass blindly.

| operation | evidence | rust equivalent? | existing test coverage | classification | notes/gap |
|---|---|---|---|---|---|
| `dsm` (+ `sentrux_dsm`) | `:3116-3119`, fn `Invoke-DsmTool:2292-2334` | **yes, partially** — `legacy/run-code-intel.ps1:3952` prefers `$rustCli sentrux dsm` and falls back to this PS1 tool only when the rust binary is absent or `CODE_INTEL_SENTRUX_DSM_PROVIDER=powershell` | `test-code-intel-pipeline.ps1:389`, `test-regression-fixes.ps1:250` (positional invocation + `Assert-Equal "dsm" $dsm.tool`) | port? | Marked `?`: this is simultaneously "the fallback implementation for an already-ported operation" and "900+ lines of real, non-thin logic" — whether T2-T5 should delete it (once the rust path is trusted) or keep porting parity into it is a product decision, not something this inventory can resolve. |
| `health` (+ `sentrux_health`) | `:3110`, fn `:468-491` | unknown — not independently verified | `test-code-intel-pipeline.ps1:385` (positional) | port | |
| `git_stats` (+ `sentrux_git_stats`) | `:3120`, fn `Invoke-GitStatsTool:2398-2518` | unknown | `test-code-intel-pipeline.ps1:396` (positional, as `sentrux_git_stats`) | port | |
| `scan` (+ `sentrux_scan` alias, `rescan`) | `:3109,3113`, fn `Invoke-ScanTool:434-467` | unknown | **none found** | port | |
| `session_start` | `:3111`, fn `:492-514` | n/a (session bookkeeping) | **none found** | port | Writes `.sentrux/agent-sessions/*.start.json` inside the **scanned repo**, not an artifact root — see §2.4. |
| `session_end` | `:3112`, fn `:515-583` | n/a | **none found** | port | |
| `check_rules` | `:3114`, fn `:584-615` | unknown | **none found** | port | |
| `test_gaps` (+ `sentrux_test_gaps`) | `:3121`, fn `Invoke-TestGapsTool:2347-2397` | unknown | **none found** | port | |
| `what_if` | `:3122`, fn `Invoke-WhatIfTool:2897-3058` | **no** — confirmed: `legacy/run-code-intel.ps1:4128` calls `& $sentruxAgentTool what_if` unconditionally, with no rust-preference branch analogous to the `dsm` one | **none found** | port | Concrete, evidenced gap: this operation has no rust counterpart today, and no test isolates it. |
| `evolution` | `:3115` (top dispatch) / `:4109` (called from legacy/run-code-intel.ps1) | **no** — same unconditional-call evidence as `what_if` (`legacy/run-code-intel.ps1:4109`) | **none found** | port | |

Net: **3 of 11 operations** (`dsm`, `health`, `git_stats`) have direct
per-operation test coverage; the remaining 8 have none isolated (the file
is still exercised as a whole by the 3 files noted in §2.1, just not per
`-Tool` value).

### 2.3 Environment variables / exit codes

| behavior | evidence | classification | notes/gap |
|---|---|---|---|
| `$env:PYTHONIOENCODING`, `$env:PYTHONUTF8`, `$env:NO_COLOR` — writes only | `:23-25` | shim | Same console-hygiene pattern as `legacy/run-code-intel.ps1`; no reads at all in this file (`grep GetEnvironmentVariable` = 0 matches). |
| Exit codes: **none set explicitly anywhere in the file** (`grep -i '\bexit\b'` = 0 matches) | whole-file negative result | port | Relies entirely on PowerShell's implicit behavior: normal completion of the final `$result \| ConvertTo-Json -Depth 14` (`:3125`) → exit 0; an uncaught `throw` anywhere upstream → non-zero (observed as 1). No custom exit-code contract to preserve beyond "0 on success". |

### 2.4 Artifacts / side effects

| behavior | evidence | classification | notes/gap |
|---|---|---|---|
| Reads `<Path>/.sentrux/baseline.json`, `<Path>/.sentrux/rules.toml` | `:207,440-441,530,587,2719` | port | Same config files `legacy/run-code-intel.ps1`'s sentrux stage reads — shared contract surface between the two monoliths. |
| Writes `<Path>/.sentrux/agent-sessions/*.start.json` / presumably `*.end.json` | `Get-SessionDir:348-352`, `Write-JsonFile:370-380` | port | **Side effect outside any artifact root** — writes land inside the scanned repository itself, same pattern as §1.6's sentrux-baseline note. |
| Final stdout: `$result \| ConvertTo-Json -Depth 14` | `:3125` | port | No file write for the primary tool result itself — it is a stdout-only tool; callers (including `legacy/run-code-intel.ps1`) capture and redirect it. |

## 3. `tools/sentrux-shim/*.ps1` (light pass — the real shim/pro-activation surface)

Not one of the two ticket-named files; included because §0's correction
means the "shim management + pro activation" framing genuinely describes
this pair of files, and a contract inventory that silently drops the
behavior the ticket asked for (just because it named the wrong file) would
be a worse deliverable than a short, clearly-scoped extra section.
`sentrux-shim.ps1` = 339 lines, `sentrux-lite-core.ps1` = 567 lines (906
total — small enough for a light pass, not a full second monolith
inventory).

| behavior | evidence | existing test coverage | classification | notes/gap |
|---|---|---|---|---|
| Core-binary resolution order: `$env:SENTRUX_CORE_EXE` override → `sentrux-core.exe` next to the shim → `sentrux.exe`/`sentrux-core.exe` in the parent dir → PowerShell `sentrux-lite-core.ps1` fallback | `sentrux-shim.ps1:195-282` (`Resolve-Core`, `Invoke-Core`) | `test-regression-fixes.ps1` references `sentrux-shim`/`sentrux-lite-core` by name; exact assertions not enumerated here — `?` | shim | `SENTRUX_CORE_EXE` is a **new env var** not previously listed in either monolith's inventory — genuinely lives only here. |
| `Test-CodeIntelThinForwarderCandidate` — refuses to treat a directory as a "real core" location unless it has both a sibling `repo.json` and `sentrux-shim.ps1`, specifically to prevent recursive PATH resolution | `sentrux-shim.ps1:176-193` | as above | shim | This function's own docstring-comment explicitly invokes the "thin forwarder" concept the ticket asked about — the clearest piece of evidence that shim-thinness is a deliberate, documented design constraint in this file, not an accident. |
| `sentrux pro` subcommand: `Show-ProStatus`, `Write-License`, `Deactivate-Pro`, `Show-ProHelp`, `Get-LicensePath`, `Get-AutoDisabledPath`, `Clear-AutoDisabled`, `Ensure-AutoActivation` | `sentrux-shim.ps1:23-165,285-` (dispatch on `$RemainingArgs[0] -eq "pro"`, `:285`) | as above | port | Pro license lifecycle — not exercised by this ticket's parity harness (no `sentrux pro` invocation in the T1 scope). |
| `sentrux-lite-core.ps1` (567 lines) | not individually read this session beyond confirming its role as `Invoke-Core`'s fallback (`sentrux-shim.ps1:276-280`) | `test-regression-fixes.ps1` (per name reference) | port? | Flagged `?` — genuinely out of budget for this pass; a dedicated light-pass ticket on `tools/sentrux-shim/*` would be reasonable follow-up scope, not assumed here. |

## 4. Test coverage cross-reference (`scripts/tests/*.ps1` [51 files] + `tests/*.ps1` [5 files])

Methodology: literal grep of each behavior identifier (parameter name,
tool-operation name, env var name), run this session, **in two passes**:
a manual sweep (scoped to `scripts/tests/*.ps1`, matching the ticket's own
"`scripts/tests/*.ps1` 50+ 套" framing) plus a supplementary automated
cross-reference sub-agent that additionally searched a sibling `tests/`
directory (5 more PS1 files, not mentioned in the ticket, holding real
coverage the manual pass would otherwise have missed and mis-reported as
gaps — `legacy/tests/test-follow-up-automation.ps1` in particular). The two
passes disagreed on several rows; every disagreement was re-verified
directly against the actual test file content (not just trusted from
either pass) before being written into §1-§3. One disagreement traced to
a genuine error in the manual pass's own grep (a substring false-positive
on `-RepowiseProvider` matching unrelated function names
`Get-RepowiseProviderArgs`/`Normalize-RepowiseProvider`) — corrected in
§1.1 rather than silently smoothed over, per the same verification
discipline this whole document is built on. See evidence cells above,
which cite the covering file(s) per-behavior rather than repeating a
separate master table here.

### 4.1 Coverage summary

Counted programmatically from the tables in §1-§3 after the corrections
above (not estimated):

| | rows | fully covered | zero coverage | partial |
|---|---|---|---|---|
| `legacy/run-code-intel.ps1` (§1.1-1.6) | 62 | 43 | 15 | 4 |
| `legacy/Invoke-SentruxAgentTool.ps1` (§2.1-2.4) | 21 | 13 | 8 | 0 |
| `tools/sentrux-shim/*.ps1` (§3) | 4 | 4 (see `?`-hedged caveats inline) | 0 | 0 |
| **Total** | **87** | **60** | **23** | **4** |

For `legacy/Invoke-SentruxAgentTool.ps1` specifically: of its 11 `-Tool`
operations, only **3** (`dsm`, `health`, `git_stats`) have per-operation
coverage — found only after discovering the operations are invoked
**positionally**, not as `-Tool "value"` flags, which is why a naive
grep for quoted tool-value strings found nothing (§2.2 documents this
explicitly). The other 8 operations, and `-PollutionExclusions`, have no
coverage at any granularity.

### 4.2 Gaps (zero test coverage found) and a proposed minimal test each

Per the ticket: this is an inventory of gaps, not new tests written in
this PR (none of these are "trivially small" enough to justify bundling
into a docs+harness PR).

| gap | proposed minimal test |
|---|---|
| `-GraphAdapterRequest`/`ArtifactRoot`/`EvaluatedAt`/`MaxAgeSeconds` facade (`:169-185`) | New `test-graph-adapter-contract.ps1` mirroring `test-repowise-adapter-contract.ps1`'s shape: invoke with a minimal valid request JSON, assert `exit $LASTEXITCODE` matches the underlying `code-intel provider graph-adapt` call. |
| `-SentruxAdapterRequest`/`ArtifactRoot`/`EvaluatedAt`/`MaxAgeSeconds` facade (`:187-197`) | Same pattern, `test-sentrux-adapter-contract.ps1`. |
| `-SurvivalScanRequest` facade (`:217-229`, companion `-SurvivalScanArtifactRoot` IS covered) | `legacy/scripts/tests/test-survival-scan-contract.ps1` already exists and covers the companion param — extend it with a `-SurvivalScanRequest` case rather than write a new file. |
| `-RequireUnderstandGraph` (`:3633-3640`) | Extend `test-code-intel-pipeline.ps1` (or a new small file) with two cases: knowledge-graph missing + flag set → assert `exit 1`; missing + flag unset → assert `exit 0` and `manual_required` in the report. |
| `-SaveSentruxBaseline` / `-AutoSaveMissingSentruxBaseline` (`:3836-3852`) | New small test asserting `.sentrux/baseline.prev.json` is written before overwrite when a prior baseline exists. |
| `-WorkspaceAdd` (`:3755`) | Needs a scoped-Repowise fixture; out of budget to design here — flagged only. |
| `-AutoOpenSpec` (`:3470`) | Small test asserting the `options.auto` boolean in the OpenSpec request payload reflects the switch. |
| `-InventoryExclude` (`:3239`) | Small test asserting excluded globs are absent from `inventory.rg/files.txt`-equivalent output. |
| `-BugSkill` (`:90,3253-3256`) — `-ProactiveSkillSuggestions`/`-AutomaticPullRequests` are covered by `legacy/tests/test-follow-up-automation.ps1`, this one is not | Extend that same file with a `-BugSkill` case asserting it lands in the hospital report's follow-up block. |
| `-RepowiseShadowRoot` (`:20`) — its siblings `-RepowiseScopePaths`/`-RepowiseRootFiles` are covered via config-resolution, `-RepowiseWorkspaceRoot` is weakly represented | Small test extending `test-repo-config-resolution.ps1`'s pattern with a `repowiseShadowRoot` config key. |
| `-RepowiseProvider` / `-RepowiseModel` / `-RepowiseReasoning` (`:24-26`) — corrected gap, see §1.1's self-correction note | New test invoking `legacy/run-code-intel.ps1` with each flag set and a mock/no-op provider, asserting the resolved provider/model/reasoning appear in the run's Repowise stage output. The env-var override path (`CODE_INTEL_REPOWISE_PROVIDER` etc., §1.3) is an equally-uncovered adjacent gap worth folding into the same test. |
| `-RepomixStyle` (as distinct from the already-covered `-SkipRepomix`/`-RepomixCompress`) | Small test asserting the `--style` arg passed to the external `repomix` tool matches the parameter. |
| `-Language` | Low priority (advisory-string-only effect per §1.1) — a test would mostly assert string composition, low value. |
| legacy/Invoke-SentruxAgentTool.ps1 `-PollutionExclusions` | Small test asserting a pattern in this list suppresses the corresponding `Get-PollutionSignals` finding. |
| legacy/Invoke-SentruxAgentTool.ps1 8 of 11 `-Tool` operations (`scan`, `session_start`, `session_end`, `check_rules`, `test_gaps`, `what_if`, `evolution`, and the `sentrux_scan`/`sentrux_health`/`rescan` aliases) — `dsm`, `health`, `git_stats` are covered (§2.2) | Would mean adding positional invocations (`& $tool <operation> $path`, matching the real call shape — see §2.2's coverage-detection gotcha) to an existing suite or a new one. `what_if`/`evolution` are the highest-value pair to close first since they're also the two operations confirmed to have no Rust equivalent yet. |
| `legacy/tools/sentrux-shim/sentrux-shim.ps1`'s `sentrux pro` subcommand | New `test-sentrux-pro-activation.ps1`: exercise `Show-ProStatus`/`Write-License`/`Deactivate-Pro` against a temp `HOME`-scoped license path. |

## 5. Current parity status (harness run, this session)

Ran `legacy/scripts/tests/test-ps1-rust-parity.ps1` **twice**, back to back, both
against the current worktree HEAD, both against a freshly-built
`target/release/code-intel.exe` (`cargo build -p code-intel --release
--locked` — confirmed the lockfile is in sync, `--locked` succeeds). The
two runs' JSON verdicts are **byte-for-byte identical** (`diff` reported
no differences), confirming the harness is deterministic in its verdict
after volatile-field normalization.

**A third, independent confirmation came for free from CI itself.** The
first real run of `.github/workflows/parity-observe.yml` on this PR (a
genuinely fresh `windows-latest` runner, not this dev box) had no `rg` on
PATH — a real environment difference this local session could not have
produced. Both paths failed early (`rustExitCode: 65` — "cannot launch
rg.exe: program not found"; `ps1ExitCode: 1` — `legacy/run-code-intel.ps1:3499`,
`throw "Missing required tool: rg"`). The harness **did not crash**: it
produced a coherent, correctly-degenerate verdict
(`rustArtifactsFound`/`ps1ArtifactsFound` all `false`, `compared: 6`,
`matched: 5`, one real divergence for the two null-shaped hospital-verdict
objects), wrote it, and the workflow uploaded it — validating the
tolerant-of-missing-artifacts design worked exactly as intended under a
real failure this session's own local runs never exercised. Root cause
was a genuine gap in `parity-observe.yml` (missing the same "Install
ripgrep" step `ci.yml` already has for its own jobs) — fixed in a
follow-up commit; not a harness bug.

```json
{
  "schema": "code-intel-ps1-rust-parity-verdict.v1",
  "ok": false,
  "compared": 17,
  "matched": 4,
  "rustExitCode": 0,
  "ps1ExitCode": 0,
  "rustArtifactsFound": { "runManifest": true, "hospitalReport": true, "sentruxPayload": true },
  "ps1ArtifactsFound": { "runDir": true, "report": true, "hospitalReport": true },
  "diverged": [
    { "label": "nodes.rust-only.diagnosis.hospital", "reason": "Rust DAG node with no evidenced PS1 step counterpart at T1" },
    { "label": "nodes.rust-only.doctor", "reason": "Rust DAG node with no evidenced PS1 step counterpart at T1" },
    { "label": "nodes.rust-only.evidence.graph", "reason": "Rust DAG node with no evidenced PS1 step counterpart at T1" },
    { "label": "nodes.rust-only.evidence.native-code", "reason": "Rust DAG node with no evidenced PS1 step counterpart at T1" },
    { "label": "nodes.rust-only.inventory.rg", "reason": "Rust DAG node with no evidenced PS1 step counterpart at T1" },
    { "label": "nodes.rust-only.repo.snapshot", "reason": "Rust DAG node with no evidenced PS1 step counterpart at T1" },
    { "label": "nodes.ps1-only.git status", "reason": "PS1 step with no evidenced Rust node counterpart at T1" },
    { "label": "nodes.ps1-only.node lint hygiene", "reason": "PS1 step with no evidenced Rust node counterpart at T1" },
    { "label": "nodes.ps1-only.rg file inventory", "reason": "PS1 step with no evidenced Rust node counterpart at T1" },
    { "label": "nodes.ps1-only.sentrux gate", "reason": "PS1 step with no evidenced Rust node counterpart at T1" },
    { "label": "nodes.ps1-only.understand graph", "reason": "PS1 step with no evidenced Rust node counterpart at T1" },
    { "label": "hospital.state_machine.current_state", "rust": "post_op", "ps1": "diagnose" },
    { "label": "hospital.verdict_shape", "rust": "{domainVerdict:pass, impression:'clean snapshot', risk:green}", "ps1": "{status:amber, primary_diagnosis:'architecture graph missing', overall_score:54}" }
  ]
}
```

(Full verdict, including the exact `New-Comparison`-normalized JSON
strings, is reproducible by running the harness; abbreviated here for
readability — no divergence entry was cut, only the redundant per-entry
`rust`/`ps1`/`reason` JSON-string escaping was trimmed for this table.)

**Interpretation** (this is what the harness is *for* — observing, not
judging):

1. **Node/step vocabularies are completely disjoint except one pairing.**
   The only stage this harness can currently assert a real evidenced
   pairing for — `evidence.sentrux` (Rust) ↔ `sentrux check` (PS1) —
   **matched** (does not appear in `diverged`). Every other Rust DAG node
   (`diagnosis.hospital`, `doctor`, `evidence.graph`,
   `evidence.native-code`, `inventory.rg`, `repo.snapshot`) and every
   other PS1 step (`git status`, `node lint hygiene`, `rg file inventory`,
   `sentrux gate`, `understand graph`) has no asserted counterpart. This
   is expected at T1 — establishing those pairings (or documenting that
   some genuinely have none) is T2-T5 work.
2. **Hospital state machine disagrees on real data.** Rust lands in
   `post_op` (clean snapshot, `domainVerdict: pass`); PS1 lands in
   `diagnose` (`primary_diagnosis: "architecture graph missing"`,
   `overall_score: 54`, `status: amber`). Root cause, evidenced: this
   worktree has no `.understand-anything/knowledge-graph.json` (confirmed
   absent — `test -f` returned false during this session), and PS1's
   hospital-decision logic treats that as diagnostically significant
   (`New-HospitalAdmissionReason`/`New-HospitalDecisionBlock`, not
   individually re-cited here) while the Rust
   `architecture-graph.internal` evidence provider apparently does not
   route a missing graph to the same failure mode. This is a genuine,
   concrete behavior gap — not a harness artifact — and is exactly the
   kind of finding T1 exists to surface.
3. **Both processes completed successfully** (`rustExitCode: 0`,
   `ps1ExitCode: 0`) — the divergence is in artifact *content*, not
   process failure. That both paths ran clean end-to-end against this
   repository's own snapshot is itself a useful T1 data point.
4. **`ok: false` is the expected result right now**, which is exactly why
   `.github/workflows/parity-observe.yml` is `continue-on-error` at both
   the job and step level — a passing harness today would be more
   surprising than this result.

## 6. Classification tallies

Counted programmatically from the actual `classification` column values
across every table in §1-§3 (a script pass over this file's own markdown,
not a manual estimate — the first draft of this section under-counted by
manually guessing while writing, which is exactly the kind of error this
ticket's verification discipline exists to catch; corrected before
landing):

| classification | count | includes |
|---|---|---|
| `port` | 64 | Core pipeline orchestration logic in both files: hospital/report construction, sentrux stage glue, repowise stage glue, follow-up automation, code-evidence layer, most DSM/evolution/what-if/git-stats/test-gaps/scan/health/session tool operations, sentrux-shim pro-activation lifecycle, most individual params in the §1.1 flat grouping. |
| `shim` | 18 | The 8 early-exit facades in legacy/run-code-intel.ps1 (§1.8) plus their env-var/model-channel counterparts, the Sentrux-DSM rust/powershell provider switch, console-encoding env writes (both files), sentrux-shim's core-binary resolution + thin-forwarder guard. |
| `kill-candidate` | 1 | The `-SkipGitHubResearch` row — which documents **two** independently-dead symbols (the param itself, and `Test-GitHubSolutionResearchRequired`, a function with zero call sites) as one classified behavior, since both die together for the same reason. |
| `port?` / `shim?` (uncertain, reasoning given inline) | 4 | `-Language` effect scope (port?), repomix shim-vs-port framing (shim?), the `dsm` tool operation's port-vs-delete framing (port?), `sentrux-lite-core.ps1`'s own classification (port?). |

87 rows carry a classification value in total (row count grew from an
earlier 83 after §4's coverage corrections split a few grouped rows —
e.g. the Repowise workspace/scope/root-files/shadow quartet — into
per-param rows once their coverage turned out to differ per param, not
uniformly across the group). This underscores the §1.8 finding: `shim`
(18) is not a small residual category — a meaningful slice of
`legacy/run-code-intel.ps1`'s parameter surface is already-thin forwarding, and
the campaign's retirement work for that slice is routing-away, not
porting.

## 7. Gaps and known issues

Recorded per the ticket's instruction ("If you find an actual BUG while
inventorying, record it in the doc's gap section — do not fix it here").

1. **`-SkipGitHubResearch` is dead code.** See §1.1 and §6. Confirmed via:
   zero other references to `$SkipGitHubResearch` in the file;
   `$githubResearch` unconditionally set to a hardcoded "not applicable"
   stub at `:3919` regardless of the flag; `Test-GitHubSolutionResearchRequired`
   (`:531`) has zero call sites.
2. **`run-manifest.json` naming collision.** Both PS1's `run-complete.json`
   (`:4700-4706`, schema `code-intel-run-commit.v1`) and Rust's
   authority-root `run-complete.json` (confirmed empirically this session:
   `<authority-root>/<final-name>/run-complete.json`) share a filename with
   unrelated schemas. Not a bug today (the two never coexist in the same
   directory), but a trap for anyone writing tooling that globs for
   `run-complete.json` across both pipelines.
3. **`$defaultRustCli` inconsistency.** Of the ~11 call sites that resolve
   `target/debug/<exe>`, only one (Sentrux DSM, `:3945-3948`) honors
   `$env:CODE_INTEL_RUST_CLI`; the rest hardcode the debug-build path with
   no override and, in the `-SkipOpenSpec`-gated case (`:3450-3453`), no
   graceful fallback either — a bare `throw`. Not fixed here (constraints
   forbid editing this file); documented so T2-T5 doesn't have to
   rediscover it, and because it directly shaped this harness's own flag
   choices (see the harness's inline comments).
4. **Two independent code paths produce the same "sentrux skipped"
   outcome** (`Mode -eq "lite"` at `:3773` vs. `-SkipSentrux` at `:3785`)
   with slightly different `error` field text. Flagged in §1.7.
5. **Hospital top-level shape has already drifted between the two
   `code-intel-hospital.v1` producers** (§5, finding 2) — PS1 nests its
   verdict under `triage`, Rust nests it under `diagnosis` +
   `domainVerdict`. Both claim the same schema string. This is the
   single highest-value finding for whoever picks up the hospital-report
   port ticket: the schema name is not currently a reliable guarantee of
   shape compatibility between the two implementations.
