# Understanding Quadrant Validator Complexity

## Problem

`validate_understanding_quadrant` combines JSON decoding, exact-shape checks, fixed policy validation, per-item classification, provenance checks, ordering, unknown visibility, and aggregate counts. Its cyclomatic complexity is 33, making it the current Sentrux surgical hotspot.

## Outcome

Preserve the existing artifact contract and error behavior while splitting the validator into small, testable helpers. The public contract registration and serialized artifact format must not change.

## Constraints

- No new dependency.
- No schema or policy change.
- Duplicate-key rejection remains before JSON decoding.
- Existing valid and invalid fixtures keep the same pass/fail result.
- Do not raise Sentrux complexity thresholds.

## Verification

- Run the focused `artifact_ref` unit tests.
- Run `cargo fmt --check`.
- Run Sentrux `session_end` and confirm no structural degradation.
