# Delivery light-speed measurement

`delivery.light-speed-measure` is a deterministic A01 capability that compares two locally recorded, A07-committed run traces. It emits a baseline/current/delta value-stream report. The report is measurement evidence only: it cannot promise a schedule, choose staffing, or authorize delivery.

## Input and authority

The capability accepts exactly eight A03-verified Artifact Refs: one `delivery.run-timing-events`, two A07 `run.commit` markers, their two `run.manifest` objects, the C01 method catalog, and the two selected Method Cards. The timing payload must validate against `code-intel-run-timing-events.v1`, use opt-in local monotonic elapsed milliseconds, set `externalPlatform` to `false`, and bind each trace to a commit Artifact Ref rather than copying self-reported commit fields. D04 resolves those refs only from the A03-verified input set, verifies each commit-to-manifest path/SHA and run/snapshot identity, and requires the current commit to match the request snapshot. Missing objects, digest changes, cross-binding mismatches, or snapshot mismatches fail with exit 65 before publication.

No external telemetry or observability service is required or contacted. The only permitted effect is writing the two declared report artifacts beneath the caller-provided staging directory.

## Deterministic attribution

The rules bind C01 Method Cards `value-stream-queue-delay` and `critical-path-pert`. The catalog and cards are both A03 inputs and must byte-match their managed C01 evidence; their Artifact Refs and SHA-256 digests are emitted in report method provenance. A structurally valid card edit is therefore still a contract failure, not an unnoticed methodology change.

- lead time is `max(completedAtMs) - min(startedAtMs)`;
- only explicit `queue` events count as queue delay;
- explicit handoff, rework, and unnecessary coordination are separate avoidable categories;
- the first `understanding` interval for a subject is irreducible work; later intervals for that subject are repeated understanding;
- required coordination is protected and reported separately;
- `test` and `verification` events must be mandatory and never count as waste;
- critical path selects the maximum-duration predecessor closure; a join includes every declared predecessor and every ancestor exactly once, with deterministic event ordering and lexical tie breaking;
- every delta is `current - baseline` in milliseconds.

Every category carries the source Artifact Ref SHA-256, JSON pointers, event IDs, and rule ID. This makes attribution auditable without inventing queueing facts that the committed trace did not record.

## Outputs and limits

The capability emits closed-contract `light-speed-report.json` and deterministic `light-speed-report.md`. Replaying identical committed input bytes produces identical output bytes; neither output contains a generation timestamp.

The report describes observed committed intervals only. It does not infer arrival rates, capacity, resource contention, future completion dates, staffing levels, or schedule confidence when those facts are absent. Mandatory verification time is engineering work, never avoidable delay.
