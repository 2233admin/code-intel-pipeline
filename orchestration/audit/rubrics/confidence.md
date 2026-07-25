# Confidence Rubric

Adapted from [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain) (MIT).

Confidence describes how sure a department is that a finding is real, independent of how severe it would be. Use exactly these three levels: `High`, `Medium`, `Low`. Confidence tracks the `status` field on the same finding (`Confirmed` / `Suspected`) but is not identical to it — a `Suspected` finding can still carry Medium confidence.

Wire values: confidence `high` / `medium` / `low`; status `confirmed` / `suspected`.

## High

- The finding is directly shown by admitted modality evidence: an `xray` file hit, a `ct`/`anatomy` structural rule result, a `governance` gate/check result, or a targeted read that quotes the exact lines.
- The evidence points at a specific file, function, or configuration key — not a general area.
- The failure scenario can be traced through the evidence without guessing intermediate steps.
- Use `status: Confirmed` together with High confidence when both hold.

## Medium

- The finding is inferred from a pattern the evidence shows (e.g., every handler in an `xray` file list follows a risky shape) rather than a single directly observed instance.
- The evidence identifies the affected area but the exact trigger condition is unclear or depends on external state the modalities did not capture.
- A dependency or configuration risk is real but the usage surface has not been fully traced.

## Low

- The finding is speculative, based on file naming, size, or structure alone with no corroborating modality signal.
- The evidence is indirect or the department has not obtained a targeted read to confirm it.
- The risk depends on conditions (scale, deployment mode, future usage) the current evidence cannot speak to.

## Guidelines

- When unsure, choose Medium or Low, not High. Confidence is a claim about the evidence, not about how worried the department is.
- A Low-confidence finding should be rare. If the evidence is this thin, prefer widening the targeted read before reporting, or file it as `Info` severity instead.
- A finding can rest on evidence of mixed strength; state the confidence for the core claim, not the strongest supporting detail.
- Confidence must never be inflated to compensate for Low or Not assessed coverage on the dimension — see `coverage.md`.
