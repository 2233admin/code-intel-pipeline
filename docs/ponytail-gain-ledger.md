# Ponytail Gain Ledger

This ledger tracks deferred minimalism gains so "later" has a concrete next harvest. A gain is a reversible deletion, demotion, or scope reduction that makes Code Intel Pipeline thinner without weakening evidence, safety, or artifact contracts.

Status meanings:

- `open`: identified, not harvested yet
- `harvest-next`: smallest useful cleanup candidate
- `blocked`: needs upstream behavior or explicit product decision first
- `keep`: audited and intentionally retained

## Open Gains

| ID | Status | Gain | Evidence | First harvest |
| --- | --- | --- | --- | --- |
| PG-001 | harvest-next | Remove tracked local repo aliases from `pipeline.config.json`. | `pipeline.config.json` contains machine-local `D:\projects\opencli-admin` aliases while `pipeline.config.example.json` already exists. | Delete tracked `pipeline.config.json`; keep real aliases in ignored `pipeline.config.local.json`; update docs only if a command assumes the tracked file. |
| PG-002 | harvest-next | Remove target-repo business taxonomy from generic Sentrux module naming. | `Invoke-SentruxAgentTool.ps1` `Get-ModuleBucket` contains `strategy`, `trading`, `brokers`, `markets`, `okx`, `ashare`, `crypto`, `sentiment`, and `narrative` buckets. | Fall back to generic path/stem grouping; move special buckets to config only if a governed repo proves it needs them. |
| PG-003 | open | Demote `crates/code-nexus-lite` until the iii worker is actually shipped. | Main pipeline still uses local PowerShell CodeNexus context; the Rust crate adds `iii-sdk`, HTTP triggers, and a separate runtime surface. | Remove it from workspace members or move it to an incubator path; keep the artifact contract in PowerShell until distribution requires the worker. |
| PG-004 | harvest-next | Delete nested `crates/code-nexus-lite/Cargo.lock`. | Workspace root already tracks `Cargo.lock`. | Remove the nested lock if `code-nexus-lite` stays in the workspace. |
| PG-005 | closed | Slim `skills/code-intel-pipeline/SKILL.md` back to the hot path. | The Skill now keeps installation, run order, failure categories, and report order in the hot path while linking detailed contracts from the installed release. | Keep project-specific examples in docs or runbooks. |
| PG-006 | open | Split or slim `Invoke-SentruxAgentTool.ps1`. | Sentrux artifact reports 2701 lines, 77 functions, `Get-ModuleBucket` cc=86, `Invoke-DsmTool` cc=44. | Harvest PG-002 first; then split only if complexity remains concentrated. Candidate seams: `dsm`, `git signals`, `what_if`. |
| PG-007 | blocked | Remove bundled Sentrux lite core fallback. | `tools/sentrux-shim/sentrux-lite-core.ps1` duplicates enough Sentrux behavior to keep new machines closed-loop. | Keep until upstream `sentrux.exe` install and Windows plugin behavior are reliable in CI and teammate setup. |
| PG-008 | open | Collapse repeated README/skill/architecture operational prose. | README, `skills/code-intel-pipeline/SKILL.md`, and `docs/code-intel-architecture.md` still overlap on the stable run and report order. | Make README narrative, Skill hot-path commands, architecture boundaries; delete duplicate command blocks elsewhere. |
| PG-009 | harvest-next | Keep Ponytail benchmark gate additions minimal. | Current benchmark test was table-driven to avoid Sentrux complexity regression, but it expanded the file more than the concept needs. | Keep the new assertions, but do not add more gates unless a published contract depends on them. Prefer one array per concept over per-phrase blocks. |
| PG-010 | blocked | Retire one compatibility branch only after E00 approval. | `compatibility.retirement-gate` projects a content-bound `approved-for-ticket` or `blocked` item from replacement, parity, registry, window, rollback execution, usage, C00, dependency, and independent-review evidence. | Open one E01 retirement ticket for the named branch; the gate itself has no deletion authority. |
| PG-011 | blocked | Retire `run-code-intel.codenexus-lite.direct` after the normal path is routed through B04 and unavailable mode selects B05. | E04's packet proves full/lite/unavailable contract fixtures, provider-owned process/storage effects, B05's `structuralVerdict=unknown` boundary, and exact rollback replay; the current normal path still contains one direct `Invoke-CodeNexusLite.ps1` block. | Substitute the single call path under a separately reviewed change, complete the 30-day usage window and independent E00 approval, then execute the one-file deletion ticket; until then `deletionExecuted=false` and `retired=false`. |
| PG-012 | blocked | Retire `run-code-intel.native-code.embedded` after normal/full execute B08 through A09. | B08 preserves normalized v1 artifacts, emits eight snapshot-bound Artifact Refs, declares `repo_read`/`local_write`, and reports unsupported relationship precision as unknown; the facade still contains the embedded function family and direct call. | Route normal/full through A09, complete the 30-day observation window and independent E00 approval, then execute the two-segment one-file deletion ticket; until then `deletionExecuted=false` and `retired=false`. |
| PG-013 | blocked | Retire `run-code-intel.hospital.embedded-diagnosis-render` after the normal facade executes B09. | B09 proves fail-closed precedence, rebuildable Markdown views, A09-seeded A01 execution, and stable machine parity against the legacy facade on the same untrusted authoritative fixture; the normal facade still owns one embedded function block and one direct invocation block. | Route the normal facade through `diagnosis.hospital`, complete the 30-day usage window and independent E00 approval, then execute the two-hunk one-file deletion ticket; until then `deletionExecuted=false` and `retired=false`. |
| PG-014 | blocked | Retire `update-code-intel-index.legacy-compatibility-traversal` after E05 and E00 approval. | Normal public refresh already routes through A08 with rebuild/incremental parity and valid-A07-only admission; explicit legacy compatibility traversal remains publicly reachable and produces diagnostic, non-authoritative output. | Keep the legacy branch until E05, the observation window, usage evidence, and independent approval pass; then execute the one-hunk one-file deletion ticket. Until then `deletionExecuted=false` and `retired=false`. |
| PG-015 | blocked | Retire `invoke-code-intel.doctor.direct-production` after public preflight routes through B10. | B10 proves one-result envelope behavior for manifest drift and present-but-nonconforming providers, secret redaction, and readiness/conformance separation; `invoke-code-intel.ps1` still invokes the retained PowerShell bootstrap directly, and that observation-only bootstrap has no declared expiry. | Keep `check-code-intel-tools.ps1`; route public preflight through `doctor`, declare an owned expiry/removal criterion for the non-authoritative bootstrap, complete the 30-day usage window and independent E00 approval, then execute only the three-hunk one-file deletion ticket. Until then `deletionExecuted=false` and `retired=false`. |

## Retained After Audit

| ID | Status | Item | Reason |
| --- | --- | --- | --- |
| PK-001 | keep | `tools/sentrux-shim` | Grounded compatibility layer: installer, CI, and fresh machines need a stable `sentrux` command path. |
| PK-002 | keep | Sentrux V overlay | Keep until upstream Windows `vlang` plugin package is known complete and tested. |
| PK-003 | keep | Hospital/artifact protocol | Product layer turns scanner evidence into deterministic next protocol; slimming is allowed, deletion is not currently justified. |

## Harvest Rule

When touching any file named in an `open` gain, spend the first pass on the smallest adjacent deletion from this ledger before adding behavior. If the deletion is not safe, update that row with the blocker instead of leaving it implicit.
