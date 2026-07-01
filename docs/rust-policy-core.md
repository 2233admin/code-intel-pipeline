# Rust Policy Core

`code-intel classify --report report.json --json` is the first Rust-owned policy
entry point.

It reads existing pipeline artifacts. It does not run scanner tools.

## Inputs

- `report.summary.failureCategories`: raw tool state.
- `report.summary.effectiveFailureCategories`: process-decision state.
- `report.summary.blockingSentruxDebt`: blocking structural debt count.
- `report.summary.knownSentruxDebt`: historical structural debt count.
- `report.hospital.nextProtocol` or `report.hospital.next_protocol`, when present.

## Outputs

- `failureCategories`: raw counters preserved from `report.json`.
- `effectiveFailureCategories`: counters used for process decisions.
- `blockingSentruxDebt`
- `knownSentruxDebt`
- `knownDebtOnly`
- `pipelineBlocking`
- `githubResearchRequired`
- `nextProtocol`
- `exitCode`

## Policy

Known Sentrux debt is visible but not blocking:

- raw `failureCategories.sentruxFail > 0`
- effective `effectiveFailureCategories.sentruxFail == 0`
- `blockingSentruxDebt == 0`
- `knownSentruxDebt > 0`

New or worsened Sentrux debt remains blocking:

- effective `effectiveFailureCategories.sentruxFail > 0`
- `pipelineBlocking == true`
- `githubResearchRequired == true`
- `exitCode == 1`

Graph missing remains a manual understanding gap, not a GitHub research trigger.
