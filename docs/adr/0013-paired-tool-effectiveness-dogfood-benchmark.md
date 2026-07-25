---
status: proposed
date: 2026-07-25
---

# Measure tool effectiveness with paired dogfood tasks

Adapter health, schema conformance, and successful execution do not establish
that a tool helps an Agent understand or debug a repository. Code Intel already
tests those operational properties well, but its tools do not share a task-level
effectiveness baseline.

The prior Design Pipeline dogfood loop is retained as a useful evidence intake
pattern: observe locally, normalize, redact, deduplicate, draft, and require
separate authority before remote publication. It is not reused as an evaluator.
Its observations contain no counterfactual control, blinded task corpus, or
external root-cause oracle.

## Decision

Code Intel will measure tool effectiveness with paired, replayable dogfood
tasks. The same frozen task is executed under three initial conditions:

- `C0`: no Code Intel context;
- `C1`: built-in deterministic core only;
- `Cfull`: normal automatic routing with every admitted available provider.

Model, reasoning configuration, prompt, task budget, repository snapshot, and
execution permissions remain equal within each pair. Condition labels are hidden
from the task Agent. Run identifiers are randomized before scoring.

The corpus pins one treatment-manifest digest for each condition and one
verifier identity. Every run carries the matching treatment digest. Its
authority-root ExternalAttestation binds run, case, snapshot, condition,
treatment, context, experiment profile, and verifier before any success metric
is accepted. V0 permits only `frozen_evidence_replay`; free-form human assertions
are not a scoring source.

V0 uses eight real historical tasks and two repetitions:

```text
8 tasks × 3 conditions × 2 repetitions = 48 Agent sessions
```

Single-adapter ablation is deferred until C1 or Cfull shows useful paired uplift.
It then runs only on the most discriminating cases. This avoids paying for every
provider permutation before the Pipeline has proved aggregate value.

## Evaluation lanes

The benchmark reports three lanes separately:

1. **Operational** — execution status, admission, deterministic replay, latency,
   artifact size, and failure classification.
2. **Evidence quality** — root-cause recall, evidence precision, rank quality,
   unsupported-claim rate, and whether known gaps are explicit.
3. **Agent outcome** — diagnosis correctness, external-attestation success,
   time to first valid hypothesis, tool calls, tokens, wall time, and cost.

No single composite score is authoritative. A provider is evaluated by paired
quality uplift and cost Pareto position. A fast tool with no root-cause evidence
does not outrank a slower useful tool, and an unavailable optional provider is
reported as `unavailable`, not scored as incorrect.

## Golden task contract

Each accepted case freezes:

- pre-fix repository commit and SnapshotSet;
- original issue text and allowed execution effects;
- task type and difficulty;
- root-cause EntityRefs;
- required and distracting evidence;
- known wrong hypotheses;
- failing test, merged PR, or reviewed human decision as ExternalAttestation;
- corpus and oracle digests.

The first corpus covers:

- 30-second project orientation;
- symbol or relationship retrieval;
- change-impact analysis;
- local bug localization;
- cross-module implicit coupling;
- runtime or test evidence;
- stale evidence detection;
- an unknown-unknown case whose root cause is outside the initial error path.

The Agent and Capsule Assembler cannot read the oracle. The scorer receives
frozen output artifacts and independently recomputes metrics. Agent-reported
scores, confidence, or success never become the oracle.

## Run bundle

Every run records:

```text
case identity
condition identity
repository and tool snapshots
model and reasoning configuration
prompt and budgets
provided context digest
actual tool calls and context consumed
hypothesis transitions
diagnosis EntityRefs
validation and ExternalAttestation refs
timing, token, CPU, and cost observations
```

Events use stable IDs, a monotonic sequence within the run, and idempotent
append semantics. Wall-clock timestamps are metadata and do not determine event
order.

## Metrics

The V0 deterministic scorer reports:

- root-cause EntityRef recall@K at capsule and diagnosis stages;
- relevant-EntityRef precision@K at the configured context budget;
- reciprocal rank of the first root-cause entity;
- diagnosis exactness;
- ExternalAttestation success;
- unsupported-claim rate;
- end-to-end wall time;
- input/output tokens and tool-call count;
- artifact bytes;
- availability, process-failure, and honest-unknown rates.

Results are reported per task and condition before aggregation. Paired deltas use
the same task and repetition seed. V0 is a recorded baseline, not a statistical
claim about all repositories.

`baseline_recorded` requires a quality-observed C0/C1/Cfull triple for every
case and repetition. Otherwise the report is retained with
`insufficient_evidence`.

Confidence calibration, time to first externally valid hypothesis, evidence-item
precision, CPU time, and monetary cost are collector extensions. They are added
only after the V0 run bundle can obtain those observations without self-report or
platform-specific guesswork.

## Dogfood feedback loop

Any real Code Intel run may emit a local evaluation candidate when it reveals a
missed root cause, stale capsule, provider failure, contradictory evidence, or
unexpectedly useful context. Candidates reuse the prior dogfood properties:
redaction, stable fingerprint, occurrence count, local draft, and explicit
remote-publication authority.

A candidate enters the golden corpus only after:

1. the root cause is externally attested;
2. the pre-fix snapshot remains reproducible;
3. the oracle is separated from Agent-visible inputs;
4. the case has a counterexample or known wrong direction;
5. a reviewer confirms that it measures product value rather than fixture trivia.

## Existing benchmarks

Existing orientation, native retrieval, multilingual extraction, parity, and
CCC slice benchmarks remain focused smoke or contract checks. They may provide
operational observations to this benchmark, but none alone is evidence of Agent
outcome uplift.

## Consequences

- Code Intel can distinguish tool readiness from tool usefulness.
- The built-in core and optional adapters compete on the same frozen tasks.
- Weak or costly adapters can remain optional, be demoted, or be retired with
  evidence rather than preference.
- External providers are not required for deterministic CI; scheduled or
  release evaluation records their absence explicitly.
- Corpus growth comes from reviewed real failures, not synthetic case inflation.
