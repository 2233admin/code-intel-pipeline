# Understanding Quadrant (D03)

`understanding.quadrant` is a deterministic projection of one A03-verified D01
`project.orientation` artifact. It makes unknowns visible and classifies every projected item by
system criticality and evidence confidence. It does not infer facts, invoke ML, or mutate the D01
source artifact.

## Classification contract

Both axes use integer scores from 0 through 100. A score of 50 is in the upper band.

| System criticality | Evidence confidence | Quadrant |
| --- | --- | --- |
| `>= 50` | `>= 50` | Known Core |
| `>= 50` | `< 50` | Critical Unknown |
| `< 50` | `>= 50` | Supporting Context |
| `< 50` | `< 50` | Deferred Unknown |

D01 confidence maps to `high = 100`, `medium = 70`, and `low = 40`. Unknown source states always
receive evidence confidence 0. Known criticality is fixed by the projection policy: identity 100,
purpose 90, boundaries 90, risks 85, entry points 80, commands 75, active change 70, evidence
availability 35, and languages 30.

Unknown fields are critical by default (90). Fields explicitly describing language,
documentation, examples, context, metadata, or style are supporting context (25). This conservative
default prevents a new or unrecognized unknown from being silently deferred.

## Evidence and stability

Each item carries the D01 provenance that supports its statement. Missing provenance and duplicate
projected item IDs fail closed. Items are emitted in stable ID order, `visibleUnknowns` lists every
item whose source state remains unknown, and quadrant counts are recomputed from the emitted items.
The output also binds the source D01 artifact SHA-256 and snapshot identity.

A01 rejects options, snapshot mismatches, and unverifiable Artifact Refs before publication. A03
registers and validates both the D01 input contract and the D03 output contract. Publication is a
single local-write artifact and identical inputs produce byte-identical output.

## C01/C02 boundary

C01 method cards and C02 method selection may consume this artifact as downstream, read-only
inputs. D03 enforces that prospective boundary: they cannot supply classification options or
rewrite scores, quadrants, provenance, unknown visibility, or source facts. This contract does not
claim that a C01 or C02 runtime consumer is already integrated. D03 accepts only D01 as runtime
input; treating method/card selection as an input would reverse the authority boundary.
