# Language Adapter Acceptance Standard v1

Every language adapter is evaluated through one policy and one evidence report. A local test, upstream project, or implementation language cannot define weaker rules for itself.

This is a component-level adapter gate. Project acceptance is owned by
`docs/code-intel-project-conformance.md`, which composes this gate with corpus, parity-floor, and
other executable conformance suites.

## Two independent axes

Claim level describes what an adapter says it knows:

| Level | Permitted claim |
| --- | --- |
| `inventory` | Files, language identity, hashes, and chunks only |
| `structural` | Declarations, containment, and import observations with measured precision/recall |
| `semantic` | Parser-backed name/type/control-flow facts proven against a semantic oracle |
| `behavioral` | Runtime or compiler behavior proven against a differential oracle |

Release stage describes how safely the claim may be used:

| Stage | Purpose |
| --- | --- |
| `research` | Evidence can be inspected; it grants no production authority |
| `candidate` | Contract, corpus, determinism, compatibility, effects, provenance, and rollback floors pass |
| `production` | Stronger measurement floors plus independent verification pass |

An adapter cannot obtain semantic or behavioral status merely by reaching production. Conversely, strong experimental semantics remain research-only until operational and governance gates pass.

## Mandatory gates

1. Contract: stable Code Evidence artifact schemas, schema validation, backward compatibility.
2. Claim boundary: every lower claim is true and every unclaimed higher level remains false.
3. Measured quality: labeled sample count, precision, recall, and declared coverage meet policy floors.
4. Unsupported behavior: unsupported code is explicit and fabricated facts equal zero.
5. Determinism: repeated runs produce stable normalized artifacts.
6. Compatibility: existing parity artifacts still match.
7. Effect boundary: observed effects are declared and policy-allowed; network and repository mutation are rejected for this baseline.
8. Provenance: revision and repository-relative implementation/test paths are pinned; the gate re-hashes both files, and candidate/production implementations require a known license.
9. Rollback: documented for every stage and tested for candidate/production.
10. Independent verification: mandatory for production.
11. Oracle depth: semantic and behavioral claims require stage-specific oracle case floors.

Thresholds live only in `orchestration/language-adapter-acceptance-policy.v1.json`. Reports contain observations and cannot lower thresholds.
The gate also enforces monotonic policy strength: candidate cannot be weaker than research, and production cannot be weaker than candidate for any threshold or boolean requirement.

## Commands

```powershell
.\Test-LanguageAdapterAcceptance.ps1 `
  -Report .\orchestration\acceptance\native-code-evidence-candidate.json `
  -Json
```

Exit code `0` means every required gate passed. Exit code `1` means the requested stage or claim is rejected; the JSON result lists exact failed gate ids. Malformed policy or report input exits `2`.

## Current native baseline

`evidence.native-code` passes `candidate + structural` at the existing measured floor: 12 labeled samples, precision 0.75, recall 0.75, coverage 0.833333, ten deterministic replays, six compatibility artifacts, explicit unsupported behavior, and zero new runtime dependencies.

It does not pass `semantic`, `behavioral`, or `production`. Those require parser/differential oracles, stronger measurements, and independent verification rather than a label change.
