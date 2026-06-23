# Harness Factory Reference

Harness Factory Reference is the future packaging and distribution layer for Code Intel Pipeline.

Tools such as MetaHarness are useful references when Code Intel needs to ship as a branded, repeatable Agent harness instead of a loose tool directory. This is a distribution concern, not a scanner concern.

## What It Can Inform

- a branded `npx code-intel` or organization-specific CLI
- Codex, Claude Code, GitHub Actions, and other host configuration bundles
- `doctor`, `validate`, `score`, `audit`, `sbom`, and release-gate command shapes
- SBOM, provenance, witness signing, and release verification
- static repository analysis that does not execute target code
- package layout for sharing the same Agent harness across a team

## Boundary

Do not make MetaHarness or any harness factory a runtime dependency of the scanner.

The scanner produces fresh repository evidence. Artifact consumers read existing evidence. Agent Goal Intake frames the task contract. A harness factory packages these surfaces for distribution after their contracts are stable.

## When To Revisit

Revisit this layer when Code Intel needs one of these outcomes:

- a public or internal `npx` distribution
- versioned org-wide Agent setup
- host-specific generated config for multiple Agent runtimes
- release provenance beyond GitHub Actions artifacts
- signed bundles or SBOM requirements

Until then, keep the implementation local and contract-first.
