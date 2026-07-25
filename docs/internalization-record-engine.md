# Internalization Record engine

`internalization.record-engine` is the executable Internalization Standard for external projects,
methods, references, adapted capabilities, and selectively owned implementations. It validates and
stores project records; it does not search for, install, upgrade, select, or approve dependencies.
R01–R26 remain the owners of their individual records and decisions.

Every record identifies the source revision, license and obligations, Open-Source Reuse Ladder rung,
owned boundary, owned modifications, compatibility/conformance/necessity evidence, measured benefit
and cost, maintenance/security evidence, update policy and check date, rollback, replacement/exit,
and retirement state/evidence. Evidence IDs are inputs from admitted evidence boundaries such as
A04; a record never turns those references into Engineering Facts.

## Enablement rule

Research remains allowed for a structurally valid record even when a required evidence class is
missing, unknown, expired, or due for update. Production enablement is fail-closed: every evidence
class must be present, known, current, and covered by the lifecycle authority event. The engine
reuses the A05 authority-event validator, including expiry, completeness, and replay protection.
It cannot create an Adoption Decision or Committed Engineering Plan.

An authority event is reported as consumed only after evidence currency, lifecycle closure, and
state-specific checks all succeed. Failed, incomplete, or expired attempts therefore remain
retryable with the same still-valid authority event after their evidence or closure is repaired.

Lifecycle states are `research`, `production_enabled`, `rollback`, `replaced`, `retired`, and
`out_of_scope`. Every state change follows the checked transition table and requires a fresh A05
authority event. Replacement requires a replacement record ID, rollback requires rollback evidence,
and retirement requires completed retirement evidence. Records are retained across rollback and
retirement; audit-only rollback never deletes provenance.

Records may declare
`authorityRequirements.repositoryGovernedAttestation: true`. Only those declared records require
the backward-compatible A05 repository attestation and trusted approver policy; ordinary v1 A05
events, including generic `research -> out_of_scope`, remain valid when the declaration is absent.
R23–R26 use this declaration so their expiring out-of-scope/defer decisions cannot be represented by
an unsigned research placeholder.

## Projections

The deterministic store emits a project `code-intel-reuse-record.v1` view and a
`code-intel-notice-provenance.v1` NOTICE/provenance view. Both are projections of the canonical
record and carry no installation, external-write, fact-promotion, or adoption authority.
Projection APIs accept only an engine-created sealed evaluation bound to the exact canonical record;
they do not accept caller-authored JSON capable of forging `productionEnabled` or NOTICE provenance.
