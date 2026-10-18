# DR-0018: Delete all PowerShell; no compatibility retention or retirement windows

Status: active
Date: 2026-10-18
Decision: All PowerShell (`.ps1`/`.psm1`) is removed from the repository in two phases. Phase 1 deletes everything that is not on the install/release path. Phase 2 ports the installer and packaging to Rust (#401) and then deletes the rest. Compatibility facades, PowerShell contract tests, parity comparisons, rollback fixtures and retirement packets are deleted rather than kept or retired through observation windows.

## Authority and scope

The owner decided this on 2026-10-18 ("我想把powshell删了", choosing "全删，分两步"). It applies the owner's standing rule that backward compatibility is not a goal and deprecated paths are deleted rather than kept behind compatibility layers, fallbacks or migrations.

This supersedes, for PowerShell:
- the E02/E03/E04/E07/E08 retirement-gate process (`compatibility_retirement_gate.rs`, `docs/compatibility-retirement-gate.md`, the per-branch `docs/compatibility-retire-*.md`, #323): usage-observation windows, rollback rehearsal and independent-approval evidence are no longer prerequisites for deleting PowerShell;
- DR-0006, which only refined that window;
- the AGENTS.md rules to keep `.ps1` as compatibility surfaces with tested forwarding shims, and not to delete historical rollback fixtures in the same change.

It does not relax behavioral coverage. When a PowerShell test is the only check of a behavior that still exists in Rust, that behavior gets a Rust test before the PowerShell test is deleted. Tests that only check PowerShell code go away with it.

## Why

- **Scale.** 106 PowerShell files remain, with about 112 references from CI. That keeps a second runtime, a second test suite and a parity harness alive, even though the production path has been the compiled Rust CLI for months.
- **The retirement process stalled.** All five retirement packets have been `blocked` since #323 (2026-08). #78 (CI call sites to zero) had no progress. The gates cost more than the code they protect.
- **The Windows dependency is the bottleneck.** Every PowerShell CI step needs pwsh. The Windows lane now sits right at its timeout (#440, raised to 45 minutes).

Counterfactual: keeping the process means waiting on telemetry windows and approval evidence for code with no production caller, while every new Rust change must keep PowerShell parity tests green.

## Order (must hold)

1. **Phase 1:** delete PowerShell outside the install/release closure. Each PR leaves CI green. Rust production code that still executes or anchors to PowerShell is changed in the same PR. Capabilities whose `productionRegistry.callSite` points into `.ps1` are re-anchored in Rust or deleted.
2. **Phase 2:** Rust `install`/`repair` and packaging (#401) ship and pass the DR-0001 install-smoke gate. The installer, packaging scripts, sentrux shim and `legacy/` directory are then deleted in one change.

## Enforcement

- **During phase 1:** review convention, tracked in the removal map issue.
- **After phase 2:** `test_repository_layout.py` asserts that no tracked `*.ps1`/`*.psm1` exists.
