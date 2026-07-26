# Security Department Prompt

Adapted from [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain) `prompts/security-audit.md` (MIT). Rewritten for Code Intel Pipeline: evidence comes from pipeline modalities plus targeted reads, and output is the fail-closed `code-intel-audit-report.v1` contract instead of a prose report.

This prompt is an operating instruction for an agent running the `security` audit department against a target repository. Read `docs/audit-report.md` (finding contract, fail-closed rules) and the rubrics in `orchestration/audit/rubrics/` before producing output.

## Inputs

- The target repository checkout (`--repo` root).
- Pipeline evidence for that repository when available, in this priority order:
  - `xray` (rg file inventory + text signals) — cheapest sweep surface.
  - `ct` (Sentrux hotspots, complexity, coupling) — ordering: audit hot, tangled files first.
  - `anatomy` (Understand Anything graph) — entry points and trust boundaries.
  - `chart` (Repowise semantic memory) — project background; use it to kill false positives, never to manufacture findings.
- If a modality is missing, proceed without it and say so in the coverage matrix `exclusions` — never guess what it would have said.

## Threat model first

Before sweeping, write down (for yourself) what the target actually is, because it decides which findings are real:

- Network service / web app: full FMSM surface applies (authn/authz, injection, SSRF, CSRF, transport).
- Local CLI or pipeline tool: the primary attackers are (1) untrusted repositories and files the tool parses, (2) untrusted process output the tool consumes, (3) the environment (keys, tokens) the tool can leak into artifacts, logs, or committed files. HTTPS/CSP/session findings are usually noise here.
- Library: its callers are the boundary; audit exported-surface input handling.

Do not report findings the target's threat model cannot exercise (FMSM "Focus" rule). A local-only tool with no listener does not get a TLS finding.

## Audit areas

1. **Untrusted input handling** — command/SQL/template injection, path traversal in file operations (joins with `..`, symlink following, archive extraction), deserialization of untrusted data, SSRF in any URL fetching.
2. **Authentication & authorization** — identity checks, privilege boundaries, hardcoded credentials or tokens, session/token lifecycle. Mark Not applicable in your notes when the target has no such surface.
3. **Secrets & configuration** — hardcoded secrets in code/tests/config, secrets read from env and written into logs, error messages, artifacts, or version control; unsafe default configuration.
4. **Process & filesystem effects** — child process invocation with attacker-influenced arguments, world-writable or predictable temp paths, file writes outside declared output roots.
5. **Operational leakage** — sensitive data in logs or debug output, error handling that leaks internal state.

Dependency and CI provenance risks belong to the `supply-chain` department — do not duplicate them here; note the handoff in `exclusions` if you spot one in passing.

## Method

1. Build the sweep from evidence: rg signal patterns (secret-shaped strings, `Command`/`spawn`/`exec`, path joins near user input, URL fetches, `unsafe`), Sentrux hotspot ranking, graph entry points.
2. A tool hit is a lead, never a finding. Every finding requires a targeted read of the code that confirms the behavior.
3. Trace at least one realistic attack path per finding: precondition → attacker input → mechanism → impact. Encode it in the finding's `failure_scenario`. No realistic path, no finding — park it as `suspected` only if the precondition is plausible but unverified.
4. Separate `confirmed` (behavior directly observed in code, file evidence with path required by the kernel) from `suspected` (pattern-inferred). Be conservative with confidence per `rubrics/confidence.md`.
5. Score per `rubrics/scoring.md`: a clean sweep may score 10.0 only with `high` coverage; state the strongest evidence in the justification.
6. Fill the coverage matrix honestly: what was swept, what was read, what was excluded and why. Absence of findings under `low` coverage is weak evidence — say so.

## Secrets red line

If you discover a real secret (key, token, password, connection string):

- Set `redacted: true` on the finding, and never place the secret value in the report, evidence excerpts, logs, or conversation output.
- Identify it by path, variable/key name, secret type, and blast radius; recommend rotation when exposure is plausible.

## Incremental runs

When this run is scoped to a diff, get the scope block first — do not hand-roll the changed-file list:

```bash
code-intel audit --operation scope --repo <target-root> --since <git-ref>
```

Embed the printed block verbatim as the report's top-level `scope` field. Restrict every finding to evidence within `scope.files` — the kernel fails closed on a finding outside the declared diff. Name the diff limitation in every coverage row's `exclusions` (e.g. "incremental run scoped to N changed files; the rest of the tree was not swept this pass").

## Output contract

Produce one `code-intel-audit-report.v1` JSON document (see `orchestration/schemas/code-intel-audit-report.v1.schema.json`):

- `departments` must list **every** registered department (kernel rule: exact registry membership). Run `security` as `assessed` (or `not_assessed` with an applicability reason if the target is out of scope); report departments whose registry entry is `enabled: false` as `disabled`.
- Findings follow the finding contract: id (`security-001`, `security-002`, …), severity/confidence/status per rubrics, evidence refs naming the modality or file (with `line_start`/`line_end` for reads), problem, failure scenario, minimal fix, regression test, estimated effort, redacted flag.
- Score dashboard and coverage matrix rows for every department you list; `assessed` requires a non-null score and non-`not_assessed` coverage.

Then validate fail-closed and fix until green:

```bash
code-intel audit --operation validate --repo <target-root> --report <report-path>
```

`--operation render` prints the human-readable audit section for hospital.md if you need to eyeball the result.
