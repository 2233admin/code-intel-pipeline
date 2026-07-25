# AI Safety Department Prompt

Adapted from [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain) `prompts/ai-safety-audit.md` (MIT). Rewritten for Code Intel Pipeline: evidence comes from pipeline modalities plus targeted reads, and output is the fail-closed `code-intel-audit-report.v1` contract.

This prompt is an operating instruction for an agent running the `ai-safety` audit department against a target repository. Read `docs/audit-report.md` and the rubrics in `orchestration/audit/rubrics/` before producing output.

## Applicability gate — run this first

This department is only meaningful when the target actually has an AI/LLM surface. Detect it before auditing:

- Provider SDK imports or HTTP calls to model endpoints.
- Prompt assets: `.md`/`.txt`/template files whose content is model instructions, or long string literals that read as instructions.
- Tool/function definitions exposed to a model: MCP servers, function-calling schemas, agent tool registries.
- Agent orchestration code: loops that feed model output back as input, or that dispatch on model-chosen actions.

Record what you find as the department's `applicability.surface_evidence`.

- **No surface found** → report the department as `not_assessed` with `applicable: "no"` and a reason naming what you searched. Do not invent findings, and do not score the department. This is the correct, honest outcome for a repository with no model integration.
- **Surface found** → set `applicable: "yes"`, list the surfaces, and audit them.

## Inputs

- The target repository checkout.
- Pipeline evidence when available: `xray` (locate provider imports, prompt assets, tool schemas), `anatomy` (paths from untrusted input to model call sites and from model output to effectful code), `governance` (existing Sentrux rules constraining tool boundaries).
- Missing modality: proceed and record the gap in `exclusions`.

## Audit areas

1. **Prompt and instruction boundaries** — untrusted content (repository files, retrieved documents, tool output, web pages) concatenated into the same channel as system or developer instructions, with no isolation or delimiting; templates that let retrieved text redefine policy.
2. **Tool and action authorization** — model output triggering file writes, network calls, process execution, or spend without a policy check between decision and effect; tool arguments trusted because a model produced them; irreversible or externally visible actions with no confirmation; scopes not least-privilege per task.
3. **Context and data leakage** — secrets or credentials reaching prompts, logs, caches, or persisted artifacts; retrieval crossing tenant, workspace, or permission boundaries; model responses cached across contexts that should not share them.
4. **Reliability and output validation** — model output parsed as if well-formed; silent provider/model fallback changing capability, cost, or safety behavior; no timeout, retry bound, or circuit breaker around model calls; model output used as fact where a deterministic check exists.
5. **Evaluation and abuse cost** — no regression corpus for injection, leakage, or tool-misuse behavior; no token budget, rate limit, or spend visibility; attacker-reachable paths that trigger expensive retrieval or recursive model/tool calls.

## Method

1. Enumerate the surface first (applicability gate) and write the inventory down — it is both the audit scope and the coverage evidence.
2. For each model call site, trace backwards: what is the most untrusted thing that can reach this prompt? For each effect site, trace backwards: can a model decision reach this effect without a deterministic gate?
3. A pattern match is a lead. Confirm the data path by reading the code before reporting.
4. Every finding names the boundary that is crossed (untrusted content → instruction channel; model output → effect; tenant A data → tenant B response) and encodes a realistic path in `failure_scenario`.
5. Separate `confirmed` from `suspected`; be conservative per `rubrics/confidence.md`.
6. Score per `rubrics/scoring.md`; state coverage honestly per `rubrics/coverage.md`.

## Secrets red line

Credentials found in prompts, provider configuration, logs, or artifacts are reported with `redacted: true`, identified by path and variable name only, never by value, with rotation advice when exposure is plausible.

## Focus

Do not spend output on:

- Model-quality or accuracy complaints that are not safety or cost boundaries.
- Injection findings where no untrusted content can reach the prompt.
- Speculative jailbreak scenarios with no path in this codebase.

## Incremental runs

When this run is scoped to a diff, get the scope block first — do not hand-roll the changed-file list:

```bash
code-intel audit --operation scope --repo <target-root> --since <git-ref>
```

Embed the printed block verbatim as the report's top-level `scope` field. Restrict every finding to evidence within `scope.files` — the kernel fails closed on a finding outside the declared diff. Name the diff limitation in every coverage row's `exclusions` (e.g. "incremental run scoped to N changed files; the rest of the tree was not swept this pass").

## Output contract

Produce one `code-intel-audit-report.v1` JSON document. `departments` must list **every** registered department (kernel rule: exact registry membership); report departments whose registry entry is `enabled: false` as `disabled`. Finding ids are `ai-safety-001`, `ai-safety-002`, … Every listed department needs a coverage row; an `assessed` department also needs a non-null score and non-`not_assessed` coverage.

Validate fail-closed and fix until green:

```bash
code-intel audit --operation validate --repo <target-root> --report <report-path>
```
