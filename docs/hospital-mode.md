# Code Intel Hospital Mode

Hospital mode is the product layer of Code Intel Pipeline. It turns raw tool output into a repeatable diagnosis and treatment workflow for humans and agents.

## Modalities

- `xray`: fast file inventory from `rg`. It sees the project surface and gives the cheapest first signal.
- `anatomy`: Understand Anything graph. It shows the structural map when the graph is available and fresh.
- `ct`: Sentrux DSM, hotspots, and file/function detail. It shows static structure, coupling, complexity, and risk.
- `mri`: CodeNexus-lite context. It narrows the next read to high-value files and references.
- `pet`: execution-risk proxy from Sentrux evolution, what-if, and test gaps. It is not a live runtime trace yet.
- `chart`: Repowise semantic memory. It carries project background and long-lived explanation when provider/index state allows it.
- `governance`: Sentrux rules, check, and gate. It enforces architecture decisions and session safety.

## Protocols

- `triage`: classify the run into provider quota, local tool error, graph missing, Sentrux failure, or clean.
- `diagnose`: produce `summary.md`, `hospital.md`, Sentrux artifacts, and CodeNexus context.
- `govern`: require rules plus gate/check before treating a scope as governed.
- `surgery_plan`: choose one hotspot, one boundary, and one verification command before editing.
- `post_op`: rerun the pipeline or `session_end` after Agent edits and compare score, signal, and rules.

## Disposition

Every hospital report carries `triage.disposition`.

- `admit`: keep the project in the hospital. Use this for missing graph, missing rules, Sentrux failures, local tool failures, or scheduled modernization debt.
- `observe`: the project can continue with explicit follow-up checks.
- `discharge_ready`: the project can leave the hospital after post-op verification. Do not mark discharge-ready merely because a scan completed.

## Artifacts

- `hospital.md`: human-facing diagnosis report.
- `hospital-report.json`: machine-facing report with `triage`, `modalities`, `report_quality`, `diagnosis`, `treatment`, and `protocols`.

The most important machine fields are:

- `triage.primary_diagnosis`
- `triage.disposition`
- `triage.overall_score`
- `triage.next_protocol`
- `triage.discharge_criteria`
- `report_quality.dimensions`
- `treatment.plan`

## Operating Rule

Do not confuse a clean scan with a healthy system. A scan is a measurement. A governed project has rules, a baseline, useful localization, and a post-op check after changes.
