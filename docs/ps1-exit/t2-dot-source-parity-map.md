# T2 step 4: dot-source parity map (issue #47)

The evidence the classification document said was required before any
PowerShell deletion:

> "a Rust suite exists" is not by itself evidence that a specific assertion
> survives. Treat any PowerShell case with no Rust counterpart as a gap to
> close before the file is deleted, not after.
>
> — [t2-launcher-classification.md](t2-launcher-classification.md) §0

That verification is now done, and **it overturns the working assumption for
about half the surface.** Measured against `5065482`.

## Result

| dot-sourced group | functions | Rust counterpart | verdict |
|---|---|---|---|
| code-evidence symbol extraction | 9 | `native_code_evidence.rs` | genuinely duplicated — retire |
| sentrux gate metric deltas | 3 | `sentrux_gate.rs` | genuinely duplicated — retire |
| hospital state machine / diagnosis | ~5 | `hospital_diagnosis.rs` | genuinely duplicated — retire |
| **hospital scoring / measurements** | **~7** | arithmetic ported to `hospital_score.rs`; not wired, inputs incomplete | **integration gap, see §3** |

## 1. Code-evidence symbol extraction — duplicated

`New-CodeEvidenceNativeSymbol`, `Get-CodeEvidencePowerShellSymbol`,
`Get-CodeEvidencePythonSymbol`, `Get-CodeEvidenceJavaScriptSymbol`,
`Get-CodeEvidenceRustSymbol`, `Get-CodeEvidenceGoSymbol`,
`Get-CodeEvidenceJavaSymbol`, `Get-CodeEvidenceSymbolCandidate`,
`Get-CodeEvidenceSymbols`.

Evidence the Rust owner covers the same ground:

- `native_code_evidence.rs` handles every extension the PowerShell set does
  and more: `ps1, psm1, py, rs, go, ts, tsx, js, jsx, mjs, cjs, java, cs`.
- It runs in production as the `evidence.native-code` DAG node; the live
  self-scan of this repository emits 2993 symbols through it.
- `tests/native_code_evidence.rs` carries an explicit legacy-parity case,
  `a01_a09_artifacts_match_the_real_legacy_producer_on_the_same_fixture`,
  plus cases for unsupported languages, binary files and non-UTF-8 source
  that the PowerShell suite does not have.

Retiring these loses nothing and removes a second implementation of symbol
extraction that nobody runs.

## 2. Sentrux gate metric deltas — duplicated

`New-SentruxMetricDelta`, `Test-SentruxGateNoDegradation`,
`Resolve-SentruxMetricRegressions`.

`sentrux_gate.rs` (1080 lines) computes the same four gated metrics and is
exercised on every CI run and every local `code-intel sentrux gate .` — this
campaign has been gated by it repeatedly, including two god-file regressions
it caught. Production use is stronger evidence than a unit test here.

## 3. Hospital scoring — was NOT duplicated, and this is the finding

`Get-StepScore`, `New-HospitalMeasurements`, `Get-ImportResolutionScore`,
`Get-SourceCoverageScore`, `New-HospitalScoreBlock`, `Read-HospitalArtifacts`,
`Read-HospitalArtifactFile`.

**Status after this change:** the arithmetic is now ported to
`crates/code-intel-cli/src/hospital_score.rs` and verified value-for-value
against the PowerShell originals (dot-sourced and executed, not read). What
remains is *integration*, not translation — the launcher is not wired to it,
and several score inputs still have no Rust producer (§3.2). Until that lands,
the emitted report is unchanged and still carries nulls.

The measurement that made this a finding rather than a de-duplication, taken
before the port on the same repository at the same commit:

| field | PowerShell producer | Rust producer |
|---|---|---|
| `triage.overall_score` | `46` | `null` |
| `report_quality.overall_score` | `46` | `null` |
| `report_quality.diagnostic_score` | `50` | `null` |
| `report_quality.governance_score` | `33` | `null` |
| `report_quality.dimensions` | populated (e.g. `source_coverage`) | `[]` |

`hospital_diagnosis.rs:410,425` hardcodes those nulls. It is not a TODO and
not a failure path — the fields are emitted with the legacy names and no
values, which `docs/hospital-diagnosis.md` describes as preserving "the legacy
stable fields used by existing readers".

Nine of the sixteen PowerShell hospital test cases exist to pin this scoring
behaviour:

- missing surgery target is unresolved
- missing current hotspot is unresolved
- known changed hotspot retains resolved behavior
- unknown import, pollution, and source coverage are explicit zero-confidence evidence
- known complete measurements retain pass scores
- known negative and partial measurements retain evidence-driven scores
- manual and skipped steps are unknown and receive zero confidence
- known modality artifacts retain established scores and explicit status
- deleted and corrupt modality artifacts remain zero-confidence

Deleting them would not be de-duplication. It would delete the only test
coverage of behaviour that exists in exactly one place.

The remaining seven cases — gate failure, unknown check, clean-gate green,
discharge evidence, missing structural summaries, provider quota, structural
completeness wiring — map onto
`precedence_matrix_matches_the_legacy_stable_diagnoses_and_fails_closed`,
`provider_quota_precedes_missing_current_graph_and_is_replay_stable`, and the
`structural_evidence_*` unit tests added in `d029b3f`. `?` on
"discharge requires affirmative resolved post-op target evidence": the Rust
ladder ends at `post_op` and no test names `discharge_ready`, so this one may
be a second, smaller gap. It needs its own check before deletion.

### 3.2 What still blocks wiring

The arithmetic is complete; roughly half its inputs are not produced by any
Rust node:

| score dimension | input | Rust producer |
|---|---|---|
| governance (rules / gate / check) | structural signals | present |
| graph | architecture graph admission | present |
| source coverage | inventory file count, scan file count | present (`inventory.rg`, sentrux payload) |
| import resolution | resolved / unresolved import counts | present (sentrux payload) |
| pollution | DSM `scope.excluded_files` | **absent** — DSM is a PowerShell tool operation, T5 (#50) |
| MRI | CodeNexus context | **absent** |
| PET | runtime CI health, or what-if + evolution | **absent** |
| memory | repowise step result | **absent from the default DAG** |

Wiring the arithmetic to inputs that do not exist would publish confidently
wrong numbers. `0` in this scheme means *observed and absent*, not *never
attempted* — the same distinction `d029b3f` drew for structural evidence, and
the reason `hospital_score.rs` ships unwired behind `allow(dead_code)`.

## 4. Product decision (settled)

Whether hospital scoring survives was not answerable from the code, so it was
escalated rather than guessed. **Decision: it survives** — the scoring is real
product behaviour, so it ports to Rust and the nine PowerShell test cases port
with it, rather than the null fields becoming the intended contract.

Consequence for T2's exit criteria: `run-code-intel.ps1` cannot reach ≤50
lines until the wiring in §3.2 lands, because the scoring engine has to stay
somewhere until Rust can produce the same numbers. The twelve genuinely
duplicated functions in §1 and §2 are unblocked and can retire first.
