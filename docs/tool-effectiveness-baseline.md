# Tool Effectiveness Baseline

Status: initial evidence inventory, 2026-07-25.

This baseline separates what Code Intel can currently prove from what its
adapters merely expose. The governing experiment design is
[ADR-0013](adr/0013-paired-tool-effectiveness-dogfood-benchmark.md).

## Current evidence

| Tool or layer | What current evidence proves | Current measured result | Product-effect status |
| --- | --- | --- | --- |
| Repository snapshot and Artifact Ref | portable identity, replay, digest and snapshot binding | contract and negative tests pass | necessary infrastructure; no standalone Agent uplift claim |
| `rg` inventory | deterministic repository inventory when snapshot consumption succeeds | covered by parity and orientation fixtures | exact inventory baseline; real self-run currently exposed a `.git` path-set failure |
| Native code evidence | heuristic structural extraction for seven declared languages | 12 labeled samples; precision `0.75`, recall `0.75`, declared coverage `0.833333`, 10 stable replays | candidate structural baseline only |
| Native retrieval slice | query-term and one-hop import selection can reduce a synthetic JavaScript repository | recall `1.0` for four expected files in one fixture | promising smoke test; precision, ranking, symbol and root-cause quality unmeasured |
| Project orientation | fixed corpus output is deterministic, provenance-complete, bounded, and honest about missing providers | representative Rust benchmark passes; typical p95 target is at most 60 seconds | strong contract/cost evidence; no downstream Agent task comparison |
| Sentrux | command output is normalized, admitted, and fail-closed across complete, partial, crash, and unknown cases | adapter and gate tests pass | architecture-detection precision and diagnosis uplift unknown |
| Internal/Understand graph | current, complete graph evidence can cross one provider-neutral port | adapter, snapshot, freshness, and fallback tests pass | edge precision, completeness, and task uplift unknown |
| Repowise | semantic-memory provider status and failures are classified without corrupting core results | availability/config/quota/local-error fixtures pass | retrieval hit rate, rank quality, and task uplift unknown |
| CodeNexus | full and lite implementations share an admitted provider port | translation, stale/wrong-snapshot, and fallback tests pass | localization and impact accuracy unknown |
| CCC/CocoIndex | command, index, search lifecycle and slice costs can be observed | benchmark accepts measured or explicit unavailable state | no shared semantic oracle; effectiveness unknown |

## Fresh dogfood observation

A normal compiled-CLI run against this repository on 2026-07-25 did not complete:

```text
inventory.rg:
inventory baseline path set differs from snapshot manifest;
extra_count=1; extra_samples=[".git"]
```

`repo.snapshot`, Sentrux evidence, and graph evidence succeeded. Native code
evidence was dependency-blocked by inventory, and doctor/hospital returned
failure. This is a session observation, not yet a golden benchmark case because
the root cause and fix attestation have not been reviewed.

It nevertheless demonstrates the evaluation gap: individual adapters can pass
their contract tests while the user-visible Pipeline fails to produce usable
core context.

## What can be claimed today

- The Pipeline has strong contract, provenance, determinism, and failure-state
  coverage.
- Native extraction has a small quantitative structural baseline.
- Native retrieval succeeds on one narrow synthetic task.
- Optional providers are replaceable and fail closed at their adapter boundaries.

The Pipeline cannot yet claim that any optional provider improves root-cause
recall, diagnosis correctness, completion time, or cost for a real Agent task.

## V0 benchmark

The first effectiveness run uses eight externally attested historical tasks
under `C0`, `C1`, and `Cfull`, twice each. It records per-task results rather
than hiding trade-offs in one score.

The deterministic scorer is available as:

```powershell
code-intel benchmark tools `
  --corpus <code-intel-tool-effectiveness-corpus.v1.json> `
  --runs <code-intel-tool-effectiveness-runs.v1.json> `
  --artifact-root <A03-authority-root> `
  --out <new-output-directory>
```

The scorer fails closed when cases, repetitions, conditions, or paired experiment
profile digests differ. The profile digest is recomputed from the frozen
snapshot, model, reasoning, prompt, budget, permissions, tool snapshot, and
seed. Corpus-pinned treatment manifests and the authority-root
ExternalAttestation bind the actual condition, context, experiment, and verifier
before a success observation can affect a score. The report preserves corpus,
run, context, treatment, profile, and attestation digests. Publication atomically
commits `report.json`, the rebuildable `report.md`, and `completion.json` without
replacing an existing result.

The authority root remains a trust boundary: Artifact Ref verification proves
integrity and binding, while the pinned external verifier owns replaying the
frozen test or PR evidence. Runs from an operator-controlled substitute root are
not comparable to the official baseline.

Initial release criteria are evidence completeness, oracle isolation, replayable
snapshots, and valid paired runs. Quality thresholds are ratcheted only after the
first baseline exists; they are not invented in advance.
