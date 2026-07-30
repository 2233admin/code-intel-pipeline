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
| **hospital scoring / measurements** | **~7** | **none — Rust emits `null`** | **gap, see §3** |

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

## 3. Hospital scoring — NOT duplicated, and this is the finding

`Get-StepScore`, `New-HospitalMeasurements`, `Get-ImportResolutionScore`,
`Get-SourceCoverageScore`, `New-HospitalScoreBlock`, `Read-HospitalArtifacts`,
`Read-HospitalArtifactFile`.

The Rust hospital **does not compute scores at all.** Measured on the same
repository, same commit:

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

## 4. Open product question

Whether hospital scoring survives is not answerable from the code:

- **If it survives**, ~7 functions plus their measurement inputs have to port
  to Rust before the PowerShell copies can go, and the nine test cases port
  with them. That is real work, not a deletion.
- **If it is deliberately dropped**, the null fields are the intended contract
  and the PowerShell scoring engine is dead code whose tests go with it —
  but `docs/hospital-mode.md` still lists `triage.overall_score` and
  `report_quality.dimensions` as part of the documented reader surface, so
  that documentation has to change in the same breath.

Recorded rather than decided. Guessing here would either delete a live
product surface or spend days porting something already abandoned.
