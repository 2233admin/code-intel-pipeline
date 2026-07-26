# Evidence Rubric

Adapted from [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain) (MIT).

An audit department is a hospital department: it consumes the modality evidence that earlier stages already admitted (`xray`, `anatomy`, `ct`, `mri`, `pet`, `chart`, `governance`) and, when that is not enough to resolve a finding, takes a targeted read of the specific file or lines the modality evidence pointed at. It does not start from an unguided browse of the repository — every `evidenceRef` on a finding must trace back to a modality signal, a command, a file the modalities surfaced, or a manual read taken to confirm one of those.

## Evidence Kinds

The finding contract (`docs/audit-report.md`) recognizes four evidence kinds:

| Kind | Example | Strength |
|------|---------|----------|
| `file` | A specific path and line range read to confirm a finding | Strong |
| `modality_signal` | A `ct` structural-rule result, an `anatomy` graph edge, a `governance` gate verdict | Strong |
| `command` | Output of a targeted check the department ran (e.g., a gate/check command, a search) | Strong to Medium, depending on specificity |
| `manual_read` | A read taken outside the admitted modalities to resolve ambiguity the modalities left open | Medium, unless it also carries a `file` reference with a path |

## Evidence Requirements by Severity

| Severity | Minimum evidence |
|----------|-------------------|
| Critical | `file` evidence with a path, or a `modality_signal` plus a confirming `file` read |
| High | `file` evidence, or a `modality_signal` with a traceable path to the affected code |
| Medium | `file` evidence, or a `modality_signal` alone when the pattern is unambiguous |
| Low | `modality_signal` or `command` output; a `file` reference is preferred but not required |
| Info | Any evidence kind |

A finding with `status: Confirmed` always needs at least one `file`-kind evidence entry with a `path` — this is enforced by the audit kernel's `validate()`, not just a style preference.

## Evidence Requirements by Confidence

| Confidence | Minimum evidence |
|------------|-------------------|
| High | `file` or `modality_signal` naming the exact location |
| Medium | `modality_signal` or `command` output that identifies the area but not the exact trigger |
| Low | `command` output or `manual_read` without a confirming path |

## What Does Not Count as Evidence

- "This looks risky" with no `evidenceRef`.
- A severity or confidence claim with no corresponding evidence entry — the schema requires `evidence` on every finding, but an entry that does not actually support the claim is worthless padding.
- Style or naming complaints with no modality signal or file reference behind them.
- Assumptions about intent that no evidence entry supports.
- Evidence copied from a different finding without re-checking it applies here.

## Evidence Fields

Every `evidenceRef` carries `kind` and `source` (both required); `file`-kind evidence should also carry `path`, and `line_start`/`line_end` when the exact lines are known. Set `modality` to the modality that surfaced the lead (`xray`/`anatomy`/`ct`/`mri`/`pet`/`chart`/`governance`) whenever the evidence originated from admitted modality output rather than a manual read. Use `note` to record anything a reviewer needs that the path and lines alone do not convey.
