# Coverage Rubric

Adapted from [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain) (MIT).

Coverage describes how completely a department inspected its dimension for this run. It is separate from finding confidence: a single finding can carry High confidence even when the department's overall coverage is Medium. Use exactly these four levels: `High`, `Medium`, `Low`, `Not assessed`.

## Levels

### High

- The modalities the department consumes (per its entry in `orchestration/audit/departments.v1.json`) were admitted and current for this run.
- Entry points, boundary files, configuration, and the artifacts relevant to the department's dimension were represented in that modality evidence.
- Any targeted reads needed to resolve ambiguity were taken.
- Excluded paths (generated, vendored, build output) are intentional and recorded in `exclusions`.
- No area the department is responsible for was skipped for lack of time rather than lack of applicability.

### Medium

- The consumed modalities were admitted, but some were partial, stale, or covered only representative files rather than the full surface.
- The department still has enough evidence to identify likely systemic risks in its dimension.
- Any zero-finding conclusion is scoped to what was actually inspected, and the coverage row's `exclusions` says so.

### Low

- The department worked from thin or indirect signal: metadata, naming, or a narrow modality slice, with modalities missing, unknown, or heavily partial.
- Findings may still be valid, but the absence of findings is weak evidence and must not be reported as a clean bill of health.
- Scores under Low coverage must be conservative and the justification must state the limitation.

### Not Assessed

- The department's `applicability` resolved to `no` (or `unknown` while the department itself is `disabled`) — see `orchestration/audit/departments.v1.json`'s `applicabilityCheck`.
- Do not score the dimension: `score_dashboard` carries `score: null` for that department, and the coverage row is `not_assessed`.
- Record why in the coverage row's `exclusions` even though nothing was scored.

## Required Report Fields

Every `code-intel-audit-report.v1` artifact carries one `coverage_matrix` row per department with:

| Field | Meaning |
|-------|---------|
| `department` | Registered department id |
| `coverage` | `high` / `medium` / `low` / `not_assessed` |
| `inspected_evidence` | Modalities, commands, or paths actually inspected |
| `exclusions` | What was not checked, and why |

## Scoring Interaction

- A department with zero findings can score `10.0` only when its coverage row is `high`. The kernel's `validate()` rejects a `10.0` with zero findings under any other coverage level.
- Medium coverage can still support a good score, but the score dashboard justification must mention the limit.
- Low coverage must not produce a high-confidence "clean" conclusion.
- `not_assessed` departments are excluded from the overall score in `score_dashboard.overall` — they do not count as zero, and they do not count as a pass.
- Coverage never downgrades a Confirmed finding's severity or confidence. It only qualifies how much of the dimension the absence of other findings actually covers.
