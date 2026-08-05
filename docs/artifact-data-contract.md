# Artifact Data Contract

Code Intel artifact files are the handoff surface between the scanner, CLI consumers, humans, and downstream agents. Treat these fields as product contracts, not incidental script output.

## Authority

The compiled `code-intel` execution kernel is the authority for fresh artifact
runs and publication. Compatibility adapters may collect evidence, but they do
not own Run Commit or index admission.

Agent Goal Intake is an upstream product boundary. It may shape the operator's goal before a scan starts, but it must not produce, mutate, or reinterpret artifact-run files after scanner execution.

An authoritative artifact run is one final-name directory for one target repository:

```text
<artifact-root>\<repo-name>\<final-name>\
```

Do not hand-edit artifact runs. Regenerate them with the compiled CLI, for example
`code-intel run execute --repo <repo-root> --out <staging-root> --authority-root <artifact-root> --final-name <name>`.

## Files

Machine-authoritative compiled-run files:

- `run-complete.json`: final `code-intel-run-commit.v1` marker. It binds the
  content-addressed terminal manifest, run identity, and snapshot identity; it is written last.
- the content-addressed `code-intel-run-manifest.v1` object referenced by the marker;
- exactly one `repository.iteration` Artifact Ref with a
  `code-intel-repository-iteration-provenance.v1` payload bound to the same run and snapshot plus
  the repository key and publication name;
- the manifest's verified, snapshot-bound Artifact Refs.

Legacy compatibility reports are advisory files, not repository-run authority:

- `report.json`: scanner execution summary, step outcomes, raw and effective failure categories, artifact paths, and compact summaries.
- `repomix-summary.json`: Repomix package metadata for the single-file AI context pack.
- `sentrux-failures.json`: normalized Sentrux check/gate failures. `sentrux check` and `sentrux gate` stdout are authoritative; hotspots and file-details are enrichment only.
- `sentrux-debt-register.json`: Sentrux failure disposition layer. It classifies normalized failures as `known_debt`, `new_debt`, `worsened_debt`, or `informational`; only `new_debt` and `worsened_debt` are blocking.
- `hospital-report.json`: diagnosis, disposition, state machine, next protocol, discharge criteria, and report-quality dimensions. Its optional `audit` block points at `audit-report.json` when an audit ran.
- `audit-report.json`: audit department findings, score dashboard, and coverage matrix (schema `code-intel-audit-report.v1`). See `docs/audit-report.md`.
- `surgery-plan.json`: first bounded repair target, operating plan, verification commands, and discharge criteria.
- `github-solution-research.json`: GitHub evidence candidates when external solution research is required.
- `greenfield-manifest.json`: optional Greenfield behavioral spec extraction state, generated prompt, workspace paths, and expected output locations.

Human/agent entry points:

- `summary.md`
- `understanding.md`
- `hospital.md`
- `surgery-plan.md`
- `github-solution-research.md`
- `greenfield-plan.md`

Tool evidence:

- `repomix-output.md`, `repomix-output.xml`, `repomix-output.json`, `repomix-output.txt`
- `sentrux-dsm.json`, `sentrux-file-details.json`, `sentrux-hotspots.json`, `sentrux-evolution.json`, `sentrux-what-if.json`
- `codenexus-context.json`
- Optional `structured-edit-plan.json` using `code-intel-structured-edit-plan.v1`. It is a
  snapshot-bound ast-grep structural search/rewrite preview. It has no repository-mutation
  authority; applying a rewrite remains a separate explicitly authorized operation.
- Optional `session-evidence.json` using `code-intel-session-evidence.v1`. It is a privacy-reduced,
  snapshot-bound session-review artifact with advisory-only authority; raw traces, prompts, event
  summaries, user-message marks, absolute paths, and outside-path values are not published.
- Optional `competitive-intelligence-request.json`, `competitive-intelligence-prompt.md`, and `competitive-score.json`. These are advisory market/product intelligence from the external `compete` workflow; they have no hospital, gate, or discharge authority.
- Repowise Understand Anything outputs referenced `report.json`
- Greenfield workspace outputs, when generated: `greenfield-workspace/output/specs`, `greenfield-workspace/output/test-vectors`, `greenfield-workspace/output/validation`, `greenfield-workspace/provenance`

## Native Code Evidence Canonical Order

The array order in native code-evidence artifacts is not semantic. Producers
and parity checks use one canonical order so filesystem traversal order cannot
change an otherwise equivalent result:

- `files` and `ranking.files`: ascending by normalized `path`;
- `symbols`: ascending by `file`, `startLine`, `kind`, then `name`;
- `chunks`: ascending by `file`, `startLine`, `endLine`, then `id`;
- symbol-to-chunk mappings: ascending by `symbolId`, then `chunkId`;
- `imports`: ascending by `file`, `line`, then `target`.

Only those array permutations are normalized. Ranking `score` and `reasons`,
field values, and the cardinality shape of each value (`null`, scalar, or
array) remain semantic and must match exactly.

## Transactional Publication Contracts

The compiled `code-intel run execute` path publishes through the Rust Run Commit boundary. It
restages verified Artifact Refs into content-addressed objects, binds the manifest, and writes the
canonical `run-complete.json` marker last. The completed-only index revalidates the marker,
manifest, refs, and exactly one repository-iteration provenance payload against the authority
directory names.

