# Hospital diagnosis atom

`diagnosis.hospital` is the deterministic B09 diagnosis atom. It consumes only
A03-verified Artifact Refs whose payload is an admitted
`code-intel-evidence-admissibility-result.v1`. A04 materializes the verified
payload `data` in that result, so diagnosis never reopens an unverified provider
file or treats enrichment as authority.

The precedence is stable and evaluated from admitted machine evidence only:

1. local tool failure
2. provider quota exhausted
3. architecture gate failure
4. architecture graph missing
5. authoritative structural evidence unavailable
6. ungoverned structural scope
7. known modernization debt
8. clean snapshot

Missing, partial, stale, rejected, or otherwise untrusted authoritative graph or
structural evidence fails closed to an `unknown` diagnosis. Native-code targets
may enrich a treatment plan, but cannot turn unknown authority into a pass.

The atom emits `hospital-report.json`, `hospital.md`, `surgery-plan.json`, and
`surgery-plan.md`. A03 registers all four schema/type pairs. The JSON documents
are machine authority; Markdown is a rebuildable view and is never an input to
diagnosis. The hospital JSON preserves the legacy stable fields used by existing
readers, including `schema`, `artifacts`, `triage`, `state_machine`,
`modalities`, `policies`, `report_quality`, `diagnosis`, `treatment`,
`protocols`, `tools`, and `surgery_plan`.

## Experimental development-readiness signal

`report_quality.decision_signal` is an advisory, replay-stable signal for the
quality of the next development decision. It is deliberately not a code-health
score and cannot override `domainVerdict`, an admitted rule failure, or an
`unknown` diagnosis.

The signal keeps five normalized dimensions visible instead of hiding them in
one opaque weight vector:

1. authoritative evidence coverage
2. admitted evidence currentness
3. structural governance coverage
4. tool/provider availability
5. actionability of the selected next protocol

The displayed 0-100 value is the unweighted geometric mean. A zero dimension
therefore vetoes readiness, and `weakest_dimensions` identifies the first
improvement target. The dimension vector, its evidence strings, and the
authoritative Hospital verdict remain available for consumers that should not
collapse the result to one number. Hard constraints stay in admitted structural
rules and diagnosis precedence; this experimental signal is never a gate.

Normalization is deterministic:

| Dimension | Normalized value |
| --- | --- |
| Authoritative coverage | admitted graph and structural modalities divided by 2 |
| Evidence currentness | current/trusted authoritative modalities divided by authoritative modalities seen; 0 when none are seen |
| Governance coverage | 1 only when trusted structural evidence contains evaluated rules; otherwise 0 |
| Tool availability | 1 when neither a local tool failure nor provider quota blocks collection; otherwise 0 |
| Actionability | 1 for a bounded next action, 0.5 while authority must be acquired or a modernization target is missing, and 0 for tool/quota blockers |

The 0-100 scale preserves the existing Hospital score surface. This adapts the
transparent-dimensions, geometric-aggregation, and weakest-factor interaction
pattern documented by [Sentrux Quality Signal](https://sentrux.dev/docs/quality-signal/),
but it does not reuse Sentrux's structural metrics or claim to measure the same
thing.

## A09 execution

The normal default DAG is intentionally unchanged because it still lacks an A01
producer for A04 admission results. A09 provides an explicit seeded diagnosis
path for already admitted Artifact Refs:

```text
code-intel run dag-coordinate --repo <repo> --out <run-dir> \
  --diagnosis-inputs <artifact-refs.json> \
  --seed-artifact-root <artifact-root>
```

Before scheduling, A09 verifies every seed through A03 against the current A02
snapshot. It then schedules a coordinator-owned seed boundary followed by the
registered A01 `diagnosis.hospital` capability, and re-verifies all four outputs
through A03. Snapshot mismatch, unknown schema/type, digest mismatch, empty
seeds, and non-admitted evidence fail closed without a hospital report.

`legacy/run-code-intel.ps1` remains the rollback facade until E08. Its stable diagnosis
labels and precedence are the compatibility baseline; new authority belongs to
the A01/A03/A04/A09 path above.
