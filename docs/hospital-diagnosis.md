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

`run-code-intel.ps1` remains the rollback facade until E08. Its stable diagnosis
labels and precedence are the compatibility baseline; new authority belongs to
the A01/A03/A04/A09 path above.