The legacy PowerShell compatibility report path uses a staging directory named
`<timestamp>.staging-<nonce>`. The scanner writes artifacts there, rewrites
published path references, promotes the directory to its final timestamped
name, but does not write a canonical marker. Its `report.json`, markdown, and auxiliary files are
a non-authoritative compatibility view. The normal index must reject those directories; explicit
legacy compatibility traversal may read them only as advisory reports.

The provenance contract is non-cryptographic. A local actor able to coherently replace a whole
publication can forge the JSON and digests; defending against that actor requires signed
attestations or an external trust root.

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

`report.summary.failed` and `report.summary.failureCategories.sentruxFail` preserve raw tool state. `report.summary.effectiveFailed`, `report.summary.effectiveFailureCategories`, `report.summary.blockingSentruxDebt`, `report.summary.knownSentruxDebt`, and `report.summary.gateFindings` are the process-decision counters.

Debt register producers must not invent symbols from aggregate-only stdout. When enrichment conflicts with stdout, `metric_conflict` stays on `sentrux-failures.json`; the register classifies the authoritative normalized record and preserves stdout-wins semantics.

Rust CLI ownership starts with the pure Sentrux contract logic:

```text
code-intel sentrux-normalize --steps report.json --out sentrux-failures.json
code-intel sentrux-debt-register --failures sentrux-failures.json --repo <repo> --out sentrux-debt-register.json
```

Shell wrappers may still orchestrate tools, but normalized Sentrux JSON and debt classification should converge on these CLI commands.

## Exit Code Contract (issue #130)

`legacy/run-code-intel.ps1` (and the legacy `check-code-intel-tools.ps1`/`code-intel.ps1`/`invoke-code-intel.ps1` compatibility chain, which forwards its exit code unchanged) uses a three-value exit code, not a binary one:

| Exit | Meaning |
|---|---|
| `0` | Clean: no failures, no gate findings. |
| `1` | The pipeline genuinely did not complete: a tool crash, missing artifacts, an unrecoverable step, or a `sentrux gate` step that failed without producing any parseable `gate:*` output at all. |
| `2` | The pipeline completed. `sentrux gate` reported architecture/quality debt (`god_files`, `coupling`, `quality`, `cycles`, ...) for the target repo. This is gate **output**, not a process failure. |

Before this fix, any blocking `sentrux gate` finding was folded into the same `effectiveFailed`/exit-1 path as a genuine crash: a repo with real architecture debt made the pipeline look like it had failed to run at all, even though every artifact was produced. The discriminator is `sentrux-failures.json`'s `gate` field (from `New-CodeIntelSentruxFailures`): non-null means a `gate:*` record was actually parsed from `sentrux gate` stdout (a finding); null means the step failed without producing any recognizable output (a genuine crash), which still counts as an effective failure.

This exemption is scoped to `sentrux gate` only. `sentrux check` (`check:*`, max_cc) keeps its prior blocking-debt-gated behavior.

`report.summary.gateFindings` is the count of blocking `sentrux gate`-sourced debt entries. It is informational — it does not raise `effectiveFailed`, `effectiveFailureCategories.sentruxFail`, or the process exit code past `2`. Consumers that read the legacy exit code directly must treat `2` the same as success unless they specifically want to gate on any Sentrux architecture/quality finding.

## Hospital Trust Contract

Hospital state and scoring are fail-closed:

- `passed` is the only passing gate/check status;
- missing, skipped, not-run, unknown, or failed evidence required for discharge cannot produce `green` or `discharge_ready`;
- a surgery target is resolved only when both the selected target and current hotspot are non-empty and different;
- unknown import resolution, source coverage, or pollution isolation has status `unknown` and score `0`.

Consumers must preserve the status alongside each score. A numeric score without its evidence status is not sufficient authority for routing or discharge.

## Scoped Repowise Egress Contract

Scoped Repowise uses a checked-out Git HEAD as its default input. Dirty, untracked, and ignored working-tree content is excluded unless the caller explicitly selects the working-tree overlay.

Before any provider process starts, the wrapper writes schema v2 `.repowise/egress-manifest.json` in the shadow worktree. The pending manifest records the selected HEAD, normalized scope paths, `scope_inventory` file hashes, effective provider, and working-tree policy. After the real traverser selects supported files, the Python boundary atomically freezes `provider_payload`; its paths and hashes must exactly match the in-memory bytes passed to the provider. Provider startup is blocked unless the frozen manifest validates.

Scope inputs must be repository-relative after normalization. Rooted paths, parent traversal, and symlink/reparse targets outside the repository boundary are rejected before shadow preparation or provider invocation.

## Required Routing Fields

Artifact consumers must preserve these fields:

- `report.summary.failed`
- `report.summary.effectiveFailed`
- `report.summary.manualRequired`
- `report.summary.failureCategories.providerQuota`
- `report.summary.failureCategories.providerUnavailable`
- `report.summary.failureCategories.configError`
- `report.summary.failureCategories.localToolError`
- `report.summary.failureCategories.graphMissing`
- `report.summary.failureCategories.sentruxFail`
- `report.summary.effectiveFailureCategories`
- `report.summary.blockingSentruxDebt`
- `report.summary.knownSentruxDebt`
- `report.summary.gateFindings`
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
