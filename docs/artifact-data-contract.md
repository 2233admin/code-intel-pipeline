# Artifact Data Contract

Code Intel artifact files are the handoff surface between the scanner, CLI consumers, humans, and downstream agents. Treat these fields as product contracts, not incidental script output.

## Authority

`run-code-intel.ps1` is the only producer of fresh artifact runs. `code-intel resume`, `code-intel classify`, `code-intel doctor`, and indexers consume existing runs; they do not replace scanner evidence.

An artifact run is one timestamped directory for one target repository:

```text
<artifact-root>\<repo-name>\<timestamp>\
```

Do not hand-edit artifact runs. Regenerate them with `invoke-code-intel.ps1` or `run-code-intel.ps1`.

## Files

Machine-authoritative files:

- `report.json`: scanner execution summary, step outcomes, raw and effective failure categories, artifact paths, and compact summaries.
- `repomix-summary.json`: Repomix package metadata for the single-file AI context pack.
- `sentrux-failures.json`: normalized Sentrux check/gate failures. `sentrux check` and `sentrux gate` stdout are authoritative; hotspots and file-details are enrichment only.
- `sentrux-debt-register.json`: Sentrux failure disposition layer. It classifies normalized failures as `known_debt`, `new_debt`, `worsened_debt`, or `informational`; only `new_debt` and `worsened_debt` are blocking.
- `hospital-report.json`: diagnosis, disposition, state machine, next protocol, discharge criteria, and report-quality dimensions.
- `surgery-plan.json`: first bounded repair target, operating plan, verification commands, and discharge criteria.
- `github-solution-research.json`: GitHub evidence candidates when external solution research is required.

Human/agent entry points:

- `summary.md`
- `understanding.md`
- `hospital.md`
- `surgery-plan.md`
- `github-solution-research.md`

Tool evidence:

- `repomix-output.md`, `repomix-output.xml`, `repomix-output.json`, `repomix-output.txt`
- `sentrux-dsm.json`, `sentrux-file-details.json`, `sentrux-hotspots.json`, `sentrux-evolution.json`, `sentrux-what-if.json`
- `codenexus-context.json`
- Repowise and Understand Anything outputs referenced by `report.json`

## Sentrux Failure Contract

`sentrux-failures.json` uses schema `code-intel-sentrux-failures.v1`.

Authority order:

- `sentrux check` stdout is authoritative for named max-cc offenders.
- `sentrux gate` stdout is authoritative for gate regressions and aggregate count changes.
- `sentrux-hotspots.json` and `sentrux-file-details.json` can enrich context but not replace the primary check/gate failure target.

Artifact-level `status` values are `ok`, `failed`, `partial`, `unparsed`, `manual_required`, `skipped`, and `not_run`.

Record `target.status` values are `resolved`, `unresolved`, `aggregate`, and `not_applicable`.

When enrichment conflicts with check/gate stdout, producers emit a `metric_conflict` entry with record ids, metric values, source names, raw pointers, bounded excerpts, parse timestamp, and `resolution = authoritative_stdout_wins`.

## Sentrux Debt Register Contract

`sentrux-debt-register.json` uses schema `code-intel-sentrux-debt-register.v1`.

`sentrux-failures.json` remains the normalized authority for what Sentrux reported. The debt register is policy classification only:

- `known_debt`: historical structural debt recorded in the current run; reported, not blocking understanding artifacts.
- `new_debt`: a structural failure not matched by known-debt policy; blocking.
- `worsened_debt`: a known or aggregate structural metric worsened in this run; blocking.
- `informational`: manual-required, skipped, unparsed, or aggregate-only output that lacks an authoritative target; reported only.

`report.summary.failed` and `report.summary.failureCategories.sentruxFail` preserve raw tool state. `report.summary.effectiveFailed`, `report.summary.effectiveFailureCategories`, `report.summary.blockingSentruxDebt`, and `report.summary.knownSentruxDebt` are the process-decision counters.

Debt register producers must not invent symbols from aggregate-only stdout. When enrichment conflicts with stdout, `metric_conflict` stays on `sentrux-failures.json`; the register classifies the authoritative normalized record and preserves stdout-wins semantics.

Rust CLI ownership starts with the pure Sentrux contract logic:

```text
code-intel sentrux-normalize --steps report.json --out sentrux-failures.json
code-intel sentrux-debt-register --failures sentrux-failures.json --repo <repo> --out sentrux-debt-register.json
```

Shell wrappers may still orchestrate tools, but normalized Sentrux JSON and debt classification should converge on these CLI commands.

## Required Routing Fields

Artifact consumers must preserve these fields:

- `report.summary.failed`
- `report.summary.effectiveFailed`
- `report.summary.manualRequired`
- `report.summary.failureCategories.providerQuota`
- `report.summary.failureCategories.localToolError`
- `report.summary.failureCategories.graphMissing`
- `report.summary.failureCategories.sentruxFail`
- `report.summary.effectiveFailureCategories`
- `report.summary.blockingSentruxDebt`
- `report.summary.knownSentruxDebt`
- `report.sentruxFailures.path`
- `report.sentruxDebtRegister.path`
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

When these fields change, update the PowerShell writer, GitHub research writer if relevant, CLI readers, cross-run indexers if searchable, README artifact lists, and contract tests together.
