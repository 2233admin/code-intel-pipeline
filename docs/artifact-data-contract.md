# Artifact Data Contract

Code Intel artifact data is the handoff surface between the scanner, the Resume CLI, humans, and downstream Agents. Treat artifact fields as product contracts, not incidental script output.

Agent Goal Intake is upstream of this contract. It can refer to artifact files as required evidence, but it does not produce or mutate artifact runs.

## Authority

`run-code-intel.ps1` is the scanner and the only producer of fresh artifact runs. It gathers current repository evidence and writes the artifact files.

`code-intel resume`, `code-intel classify`, and `code-intel doctor` are artifact consumers. They read or check existing artifact runs; they do not produce fresh repository evidence and do not replace the scanner.

`update-code-intel-index.ps1` derives cross-run index data from existing artifact runs.

## Artifact Run

An artifact run is one timestamped directory for one target repository:

```text
<artifact-root>\<repo-name>\<timestamp>\
```

Do not hand-edit artifact runs. Regenerate them with `invoke-code-intel.ps1` or `run-code-intel.ps1`.

## Files

Machine-authoritative files:

- `report.json`: scanner execution summary, step outcomes, failure categories, artifact paths, and compact summaries.
- `hospital-report.json`: diagnosis, disposition, state machine, next protocol, discharge criteria, and report-quality dimensions.
- `surgery-plan.json`: first bounded repair target, operating plan, verification commands, and discharge criteria.
- `github-solution-research.json`: GitHub evidence candidates or the recorded reason evidence must be gathered manually.

Human and Agent entry points:

- `summary.md`: first human-readable entry point for a completed run.
- `understanding.md`: Agent handoff for continuing work from the run.
- `hospital.md`: human-readable diagnosis.
- `surgery-plan.md`: human-readable first repair plan.
- `github-solution-research.md`: human-readable upstream-evidence follow-up when required.

Tool evidence:

- `sentrux-dsm.json`, `sentrux-file-details.json`, `sentrux-hotspots.json`, `sentrux-evolution.json`, `sentrux-what-if.json`
- `codenexus-context.json`
- Repowise and Understand Anything outputs referenced by the run report.

## Required Routing Fields

Artifact consumers must preserve these routing fields:

- `report.summary.failed`
- `report.summary.manualRequired`
- `report.summary.failureCategories.providerQuota`
- `report.summary.failureCategories.localToolError`
- `report.summary.failureCategories.graphMissing`
- `report.summary.failureCategories.sentruxFail`
- `report.githubResearch.status`
- `report.githubResearch.required`
- `report.githubResearch.path`
- `report.githubResearch.markdown`
- `hospital-report.json.triage.status`
- `hospital-report.json.triage.disposition`
- `hospital-report.json.triage.primary_diagnosis`
- `hospital-report.json.triage.next_protocol`
- `hospital-report.json.triage.research_status`
- `hospital-report.json.triage.research_required`
- `hospital-report.json.state_machine.current_state`

`next_protocol` is an enum-like routing value. Current values are:

- `triage`
- `diagnose`
- `govern`
- `github_solution_research`
- `surgery_plan`
- `post_op`

## Maintenance Rule

When adding, renaming, or changing artifact fields, update these surfaces together:

- PowerShell writer in `run-code-intel.ps1`
- GitHub research writer in `Invoke-GitHubSolutionResearch.ps1`, when research fields change
- Rust reader in `crates/code-intel-cli/src/main.rs`
- Cross-run indexer, when the field should be searchable across runs
- `README.md` artifact list
- `CONTEXT.md` when the field changes project language
- Contract tests and fixture cases

For quantitative-system repositories, preserve lineage: an Agent should be able to tell which target repository, target scope, artifact run, diagnosis, and next protocol produced the recommendation it is following.
