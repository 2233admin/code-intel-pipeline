# Code Intel Audit Report

The audit layer runs audit dimensions as hospital departments. Each department consumes the modality evidence hospital mode already admits (`xray`, `anatomy`, `ct`, `mri`, `pet`, `chart`, `governance`, see `docs/hospital-mode.md`) plus any targeted reads it takes to resolve a finding, and produces findings, a score dashboard, and a coverage matrix in a shared, fail-closed contract. This is the audit kernel: the artifact contract, the finding contract, and the validation invariants every department's output must satisfy. It does not run departments itself — `orchestration/audit/departments.v1.json` currently registers three (`security`, `ai-safety`, `supply-chain`), all `enabled: false`.

Methodology adapted from [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain) (MIT): the rubrics in `orchestration/audit/rubrics/` and the finding contract below are rewritten for this repo's evidence-first context, not copied.

## Artifact Location

`audit-report.json` uses schema `code-intel-audit-report.v1` (`orchestration/schemas/code-intel-audit-report.v1.schema.json`). It is written into the run's artifact directory alongside `hospital-report.json` (see `docs/artifact-data-contract.md`), not nested under a capability-specific subdirectory.

`hospital-report.json` carries an optional `audit` block (`status`, `artifact` path, `overall` score, `findings_total`, `by_severity` counts) that points at the full `audit-report.json`; `hospital.md` renders the full score dashboard, coverage matrix, and top findings from that artifact when it is present, and renders nothing when it is absent.

## Finding Contract

Every finding in `audit-report.json.findings` is one object with these fields. Adapted from FMSM's issue-card template.

| Field | Type | Required | Meaning |
|-------|------|----------|---------|
| `id` | string, pattern `^[a-z0-9-]+-[0-9]{3}$` | yes | Stable id, e.g. `security-001`. |
| `department` | string | yes | Registered department id from `orchestration/audit/departments.v1.json`. |
| `title` | string | yes | Short summary. |
| `severity` | `Critical` / `High` / `Medium` / `Low` / `Info` | yes | See `orchestration/audit/rubrics/severity.md`. |
| `confidence` | `High` / `Medium` / `Low` | yes | See `orchestration/audit/rubrics/confidence.md`. |
| `status` | `Confirmed` / `Suspected` | yes | Whether the evidence directly confirms the finding. |
| `affected_area` | string | no | Module, component, or subsystem. |
| `evidence` | array of evidence refs, >= 1 entry | yes | See `orchestration/audit/rubrics/evidence.md`; each entry carries `kind` (`file` / `modality_signal` / `command` / `manual_read`), `source`, and optionally `path`, `line_start`, `line_end`, `modality`, `note`. |
| `problem` | string | yes | One-paragraph description of the issue. |
| `failure_scenario` | string | yes | A realistic, specific sequence of events that leads to failure. |
| `minimal_fix` | string | yes | The smallest change that removes the risk. |
| `long_term_fix` | string | no | An architectural improvement, if warranted. |
| `regression_test` | string | yes | A specific test that would catch this issue. |
| `estimated_effort` | `minutes` / `hours` / `days` | yes | Rough size of the fix. |
| `redacted` | boolean | yes | See below. |

### Secrets and redaction

Findings must never write secret material in plaintext. A finding about a leaked or hardcoded secret sets `redacted: true` and its evidence and `problem`/`failure_scenario` text reference only the file `path` and the variable/key name that holds the secret — never the secret value itself, not even truncated.

## Fail-Closed Rules

`crates/code-intel-cli/src/audit_report.rs` parses and validates every `audit-report.json`. Parsing itself enforces the JSON Schema contract (required fields, closed objects — no `additionalProperties`, enum values, the finding `id` pattern, the `evidence` minItems). `validate()` then enforces invariants the schema cannot express, each producing a distinct error:

1. Every finding has at least one evidence entry (schema-enforced), and a finding with `status: confirmed` has at least one `file`-kind evidence entry with a `path`.
2. Every `finding.department`, `score_dashboard` entry department, and `coverage_matrix` row department is a department id registered in `orchestration/audit/departments.v1.json`.
3. A department whose `status` is `not_assessed` or `disabled` has a null score in `score_dashboard` and a `not_assessed` row in `coverage_matrix`.
4. `score_dashboard.overall` is the mean of the non-null entry scores, rounded to 1 decimal place, or `null` when nothing scored; `validate()` recomputes it and rejects a mismatch.
5. A department that scores `10.0` with zero findings must have `coverage: high` — see `orchestration/audit/rubrics/scoring.md`.
6. Every department listed in the report has exactly one `coverage_matrix` row and at most one `score_dashboard` entry.

The registry itself (`orchestration/audit/departments.v1.json`) has its own invariants, checked by `DepartmentRegistry::validate()`: department ids are unique, every rubric file it points at exists on disk, and every `enabled: true` department's prompt file exists on disk. A disabled department may point at a prompt file that does not exist yet — that file is the department ticket's job, not the kernel's.

## Registering a Department

Adding a department never requires a kernel change:

1. Add an entry to `orchestration/audit/departments.v1.json`: `id`, `title`, `prompt` (path to the department's prompt file), `consumes` (which modalities it reads), `applicabilityCheck`, and `trackingIssue`. Leave `enabled: false` until the prompt is ready.
2. Write the prompt file at the path the entry declares.
3. Flip `enabled: true` once the prompt is ready to run. `DepartmentRegistry::validate()` will fail closed if the prompt file is still missing.

The kernel does not care how a department produces its `audit-report.json` — only that the result satisfies the finding contract and the fail-closed invariants above.
