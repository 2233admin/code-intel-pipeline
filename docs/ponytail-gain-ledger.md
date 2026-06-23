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
| PG-005 | open | Slim `skill/SKILL.md` back to the hot path. | Skill file repeats project examples, Sentrux shim details, V overlay details, and long scoped-repo examples. | Keep canonical files, command order, failure categories, and read order; move project-specific examples to docs or runbooks. |
| PG-006 | open | Split or slim `Invoke-SentruxAgentTool.ps1`. | Sentrux artifact reports 2701 lines, 77 functions, `Get-ModuleBucket` cc=86, `Invoke-DsmTool` cc=44. | Harvest PG-002 first; then split only if complexity remains concentrated. Candidate seams: `dsm`, `git signals`, `what_if`. |
| PG-007 | blocked | Remove bundled Sentrux lite core fallback. | `tools/sentrux-shim/sentrux-lite-core.ps1` duplicates enough Sentrux behavior to keep new machines closed-loop. | Keep until upstream `sentrux.exe` install and Windows plugin behavior are reliable in CI and teammate setup. |
| PG-008 | open | Collapse repeated README/skill/architecture operational prose. | README, `skill/SKILL.md`, and `docs/code-intel-architecture.md` all repeat install, doctor, normal run, Sentrux, Repowise, and hospital read order. | Make README narrative, skill hot-path commands, architecture boundaries; delete duplicate command blocks elsewhere. |
| PG-009 | harvest-next | Keep Ponytail benchmark gate additions minimal. | Current benchmark test was table-driven to avoid Sentrux complexity regression, but it expanded the file more than the concept needs. | Keep the new assertions, but do not add more gates unless a published contract depends on them. Prefer one array per concept over per-phrase blocks. |

## Retained After Audit

| ID | Status | Item | Reason |
| --- | --- | --- | --- |
| PK-001 | keep | `tools/sentrux-shim` | Grounded compatibility layer: installer, CI, and fresh machines need a stable `sentrux` command path. |
| PK-002 | keep | Sentrux V overlay | Keep until upstream Windows `vlang` plugin package is known complete and tested. |
| PK-003 | keep | Hospital/artifact protocol | Product layer turns scanner evidence into deterministic next protocol; slimming is allowed, deletion is not currently justified. |

## Harvest Rule

When touching any file named in an `open` gain, spend the first pass on the smallest adjacent deletion from this ledger before adding behavior. If the deletion is not safe, update that row with the blocker instead of leaving it implicit.
